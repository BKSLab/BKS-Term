//! App-owned native modal messages and common dialog chrome.
//!
//! DialogBoxIndirectParam keeps modal ownership, Tab/Enter/Escape and native
//! STATIC/BUTTON accessibility. Text, results and destructive confirmations are
//! retained; the palette comes from a local Theme, including startup failures.

use crate::theme::{draw_mode_button, Theme};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{
    CreatePen, DeleteObject, DrawTextW, Ellipse, FillRect, GetDC, GetMonitorInfoW, GetStockObject,
    InvalidateRect, LineTo, MonitorFromWindow, MoveToEx, ReleaseDC, SelectObject, SetBkColor,
    SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, HDC, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, NULL_BRUSH, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::{SS_NOPREFIX, SS_OWNERDRAW};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_HOTLIGHT};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DialogBoxIndirectParamW, EnableMenuItem, EndDialog, GetClientRect,
    GetSystemMenu, GetWindowLongPtrW, GetWindowRect, MessageBoxW, MoveWindow, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, BM_SETSTYLE, BS_DEFPUSHBUTTON, BS_OWNERDRAW,
    DLGC_DEFPUSHBUTTON, DLGC_UNDEFPUSHBUTTON, DM_SETDEFID, DS_MODALFRAME, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_READONLY, GWLP_USERDATA, HMENU, IDCANCEL, IDNO, IDOK, IDYES, MB_OK,
    MB_OKCANCEL, MB_YESNO, MB_YESNOCANCEL, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, MF_BYCOMMAND,
    MF_GRAYED, SC_CLOSE, SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORDLG, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC,
    WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_GETDLGCODE, WM_INITDIALOG, WM_NCDESTROY,
    WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_THEMECHANGED, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_GROUP, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const DECORATION: usize = 0x424b_5344;
const BODY: usize = 4701;
const ICON: usize = 4702;
const MESSAGE_BUTTON: usize = 0x424b_5342;

thread_local! {
    // Borrowed resources: owners call decorate_dialog before retiring old Theme.
    static DIALOGS: RefCell<HashMap<usize, Theme>> = RefCell::new(HashMap::new());
}

/// Common neutral title/border and native combo list colors. This does not
/// alter the main window's glass backend, opacity or native window styles.
pub fn decorate_dialog(hwnd: HWND, theme: &Theme) {
    unsafe {
        let color = |value: u32| {
            if theme.high_contrast {
                0xffff_ffff
            } else {
                value
            }
        };
        for (attribute, value) in [
            (
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                u32::from(!theme.high_contrast),
            ),
            (DWMWA_WINDOW_CORNER_PREFERENCE, 2),
            (DWMWA_CAPTION_COLOR, color(theme.colors.window.0)),
            (DWMWA_TEXT_COLOR, color(theme.colors.text_primary.0)),
            (DWMWA_BORDER_COLOR, color(crate::theme::BORDER.0)),
        ] {
            set_attribute(hwnd, attribute, value);
        }
        if SetWindowSubclass(hwnd, Some(decoration_proc), DECORATION, 0).as_bool() {
            DIALOGS.with(|dialogs| {
                dialogs.borrow_mut().insert(hwnd.0 as usize, *theme);
            });
        }
    }
}

unsafe fn set_attribute(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: u32) {
    let _ = DwmSetWindowAttribute(hwnd, attribute, (&value as *const u32).cast(), 4);
}

unsafe extern "system" fn decoration_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_CTLCOLORLISTBOX {
        if let Some(theme) =
            DIALOGS.with(|dialogs| dialogs.borrow().get(&(hwnd.0 as usize)).copied())
        {
            let dc = HDC(wparam.0 as *mut _);
            SetTextColor(dc, theme.colors.text_primary);
            SetBkColor(dc, theme.colors.surface);
            return LRESULT(theme.surface.0 as isize);
        }
    }
    if message == WM_NCDESTROY {
        DIALOGS.with(|dialogs| {
            dialogs.borrow_mut().remove(&(hwnd.0 as usize));
        });
        let _ = RemoveWindowSubclass(hwnd, Some(decoration_proc), DECORATION);
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

#[derive(Clone, Copy)]
struct Button {
    result: MESSAGEBOX_RESULT,
    label: &'static str,
}

fn buttons(flags: MESSAGEBOX_STYLE) -> Option<Vec<Button>> {
    let row = |result, label| Button { result, label };
    Some(match flags.0 & 0x0f {
        value if value == MB_OK.0 => vec![row(IDOK, "ОК")],
        value if value == MB_OKCANCEL.0 => vec![row(IDOK, "ОК"), row(IDCANCEL, "Отмена")],
        value if value == MB_YESNO.0 => vec![row(IDYES, "&Да"), row(IDNO, "&Нет")],
        value if value == MB_YESNOCANCEL.0 => vec![
            row(IDYES, "&Да"),
            row(IDNO, "&Нет"),
            row(IDCANCEL, "Отмена"),
        ],
        _ => return None,
    })
}

fn cancel_result(buttons: &[Button]) -> Option<MESSAGEBOX_RESULT> {
    if buttons.iter().any(|button| button.result == IDCANCEL) {
        Some(IDCANCEL)
    } else if buttons.len() == 1 && buttons[0].result == IDOK {
        Some(IDOK)
    } else {
        None
    }
}

struct State {
    theme: Cell<Theme>,
    dpi: Cell<u32>,
    text: Vec<u16>,
    title: Vec<u16>,
    buttons: Vec<Button>,
    default: usize,
    flags: MESSAGEBOX_STYLE,
    body: Cell<HWND>,
    icon: Cell<HWND>,
    controls: [Cell<HWND>; 3],
    owner: Option<HWND>,
}

/// Same arguments/results as MessageBoxW for the app's supported message types.
/// Unknown types and allocation/window failures retain the native error path.
/// # Safety
/// `text` and `title` must point to valid NUL-terminated UTF-16 strings or be null.
pub unsafe fn show(
    owner: Option<HWND>,
    text: PCWSTR,
    title: PCWSTR,
    flags: MESSAGEBOX_STYLE,
) -> MESSAGEBOX_RESULT {
    let Some(buttons) = buttons(flags) else {
        return MessageBoxW(owner, text, title, flags);
    };
    let dpi = owner
        .map(|hwnd| GetDpiForWindow(hwnd))
        .unwrap_or_else(|| GetDpiForSystem())
        .max(96);
    let theme = Theme::new(dpi);
    let mut state = State {
        theme: Cell::new(theme),
        dpi: Cell::new(dpi),
        text: if text.is_null() {
            vec![0]
        } else {
            text.as_wide().iter().copied().chain([0]).collect()
        },
        title: if title.is_null() {
            vec![0]
        } else {
            title.as_wide().iter().copied().chain([0]).collect()
        },
        default: (((flags.0 >> 8) & 3) as usize).min(buttons.len() - 1),
        buttons,
        flags,
        body: Cell::new(HWND::default()),
        icon: Cell::new(HWND::default()),
        controls: std::array::from_fn(|_| Cell::new(HWND::default())),
        owner,
    };
    let template = dialog_template(&state.title);
    let instance = GetModuleHandleW(None)
        .ok()
        .map(|module| HINSTANCE(module.0));
    let result = DialogBoxIndirectParamW(
        instance,
        template.as_ptr().cast(),
        owner,
        Some(dialog_proc),
        LPARAM((&mut state as *mut State) as isize),
    );
    crate::profile_editor::release_theme(state.theme.get());
    if result <= 0 {
        MessageBoxW(owner, text, title, flags)
    } else {
        MESSAGEBOX_RESULT(result as i32)
    }
}

fn dialog_template(title: &[u16]) -> Vec<u32> {
    let style = (WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN).0 | DS_MODALFRAME as u32;
    // Standard DLGTEMPLATE: 18-byte header, followed by menu/class/title words.
    let mut words = vec![
        style as u16,
        (style >> 16) as u16,
        0,
        0,
        0,
        0,
        0,
        340,
        180,
        0,
        0,
    ];
    words.extend_from_slice(title);
    if words.len() % 2 != 0 {
        words.push(0);
    }
    // A Vec<u32> guarantees the DWORD alignment required by DialogBoxIndirectParam.
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16))
        .collect()
}

unsafe fn create_child(
    hwnd: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    id: usize,
) -> HWND {
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        text,
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        0,
        0,
        Some(hwnd),
        Some(HMENU(id as *mut _)),
        None,
        None,
    )
    .unwrap_or_default()
}

unsafe fn initialize(hwnd: HWND, state: &State) -> bool {
    let theme = state.theme.get();
    let s = |value| crate::dpi::scale(value, state.dpi.get());
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMonitorInfoW(
        MonitorFromWindow(state.owner.unwrap_or(hwnd), MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let work = monitor.rcWork;
    let width = s(560).min((work.right - work.left - s(32)).max(s(320)));
    let mut measured = RECT {
        right: width - s(96),
        ..Default::default()
    };
    let dc = GetDC(Some(hwnd));
    let old = SelectObject(dc, theme.ui_font.into());
    let mut text = state.text[..state.text.len() - 1].to_vec();
    DrawTextW(
        dc,
        &mut text,
        &mut measured,
        DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
    );
    SelectObject(dc, old);
    ReleaseDC(Some(hwnd), dc);
    let max_body = ((work.bottom - work.top) * 2 / 3 - s(120)).max(s(100));
    let body_height = measured.bottom.max(s(50)).min(max_body);
    let long = measured.bottom > max_body;
    state.body.set(create_child(
        hwnd,
        if long { w!("EDIT") } else { w!("STATIC") },
        PCWSTR(state.text.as_ptr()),
        if long {
            WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32)
                | WS_VSCROLL
                | WS_TABSTOP
        } else {
            WINDOW_STYLE(SS_NOPREFIX.0)
        },
        BODY,
    ));
    if matches!(state.flags.0 & 0xf0, 0x10 | 0x20 | 0x30 | 0x40) {
        state.icon.set(create_child(
            hwnd,
            w!("STATIC"),
            PCWSTR::null(),
            WINDOW_STYLE(SS_OWNERDRAW.0),
            ICON,
        ));
    }
    for (index, button) in state.buttons.iter().enumerate() {
        let label: Vec<u16> = button.label.encode_utf16().chain([0]).collect();
        state.controls[index].set(create_child(
            hwnd,
            w!("BUTTON"),
            PCWSTR(label.as_ptr()),
            WINDOW_STYLE(BS_OWNERDRAW as u32)
                | WS_TABSTOP
                | if index == 0 {
                    WS_GROUP
                } else {
                    WINDOW_STYLE::default()
                },
            button.result.0 as usize,
        ));
        crate::control_state::install(state.controls[index].get());
        let _ = SetWindowSubclass(
            state.controls[index].get(),
            Some(button_proc),
            MESSAGE_BUTTON,
            usize::from(index == state.default),
        );
    }
    if state.body.get().0.is_null()
        || state
            .controls
            .iter()
            .take(state.buttons.len())
            .any(|control| control.get().0.is_null())
    {
        let _ = EndDialog(hwnd, -1);
        return false;
    }
    apply_fonts(hwnd, state);
    SendMessageW(
        hwnd,
        DM_SETDEFID,
        Some(WPARAM(state.buttons[state.default].result.0 as usize)),
        None,
    );
    let mut outer = RECT {
        right: width,
        bottom: body_height + s(112),
        ..Default::default()
    };
    let _ = AdjustWindowRectExForDpi(
        &mut outer,
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
        false,
        WINDOW_EX_STYLE::default(),
        state.dpi.get(),
    );
    let mut owner = work;
    if let Some(parent) = state.owner {
        let _ = GetWindowRect(parent, &mut owner);
    }
    let w = (outer.right - outer.left).min(work.right - work.left);
    let h = (outer.bottom - outer.top).min(work.bottom - work.top);
    let x = (owner.left + (owner.right - owner.left - w) / 2)
        .clamp(work.left, (work.right - w).max(work.left));
    let y = (owner.top + (owner.bottom - owner.top - h) / 2)
        .clamp(work.top, (work.bottom - h).max(work.top));
    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    if cancel_result(&state.buttons).is_none() {
        let _ = EnableMenuItem(
            GetSystemMenu(hwnd, false),
            SC_CLOSE,
            MF_BYCOMMAND | MF_GRAYED,
        );
    }
    layout(hwnd, state);
    let _ = SetFocus(Some(state.controls[state.default].get()));
    false
}

unsafe fn apply_fonts(hwnd: HWND, state: &State) {
    let theme = state.theme.get();
    for control in std::iter::once(state.body.get()).chain(state.controls.iter().map(Cell::get)) {
        if !control.0.is_null() {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme.ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
    crate::ui_menu::attach_edit(state.body.get());
    decorate_dialog(hwnd, &theme);
}

unsafe fn layout(hwnd: HWND, state: &State) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let s = |value| crate::dpi::scale(value, state.dpi.get());
    let margin = s(24);
    let body_x = margin
        + if state.icon.get().0.is_null() {
            0
        } else {
            s(48)
        };
    let button_y = (rc.bottom - margin - s(36)).max(margin);
    let _ = MoveWindow(
        state.body.get(),
        body_x,
        margin,
        (rc.right - body_x - margin).max(1),
        (button_y - margin - s(20)).max(1),
        true,
    );
    if !state.icon.get().0.is_null() {
        let _ = MoveWindow(state.icon.get(), margin, margin, s(32), s(32), true);
    }
    let button_width = s(112);
    let start = rc.right - margin - (button_width + s(10)) * state.buttons.len() as i32 + s(10);
    for (index, control) in state.controls.iter().take(state.buttons.len()).enumerate() {
        let _ = MoveWindow(
            control.get(),
            start + index as i32 * (button_width + s(10)),
            button_y,
            button_width,
            s(36),
            true,
        );
    }
}

/// Owner-draw buttons still participate in the native dialog manager's default
/// button protocol. Without DLGC_*PUSHBUTTON, Enter on focused No becomes IDOK.
unsafe extern "system" fn button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    default: usize,
) -> LRESULT {
    if message == WM_GETDLGCODE {
        let base = DefSubclassProc(hwnd, message, wparam, lparam).0 as u32;
        return LRESULT(
            ((base & !(DLGC_DEFPUSHBUTTON | DLGC_UNDEFPUSHBUTTON))
                | if default != 0 {
                    DLGC_DEFPUSHBUTTON
                } else {
                    DLGC_UNDEFPUSHBUTTON
                }) as isize,
        );
    }
    if message == BM_SETSTYLE {
        let is_default = wparam.0 & 0x0f == BS_DEFPUSHBUTTON as usize;
        let _ = SetWindowSubclass(
            hwnd,
            Some(button_proc),
            MESSAGE_BUTTON,
            usize::from(is_default),
        );
        // Keep GDI owner drawing while honoring the logical default-button state.
        return DefSubclassProc(
            hwnd,
            message,
            WPARAM((wparam.0 & !0x0f) | BS_OWNERDRAW as usize),
            lparam,
        );
    }
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(button_proc), MESSAGE_BUTTON);
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

/// Decorative geometry uses the same line weight as the app's shell icons.
/// The native body carries the complete message; the symbol adds no tab stop.
unsafe fn draw_symbol(item: &DRAWITEMSTRUCT, theme: &Theme, flags: MESSAGEBOX_STYLE) {
    FillRect(item.hDC, &item.rcItem, theme.window);
    let kind = flags.0 & 0xf0;
    let color = if theme.high_contrast {
        theme.colors.text_primary
    } else {
        match kind {
            0x10 => theme.colors.status_error,
            0x30 => COLORREF(0x006c_c5f5), // restrained amber, #F5C56C
            _ => theme.colors.text_secondary,
        }
    };
    let size = (item.rcItem.right - item.rcItem.left).min(item.rcItem.bottom - item.rcItem.top);
    let x = |value: i32| item.rcItem.left + value * size / 32;
    let y = |value: i32| item.rcItem.top + value * size / 32;
    let pen = CreatePen(PS_SOLID, (size / 16).max(1), color);
    let old_pen = SelectObject(item.hDC, pen.into());
    let old_brush = SelectObject(item.hDC, GetStockObject(NULL_BRUSH));
    let _ = Ellipse(item.hDC, x(3), y(3), x(29), y(29));
    let line = |ax, ay, bx, by| {
        let _ = MoveToEx(item.hDC, x(ax), y(ay), None);
        let _ = LineTo(item.hDC, x(bx), y(by));
    };
    match kind {
        0x10 => {
            line(11, 11, 22, 22);
            line(21, 11, 10, 22);
        }
        0x20 => {
            // Compact question mark without relying on an icon font.
            for (ax, ay, bx, by) in [
                (11, 12, 13, 9),
                (13, 9, 19, 9),
                (19, 9, 21, 12),
                (21, 12, 20, 15),
                (20, 15, 16, 18),
                (16, 18, 16, 20),
            ] {
                line(ax, ay, bx, by);
            }
            line(16, 23, 16, 25);
        }
        0x30 => {
            line(16, 9, 16, 19);
            line(16, 22, 16, 24);
        }
        _ => {
            line(16, 9, 16, 11);
            line(16, 14, 16, 24);
        }
    }
    SelectObject(item.hDC, old_brush);
    SelectObject(item.hDC, old_pen);
    let _ = DeleteObject(pen.into());
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if message == WM_INITDIALOG {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, lparam.0);
        return isize::from(initialize(hwnd, &*(lparam.0 as *const State)));
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    if pointer.is_null() {
        return 0;
    }
    let state = &*pointer;
    let theme = state.theme.get();
    match message {
        WM_COMMAND => {
            let id = MESSAGEBOX_RESULT((wparam.0 & 0xffff) as i32);
            let result = if id == IDCANCEL {
                cancel_result(&state.buttons)
            } else if id == IDOK && !state.buttons.iter().any(|button| button.result == IDOK) {
                let focus = GetFocus();
                state
                    .controls
                    .iter()
                    .position(|control| control.get() == focus)
                    .and_then(|index| state.buttons.get(index))
                    .or_else(|| state.buttons.get(state.default))
                    .map(|button| button.result)
            } else {
                state
                    .buttons
                    .iter()
                    .find(|button| button.result == id)
                    .map(|button| button.result)
            };
            if let Some(result) = result {
                let _ = EndDialog(hwnd, result.0 as isize);
            }
            1
        }
        WM_CLOSE => {
            if let Some(result) = cancel_result(&state.buttons) {
                let _ = EndDialog(hwnd, result.0 as isize);
            }
            1
        }
        WM_ERASEBKGND => {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(HDC(wparam.0 as *mut _), &rc, theme.window);
            1
        }
        WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let dc = HDC(wparam.0 as *mut _);
            SetTextColor(dc, theme.colors.text_primary);
            SetBkColor(dc, theme.colors.window);
            theme.window.0 as isize
        }
        WM_DRAWITEM if lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if item.CtlID == ICON as u32 {
                draw_symbol(&item, &theme, state.flags);
                return 1;
            }
            if let Some(button) = state
                .buttons
                .iter()
                .find(|button| button.result.0 as u32 == item.CtlID)
            {
                if crate::control_state::is_hot(item.hwndItem) {
                    item.itemState.0 |= ODS_HOTLIGHT.0;
                }
                draw_mode_button(&item, &theme, button.label);
                1
            } else {
                0
            }
        }
        WM_SIZE => {
            layout(hwnd, state);
            1
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DPICHANGED => {
            state.dpi.set(GetDpiForWindow(hwnd).max(96));
            let old = state.theme.replace(Theme::new(state.dpi.get()));
            apply_fonts(hwnd, state);
            if message == WM_DPICHANGED && lparam.0 != 0 {
                let rc = *(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            layout(hwnd, state);
            crate::profile_editor::release_theme(old);
            let _ = InvalidateRect(Some(hwnd), None, true);
            1
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetWindowLongW, BS_PUSHBUTTON, GWL_STYLE,
    };

    #[test]
    fn native_button_keeps_owner_drawing_when_dialog_changes_default() {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Нет"),
                WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                100,
                30,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert!(SetWindowSubclass(hwnd, Some(button_proc), MESSAGE_BUTTON, 0).as_bool());
            for (style, expected) in [
                (BS_PUSHBUTTON, DLGC_UNDEFPUSHBUTTON),
                (BS_DEFPUSHBUTTON, DLGC_DEFPUSHBUTTON),
                (BS_PUSHBUTTON, DLGC_UNDEFPUSHBUTTON),
            ] {
                SendMessageW(hwnd, BM_SETSTYLE, Some(WPARAM(style as usize)), None);
                let code = SendMessageW(hwnd, WM_GETDLGCODE, None, None).0 as u32;
                assert_eq!(code & (DLGC_DEFPUSHBUTTON | DLGC_UNDEFPUSHBUTTON), expected);
                assert_eq!(GetWindowLongW(hwnd, GWL_STYLE) & 0x0f, BS_OWNERDRAW);
            }
            DestroyWindow(hwnd).unwrap();
        }
    }

    #[test]
    fn destructive_yes_no_does_not_gain_an_escape_confirmation() {
        let buttons = buttons(MB_YESNO).unwrap();
        assert_eq!(
            buttons
                .iter()
                .map(|button| button.result.0)
                .collect::<Vec<_>>(),
            vec![IDYES.0, IDNO.0]
        );
        assert_eq!(cancel_result(&buttons), None);
        assert_eq!(cancel_result(&self::buttons(MB_OK).unwrap()), Some(IDOK));
        assert_eq!(
            cancel_result(&self::buttons(MB_OKCANCEL).unwrap()),
            Some(IDCANCEL)
        );
    }

    #[test]
    fn native_dialog_template_is_dword_aligned_with_utf16_caption() {
        let title: Vec<_> = "Сообщение".encode_utf16().chain([0]).collect();
        let template = dialog_template(&title);
        assert_eq!(template.as_ptr() as usize % 4, 0);
        let words: Vec<u16> = template
            .iter()
            .flat_map(|word| [*word as u16, (word >> 16) as u16])
            .collect();
        assert_eq!(&words[11..11 + title.len()], title.as_slice());
        assert_eq!(words[4], 0); // controls are created during WM_INITDIALOG
    }
}
