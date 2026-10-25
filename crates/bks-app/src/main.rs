#![windows_subsystem = "windows"]

mod accessibility;
pub mod ansi_palette;
mod background;
mod command_blocks;
mod control_state;
mod dpi;
mod layout;
mod profile;
mod profile_icons;
mod profile_menu;
mod session;
mod shell;
mod system_colors;
mod terminal_input;
mod terminal_output;
mod theme;
mod window_frame;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;

use accessibility::{set_accessible_name, ComApartment};
use bks_pty::PtySize;
use command_blocks::CommandBlocks;
use profile::ProfileManager;
use session::SessionManager;
use shell::{
    draw_connection_status, draw_sidebar_button, draw_tab_background, draw_tab_button,
    draw_tab_close, draw_title_bar, draw_window_button, SidebarButton, WindowButton,
};
use terminal_input::{special_key_bytes, SpecialKey, Utf16Encoder};
use theme::{draw_execute_button, draw_mode_button, Theme};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetDC, GetTextMetricsW, InvalidateRect, ReleaseDC,
    SelectObject, SetBkColor, SetTextColor, HBRUSH, HDC, PAINTSTRUCT, TEXTMETRICW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_OWNERDRAW};
use windows::Win32::UI::Controls::{
    SetWindowTheme, DRAWITEMSTRUCT, EM_GETFIRSTVISIBLELINE, EM_GETSEL, EM_LINESCROLL,
    EM_REPLACESEL, EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETSEL, ODS_HOTLIGHT,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_BACK, VK_C, VK_CONTROL, VK_D, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_MENU, VK_NEXT, VK_P, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_T, VK_TAB, VK_UP, VK_W, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, IsZoomed, KillTimer,
    LoadCursorW, LoadIconW, MessageBoxW, MoveWindow, PostQuitMessage, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, TrackPopupMenu,
    TranslateMessage, BN_CLICKED, BS_OWNERDRAW, CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_READONLY, ES_WANTRETURN, HMENU, ICON_BIG, ICON_SMALL, IDC_ARROW, IDYES, MB_ICONERROR,
    MB_ICONWARNING, MB_OK, MB_YESNO, MINMAXINFO, MSG, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE,
    SC_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    TPM_NONOTIFY, TPM_RETURNCMD, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CHAR, WM_COMMAND, WM_COPY,
    WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_MEASUREITEM, WM_MENUCHAR, WM_NCCALCSIZE, WM_NCHITTEST,
    WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SETICON, WM_SETREDRAW, WM_SIZE, WM_SYSCOMMAND,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_OUTPUT: usize = 1001;
const ID_INPUT: usize = 1002;
const ID_APPEND: usize = 1003;
const ID_SESSION: usize = 1004;
const ID_STATUS_CONNECTION: usize = 1005;
const ID_STATUS_ENCODING: usize = 1006;
const ID_SIDEBAR_TOGGLE: usize = 1007;
const ID_SIDEBAR_TERMINALS: usize = 1008;
const ID_MODE_TOGGLE: usize = 1009;
const ID_NEW_TAB: usize = 1010;
const ID_MINIMIZE: usize = 1012;
const ID_MAXIMIZE: usize = 1013;
const ID_CLOSE_WINDOW: usize = 1014;
const ID_STATUS_SIZE: usize = 1015;
const ID_SIDEBAR_PROFILES: usize = 1016;
const MENU_INPUT_MODE: usize = 60001;
const ID_TAB_BASE: usize = 2000;
const ID_TAB_CLOSE_BASE: usize = 3000;
const OUTPUT_TIMER_ID: usize = 1;
const OUTPUT_POLL_MS: u32 = 50;
const STARTUP_TEXT: &str = "Запуск PowerShell...\r\nAlt+Up/Down — результаты команд, Ctrl+Shift+C — копировать результат.\r\n";

#[derive(Clone, Copy, Default)]
struct Controls {
    session_header: HWND,
    output: HWND,
    input_label: HWND,
    input: HWND,
    append: HWND,
    status_connection: HWND,
    status_encoding: HWND,
    status_size: HWND,
    sidebar_toggle: HWND,
    sidebar_terminals: HWND,
    sidebar_profiles: HWND,
    new_tab: HWND,
    minimize: HWND,
    maximize: HWND,
    close_window: HWND,
}

#[derive(Clone, Copy)]
struct TabControls {
    tab: HWND,
    close: HWND,
}

thread_local! {
    static CONTROLS: RefCell<Controls> = RefCell::new(Controls::default());
    static TAB_CONTROLS: RefCell<Vec<TabControls>> = const { RefCell::new(Vec::new()) };
    static TAB_SCROLL: Cell<usize> = const { Cell::new(0) };
    static THEME: RefCell<Theme> = RefCell::new(Theme::default());
    static DPI: Cell<u32> = const { Cell::new(dpi::BASE_DPI) };
    static SIDEBAR_EXPANDED: Cell<bool> = const { Cell::new(true) };
    static UTF16_ENCODER: RefCell<Utf16Encoder> = RefCell::new(Utf16Encoder::new());
    static CONNECTION_STATUS: RefCell<Option<bool>> = const { RefCell::new(None) };
    static PROFILES: RefCell<ProfileManager> = RefCell::new(ProfileManager::discover());
    static SESSIONS: RefCell<SessionManager> = RefCell::new(SessionManager::new());
}

fn main() {
    if let Err(error) = run() {
        show_error("Не удалось запустить BKS Term", &error);
    }
}

fn run() -> Result<()> {
    let _com = ComApartment::initialize()?;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let initial_dpi = GetDpiForSystem();
        let theme = Theme::new(initial_dpi);
        DPI.set(initial_dpi);
        THEME.with(|current| *current.borrow_mut() = theme);
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let icon = LoadIconW(Some(instance), w!("BKS_TERM"))?;
        let window_class = WNDCLASSW {
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance,
            hbrBackground: theme.window,
            lpszClassName: w!("BksTermWindow"),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(Error::from_thread());
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BksTermWindow"),
            w!("BKS Term"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            dpi::scale(1200, initial_dpi),
            dpi::scale(800, initial_dpi),
            None,
            None,
            Some(instance),
            None,
        )?;
        for icon_type in [ICON_BIG, ICON_SMALL] {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(icon_type as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
        }
        apply_dark_title(hwnd);
        refresh_layout(hwnd);
        start_terminal(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(Some(controls().input));

        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 == -1 {
                return Err(Error::from_thread());
            }
            if result.0 == 0 {
                break;
            }
            if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                && handle_block_shortcut(message.wParam.0)
            {
                continue;
            }
            if message.message == WM_KEYDOWN && handle_app_shortcut(hwnd, message.wParam.0) {
                continue;
            }
            if message.message == WM_KEYDOWN
                && message.wParam.0 == VK_RETURN.0 as usize
                && GetFocus() == controls().input
            {
                if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    insert_input_newline();
                } else {
                    execute_input();
                }
                continue;
            }
            if direct_input_active() && GetFocus() == controls().output {
                if message.message == WM_KEYDOWN && handle_direct_key(message.wParam.0) {
                    continue;
                }
                if message.message == WM_CHAR {
                    handle_direct_char(message.wParam.0 as u16);
                    continue;
                }
            }
            if message.message == WM_KEYDOWN && message.wParam.0 == VK_TAB.0 as usize {
                let focus = GetFocus();
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let controls = controls();
                let cycle = [
                    controls.sidebar_toggle,
                    controls.sidebar_terminals,
                    controls.sidebar_profiles,
                    controls.output,
                    controls.input,
                    controls.append,
                    controls.new_tab,
                ];
                if let Some(index) = cycle.iter().position(|control| *control == focus) {
                    let mut next = index;
                    loop {
                        next = if shift {
                            (next + cycle.len() - 1) % cycle.len()
                        } else {
                            (next + 1) % cycle.len()
                        };
                        if !direct_input_active()
                            || ![controls.input, controls.append].contains(&cycle[next])
                        {
                            break;
                        }
                    }
                    let _ = SetFocus(Some(cycle[next]));
                    continue;
                }
            }
            if IsDialogMessageW(hwnd, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => window_frame::calculate_client(hwnd, lparam),
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCHITTEST => window_frame::hit_test(hwnd, lparam, current_dpi()),
        WM_CREATE => match create_controls(hwnd) {
            Ok(()) => {
                let window_dpi = GetDpiForWindow(hwnd);
                if window_dpi != current_dpi() {
                    update_dpi(window_dpi);
                }
                LRESULT(0)
            }
            Err(_) => LRESULT(-1),
        },
        WM_PAINT => {
            paint_shell(hwnd);
            LRESULT(0)
        }
        WM_SETFOCUS => {
            let input = if direct_input_active() {
                controls().output
            } else {
                controls().input
            };
            if !input.0.is_null() {
                let _ = SetFocus(Some(input));
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            layout_controls(width, height);
            rebuild_tab_controls(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, true);
            let _ = InvalidateRect(Some(controls().maximize), None, true);
            let maximize_name = if IsZoomed(hwnd).as_bool() {
                "Восстановить окно"
            } else {
                "Развернуть окно"
            };
            let _ = set_accessible_name(controls().maximize, maximize_name);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xffff) as u32;
            update_dpi(new_dpi);
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let (min_width, min_height) = dpi::min_track_size(current_dpi());
                info.ptMinTrackSize = POINT {
                    x: min_width,
                    y: min_height,
                };
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let current_theme = theme();
            if HWND(lparam.0 as *mut c_void) == controls().output {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.terminal,
                    current_theme.terminal,
                )
            } else {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.surface,
                    current_theme.surface,
                )
            }
        }
        WM_CTLCOLORSTATIC => {
            let control = HWND(lparam.0 as *mut c_void);
            let current_theme = theme();
            if control == controls().output {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.terminal,
                    current_theme.terminal,
                )
            } else if control == controls().status_connection {
                let color = match CONNECTION_STATUS.with(|status| *status.borrow()) {
                    Some(true) => current_theme.colors.status_success,
                    Some(false) => current_theme.colors.status_error,
                    None => current_theme.colors.text_muted,
                };
                control_color(
                    wparam,
                    color,
                    current_theme.colors.toolbar,
                    current_theme.toolbar,
                )
            } else if control == controls().status_encoding || control == controls().status_size {
                control_color(
                    wparam,
                    current_theme.colors.text_secondary,
                    current_theme.colors.surface,
                    current_theme.surface,
                )
            } else {
                control_color(
                    wparam,
                    current_theme.colors.text_secondary,
                    current_theme.colors.window,
                    current_theme.window,
                )
            }
        }
        WM_MEASUREITEM => {
            if profile_menu::measure(lparam, current_dpi()) {
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_MENUCHAR => profile_menu::menu_char(wparam, lparam)
            .unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam)),
        WM_DRAWITEM => {
            let mut draw_item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if profile_menu::draw(&draw_item, &theme(), current_dpi()) {
                return LRESULT(1);
            }
            if control_state::is_hot(draw_item.hwndItem) {
                draw_item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            let item = &draw_item;
            let expanded = sidebar_expanded_for_width(client_width(hwnd));
            if wparam.0 == ID_STATUS_CONNECTION {
                draw_connection_status(
                    item,
                    &theme(),
                    current_dpi(),
                    CONNECTION_STATUS.with(|status| *status.borrow()),
                );
                LRESULT(1)
            } else if wparam.0 == ID_APPEND {
                draw_execute_button(item, &theme());
                LRESULT(1)
            } else if wparam.0 == ID_SESSION {
                draw_tab_background(item, &theme(), current_dpi());
                LRESULT(1)
            } else if wparam.0 == ID_NEW_TAB {
                draw_mode_button(item, &theme(), "+");
                LRESULT(1)
            } else if (ID_TAB_BASE..ID_TAB_CLOSE_BASE).contains(&wparam.0) {
                let id = (wparam.0 - ID_TAB_BASE) as u64;
                draw_tab_button(
                    item,
                    &theme(),
                    current_dpi(),
                    active_session_id() == Some(id),
                );
                LRESULT(1)
            } else if wparam.0 >= ID_TAB_CLOSE_BASE {
                draw_tab_close(
                    item,
                    &theme(),
                    current_dpi(),
                    active_session_id() == Some((wparam.0 - ID_TAB_CLOSE_BASE) as u64),
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_TOGGLE {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Toggle,
                    expanded,
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_TERMINALS {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Terminals,
                    expanded,
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_PROFILES {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Profiles,
                    expanded,
                );
                LRESULT(1)
            } else if [ID_MINIMIZE, ID_MAXIMIZE, ID_CLOSE_WINDOW].contains(&wparam.0) {
                let kind = match wparam.0 {
                    ID_MINIMIZE => WindowButton::Minimize,
                    ID_MAXIMIZE if IsZoomed(hwnd).as_bool() => WindowButton::Restore,
                    ID_MAXIMIZE => WindowButton::Maximize,
                    _ => WindowButton::Close,
                };
                draw_window_button(item, &theme(), current_dpi(), kind);
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_COMMAND => {
            let control_id = wparam.0 & 0xffff;
            let notification = (wparam.0 >> 16) & 0xffff;
            if notification != BN_CLICKED as usize {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            match control_id {
                ID_MINIMIZE | ID_MAXIMIZE | ID_CLOSE_WINDOW => {
                    let command = match control_id {
                        ID_MINIMIZE => SC_MINIMIZE,
                        ID_MAXIMIZE if IsZoomed(hwnd).as_bool() => SC_RESTORE,
                        ID_MAXIMIZE => SC_MAXIMIZE,
                        _ => SC_CLOSE,
                    };
                    SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM(command as usize)), None);
                    LRESULT(0)
                }
                ID_APPEND => {
                    execute_input();
                    LRESULT(0)
                }
                ID_SIDEBAR_TOGGLE => {
                    toggle_sidebar(hwnd);
                    LRESULT(0)
                }
                ID_SIDEBAR_TERMINALS => {
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(Some(controls().output));
                    LRESULT(0)
                }
                ID_SIDEBAR_PROFILES => {
                    show_profile_menu_at(hwnd, controls().sidebar_profiles, true);
                    LRESULT(0)
                }
                ID_MODE_TOGGLE => {
                    toggle_input_mode(hwnd);
                    LRESULT(0)
                }
                ID_NEW_TAB => {
                    show_profile_menu_at(hwnd, controls().new_tab, false);
                    LRESULT(0)
                }
                id if (ID_TAB_BASE..ID_TAB_CLOSE_BASE).contains(&id) => {
                    activate_session(hwnd, (id - ID_TAB_BASE) as u64);
                    LRESULT(0)
                }
                id if id >= ID_TAB_CLOSE_BASE => {
                    close_session(hwnd, (id - ID_TAB_CLOSE_BASE) as u64);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_TIMER => {
            if wparam.0 == OUTPUT_TIMER_ID {
                poll_terminal();
                update_connection_status();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), OUTPUT_TIMER_ID);
            SESSIONS.with(|sessions| sessions.borrow_mut().sessions.clear());
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn create_controls(parent: HWND) -> Result<()> {
    let module = unsafe { GetModuleHandleW(None)? };
    let instance = HINSTANCE(module.0);
    let new_controls = Controls {
        session_header: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("PowerShell"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(SS_OWNERDRAW.0),
            parent,
            Some(ID_SESSION),
            instance,
        )?,
        output: create_control(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("Запуск PowerShell...\r\nAlt+Up/Down — результаты команд, Ctrl+Shift+C — копировать результат.\r\n"),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
            parent,
            Some(ID_OUTPUT),
            instance,
        )?,
        input_label: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Команда (Enter — выполнить, Shift+Enter — новая строка)"),
            WS_CHILD | WS_VISIBLE,
            parent,
            None,
            instance,
        )?,
        input: create_control(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
            parent,
            Some(ID_INPUT),
            instance,
        )?,
        append: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Выполнить"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_APPEND),
            instance,
        )?,
        status_connection: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("PowerShell · Запуск"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_OWNERDRAW.0),
            parent,
            Some(ID_STATUS_CONNECTION),
            instance,
        )?,
        status_encoding: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("UTF-8"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0),
            parent,
            Some(ID_STATUS_ENCODING),
            instance,
        )?,
        status_size: create_control(
            WINDOW_EX_STYLE::default(), w!("STATIC"), w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0),
            parent, Some(ID_STATUS_SIZE), instance,
        )?,
        sidebar_toggle: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Свернуть панель"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_SIDEBAR_TOGGLE),
            instance,
        )?,
        sidebar_terminals: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Терминалы"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_SIDEBAR_TERMINALS),
            instance,
        )?,
        sidebar_profiles: create_control(
            WINDOW_EX_STYLE::default(), w!("BUTTON"), w!("Профили"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent, Some(ID_SIDEBAR_PROFILES), instance,
        )?,
        new_tab: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("+"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_NEW_TAB),
            instance,
        )?,
        minimize: create_control(
            WINDOW_EX_STYLE::default(), w!("BUTTON"), w!("Свернуть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent, Some(ID_MINIMIZE), instance,
        )?,
        maximize: create_control(
            WINDOW_EX_STYLE::default(), w!("BUTTON"), w!("Развернуть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent, Some(ID_MAXIMIZE), instance,
        )?,
        close_window: create_control(
            WINDOW_EX_STYLE::default(), w!("BUTTON"), w!("Закрыть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent, Some(ID_CLOSE_WINDOW), instance,
        )?,
    };
    CONTROLS.with(|controls| {
        *controls.borrow_mut() = new_controls;
    });
    set_accessible_name(new_controls.output, "Журнал терминала")?;
    set_accessible_name(new_controls.input, "Команда")?;
    set_accessible_name(new_controls.session_header, "Вкладки терминала")?;
    set_accessible_name(new_controls.sidebar_toggle, "Свернуть боковую панель")?;
    set_accessible_name(new_controls.sidebar_terminals, "Терминалы, выбрано")?;
    set_accessible_name(
        new_controls.sidebar_profiles,
        "Профили оболочек: выбрать и открыть вкладку",
    )?;
    set_accessible_name(new_controls.new_tab, "Новая вкладка: выбрать профиль")?;
    set_accessible_name(new_controls.minimize, "Свернуть окно")?;
    set_accessible_name(new_controls.maximize, "Развернуть окно")?;
    set_accessible_name(new_controls.close_window, "Закрыть окно")?;
    set_accessible_name(new_controls.status_size, "Размер терминала")?;
    unsafe {
        SendMessageW(
            new_controls.output,
            EM_SETLIMITTEXT,
            Some(WPARAM(0x7FFF_FFFE)),
            None,
        );
        apply_fonts();
        let _ = SetWindowTheme(new_controls.output, w!("DarkMode_Explorer"), PCWSTR::null());
        let _ = SetWindowTheme(new_controls.input, w!("DarkMode_Explorer"), PCWSTR::null());
    }
    Ok(())
}

fn create_control(
    ex_style: WINDOW_EX_STYLE,
    class_name: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    parent: HWND,
    id: Option<usize>,
    instance: HINSTANCE,
) -> Result<HWND> {
    unsafe {
        let control = CreateWindowExW(
            ex_style,
            class_name,
            text,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            id.map(|id| HMENU(id as *mut c_void)),
            Some(instance),
            None,
        )?;
        if style.0 & 0xf == BS_OWNERDRAW as u32 {
            control_state::install(control);
        }
        Ok(control)
    }
}

fn apply_dark_title(hwnd: HWND) {
    if theme().high_contrast {
        return;
    }
    let dark = 1_u32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const _ as *const c_void,
            std::mem::size_of_val(&DWMWCP_ROUND) as u32,
        );
        let border = theme().colors.selected;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const c_void,
            std::mem::size_of_val(&border) as u32,
        );
    }
}

fn control_color(wparam: WPARAM, text: COLORREF, background: COLORREF, brush: HBRUSH) -> LRESULT {
    unsafe {
        let hdc = HDC(wparam.0 as *mut c_void);
        SetTextColor(hdc, text);
        SetBkColor(hdc, background);
    }
    LRESULT(brush.0 as isize)
}

fn current_dpi() -> u32 {
    DPI.with(Cell::get)
}

fn update_dpi(new_dpi: u32) {
    DPI.set(new_dpi);
    THEME.with(|theme| theme.borrow_mut().update_fonts(new_dpi));
    apply_fonts();
    resize_active_terminal();
}

fn apply_fonts() {
    let theme = theme();
    unsafe {
        for control in [
            controls().session_header,
            controls().input_label,
            controls().append,
            controls().status_connection,
            controls().status_encoding,
            controls().status_size,
            controls().sidebar_toggle,
            controls().sidebar_terminals,
            controls().sidebar_profiles,
            controls().new_tab,
            controls().minimize,
            controls().maximize,
            controls().close_window,
        ] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme.ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        for control in [controls().output, controls().input] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme.terminal_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

fn paint_shell(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let mut client = RECT::default();
    unsafe {
        let hdc = BeginPaint(hwnd, &mut paint);
        let _ = GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let shell_layout = layout::calculate(
            width,
            height,
            current_dpi(),
            sidebar_expanded_for_width(width),
        );
        let sidebar = native_rect(shell_layout.sidebar);
        FillRect(hdc, &client, theme().terminal);
        draw_title_bar(
            hdc,
            native_rect(shell_layout.title_bar),
            &theme(),
            current_dpi(),
        );
        let sidebar_border = RECT {
            left: (sidebar.right - dpi::scale(1, current_dpi())).max(sidebar.left),
            top: sidebar.top,
            right: sidebar.right,
            bottom: sidebar.bottom,
        };
        let status_rect = native_rect(shell_layout.status_bar);
        let status_border = RECT {
            left: status_rect.left,
            top: status_rect.top,
            right: status_rect.right,
            bottom: status_rect.top + dpi::scale(1, current_dpi()),
        };
        background::paint(
            hdc,
            sidebar,
            sidebar,
            background::Surface::Sidebar,
            &theme(),
        );
        FillRect(hdc, &sidebar_border, theme().border_subtle);
        FillRect(hdc, &status_rect, theme().surface);
        FillRect(hdc, &status_border, theme().border_subtle);
        let _ = EndPaint(hwnd, &paint);
    }
}

fn native_rect(rect: layout::Rect) -> RECT {
    RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.width,
        bottom: rect.y + rect.height,
    }
}

fn sidebar_expanded_for_width(width: i32) -> bool {
    SIDEBAR_EXPANDED.with(Cell::get) && width >= dpi::scale(640, current_dpi())
}

fn theme() -> Theme {
    THEME.with(|theme| *theme.borrow())
}

fn controls() -> Controls {
    CONTROLS.with(|controls| *controls.borrow())
}

fn with_blocks<R>(action: impl FnOnce(&CommandBlocks) -> R) -> R {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active().expect("active terminal session");
        action(active.blocks())
    })
}

fn with_blocks_mut<R>(action: impl FnOnce(&mut CommandBlocks) -> R) -> R {
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut().expect("active terminal session");
        action(active.blocks_mut())
    })
}

fn active_session_id() -> Option<u64> {
    SESSIONS.with(|sessions| sessions.borrow().active_id)
}

fn layout_controls(width: i32, height: i32) {
    let controls = controls();
    let dpi = current_dpi();
    let expanded = sidebar_expanded_for_width(width);
    let direct_input = direct_input_active();
    let layout = layout::calculate(width, height, dpi, expanded);
    let mut output_rect = layout.output;
    if direct_input {
        output_rect.height = (layout.input.y + layout.input.height - output_rect.y).max(0);
    }
    let sidebar_inset = dpi::scale(8, dpi);
    let sidebar_width = (layout.sidebar.width - 2 * sidebar_inset).max(0);
    let caption_w = dpi::scale(layout::CAPTION_BUTTON_WIDTH, dpi);
    let caption_h = dpi::scale(32, dpi).min(layout.title_bar.height);
    let caption_y = (layout.title_bar.height - caption_h) / 2;
    let status_inset = dpi::scale(16, dpi);
    let size_width = dpi::scale(88, dpi);
    let encoding_width = dpi::scale(56, dpi);
    unsafe {
        for (control, rect) in [
            (controls.session_header, layout.session_header),
            (controls.output, output_rect),
            (controls.input_label, layout.input_label),
            (controls.input, layout.input),
            (controls.append, layout.execute_button),
        ] {
            move_rect(control, rect);
        }
        for (index, control) in [controls.minimize, controls.maximize, controls.close_window]
            .iter()
            .enumerate()
        {
            let _ = MoveWindow(
                *control,
                width - (3 - index as i32) * caption_w,
                caption_y,
                caption_w,
                caption_h,
                true,
            );
        }
        let _ = MoveWindow(
            controls.sidebar_toggle,
            sidebar_inset,
            layout.sidebar.y + dpi::scale(12, dpi),
            sidebar_width,
            dpi::scale(40, dpi),
            true,
        );
        let _ = MoveWindow(
            controls.sidebar_terminals,
            sidebar_inset,
            layout.sidebar.y + layout.session_header.height + dpi::scale(16, dpi),
            sidebar_width,
            dpi::scale(52, dpi),
            true,
        );
        let _ = MoveWindow(
            controls.sidebar_profiles,
            sidebar_inset,
            layout.sidebar.y + layout.session_header.height + dpi::scale(92, dpi),
            sidebar_width,
            dpi::scale(48, dpi),
            true,
        );
        let connection_w =
            (layout.status_bar.width - 2 * status_inset - size_width - encoding_width).max(0);
        let _ = MoveWindow(
            controls.status_connection,
            layout.status_bar.x + status_inset,
            layout.status_bar.y + 1,
            connection_w,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        let _ = MoveWindow(
            controls.status_encoding,
            width - status_inset - size_width - encoding_width,
            layout.status_bar.y + 1,
            encoding_width,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        let _ = MoveWindow(
            controls.status_size,
            width - status_inset - size_width,
            layout.status_bar.y + 1,
            size_width,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        for control in [controls.input_label, controls.input, controls.append] {
            let _ = ShowWindow(control, if direct_input { SW_HIDE } else { SW_SHOW });
        }
        let hint = if expanded {
            "Командный режим · Enter — выполнить, Shift+Enter — новая строка"
        } else {
            "Enter — выполнить · Shift+Enter — новая строка"
        };
        let _ = SetWindowTextW(controls.input_label, PCWSTR(to_wide(hint).as_ptr()));
        let _ = InvalidateRect(Some(controls.sidebar_toggle), None, true);
        let _ = InvalidateRect(Some(controls.sidebar_terminals), None, true);
        let _ = InvalidateRect(Some(controls.sidebar_profiles), None, true);
    }
    update_sidebar_toggle_name(expanded);
    update_mode_controls();
    resize_active_terminal();
}

fn move_rect(control: HWND, rect: layout::Rect) {
    unsafe {
        let _ = MoveWindow(control, rect.x, rect.y, rect.width, rect.height, true);
    }
}

fn client_width(hwnd: HWND) -> i32 {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    client.right - client.left
}

fn toggle_sidebar(hwnd: HWND) {
    let expanded = sidebar_expanded_for_width(client_width(hwnd));
    SIDEBAR_EXPANDED.set(!expanded);
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right - client.left, client.bottom - client.top);
        rebuild_tab_controls(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = SetFocus(Some(controls().sidebar_toggle));
    }
}

fn update_sidebar_toggle_name(expanded: bool) {
    let text = if expanded {
        "Свернуть боковую панель"
    } else {
        "Развернуть боковую панель"
    };
    let wide = to_wide(text);
    unsafe {
        let _ = SetWindowTextW(controls().sidebar_toggle, PCWSTR(wide.as_ptr()));
    }
    let _ = set_accessible_name(controls().sidebar_toggle, text);
}

fn direct_input_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.direct_input)
    })
}

fn update_mode_controls() {
    let mode = if direct_input_active() {
        "прямой ввод"
    } else {
        "командный режим"
    };
    let _ = set_accessible_name(
        controls().sidebar_profiles,
        &format!("Профили и действия; {mode}"),
    );
}

fn toggle_input_mode(hwnd: HWND) {
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.direct_input = !active.direct_input;
        }
    });
    let direct_input = direct_input_active();
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right - client.left, client.bottom - client.top);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = SetFocus(Some(if direct_input {
            controls().output
        } else {
            controls().input
        }));
    }
}

fn resize_active_terminal() {
    let output = controls().output;
    if output.0.is_null() {
        return;
    }
    let mut client = RECT::default();
    let mut metrics = TEXTMETRICW::default();
    unsafe {
        let _ = GetClientRect(output, &mut client);
        let hdc = GetDC(Some(output));
        if hdc.0.is_null() {
            return;
        }
        let old_font = SelectObject(hdc, theme().terminal_font.into());
        let _ = GetTextMetricsW(hdc, &mut metrics);
        let _ = SelectObject(hdc, old_font);
        let _ = ReleaseDC(Some(output), hdc);
    }
    if metrics.tmAveCharWidth <= 0
        || metrics.tmHeight <= 0
        || client.right - client.left < metrics.tmAveCharWidth
        || client.bottom - client.top < metrics.tmHeight
    {
        return;
    }
    let cols =
        ((client.right - client.left) / metrics.tmAveCharWidth).clamp(1, u16::MAX as i32) as u16;
    let rows = ((client.bottom - client.top) / metrics.tmHeight).clamp(1, u16::MAX as i32) as u16;
    let size = SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            let _ = active.resize(PtySize::new(cols, rows));
            Some(active.size)
        } else {
            None
        }
    });
    if let Some(size) = size {
        let text = format!("{}×{}", size.cols, size.rows);
        if window_text(controls().status_size) != text {
            unsafe {
                let _ = SetWindowTextW(controls().status_size, PCWSTR(to_wide(&text).as_ptr()));
            }
            let _ =
                set_accessible_name(controls().status_size, &format!("Размер терминала: {text}"));
        }
    }
}

fn create_default_session(hwnd: HWND) {
    let profile = PROFILES.with(|profiles| profiles.borrow().default_profile().clone());
    create_session(hwnd, &profile);
}

fn create_session(hwnd: HWND, profile: &profile::Profile) {
    if let Err(error) = profile.validate() {
        append_output_text(&format!("\r\n[Профиль не запущен] {error}\r\n"));
        return;
    }
    let size = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.size)
            .unwrap_or_default()
    });
    save_active_view();
    let result = SESSIONS.with(|sessions| sessions.borrow_mut().create(profile, size));
    match result {
        Ok(_) => {
            SESSIONS.with(|sessions| {
                if let Some(active) = sessions.borrow_mut().active_mut() {
                    active.buffer = STARTUP_TEXT.to_string();
                }
            });
            load_active_view();
            refresh_layout(hwnd);
            rebuild_tab_controls(hwnd);
            update_connection_status();
            unsafe {
                let _ = SetFocus(Some(if direct_input_active() {
                    controls().output
                } else {
                    controls().input
                }));
            }
        }
        Err(error) => append_output_text(&format!("\r\n[Профиль не запущен] {error}\r\n")),
    }
}

fn activate_session(hwnd: HWND, id: u64) {
    save_active_view();
    if !SESSIONS.with(|sessions| sessions.borrow_mut().activate(id)) {
        return;
    }
    load_active_view();
    refresh_layout(hwnd);
    rebuild_tab_controls(hwnd);
    update_connection_status();
    unsafe {
        let _ = SetFocus(Some(if direct_input_active() {
            controls().output
        } else {
            controls().input
        }));
    }
}

fn close_session(hwnd: HWND, id: u64) {
    let (title, profile_id, alive) = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .get(id)
            .map(|session| {
                (
                    session.title.clone(),
                    session.profile_id.clone(),
                    session.is_alive(),
                )
            })
            .unwrap_or_default()
    });
    if alive {
        let text = to_wide(&format!(
            "Закрыть вкладку «{title}» ({profile_id}) и завершить её процесс?"
        ));
        let answer = unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                w!("Закрытие вкладки"),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        if answer != IDYES {
            return;
        }
    }
    save_active_view();
    if !SESSIONS.with(|sessions| sessions.borrow_mut().close(id)) {
        return;
    }
    if active_session_id().is_some() {
        load_active_view();
    } else {
        set_output_text("");
        unsafe {
            let _ = SetWindowTextW(controls().input, w!(""));
        }
    }
    rebuild_tab_controls(hwnd);
    refresh_layout(hwnd);
    update_connection_status();
}

fn refresh_layout(hwnd: HWND) {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right, client.bottom);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn save_active_view() {
    let selection = output_selection();
    let first_visible =
        unsafe { SendMessageW(controls().output, EM_GETFIRSTVISIBLELINE, None, None).0 as usize };
    let draft = window_text(controls().input);
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.selection = selection;
            active.first_visible = first_visible;
            active.draft = draft;
        }
    });
}

fn load_active_view() {
    let (buffer, draft, selection, first_visible) = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| {
                (
                    active.buffer.clone(),
                    active.draft.clone(),
                    active.selection,
                    active.first_visible,
                )
            })
            .unwrap_or_default()
    });
    set_output_text(&buffer);
    unsafe {
        let _ = SetWindowTextW(controls().input, PCWSTR(to_wide(&draft).as_ptr()));
        SendMessageW(
            controls().output,
            EM_SETSEL,
            Some(WPARAM(selection.0 as usize)),
            Some(LPARAM(selection.1 as isize)),
        );
        let current_first = SendMessageW(controls().output, EM_GETFIRSTVISIBLELINE, None, None).0;
        SendMessageW(
            controls().output,
            EM_LINESCROLL,
            Some(WPARAM(0)),
            Some(LPARAM(first_visible as isize - current_first)),
        );
    }
}

fn rebuild_tab_controls(hwnd: HWND) {
    TAB_CONTROLS.with(|controls| {
        for control in controls.borrow_mut().drain(..) {
            unsafe {
                let _ = DestroyWindow(control.tab);
                let _ = DestroyWindow(control.close);
            }
        }
    });
    let metadata = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        sessions
            .sessions
            .iter()
            .map(|session| {
                (
                    session.id,
                    session.title.clone(),
                    sessions.active_id == Some(session.id),
                )
            })
            .collect::<Vec<_>>()
    });
    let active = metadata
        .iter()
        .position(|(_, _, active)| *active)
        .unwrap_or(0);
    let mut client = RECT::default();
    let module = unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        GetModuleHandleW(None)
    };
    let Ok(module) = module else {
        return;
    };
    let dpi = current_dpi();
    let layout = layout::calculate(
        client.right,
        client.bottom,
        dpi,
        sidebar_expanded_for_width(client.right),
    );
    let strip = layout::tabs(
        layout.session_header,
        dpi,
        metadata.len(),
        active,
        TAB_SCROLL.with(Cell::get),
    );
    TAB_SCROLL.set(strip.first_visible);
    move_rect(controls().new_tab, strip.new_tab);
    unsafe {
        let _ = SetWindowPos(
            controls().new_tab,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    let mut new_controls = Vec::new();
    for placement in strip.tabs {
        let (id, title, active) = &metadata[placement.index];
        let tab = create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(to_wide(title).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_OWNERDRAW as u32),
            hwnd,
            Some(ID_TAB_BASE + *id as usize),
            HINSTANCE(module.0),
        );
        let close = create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("×"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_OWNERDRAW as u32),
            hwnd,
            Some(ID_TAB_CLOSE_BASE + *id as usize),
            HINSTANCE(module.0),
        );
        let (Ok(tab), Ok(close)) = (tab, close) else {
            continue;
        };
        move_rect(tab, placement.tab);
        move_rect(close, placement.close);
        unsafe {
            let _ = SetWindowPos(
                tab,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = SetWindowPos(
                close,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            SendMessageW(
                tab,
                WM_SETFONT,
                Some(WPARAM(theme().ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
            SendMessageW(
                close,
                WM_SETFONT,
                Some(WPARAM(theme().ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let selected = if *active { ", выбрана" } else { "" };
        let _ = set_accessible_name(
            tab,
            &format!(
                "Вкладка {title}, {} из {}{selected}",
                placement.index + 1,
                metadata.len()
            ),
        );
        let _ = set_accessible_name(close, &format!("Закрыть вкладку {title}"));
        new_controls.push(TabControls { tab, close });
    }
    TAB_CONTROLS.with(|controls| *controls.borrow_mut() = new_controls);
}

fn show_profile_menu(hwnd: HWND) {
    show_profile_menu_at(hwnd, controls().sidebar_profiles, true);
}

fn show_profile_menu_at(hwnd: HWND, anchor: HWND, include_actions: bool) {
    let profiles = PROFILES.with(|profiles| profiles.borrow().profiles.clone());
    if profiles.is_empty() {
        return;
    }
    unsafe {
        let Ok(menu) = profile_menu::ProfileMenu::new(
            &profiles,
            direct_input_active(),
            include_actions,
            &theme(),
            current_dpi(),
        ) else {
            return;
        };
        let mut button = RECT::default();
        let _ = GetWindowRect(anchor, &mut button);
        let selected = TrackPopupMenu(
            menu.handle(),
            TPM_RETURNCMD | TPM_NONOTIFY,
            button.left,
            button.bottom,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;
        if selected == MENU_INPUT_MODE {
            toggle_input_mode(hwnd);
            return;
        }
        if let Some(profile) = selected
            .checked_sub(1)
            .and_then(|index| profiles.get(index))
        {
            PROFILES.with(|manager| {
                let mut manager = manager.borrow_mut();
                manager.default_profile_id = profile.id.clone();
                let _ = manager.save(&profiles_path());
            });
            create_session(hwnd, profile);
        }
    }
}

fn start_terminal(hwnd: HWND) {
    let manager = ProfileManager::load_or_default(&profiles_path());
    let profile = manager.default_profile().clone();
    PROFILES.with(|profiles| *profiles.borrow_mut() = manager);
    let result = profile
        .validate()
        .map_err(|error| error.to_string())
        .and_then(|()| {
            SESSIONS.with(|sessions| {
                sessions
                    .borrow_mut()
                    .create(&profile, PtySize::default())
                    .map_err(|error| error.to_string())
            })
        });
    match result {
        Ok(id) => {
            SESSIONS.with(|sessions| {
                if let Some(active) = sessions.borrow_mut().active_mut() {
                    active.buffer = STARTUP_TEXT.to_string();
                }
            });
            set_output_text(STARTUP_TEXT);
            let timer = unsafe { SetTimer(Some(hwnd), OUTPUT_TIMER_ID, OUTPUT_POLL_MS, None) };
            if timer == 0 {
                SESSIONS.with(|sessions| {
                    sessions.borrow_mut().close(id);
                });
                set_output_text("Не удалось запустить обработку вывода PowerShell.");
            }
        }
        Err(error) => set_output_text(&format!("Не удалось запустить PowerShell:\r\n{error}")),
    }
    update_connection_status();
    resize_active_terminal();
    rebuild_tab_controls(hwnd);
}

fn profiles_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| std::env::var_os("TEMP").unwrap_or_default());
    PathBuf::from(base).join("BKS Term").join("profiles.json")
}

fn update_connection_status() {
    let (title, connected) = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| (active.title.clone(), active.is_alive()))
            .unwrap_or_else(|| ("Нет сессии".to_string(), false))
    });
    CONNECTION_STATUS.with(|status| *status.borrow_mut() = Some(connected));
    let state = if connected {
        "Подключено"
    } else {
        "Отключено"
    };
    let text = format!("{title} · {state}");
    if window_text(controls().status_connection) == text {
        return;
    }
    let wide = to_wide(&text);
    unsafe {
        let _ = SetWindowTextW(controls().status_connection, PCWSTR(wide.as_ptr()));
        let _ = InvalidateRect(Some(controls().status_connection), None, true);
    }
}

fn poll_terminal() {
    let active_id = active_session_id();
    for (id, text) in SESSIONS.with(|sessions| sessions.borrow_mut().poll()) {
        if Some(id) == active_id {
            append_output_text(&text);
        }
    }
    if active_id.is_some() {
        let needs_output_start = with_blocks(|blocks| blocks.needs_output_start());
        if needs_output_start {
            let text = window_text(controls().output);
            with_blocks_mut(|blocks| blocks.text_updated(&text));
        }
    }
}

fn set_output_text(text: &str) {
    let text = to_wide(text);
    unsafe {
        let _ = SetWindowTextW(controls().output, PCWSTR(text.as_ptr()));
    }
}

fn append_output_text(text: &str) {
    if text.is_empty() {
        return;
    }
    let output = controls().output;
    let text = to_wide(text);
    unsafe {
        let length = GetWindowTextLengthW(output).max(0) as u32;
        let mut selection_start = 0_u32;
        let mut selection_end = 0_u32;
        SendMessageW(
            output,
            EM_GETSEL,
            Some(WPARAM(&mut selection_start as *mut u32 as usize)),
            Some(LPARAM(&mut selection_end as *mut u32 as isize)),
        );
        let first_visible = SendMessageW(output, EM_GETFIRSTVISIBLELINE, None, None).0;
        let follow_output = selection_start == length && selection_end == length;

        SendMessageW(output, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(length as usize)),
            Some(LPARAM(length as isize)),
        );
        SendMessageW(
            output,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(text.as_ptr() as isize)),
        );
        if follow_output {
            SendMessageW(output, EM_SCROLLCARET, None, None);
        } else {
            SendMessageW(
                output,
                EM_SETSEL,
                Some(WPARAM(selection_start as usize)),
                Some(LPARAM(selection_end as isize)),
            );
            let current_first = SendMessageW(output, EM_GETFIRSTVISIBLELINE, None, None).0;
            SendMessageW(
                output,
                EM_LINESCROLL,
                Some(WPARAM(0)),
                Some(LPARAM(first_visible - current_first)),
            );
        }
        SendMessageW(output, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        let _ = InvalidateRect(Some(output), None, true);
    }
}

fn execute_input() {
    let input = controls().input;
    let text = window_text(input);
    if text.trim().is_empty() || !with_blocks(|blocks| blocks.can_execute()) {
        return;
    }
    let mut command = text.clone().into_bytes();
    command.extend_from_slice(b"\r\n");
    let result = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .ok_or_else(|| "Нет активной сессии.".to_string())
            .and_then(|active| active.write(&command).map_err(|error| error.to_string()))
    });
    match result {
        Ok(()) => {
            with_blocks_mut(|blocks| blocks.execute(&text, ""));
            unsafe {
                let _ = SetWindowTextW(input, w!(""));
                let _ = SetFocus(Some(input));
            }
        }
        Err(error) => append_output_text(&format!("\r\n[Ошибка отправки команды] {error}\r\n")),
    }
}

fn window_text(window: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = GetWindowTextW(window, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

fn handle_app_shortcut(hwnd: HWND, key: usize) -> bool {
    let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let alt = unsafe { GetKeyState(VK_MENU.0 as i32) < 0 };
    if control && key == VK_TAB.0 as usize {
        activate_adjacent_session(hwnd, shift);
        return true;
    }
    if control && shift {
        match key {
            key if key == VK_T.0 as usize => {
                create_default_session(hwnd);
                return true;
            }
            key if key == VK_P.0 as usize => {
                show_profile_menu(hwnd);
                return true;
            }
            key if key == VK_W.0 as usize => {
                if let Some(id) = active_session_id() {
                    close_session(hwnd, id);
                }
                return true;
            }
            _ => {}
        }
    }
    if alt && (0x31..=0x39).contains(&key) {
        activate_session_index(hwnd, key - 0x31);
        return true;
    }
    false
}

fn activate_adjacent_session(hwnd: HWND, previous: bool) {
    let ids = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>()
    });
    if ids.is_empty() {
        return;
    }
    let active_index = active_session_id()
        .and_then(|active| ids.iter().position(|id| *id == active))
        .unwrap_or(0);
    let next = if previous {
        (active_index + ids.len() - 1) % ids.len()
    } else {
        (active_index + 1) % ids.len()
    };
    activate_session(hwnd, ids[next]);
}

fn activate_session_index(hwnd: HWND, index: usize) {
    let id = SESSIONS.with(|sessions| sessions.borrow().sessions.get(index).map(|s| s.id));
    if let Some(id) = id {
        activate_session(hwnd, id);
    }
}

fn handle_direct_key(key: usize) -> bool {
    let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let special = match key {
        key if key == VK_RETURN.0 as usize => SpecialKey::Enter,
        key if key == VK_BACK.0 as usize => SpecialKey::Backspace,
        key if key == VK_TAB.0 as usize => SpecialKey::Tab,
        key if key == VK_ESCAPE.0 as usize => SpecialKey::Escape,
        key if key == VK_UP.0 as usize => SpecialKey::Up,
        key if key == VK_DOWN.0 as usize => SpecialKey::Down,
        key if key == VK_LEFT.0 as usize => SpecialKey::Left,
        key if key == VK_RIGHT.0 as usize => SpecialKey::Right,
        key if key == VK_HOME.0 as usize => SpecialKey::Home,
        key if key == VK_END.0 as usize => SpecialKey::End,
        key if key == VK_DELETE.0 as usize => SpecialKey::Delete,
        key if key == VK_INSERT.0 as usize => SpecialKey::Insert,
        key if key == VK_PRIOR.0 as usize => SpecialKey::PageUp,
        key if key == VK_NEXT.0 as usize => SpecialKey::PageDown,
        key if control && key == VK_C.0 as usize => SpecialKey::CtrlC,
        key if control && key == VK_D.0 as usize => SpecialKey::CtrlD,
        key if control && key == VK_Z.0 as usize => SpecialKey::CtrlZ,
        _ => return false,
    };
    write_active_input(special_key_bytes(special));
    true
}

fn handle_direct_char(unit: u16) {
    if let Some(bytes) = UTF16_ENCODER.with(|encoder| encoder.borrow_mut().push(unit)) {
        write_active_input(&bytes);
    }
}

fn write_active_input(data: &[u8]) {
    let result = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .ok_or_else(|| "Нет активной сессии.".to_string())
            .and_then(|active| active.write(data).map_err(|error| error.to_string()))
    });
    if let Err(error) = result {
        append_output_text(&format!("\r\n[Ошибка прямого ввода] {error}\r\n"));
    }
}

fn handle_block_shortcut(key: usize) -> bool {
    unsafe {
        let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let alt = GetKeyState(VK_MENU.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        if key == VK_C.0 as usize && control && shift && !alt {
            return copy_current_output_block();
        }
        if key != VK_UP.0 as usize && key != VK_DOWN.0 as usize {
            return false;
        }
        if control && !alt && !shift && key == VK_UP.0 as usize {
            return focus_last_output_block();
        }
        if alt && !control && !shift {
            return focus_adjacent_output_block(key == VK_UP.0 as usize);
        }
        false
    }
}

fn focus_last_output_block() -> bool {
    let anchor = with_blocks(|blocks| blocks.last());
    anchor.is_some_and(focus_output_position)
}

fn focus_adjacent_output_block(previous: bool) -> bool {
    let output = controls().output;
    let in_output = unsafe { GetFocus() == output };
    let position = in_output.then(output_selection).map(|(start, _)| start);
    let anchor = with_blocks(|blocks| match (previous, position) {
        (true, Some(position)) => blocks.previous(position),
        (true, None) => blocks.last(),
        (false, Some(position)) => blocks.next(position),
        (false, None) => blocks.next(0),
    });
    anchor.is_some_and(focus_output_position)
}

fn copy_current_output_block() -> bool {
    let output = controls().output;
    if unsafe { GetFocus() != output } {
        return false;
    }
    let (position, _) = output_selection();
    let range = with_blocks(|blocks| blocks.current(position));
    let Some((start, Some(end))) = range else {
        return false;
    };
    if end <= start {
        return false;
    }
    unsafe {
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(start as usize)),
            Some(LPARAM(end as isize)),
        );
        SendMessageW(output, WM_COPY, None, None);
    }
    true
}

fn output_selection() -> (u32, u32) {
    let mut start = 0_u32;
    let mut end = 0_u32;
    unsafe {
        SendMessageW(
            controls().output,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
    }
    (start, end)
}

fn focus_output_position(position: u32) -> bool {
    let output = controls().output;
    unsafe {
        if position as i32 > GetWindowTextLengthW(output) {
            return false;
        }
        let _ = SetFocus(Some(output));
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(position as usize)),
            Some(LPARAM(position as isize)),
        );
        SendMessageW(output, EM_SCROLLCARET, None, None);
    }
    true
}

fn insert_input_newline() {
    let newline = to_wide("\r\n");
    unsafe {
        SendMessageW(
            controls().input,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(newline.as_ptr() as isize)),
        );
        SendMessageW(controls().input, EM_SCROLLCARET, None, None);
    }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_error(context: &str, error: &Error) {
    let text = to_wide(&format!("{context}\r\n{error}"));
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("BKS Term"),
            MB_OK | MB_ICONERROR,
        );
    }
}
