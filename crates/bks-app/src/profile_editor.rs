use std::cell::RefCell;
use std::ffi::c_void;

use crate::accessibility::set_accessible_name;
use crate::dpi;
use crate::profile::{Profile, ProfileKind, ProfileManager, SshOptions};
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, FillRect, GetMonitorInfoW, InvalidateRect, MonitorFromWindow, SetBkColor,
    SetTextColor, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, EM_SETSEL, ODS_HOTLIGHT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_ESCAPE, VK_MENU,
    VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassInfoW, GetClientRect,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsChild, IsDialogMessageW, IsIconic,
    IsWindow, IsWindowVisible, LoadCursorW, LoadIconW, MessageBoxW, MoveWindow, RegisterClassW,
    SendMessageW, SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    BN_CLICKED, BS_OWNERDRAW, CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NUMBER, ES_READONLY, HMENU,
    IDC_ARROW, MB_ICONWARNING, MB_OK, MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW, WNDPROC, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

pub const ID_PROFILE: usize = 4100;
pub const ID_NAME: usize = 4101;
pub const ID_HOST: usize = 4102;
pub const ID_USER: usize = 4103;
pub const ID_PORT: usize = 4104;
pub const ID_IDENTITY: usize = 4105;
pub const ID_SAVE: usize = 4106;
pub const ID_CANCEL: usize = 4107;

/// Saved means the user confirmed valid settings; the caller persists them.
pub enum Outcome {
    Saved(Profile),
    Cancelled,
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    labels: [HWND; 6],
    fields: [HWND; 6],
    hint: HWND,
    save: HWND,
    cancel: HWND,
    dpi: u32,
    theme: Theme,
}

struct State {
    view: View,
    manager: ProfileManager,
    selected_id: Option<String>,
    editable: bool,
}

thread_local! {
    static EDITOR: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Outcome>> = const { RefCell::new(None) };
}

fn current() -> Option<View> {
    EDITOR.with(|editor| editor.borrow().as_ref().map(|state| state.view))
}

pub fn take_result() -> Option<Outcome> {
    OUTCOME.with(|result| result.borrow_mut().take())
}

pub fn show(owner: HWND, manager: &ProfileManager, selected_id: Option<&str>) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.fields[0]);
        return Ok(());
    }
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermSshProfileEditor"),
        w!("SSH-профили — BKS Term"),
        Some(window_proc),
        640,
        600,
    )?;
    let view = View {
        hwnd,
        owner,
        previous_focus: unsafe { GetFocus() },
        labels: [HWND::default(); 6],
        fields: [HWND::default(); 6],
        hint: HWND::default(),
        save: HWND::default(),
        cancel: HWND::default(),
        dpi,
        theme: Theme::new(dpi),
    };
    EDITOR.with(|editor| {
        *editor.borrow_mut() = Some(State {
            view,
            manager: manager.clone(),
            selected_id: None,
            editable: true,
        });
    });
    if let Err(error) = create_controls(view, instance, manager, selected_id) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    let view = current().expect("profile editor initialized");
    apply_fonts(view);
    layout(view);
    load_selection(view);
    activate(hwnd, view.fields[0]);
    Ok(())
}

/// Dispatch all messages belonging to the editor before application shortcuts.
pub fn handle_message(message: &MSG) -> bool {
    let Some(view) = current() else {
        return false;
    };
    if !owns_message(view.hwnd, message) {
        return false;
    }
    unsafe {
        if message.message == WM_KEYDOWN {
            let focus = GetFocus();
            let mut stops = view.fields.to_vec();
            stops.push(view.hint);
            if IsWindowEnabled(view.save).as_bool() {
                stops.push(view.save);
            }
            stops.push(view.cancel);
            if handle_tab(message.wParam.0, &stops) {
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize
                || (message.wParam.0 == VK_RETURN.0 as usize && focus == view.cancel)
            {
                close(view, Outcome::Cancelled);
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize
                && focus != view.fields[0]
                && focus != view.hint
            {
                save(view);
                return true;
            }
            if (view.fields[1..].contains(&focus) || focus == view.hint)
                && handle_select_all(message.wParam.0, focus)
            {
                return true;
            }
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

fn create_controls(
    view: View,
    instance: HINSTANCE,
    manager: &ProfileManager,
    selected: Option<&str>,
) -> Result<()> {
    let names = [
        "SSH-профиль",
        "Название профиля",
        "Адрес или алиас OpenSSH",
        "Пользователь (необязательно)",
        "Порт (пусто — конфигурация OpenSSH или 22)",
        "Путь к файлу ключа (необязательно)",
    ];
    let mut next = view;
    let ids = [ID_PROFILE, ID_NAME, ID_HOST, ID_USER, ID_PORT, ID_IDENTITY];
    for (index, name) in names.into_iter().enumerate() {
        next.labels[index] = child(
            view.hwnd,
            instance,
            w!("STATIC"),
            name,
            WINDOW_STYLE::default(),
            4200 + index,
        )?;
        next.fields[index] = child(
            view.hwnd,
            instance,
            if index == 0 {
                w!("COMBOBOX")
            } else {
                w!("EDIT")
            },
            "",
            WS_TABSTOP
                | if index == 0 {
                    WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32)
                } else {
                    WINDOW_STYLE((ES_AUTOHSCROLL | if index == 4 { ES_NUMBER } else { 0 }) as u32)
                },
            ids[index],
        )?;
        set_accessible_name(next.fields[index], name)?;
    }
    next.hint = child(view.hwnd, instance, w!("EDIT"),
        "Алиасы обрабатывает OpenSSH. Сохраняется только путь к ключу. Пароли и содержимое ключей не сохраняются.",
        WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32), 4206)?;
    set_accessible_name(next.hint, "Подсказка о профиле SSH и хранении ключей")?;
    next.save = child(
        view.hwnd,
        instance,
        w!("BUTTON"),
        "Сохранить",
        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        ID_SAVE,
    )?;
    next.cancel = child(
        view.hwnd,
        instance,
        w!("BUTTON"),
        "Отмена",
        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        ID_CANCEL,
    )?;
    set_accessible_name(next.save, "Сохранить SSH-профиль")?;
    set_accessible_name(next.cancel, "Отменить редактирование SSH-профиля")?;
    crate::control_state::install(next.save);
    crate::control_state::install(next.cancel);
    EDITOR.with(|editor| {
        if let Some(state) = editor.borrow_mut().as_mut() {
            state.view = next;
        }
    });
    unsafe {
        add_choice(next.fields[0], "Новый SSH-профиль");
        let mut selection = 0;
        for (index, profile) in manager
            .profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::Ssh)
            .enumerate()
        {
            add_choice(next.fields[0], &profile.name);
            if selected == Some(profile.id.as_str()) {
                selection = index + 1;
            }
        }
        SendMessageW(next.fields[0], CB_SETCURSEL, Some(WPARAM(selection)), None);
    }
    Ok(())
}

unsafe fn add_choice(combo: HWND, name: &str) {
    let name = wide(name);
    SendMessageW(
        combo,
        CB_ADDSTRING,
        None,
        Some(LPARAM(name.as_ptr() as isize)),
    );
}

fn load_selection(view: View) {
    let index = unsafe { SendMessageW(view.fields[0], CB_GETCURSEL, None, None).0 };
    let profile = EDITOR.with(|editor| {
        editor.borrow().as_ref().and_then(|state| {
            usize::try_from(index - 1).ok().and_then(|index| {
                state
                    .manager
                    .profiles
                    .iter()
                    .filter(|profile| profile.kind == ProfileKind::Ssh)
                    .nth(index)
                    .cloned()
            })
        })
    });
    let selected_id = profile.as_ref().map(|profile| profile.id.clone());
    let options = profile
        .as_ref()
        .map_or_else(|| Ok(SshOptions::default()), Profile::ssh_options);
    let editable = options.is_ok();
    let values = match &options {
        Ok(options) => [
            profile
                .as_ref()
                .map_or(String::new(), |profile| profile.name.clone()),
            options.host.clone(),
            options.user.clone().unwrap_or_default(),
            options
                .port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            options.identity_file.clone().unwrap_or_default(),
        ],
        Err(_) => [
            profile
                .as_ref()
                .map_or(String::new(), |profile| profile.name.clone()),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ],
    };
    EDITOR.with(|editor| {
        if let Some(state) = editor.borrow_mut().as_mut() {
            state.selected_id = selected_id;
            state.editable = editable;
        }
    });
    for (field, value) in view.fields[1..].iter().zip(values) {
        set_text(*field, &value);
    }
    unsafe {
        let _ = EnableWindow(view.save, editable);
    }
    if let Err(error) = options {
        show_validation(view.hwnd, &format!("{error}\r\nПрофиль не изменён. Выберите поддерживаемый профиль или создайте новый."));
    }
}

fn save(view: View) {
    let input: Vec<String> = view.fields[1..]
        .iter()
        .map(|field| control_text(*field))
        .collect();
    let result =
        SshOptions::parse(&input[1], &input[2], &input[3], &input[4]).and_then(|options| {
            EDITOR.with(|editor| {
                let editor = editor.borrow();
                let state = editor.as_ref().expect("profile editor open");
                if !state.editable {
                    return Err("Этот профиль содержит неподдерживаемые параметры SSH.".to_owned());
                }
                state
                    .manager
                    .make_ssh_profile(state.selected_id.as_deref(), &input[0], &options)
            })
        });
    match result {
        Ok(profile) => close(view, Outcome::Saved(profile)),
        Err(error) => show_validation(view.hwnd, &error),
    }
}

fn close(view: View, result: Outcome) {
    OUTCOME.with(|outcome| *outcome.borrow_mut() = Some(result));
    unsafe {
        let _ = DestroyWindow(view.hwnd);
    }
    restore_focus(view.owner, view.previous_focus);
}

fn apply_fonts(view: View) {
    let mut all = view.labels.to_vec();
    all.extend(view.fields);
    all.extend([view.hint, view.save, view.cancel]);
    let mut edits = view.fields.to_vec();
    edits.push(view.hint);
    apply_theme(view.hwnd, &view.theme, &all, &edits);
}

fn layout(view: View) {
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(view.hwnd, &mut rect).is_err() {
            return;
        }
        let margin = dpi::scale(16, view.dpi);
        let gap = dpi::scale(8, view.dpi);
        let width = (rect.right - 2 * margin).max(1);
        let button_h = dpi::scale(36, view.dpi);
        let button_y = (rect.bottom - margin - button_h).max(margin);
        let hint_h = dpi::scale(48, view.dpi);
        let hint_y = (button_y - gap - hint_h).max(margin);
        let row_h = ((hint_y - gap - margin) / 6).clamp(1, dpi::scale(62, view.dpi));
        for index in 0..6 {
            let y = margin + row_h * index as i32;
            let label_h = dpi::scale(20, view.dpi).min(row_h / 2);
            let field_y = y + label_h;
            let field_h = (row_h - label_h - dpi::scale(4, view.dpi))
                .max(1)
                .min(dpi::scale(30, view.dpi));
            let _ = MoveWindow(view.labels[index], margin, y, width, label_h, true);
            let _ = MoveWindow(
                view.fields[index],
                margin,
                field_y,
                width,
                if index == 0 {
                    dpi::scale(200, view.dpi)
                } else {
                    field_h
                },
                true,
            );
        }
        let _ = MoveWindow(view.hint, margin, hint_y, width, hint_h, true);
        let button_w = dpi::scale(120, view.dpi).min((width - gap).max(2) / 2);
        let _ = MoveWindow(
            view.save,
            rect.right - margin - 2 * button_w - gap,
            button_y,
            button_w,
            button_h,
            true,
        );
        let _ = MoveWindow(
            view.cancel,
            rect.right - margin - button_w,
            button_y,
            button_w,
            button_h,
            true,
        );
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(view) = current().filter(|view| view.hwnd == hwnd) else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_SETFOCUS => {
            let _ = SetFocus(Some(view.fields[0]));
            LRESULT(0)
        }
        WM_CLOSE => {
            close(view, Outcome::Cancelled);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            let code = (wparam.0 >> 16) as u32;
            if id == ID_PROFILE && code == CBN_SELCHANGE {
                load_selection(view);
            } else if id == ID_SAVE && code == BN_CLICKED {
                save(view);
            } else if id == ID_CANCEL && code == BN_CLICKED {
                close(view, Outcome::Cancelled);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            layout(view);
            LRESULT(0)
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            let dpi = if message == WM_DPICHANGED {
                ((wparam.0 & 0xffff) as u32).max(dpi::BASE_DPI)
            } else {
                view.dpi
            };
            let next = View {
                dpi,
                theme: Theme::new(dpi),
                ..view
            };
            EDITOR.with(|editor| {
                if let Some(state) = editor.borrow_mut().as_mut() {
                    state.view = next;
                }
            });
            apply_fonts(next);
            release_theme(view.theme);
            if message == WM_DPICHANGED {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            layout(next);
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = POINT {
                    x: dpi::scale(420, view.dpi),
                    y: dpi::scale(460, view.dpi),
                };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => erase_background(hwnd, wparam, &view.theme),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            control_color(wparam, &view.theme, message == WM_CTLCOLOREDIT)
        }
        WM_DRAWITEM if lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if crate::control_state::is_hot(item.hwndItem) {
                item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            draw_mode_button(
                &item,
                &view.theme,
                if wparam.0 == ID_SAVE {
                    "Сохранить"
                } else {
                    "Отмена"
                },
            );
            LRESULT(1)
        }
        WM_NCDESTROY => {
            EDITOR.with(|editor| *editor.borrow_mut() = None);
            release_theme(view.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

// Shared native-dialog helpers; none of these read password controls.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

fn control_text(hwnd: HWND) -> String {
    unsafe {
        let mut text = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let count = GetWindowTextW(hwnd, &mut text).max(0) as usize;
        String::from_utf16_lossy(&text[..count])
    }
}

pub(crate) fn child(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
) -> Result<HWND> {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut c_void)),
            Some(instance),
            None,
        )
    }
}

pub(crate) fn create_dialog(
    owner: HWND,
    class: PCWSTR,
    title: PCWSTR,
    proc: WNDPROC,
    width: i32,
    height: i32,
) -> Result<(HWND, HINSTANCE, u32)> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut existing = WNDCLASSW::default();
        if GetClassInfoW(Some(instance), class, &mut existing).is_err() {
            let class = WNDCLASSW {
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: LoadIconW(Some(instance), w!("BKS_TERM")).unwrap_or_default(),
                lpszClassName: class,
                lpfnWndProc: proc,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(Error::from_thread());
            }
        }
        let dpi = GetDpiForWindow(owner).max(dpi::BASE_DPI);
        let mut owner_rect = RECT::default();
        GetWindowRect(owner, &mut owner_rect)?;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            monitor.rcWork
        } else {
            owner_rect
        };
        let width = dpi::scale(width, dpi).min((work.right - work.left).max(1));
        let height = dpi::scale(height, dpi).min((work.bottom - work.top).max(1));
        let x = (owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2)
            .clamp(work.left, work.right - width);
        let y = (owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 2)
            .clamp(work.top, work.bottom - height);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class,
            title,
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            Some(owner),
            None,
            Some(instance),
            None,
        )?;
        Ok((hwnd, instance, dpi))
    }
}

pub(crate) fn owns_message(hwnd: HWND, message: &MSG) -> bool {
    message.hwnd == hwnd || unsafe { IsChild(hwnd, message.hwnd).as_bool() }
}

pub(crate) unsafe fn dispatch_owned(hwnd: HWND, message: &MSG) {
    if !IsDialogMessageW(hwnd, message).as_bool() {
        let _ = TranslateMessage(message);
        DispatchMessageW(message);
    }
}

pub(crate) fn handle_tab(key: usize, stops: &[HWND]) -> bool {
    unsafe {
        if key != VK_TAB.0 as usize
            || GetKeyState(VK_CONTROL.0 as i32) < 0
            || GetKeyState(VK_MENU.0 as i32) < 0
        {
            return false;
        }
        if let Some(next) = crate::keyboard_navigation::next_tab_stop(
            stops,
            GetFocus(),
            GetKeyState(VK_SHIFT.0 as i32) < 0,
        ) {
            let _ = SetFocus(Some(next));
        }
    }
    true
}

pub(crate) fn handle_select_all(key: usize, field: HWND) -> bool {
    unsafe {
        if key == 0x41 && GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0
        {
            SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            return true;
        }
    }
    false
}

pub(crate) fn activate(hwnd: HWND, field: HWND) {
    unsafe {
        let _ = ShowWindow(
            hwnd,
            if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(field));
    }
}

pub(crate) fn restore_focus(owner: HWND, previous: HWND) {
    unsafe {
        if !IsWindow(Some(owner)).as_bool() {
            return;
        }
        let _ = SetForegroundWindow(owner);
        let target = if IsWindow(Some(previous)).as_bool()
            && IsWindowVisible(previous).as_bool()
            && IsWindowEnabled(previous).as_bool()
            && (previous == owner || IsChild(owner, previous).as_bool())
        {
            previous
        } else {
            owner
        };
        let _ = SetFocus(Some(target));
    }
}

pub(crate) fn show_validation(hwnd: HWND, message: &str) {
    let message = wide(message);
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            w!("Проверьте введённые данные"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

pub(crate) fn apply_theme(hwnd: HWND, theme: &Theme, all: &[HWND], edits: &[HWND]) {
    unsafe {
        for control in all {
            SendMessageW(
                *control,
                WM_SETFONT,
                Some(WPARAM(theme.ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let name = if theme.high_contrast {
            PCWSTR::null()
        } else {
            w!("DarkMode_Explorer")
        };
        for control in edits {
            let _ = SetWindowTheme(*control, name, PCWSTR::null());
        }
        let dark = u32::from(!theme.high_contrast);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

pub(crate) unsafe fn erase_background(hwnd: HWND, wparam: WPARAM, theme: &Theme) -> LRESULT {
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    FillRect(HDC(wparam.0 as *mut c_void), &rect, theme.window);
    LRESULT(1)
}

pub(crate) unsafe fn control_color(wparam: WPARAM, theme: &Theme, edit: bool) -> LRESULT {
    let dc = HDC(wparam.0 as *mut c_void);
    SetTextColor(dc, theme.colors.text_primary);
    SetBkColor(
        dc,
        if edit {
            theme.colors.surface
        } else {
            theme.colors.window
        },
    );
    LRESULT(if edit {
        theme.surface.0 as isize
    } else {
        theme.window.0 as isize
    })
}

pub(crate) fn release_theme(theme: Theme) {
    unsafe {
        for font in [theme.ui_font, theme.title_font, theme.terminal_font] {
            if !font.0.is_null() {
                let _ = DeleteObject(font.into());
            }
        }
        if !theme.high_contrast {
            for brush in [
                theme.window,
                theme.terminal,
                theme.toolbar,
                theme.sidebar,
                theme.surface,
                theme.hover,
                theme.selected,
                theme.border,
                theme.border_subtle,
                theme.brand,
            ] {
                if !brush.0.is_null() {
                    let _ = DeleteObject(brush.into());
                }
            }
        }
    }
}
