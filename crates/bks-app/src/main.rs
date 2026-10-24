#![windows_subsystem = "windows"]

mod command_blocks;
mod terminal_output;

use std::cell::RefCell;
use std::ffi::c_void;

use bks_pty::{PtySession, PtySize};
use command_blocks::CommandBlocks;
use terminal_output::{TerminalEvent, TerminalOutput};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DrawTextW, FillRect, FrameRect, InvalidateRect, SetBkColor,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FF_MODERN, FIXED_PITCH, FW_NORMAL, HBRUSH, HDC, HFONT,
    OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    SetWindowTheme, DRAWITEMSTRUCT, EM_GETFIRSTVISIBLELINE, EM_GETSEL, EM_LINESCROLL,
    EM_REPLACESEL, EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETSEL, ODS_FOCUS, ODS_SELECTED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_C, VK_CONTROL, VK_DOWN, VK_MENU, VK_RETURN, VK_SHIFT,
    VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowTextLengthW,
    GetWindowTextW, IsDialogMessageW, KillTimer, LoadCursorW, LoadIconW, MessageBoxW, MoveWindow,
    PostQuitMessage, RegisterClassW, SendMessageW, SetTimer, SetWindowTextW, ShowWindow,
    TranslateMessage, BN_CLICKED, BS_OWNERDRAW, CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_READONLY, ES_WANTRETURN, HMENU, ICON_BIG, ICON_SMALL, IDC_ARROW, MB_ICONERROR, MB_OK,
    MINMAXINFO, MSG, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_COPY, WM_CREATE,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_SETFOCUS, WM_SETFONT, WM_SETICON, WM_SETREDRAW, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW,
    WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_OUTPUT: usize = 1001;
const ID_INPUT: usize = 1002;
const ID_APPEND: usize = 1003;
const OUTPUT_TIMER_ID: usize = 1;
const OUTPUT_POLL_MS: u32 = 50;
const POWERSHELL_COMMAND: &str = r#"powershell.exe -NoLogo -NoProfile -NoExit -Command "function global:prompt { [char]27 + ']133;A' + [char]27 + '\PS ' + $executionContext.SessionState.Path.CurrentLocation + '> ' + [char]27 + ']133;B' + [char]27 + '\' }""#;
const MARGIN: i32 = 12;
const GAP: i32 = 8;
const LABEL_HEIGHT: i32 = 20;
const INPUT_HEIGHT: i32 = 64;
const BUTTON_HEIGHT: i32 = 28;
const BUTTON_WIDTH: i32 = 170;
const COLOR_BACKGROUND: COLORREF = COLORREF(0x0017_110D);
const COLOR_SURFACE: COLORREF = COLORREF(0x0009_0401);
const COLOR_FOREGROUND: COLORREF = COLORREF(0x009B_FF7C);
const COLOR_MUTED: COLORREF = COLORREF(0x009E_948B);

#[derive(Clone, Copy, Default)]
struct Controls {
    output_label: HWND,
    output: HWND,
    input_label: HWND,
    input: HWND,
    append: HWND,
}

#[derive(Clone, Copy, Default)]
struct Theme {
    background: HBRUSH,
    surface: HBRUSH,
    button: HBRUSH,
    button_active: HBRUSH,
    border: HBRUSH,
    accent: HBRUSH,
    font: HFONT,
}

thread_local! {
    static CONTROLS: RefCell<Controls> = RefCell::new(Controls::default());
    static THEME: RefCell<Theme> = RefCell::new(Theme::default());
    static TERMINAL: RefCell<Option<PtySession>> = const { RefCell::new(None) };
    static TERMINAL_OUTPUT: RefCell<TerminalOutput> = RefCell::new(TerminalOutput::new());
    static COMMAND_BLOCKS: RefCell<CommandBlocks> = RefCell::new(CommandBlocks::new());
}

fn main() {
    if let Err(error) = run() {
        show_error("Не удалось запустить BKS Term", &error);
    }
}

fn run() -> Result<()> {
    unsafe {
        let theme = initialize_theme();
        THEME.with(|current| *current.borrow_mut() = theme);
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let icon = LoadIconW(Some(instance), w!("BKS_TERM"))?;
        let window_class = WNDCLASSW {
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance,
            hbrBackground: theme.background,
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
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            600,
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
            if message.message == WM_KEYDOWN && message.wParam.0 == VK_TAB.0 as usize {
                let focus = GetFocus();
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let next = if focus == controls().output {
                    Some(if shift {
                        controls().append
                    } else {
                        controls().input
                    })
                } else if focus == controls().input {
                    Some(if shift {
                        controls().output
                    } else {
                        controls().append
                    })
                } else {
                    None
                };
                if let Some(next) = next {
                    let _ = SetFocus(Some(next));
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
        WM_CREATE => match create_controls(hwnd) {
            Ok(()) => LRESULT(0),
            Err(_) => LRESULT(-1),
        },
        WM_SETFOCUS => {
            let input = controls().input;
            if !input.0.is_null() {
                let _ = SetFocus(Some(input));
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            layout_controls(width, height);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: 480, y: 320 };
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => control_color(wparam, COLOR_FOREGROUND, COLOR_SURFACE, theme().surface),
        WM_CTLCOLORSTATIC => {
            if HWND(lparam.0 as *mut c_void) == controls().output {
                control_color(wparam, COLOR_FOREGROUND, COLOR_SURFACE, theme().surface)
            } else {
                control_color(wparam, COLOR_MUTED, COLOR_BACKGROUND, theme().background)
            }
        }
        WM_DRAWITEM => {
            if wparam.0 == ID_APPEND {
                draw_append_button(&*(lparam.0 as *const DRAWITEMSTRUCT));
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_COMMAND => {
            let control_id = wparam.0 & 0xffff;
            let notification = (wparam.0 >> 16) & 0xffff;
            if control_id == ID_APPEND && notification == BN_CLICKED as usize {
                execute_input();
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_TIMER => {
            if wparam.0 == OUTPUT_TIMER_ID {
                poll_terminal();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), OUTPUT_TIMER_ID);
            TERMINAL.with(|terminal| terminal.borrow_mut().take());
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
        output_label: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Вывод"),
            WS_CHILD | WS_VISIBLE,
            parent,
            None,
            instance,
        )?,
        output: create_control(
            WS_EX_CLIENTEDGE,
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
            WS_EX_CLIENTEDGE,
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
    };
    CONTROLS.with(|controls| {
        *controls.borrow_mut() = new_controls;
    });
    unsafe {
        SendMessageW(
            new_controls.output,
            EM_SETLIMITTEXT,
            Some(WPARAM(0x7FFF_FFFE)),
            None,
        );
        for control in [
            new_controls.output_label,
            new_controls.output,
            new_controls.input_label,
            new_controls.input,
            new_controls.append,
        ] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme().font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
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
        CreateWindowExW(
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
        )
    }
}

fn initialize_theme() -> Theme {
    unsafe {
        Theme {
            background: CreateSolidBrush(COLOR_BACKGROUND),
            surface: CreateSolidBrush(COLOR_SURFACE),
            button: CreateSolidBrush(COLORREF(0x0022_1B16)),
            button_active: CreateSolidBrush(COLORREF(0x002D_2621)),
            border: CreateSolidBrush(COLORREF(0x003D_3630)),
            accent: CreateSolidBrush(COLOR_FOREGROUND),
            font: CreateFontW(
                -16,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                (FIXED_PITCH.0 | FF_MODERN.0) as u32,
                w!("Consolas"),
            ),
        }
    }
}

fn apply_dark_title(hwnd: HWND) {
    let dark = 1_u32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
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

fn draw_append_button(item: &DRAWITEMSTRUCT) {
    let theme = theme();
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let fill = if pressed {
        theme.button_active
    } else {
        theme.button
    };
    let border = if focused { theme.accent } else { theme.border };
    let mut text: Vec<u16> = "Выполнить".encode_utf16().collect();
    let mut rect: RECT = item.rcItem;
    unsafe {
        FillRect(item.hDC, &item.rcItem, fill);
        SetTextColor(item.hDC, COLOR_FOREGROUND);
        SetBkMode(item.hDC, TRANSPARENT);
        DrawTextW(
            item.hDC,
            &mut text,
            &mut rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        FrameRect(item.hDC, &item.rcItem, border);
    }
}

fn theme() -> Theme {
    THEME.with(|theme| *theme.borrow())
}

fn controls() -> Controls {
    CONTROLS.with(|controls| *controls.borrow())
}

fn layout_controls(width: i32, height: i32) {
    let controls = controls();
    let content_width = (width - 2 * MARGIN).max(0);
    let output_y = MARGIN + LABEL_HEIGHT + GAP;
    let input_y = (height - MARGIN - INPUT_HEIGHT).max(MARGIN);
    let input_label_y = (input_y - GAP - LABEL_HEIGHT).max(MARGIN);
    let output_height = (input_label_y - GAP - output_y).max(0);
    let button_x = (width - MARGIN - BUTTON_WIDTH).max(MARGIN);
    let input_width = (button_x - GAP - MARGIN).max(0);

    unsafe {
        let _ = MoveWindow(
            controls.output_label,
            MARGIN,
            MARGIN,
            content_width,
            LABEL_HEIGHT,
            true,
        );
        let _ = MoveWindow(
            controls.output,
            MARGIN,
            output_y,
            content_width,
            output_height,
            true,
        );
        let _ = MoveWindow(
            controls.input_label,
            MARGIN,
            input_label_y,
            content_width,
            LABEL_HEIGHT,
            true,
        );
        let _ = MoveWindow(
            controls.input,
            MARGIN,
            input_y,
            input_width,
            INPUT_HEIGHT,
            true,
        );
        let _ = MoveWindow(
            controls.append,
            button_x,
            input_y,
            BUTTON_WIDTH,
            BUTTON_HEIGHT,
            true,
        );
    }
}

fn start_terminal(hwnd: HWND) {
    match PtySession::spawn(POWERSHELL_COMMAND, PtySize::default()) {
        Ok(session) => {
            TERMINAL.with(|terminal| *terminal.borrow_mut() = Some(session));
            let timer = unsafe { SetTimer(Some(hwnd), OUTPUT_TIMER_ID, OUTPUT_POLL_MS, None) };
            if timer == 0 {
                TERMINAL.with(|terminal| terminal.borrow_mut().take());
                set_output_text("Не удалось запустить обработку вывода PowerShell.");
            }
        }
        Err(error) => set_output_text(&format!("Не удалось запустить PowerShell:\r\n{error}")),
    }
}

fn poll_terminal() {
    let bytes = TERMINAL.with(|terminal| {
        terminal
            .borrow()
            .as_ref()
            .map(PtySession::drain_output)
            .unwrap_or_default()
    });
    if bytes.is_empty() {
        return;
    }
    let chunk = TERMINAL_OUTPUT.with(|output| output.borrow_mut().push_chunk(&bytes));
    let base = unsafe { GetWindowTextLengthW(controls().output).max(0) as u32 };
    COMMAND_BLOCKS.with(|blocks| {
        let mut blocks = blocks.borrow_mut();
        for event in chunk.events {
            let position = base + event_position(&event);
            match event {
                TerminalEvent::PromptStart(_) => blocks.prompt_started(position),
                TerminalEvent::CommandStart(_) => blocks.command_started(position),
            }
        }
    });
    append_output_text(&chunk.text);
    let needs_output_start = COMMAND_BLOCKS.with(|blocks| blocks.borrow().needs_output_start());
    if needs_output_start {
        let text = window_text(controls().output);
        COMMAND_BLOCKS.with(|blocks| blocks.borrow_mut().text_updated(&text));
    }
}

fn event_position(event: &TerminalEvent) -> u32 {
    match event {
        TerminalEvent::PromptStart(position) | TerminalEvent::CommandStart(position) => {
            *position as u32
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
    if text.trim().is_empty() || !COMMAND_BLOCKS.with(|blocks| blocks.borrow().can_execute()) {
        return;
    }
    let mut command = text.clone().into_bytes();
    command.extend_from_slice(b"\r\n");
    let result = TERMINAL.with(|terminal| {
        let terminal = terminal.borrow();
        match terminal.as_ref() {
            Some(session) => session
                .write_input(&command)
                .map_err(|error| error.to_string()),
            None => Err("PowerShell не запущен.".to_string()),
        }
    });
    match result {
        Ok(()) => {
            COMMAND_BLOCKS.with(|blocks| blocks.borrow_mut().execute(&text, ""));
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
    let anchor = COMMAND_BLOCKS.with(|blocks| blocks.borrow().last());
    anchor.is_some_and(focus_output_position)
}

fn focus_adjacent_output_block(previous: bool) -> bool {
    let output = controls().output;
    let in_output = unsafe { GetFocus() == output };
    let position = in_output.then(output_selection).map(|(start, _)| start);
    let anchor = COMMAND_BLOCKS.with(|blocks| {
        let blocks = blocks.borrow();
        match (previous, position) {
            (true, Some(position)) => blocks.previous(position),
            (true, None) => blocks.last(),
            (false, Some(position)) => blocks.next(position),
            (false, None) => blocks.next(0),
        }
    });
    anchor.is_some_and(focus_output_position)
}

fn copy_current_output_block() -> bool {
    let output = controls().output;
    if unsafe { GetFocus() != output } {
        return false;
    }
    let (position, _) = output_selection();
    let range = COMMAND_BLOCKS.with(|blocks| blocks.borrow().current(position));
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
