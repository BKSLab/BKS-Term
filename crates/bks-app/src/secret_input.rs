//! Explicit protected input. Values never enter a command draft or history.
use std::cell::RefCell;

use crate::accessibility::set_accessible_name;
use crate::dpi;
use crate::profile_editor::{
    activate, apply_theme, child, control_color, create_dialog, dispatch_owned, erase_background,
    handle_select_all, handle_tab, owns_message, release_theme, restore_focus, show_validation,
};
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_HOTLIGHT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetClientRect, GetWindowTextLengthW, GetWindowTextW, MoveWindow,
    SetWindowPos, SetWindowTextW, BN_CLICKED, BS_OWNERDRAW, ES_AUTOHSCROLL, ES_PASSWORD,
    MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WS_TABSTOP,
};

pub const ID_SECRET: usize = 4301;
pub const ID_SEND: usize = 4302;
pub const ID_CANCEL: usize = 4303;

/// UTF-8 plus one terminal carriage return, ready for Session.write.
/// No Clone/Debug/Display implementation can accidentally copy or log it.
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn from_utf16(units: &[u16]) -> std::result::Result<Self, &'static str> {
        // Three bytes per UTF-16 unit is enough, including surrogate pairs;
        // preallocation avoids reallocating and leaving an old secret buffer.
        let mut result = Self(Vec::with_capacity(
            units.len().saturating_mul(3).saturating_add(1),
        ));
        for character in char::decode_utf16(units.iter().copied()) {
            let character =
                character.map_err(|_| "Не удалось прочитать защищённый ввод. Повторите ввод.")?;
            if matches!(character, '\0' | '\r' | '\n') {
                return Err(
                    "Защищённый ввод должен состоять из одной строки без нулевых символов.",
                );
            }
            let mut encoded = [0; 4];
            result
                .0
                .extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            wipe(&mut encoded);
        }
        result.0.push(b'\r');
        Ok(result)
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

struct SecretUnits(Vec<u16>);
impl Drop for SecretUnits {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

fn wipe<T: Copy + Default>(values: &mut [T]) {
    for value in values {
        unsafe {
            std::ptr::write_volatile(value, T::default());
        }
    }
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    owner_was_enabled: bool,
    context: HWND,
    label: HWND,
    secret: HWND,
    send: HWND,
    cancel: HWND,
    dpi: u32,
    theme: Theme,
}

struct State {
    view: View,
    on_submit: Option<Box<dyn FnOnce(SecretBytes)>>,
}

thread_local! { static INPUT: RefCell<Option<State>> = const { RefCell::new(None) }; }

fn current() -> Option<View> {
    INPUT.with(|input| input.borrow().as_ref().map(|state| state.view))
}

/// Root binds the callback to a concrete session ID and writes directly to
/// that session. This never detects prompts or confirms host keys itself.
pub fn show(
    owner: HWND,
    context_label: &str,
    on_submit: impl FnOnce(SecretBytes) + 'static,
) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.secret);
        return Ok(());
    }
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermSecretInput"),
        w!("Защищённый ввод — BKS Term"),
        Some(window_proc),
        560,
        260,
    )?;
    let mut view = View {
        hwnd,
        owner,
        previous_focus: unsafe { GetFocus() },
        owner_was_enabled: unsafe { IsWindowEnabled(owner).as_bool() },
        context: HWND::default(),
        label: HWND::default(),
        secret: HWND::default(),
        send: HWND::default(),
        cancel: HWND::default(),
        dpi,
        theme: Theme::new(dpi),
    };
    INPUT.with(|input| {
        *input.borrow_mut() = Some(State {
            view,
            on_submit: Some(Box::new(on_submit)),
        })
    });
    let result: Result<()> = (|| {
        view.context = child(
            hwnd,
            instance,
            w!("STATIC"),
            context_label,
            WINDOW_STYLE::default(),
            4300,
        )?;
        view.label = child(
            hwnd,
            instance,
            w!("STATIC"),
            "Пароль или ключевая фраза",
            WINDOW_STYLE::default(),
            4304,
        )?;
        view.secret = child(
            hwnd,
            instance,
            w!("EDIT"),
            "",
            WS_TABSTOP | WINDOW_STYLE((ES_PASSWORD | ES_AUTOHSCROLL) as u32),
            ID_SECRET,
        )?;
        view.send = child(
            hwnd,
            instance,
            w!("BUTTON"),
            "Отправить",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            ID_SEND,
        )?;
        view.cancel = child(
            hwnd,
            instance,
            w!("BUTTON"),
            "Отмена",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            ID_CANCEL,
        )?;
        set_accessible_name(view.secret, "Пароль или ключевая фраза")?;
        set_accessible_name(view.send, "Отправить защищённый ввод в указанную сессию")?;
        set_accessible_name(view.cancel, "Отменить защищённый ввод")?;
        crate::control_state::install(view.send);
        crate::control_state::install(view.cancel);
        Ok(())
    })();
    INPUT.with(|input| {
        if let Some(state) = input.borrow_mut().as_mut() {
            state.view = view;
        }
    });
    if let Err(error) = result {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    apply_fonts(view);
    layout(view);
    unsafe {
        let _ = EnableWindow(owner, false);
    }
    activate(hwnd, view.secret);
    Ok(())
}

/// Consume every own message before root shortcuts/terminal input. In
/// particular, no WM_CHAR or Ctrl+Tab/Ctrl+F from this window reaches PTY.
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
            if handle_tab(message.wParam.0, &[view.secret, view.send, view.cancel]) {
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize
                || (message.wParam.0 == VK_RETURN.0 as usize && focus == view.cancel)
            {
                close(view);
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                submit(view);
                return true;
            }
            if focus == view.secret && handle_select_all(message.wParam.0, focus) {
                return true;
            }
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

fn submit(view: View) {
    let payload = unsafe {
        let mut units = SecretUnits(vec![
            0;
            GetWindowTextLengthW(view.secret).max(0) as usize + 1
        ]);
        let count = GetWindowTextW(view.secret, &mut units.0).max(0) as usize;
        SecretBytes::from_utf16(&units.0[..count])
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(error) => {
            show_validation(view.hwnd, error);
            return;
        }
    };
    let callback = INPUT.with(|input| {
        input
            .borrow_mut()
            .as_mut()
            .and_then(|state| state.on_submit.take())
    });
    // Clear and destroy the password control before the callback can write.
    close(view);
    if let Some(callback) = callback {
        callback(payload);
    }
}

fn close(view: View) {
    unsafe {
        if !view.secret.0.is_null() {
            let _ = SetWindowTextW(view.secret, w!(""));
        }
        let _ = DestroyWindow(view.hwnd);
        if view.owner_was_enabled {
            let _ = EnableWindow(view.owner, true);
        }
    }
    restore_focus(view.owner, view.previous_focus);
}

fn apply_fonts(view: View) {
    apply_theme(
        view.hwnd,
        &view.theme,
        &[
            view.context,
            view.label,
            view.secret,
            view.send,
            view.cancel,
        ],
        &[view.secret],
    );
}

fn layout(view: View) {
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(view.hwnd, &mut rect).is_err() {
            return;
        }
        let margin = dpi::scale(16, view.dpi);
        let width = (rect.right - 2 * margin).max(1);
        let label_y = margin + dpi::scale(42, view.dpi);
        let field_y = label_y + dpi::scale(22, view.dpi);
        let _ = MoveWindow(
            view.context,
            margin,
            margin,
            width,
            dpi::scale(38, view.dpi),
            true,
        );
        let _ = MoveWindow(
            view.label,
            margin,
            label_y,
            width,
            dpi::scale(20, view.dpi),
            true,
        );
        let _ = MoveWindow(
            view.secret,
            margin,
            field_y,
            width,
            dpi::scale(30, view.dpi),
            true,
        );
        let gap = dpi::scale(8, view.dpi);
        let button_w = dpi::scale(120, view.dpi).min((width - gap).max(2) / 2);
        let button_h = dpi::scale(36, view.dpi);
        let y = (rect.bottom - margin - button_h).max(field_y + dpi::scale(38, view.dpi));
        let _ = MoveWindow(
            view.send,
            rect.right - margin - 2 * button_w - gap,
            y,
            button_w,
            button_h,
            true,
        );
        let _ = MoveWindow(
            view.cancel,
            rect.right - margin - button_w,
            y,
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
            let _ = SetFocus(Some(view.secret));
            LRESULT(0)
        }
        WM_CLOSE => {
            close(view);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            if (wparam.0 >> 16) as u32 == BN_CLICKED {
                if id == ID_SEND {
                    submit(view);
                } else if id == ID_CANCEL {
                    close(view);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !view.secret.0.is_null() {
                let _ = SetWindowTextW(view.secret, w!(""));
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
            INPUT.with(|input| {
                if let Some(state) = input.borrow_mut().as_mut() {
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
                    x: dpi::scale(380, view.dpi),
                    y: dpi::scale(240, view.dpi),
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
                if wparam.0 == ID_SEND {
                    "Отправить"
                } else {
                    "Отмена"
                },
            );
            LRESULT(1)
        }
        WM_NCDESTROY => {
            INPUT.with(|input| *input.borrow_mut() = None);
            release_theme(view.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_payload_preserves_unicode_and_spaces_and_adds_only_one_cr() {
        let input: Vec<u16> = "  пароль😀\t ".encode_utf16().collect();
        let bytes = SecretBytes::from_utf16(&input).unwrap();
        assert_eq!(bytes.as_bytes(), "  пароль😀\t \r".as_bytes());
        assert_eq!(SecretBytes::from_utf16(&[]).unwrap().as_bytes(), b"\r");
    }

    #[test]
    fn protected_payload_rejects_multiline_nul_and_broken_utf16() {
        for value in ["first\rsecond", "first\nsecond", "first\0second"] {
            let units: Vec<u16> = value.encode_utf16().collect();
            assert!(SecretBytes::from_utf16(&units).is_err());
        }
        assert!(SecretBytes::from_utf16(&[0xd800]).is_err());
        assert!(SecretBytes::from_utf16(&[0xdc00]).is_err());
    }
}
