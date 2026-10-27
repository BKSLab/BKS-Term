//! Native, modeless settings editor. The owner persists a confirmed result
//! before publishing it; this window never writes files or starts sessions.

use std::cell::RefCell;

use crate::accessibility::set_accessible_name;
use crate::dpi;
use crate::output_speech::SpeechMode;
use crate::profile_editor::{
    activate, apply_theme, child, control_color, create_dialog, dispatch_owned, erase_background,
    handle_select_all, handle_tab, owns_message, release_theme, restore_focus, set_text,
    show_validation, wide,
};
use crate::settings::{LastTabPolicy, Settings};
use crate::shortcuts::{parse_bindings, AppAction, ShortcutMap};
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETLIMITTEXT, ODS_HOTLIGHT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetClientRect, GetWindowTextLengthW, GetWindowTextW, MoveWindow,
    SendMessageW, SetWindowPos, BN_CLICKED, BS_OWNERDRAW, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_READONLY, MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WS_TABSTOP,
    WS_VSCROLL,
};

const ID_SPEECH: usize = 4600;
const ID_LAST_TAB: usize = 4601;
const ID_REMEMBER: usize = 4602;
const ID_ACTION: usize = 4603;
const ID_CHORD: usize = 4604;
const ID_ASSIGN: usize = 4605;
const ID_DEFAULTS: usize = 4606;
const ID_SAVE: usize = 4607;
const ID_CANCEL: usize = 4608;
const ID_HINT: usize = 4609;

const HINT: &str = "Сочетания: Ctrl+Shift+T. Несколько вариантов разделяйте точкой с запятой: Ctrl+Up; Alt+Up. Пустое поле отключает сочетание; команда остаётся в меню. F1 (справка) и Ctrl+Shift+Space (ввод и чтение) обязательны для возврата управления; к ним можно добавить другие сочетания. Конфликты и клавиши Windows, NVDA и редактирования не допускаются.\r\nНедавние профили сохраняются только по вашему выбору: идентификаторы и папки, без команд, черновиков, вывода и секретов. Восстановление требует выбора профиля в меню; команды и SSH автоматически не запускаются.";

pub enum Outcome {
    Saved(Settings),
    Cancelled,
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    labels: [HWND; 5],
    fields: [HWND; 5],
    hint: HWND,
    assign: HWND,
    defaults: HWND,
    save: HWND,
    cancel: HWND,
    dpi: u32,
    theme: Theme,
}

struct State {
    view: View,
    settings: Settings,
    action: usize,
}

thread_local! {
    static DIALOG: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Outcome>> = const { RefCell::new(None) };
}

fn current() -> Option<View> {
    DIALOG.with(|dialog| dialog.borrow().as_ref().map(|state| state.view))
}

pub fn take_result() -> Option<Outcome> {
    OUTCOME.with(|outcome| outcome.borrow_mut().take())
}

pub fn show(owner: HWND, settings: &Settings) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.fields[0]);
        return Ok(());
    }
    let previous_focus = unsafe { GetFocus() };
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermPreferences"),
        w!("Настройки — BKS Term"),
        Some(window_proc),
        760,
        680,
    )?;
    let view = View {
        hwnd,
        owner,
        previous_focus,
        labels: [HWND::default(); 5],
        fields: [HWND::default(); 5],
        hint: HWND::default(),
        assign: HWND::default(),
        defaults: HWND::default(),
        save: HWND::default(),
        cancel: HWND::default(),
        dpi,
        theme: Theme::new(dpi),
    };
    OUTCOME.with(|outcome| *outcome.borrow_mut() = None);
    DIALOG.with(|dialog| {
        *dialog.borrow_mut() = Some(State {
            view,
            settings: settings.clone(),
            action: 0,
        });
    });
    if let Err(error) = create_controls(view, instance, settings) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    let view = current().expect("preferences initialized");
    apply_fonts(view);
    load_binding(view);
    layout(view);
    activate(hwnd, view.fields[0]);
    Ok(())
}

/// Run before main-window shortcuts so editing never reaches ConPTY.
pub fn handle_message(message: &MSG) -> bool {
    let Some(view) = current() else {
        return false;
    };
    if !owns_message(view.hwnd, message) {
        return false;
    }
    unsafe {
        if message.message == WM_KEYDOWN {
            let stops = [
                view.fields[0],
                view.fields[1],
                view.fields[2],
                view.fields[3],
                view.fields[4],
                view.assign,
                view.defaults,
                view.hint,
                view.save,
                view.cancel,
            ];
            if handle_tab(message.wParam.0, &stops) {
                return true;
            }
            let focus = GetFocus();
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                close(view, Outcome::Cancelled);
                return true;
            }
            if (focus == view.fields[4] || focus == view.hint)
                && handle_select_all(message.wParam.0, focus)
            {
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                if focus == view.fields[4] || focus == view.assign {
                    apply_binding(view);
                    return true;
                }
                if focus == view.defaults {
                    reset_bindings(view);
                    return true;
                }
                if focus == view.save {
                    save(view);
                    return true;
                }
                if focus == view.cancel {
                    close(view, Outcome::Cancelled);
                    return true;
                }
            }
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

fn create_controls(view: View, instance: HINSTANCE, settings: &Settings) -> Result<()> {
    let names = [
        "Автоматические речевые уведомления",
        "После закрытия последней вкладки",
        "Недавние профили и рабочие папки",
        "Действие для настройки сочетаний",
        "Сочетания клавиш (пусто — отключены)",
    ];
    let ids = [ID_SPEECH, ID_LAST_TAB, ID_REMEMBER, ID_ACTION, ID_CHORD];
    let mut next = view;
    for (index, name) in names.into_iter().enumerate() {
        next.labels[index] = child(
            view.hwnd,
            instance,
            w!("STATIC"),
            name,
            WINDOW_STYLE::default(),
            4620 + index,
        )?;
        next.fields[index] = child(
            view.hwnd,
            instance,
            if index == 4 {
                w!("EDIT")
            } else {
                w!("COMBOBOX")
            },
            "",
            WS_TABSTOP
                | if index == 4 {
                    WINDOW_STYLE(ES_AUTOHSCROLL as u32)
                } else {
                    WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32)
                },
            ids[index],
        )?;
        set_accessible_name(next.fields[index], name)?;
    }
    unsafe {
        for label in [
            "Только по запросу",
            "Только важные события",
            "Все устойчивые новые строки",
        ] {
            add_choice(next.fields[0], label);
        }
        for label in ["Оставить окно без вкладок", "Закрыть окно"]
        {
            add_choice(next.fields[1], label);
        }
        for label in [
            "Не сохранять (удалить сохранённый список)",
            "Сохранять только профили и папки",
        ] {
            add_choice(next.fields[2], label);
        }
        for action in AppAction::ALL {
            add_choice(next.fields[3], action.label());
        }
        for (field, selection) in [
            (
                next.fields[0],
                match settings.speech_mode {
                    SpeechMode::Manual => 0,
                    SpeechMode::Important => 1,
                    SpeechMode::All => 2,
                },
            ),
            (
                next.fields[1],
                usize::from(settings.last_tab == LastTabPolicy::CloseWindow),
            ),
            (next.fields[2], usize::from(settings.remember_workspace)),
            (next.fields[3], 0),
        ] {
            SendMessageW(field, CB_SETCURSEL, Some(WPARAM(selection)), None);
        }
        SendMessageW(next.fields[4], EM_SETLIMITTEXT, Some(WPARAM(200)), None);
    }
    next.hint = child(
        view.hwnd,
        instance,
        w!("EDIT"),
        HINT,
        WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
        ID_HINT,
    )?;
    set_accessible_name(
        next.hint,
        "Подсказка: сочетания клавиш и сохранение профилей",
    )?;
    next.assign = button(view, instance, "Назначить", ID_ASSIGN)?;
    next.defaults = button(view, instance, "Сбросить сочетания", ID_DEFAULTS)?;
    next.save = button(view, instance, "Сохранить", ID_SAVE)?;
    next.cancel = button(view, instance, "Отмена", ID_CANCEL)?;
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.view = next;
        }
    });
    Ok(())
}

fn button(view: View, instance: HINSTANCE, label: &str, id: usize) -> Result<HWND> {
    let hwnd = child(
        view.hwnd,
        instance,
        w!("BUTTON"),
        label,
        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        id,
    )?;
    set_accessible_name(hwnd, label)?;
    crate::control_state::install(hwnd);
    Ok(hwnd)
}

unsafe fn add_choice(combo: HWND, text: &str) {
    let text = wide(text);
    SendMessageW(
        combo,
        CB_ADDSTRING,
        None,
        Some(LPARAM(text.as_ptr() as isize)),
    );
}

fn text(hwnd: HWND) -> String {
    unsafe {
        let mut buffer = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let count = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..count])
    }
}

fn apply_binding(view: View) -> bool {
    let input = text(view.fields[4]);
    let result = parse_bindings(&input).and_then(|chords| {
        DIALOG.with(|dialog| {
            let mut dialog = dialog.borrow_mut();
            let state = dialog.as_mut().expect("preferences open");
            state
                .settings
                .shortcuts
                .set(AppAction::ALL[state.action], chords)
        })
    });
    if let Err(error) = result {
        show_validation(view.hwnd, &error);
        unsafe {
            let _ = SetFocus(Some(view.fields[4]));
        }
        return false;
    }
    load_binding(view);
    true
}

fn load_binding(view: View) {
    let binding = DIALOG.with(|dialog| {
        dialog.borrow().as_ref().map(|state| {
            state
                .settings
                .shortcuts
                .binding_text(AppAction::ALL[state.action])
        })
    });
    if let Some(binding) = binding {
        set_text(view.fields[4], &binding);
    }
}

fn change_action(view: View) {
    let index = unsafe { SendMessageW(view.fields[3], CB_GETCURSEL, None, None).0 };
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    if index >= AppAction::ALL.len() {
        return;
    }
    if !apply_binding(view) {
        let previous =
            DIALOG.with(|dialog| dialog.borrow().as_ref().map_or(0, |state| state.action));
        unsafe {
            SendMessageW(view.fields[3], CB_SETCURSEL, Some(WPARAM(previous)), None);
        }
        return;
    }
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.action = index;
        }
    });
    load_binding(view);
}

fn reset_bindings(view: View) {
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.settings.shortcuts = ShortcutMap::default();
        }
    });
    load_binding(view);
}

fn save(view: View) {
    if !apply_binding(view) {
        return;
    }
    let choices = view.fields[..3]
        .iter()
        .map(|field| unsafe { SendMessageW(*field, CB_GETCURSEL, None, None).0 })
        .collect::<Vec<_>>();
    let settings = DIALOG.with(|dialog| {
        let dialog = dialog.borrow();
        let mut settings = dialog.as_ref().expect("preferences open").settings.clone();
        settings.speech_mode = match choices[0] {
            0 => SpeechMode::Manual,
            2 => SpeechMode::All,
            _ => SpeechMode::Important,
        };
        settings.last_tab = if choices[1] == 1 {
            LastTabPolicy::CloseWindow
        } else {
            LastTabPolicy::KeepWindow
        };
        settings.remember_workspace = choices[2] == 1;
        settings
    });
    close(view, Outcome::Saved(settings));
}

fn close(view: View, outcome: Outcome) {
    OUTCOME.with(|result| *result.borrow_mut() = Some(outcome));
    unsafe {
        let _ = DestroyWindow(view.hwnd);
    }
    restore_focus(view.owner, view.previous_focus);
}

fn apply_fonts(view: View) {
    let mut all = view.labels.to_vec();
    all.extend(view.fields);
    all.extend([
        view.hint,
        view.assign,
        view.defaults,
        view.save,
        view.cancel,
    ]);
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
        let row_h = dpi::scale(58, view.dpi);
        let label_h = dpi::scale(22, view.dpi);
        let field_h = dpi::scale(30, view.dpi);
        for index in 0..5 {
            let y = margin + row_h * index as i32;
            let _ = MoveWindow(view.labels[index], margin, y, width, label_h, true);
            let _ = MoveWindow(
                view.fields[index],
                margin,
                y + label_h,
                width,
                if index == 4 {
                    field_h
                } else {
                    dpi::scale(260, view.dpi)
                },
                true,
            );
        }
        let button_h = dpi::scale(36, view.dpi);
        let button_w = dpi::scale(180, view.dpi).min((width - gap).max(2) / 2);
        let assign_y = margin + row_h * 5;
        let _ = MoveWindow(view.assign, margin, assign_y, button_w, button_h, true);
        let _ = MoveWindow(
            view.defaults,
            margin + button_w + gap,
            assign_y,
            button_w,
            button_h,
            true,
        );
        let footer_y = (rect.bottom - margin - button_h).max(assign_y + button_h + 2 * gap);
        let hint_y = assign_y + button_h + gap;
        let _ = MoveWindow(
            view.hint,
            margin,
            hint_y,
            width,
            (footer_y - gap - hint_y).max(1),
            true,
        );
        let _ = MoveWindow(
            view.save,
            rect.right - margin - 2 * button_w - gap,
            footer_y,
            button_w,
            button_h,
            true,
        );
        let _ = MoveWindow(
            view.cancel,
            rect.right - margin - button_w,
            footer_y,
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
            if id == ID_ACTION && code == CBN_SELCHANGE {
                change_action(view);
            } else if code == BN_CLICKED {
                match id {
                    ID_ASSIGN => {
                        apply_binding(view);
                    }
                    ID_DEFAULTS => reset_bindings(view),
                    ID_SAVE => save(view),
                    ID_CANCEL => close(view, Outcome::Cancelled),
                    _ => {}
                }
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
            DIALOG.with(|dialog| {
                if let Some(state) = dialog.borrow_mut().as_mut() {
                    state.view = next;
                }
            });
            apply_fonts(next);
            release_theme(view.theme);
            if message == WM_DPICHANGED && lparam.0 != 0 {
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
                    x: dpi::scale(480, view.dpi),
                    y: dpi::scale(540, view.dpi),
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
            let label = match wparam.0 {
                ID_ASSIGN => "Назначить",
                ID_DEFAULTS => "Сбросить сочетания",
                ID_SAVE => "Сохранить",
                _ => "Отмена",
            };
            draw_mode_button(&item, &view.theme, label);
            LRESULT(1)
        }
        WM_NCDESTROY => {
            DIALOG.with(|dialog| *dialog.borrow_mut() = None);
            release_theme(view.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
