//! Native, accessible editing menus using the same renderer as profile menus.
//! Subclassing changes only WM_CONTEXTMENU; commands still reach the control's
//! existing input/clipboard handlers. No global menu hooks or system menus.

use crate::profile_menu::{self, Command, ProfileMenu};
use crate::theme::Theme;
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::UI::Controls::RichEdit::EM_CANPASTE;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_CANUNDO, EM_GETSEL, EM_SETSEL};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCaretPos, GetClassNameW, GetClientRect, GetWindowLongW, GetWindowTextLengthW, SendMessageW,
    TrackPopupMenu, ES_PASSWORD, ES_READONLY, GWL_STYLE, TPM_NONOTIFY, TPM_RETURNCMD, WM_CLEAR,
    WM_CONTEXTMENU, WM_COPY, WM_CUT, WM_DRAWITEM, WM_MEASUREITEM, WM_MENUCHAR, WM_NCDESTROY,
    WM_PASTE, WM_UNDO,
};

const SUBCLASS: usize = 0x424b_534d;
const SELECT_ALL: u32 = 0xfffe;

thread_local! {
    // TrackPopupMenu is synchronous, including its accessibility/keyboard loop.
    static ACTIVE: RefCell<Vec<(HWND, Theme, u32)>> = const { RefCell::new(Vec::new()) };
}

/// Attach after the control's existing subclasses, on its owning UI thread.
/// Other classes are deliberately ignored (notably password UI containers).
pub fn attach_edit(hwnd: HWND) {
    let class = class_name(hwnd);
    if class == "edit" || class.starts_with("richedit") {
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(edit_proc), SUBCLASS, 0);
        }
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut name = [0_u16; 96];
    let count = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
    String::from_utf16_lossy(&name[..count]).to_ascii_lowercase()
}

fn commands(hwnd: HWND) -> Vec<Command<'static>> {
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let writable = IsWindowEnabled(hwnd).as_bool() && style & ES_READONLY as u32 == 0;
        let password = style & ES_PASSWORD as u32 != 0;
        let (mut first, mut last) = (0_u32, 0_u32);
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM((&mut first as *mut u32) as usize)),
            Some(LPARAM((&mut last as *mut u32) as isize)),
        );
        let selected = first != last;
        let paste = if class_name(hwnd).starts_with("richedit") {
            SendMessageW(hwnd, EM_CANPASTE, None, None).0 != 0
        } else {
            IsClipboardFormatAvailable(13).is_ok() || IsClipboardFormatAvailable(1).is_ok()
        };
        [
            (
                WM_UNDO,
                "&Отменить (Ctrl+Z)",
                writable && SendMessageW(hwnd, EM_CANUNDO, None, None).0 != 0,
            ),
            (0, "", true),
            (
                WM_CUT,
                "В&ырезать (Ctrl+X)",
                writable && selected && !password,
            ),
            (WM_COPY, "&Копировать (Ctrl+C)", selected && !password),
            (WM_PASTE, "&Вставить (Ctrl+V)", writable && paste),
            (WM_CLEAR, "&Удалить", writable && selected),
            (0, "", true),
            (
                SELECT_ALL,
                "Выделить в&сё (Ctrl+A)",
                GetWindowTextLengthW(hwnd) > 0,
            ),
        ]
        .into_iter()
        .map(|(id, label, enabled)| Command {
            id,
            label,
            enabled,
            checked: false,
        })
        .collect()
    }
}

fn show(hwnd: HWND, lparam: LPARAM) {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let theme = Theme::new(dpi);
    let menu = ProfileMenu::commands(&commands(hwnd), &theme, dpi);
    if let Ok(menu) = menu {
        let mut point = POINT {
            x: (lparam.0 as u32 & 0xffff) as i16 as i32,
            y: ((lparam.0 as u32 >> 16) & 0xffff) as i16 as i32,
        };
        unsafe {
            if lparam.0 == -1 {
                let mut bounds = RECT::default();
                let _ = GetClientRect(hwnd, &mut bounds);
                if GetCaretPos(&mut point).is_err() {
                    point = POINT { x: 12, y: 12 };
                }
                point.x = point.x.clamp(0, bounds.right.max(0));
                point.y = (point.y + crate::dpi::scale(20, dpi)).clamp(0, bounds.bottom.max(0));
                let _ = ClientToScreen(hwnd, &mut point);
            }
            ACTIVE.with(|active| active.borrow_mut().push((hwnd, theme, dpi)));
            let selected = TrackPopupMenu(
                menu.handle(),
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                Some(0),
                hwnd,
                None,
            )
            .0 as u32;
            ACTIVE.with(|active| {
                active.borrow_mut().pop();
            });
            match selected {
                SELECT_ALL => {
                    SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                }
                WM_UNDO | WM_CUT | WM_COPY | WM_PASTE | WM_CLEAR => {
                    SendMessageW(hwnd, selected, None, None);
                }
                _ => {}
            }
        }
    }
    crate::profile_editor::release_theme(theme);
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_CONTEXTMENU {
        show(hwnd, lparam);
        return LRESULT(0);
    }
    let context = ACTIVE.with(|active| {
        active
            .borrow()
            .iter()
            .rev()
            .find(|(owner, _, _)| *owner == hwnd)
            .copied()
    });
    if let Some((_, theme, dpi)) = context {
        match message {
            WM_MEASUREITEM if profile_menu::measure(lparam, dpi) => return LRESULT(1),
            WM_DRAWITEM
                if lparam.0 != 0
                    && profile_menu::draw(&*(lparam.0 as *const DRAWITEMSTRUCT), &theme, dpi) =>
            {
                return LRESULT(1)
            }
            WM_MENUCHAR => {
                if let Some(result) = profile_menu::menu_char(wparam, lparam) {
                    return result;
                }
            }
            _ => {}
        }
    }
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), SUBCLASS);
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    };

    #[test]
    fn native_readonly_and_password_fields_keep_clipboard_restrictions() {
        unsafe {
            for (style, copy, cut) in [
                (ES_READONLY, true, false),
                (ES_PASSWORD, false, false),
                (0, true, true),
            ] {
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!("sample"),
                    WINDOW_STYLE(style as u32),
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
                SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(3)));
                let rows = commands(hwnd);
                assert_eq!(
                    rows.iter().find(|row| row.id == WM_COPY).unwrap().enabled,
                    copy
                );
                assert_eq!(
                    rows.iter().find(|row| row.id == WM_CUT).unwrap().enabled,
                    cut
                );
                assert!(
                    rows.iter()
                        .find(|row| row.id == SELECT_ALL)
                        .unwrap()
                        .enabled
                );
                DestroyWindow(hwnd).unwrap();
            }
        }
    }
}
