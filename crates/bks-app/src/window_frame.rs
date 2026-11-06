use crate::{dpi, layout};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetClientRect, IsIconic, IsWindowVisible, IsZoomed, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
    NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, WM_NCACTIVATE, WM_NCPAINT, WM_SETICON,
    WM_SETREDRAW, WM_SETTEXT,
};

// UxTheme's separate caption/frame paint paths are not exposed by windows-rs.
// They can be sent during icon/title changes even with DWM NC rendering disabled.
const WM_NCUAHDRAWCAPTION: u32 = 0x00ae;
const WM_NCUAHDRAWFRAME: u32 = 0x00af;

/// Only used for the Acrylic custom frame. Keep native window state and commands,
/// while preventing legacy non-client painting over the application's glass.
pub fn custom_frame_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    unsafe {
        match message {
            WM_NCPAINT | WM_NCUAHDRAWCAPTION | WM_NCUAHDRAWFRAME => Some(LRESULT(0)),
            WM_NCACTIVATE if !IsIconic(hwnd).as_bool() => {
                // TRUE permits activation/deactivation. DefWindowProc(-1) still
                // paints a classic frame with DWMNCRP_DISABLED on Windows 11.
                Some(LRESULT(1))
            }
            WM_SETTEXT | WM_SETICON => {
                // These default handlers may paint the caption directly, without
                // WM_NCPAINT or UAH messages. A scoped native redraw lock also
                // prevents that path, while preserving GetWindowText/GetIcon.
                // WM_SETREDRAW clears WS_VISIBLE: don't re-enable a hidden or
                // already redraw-locked window.
                let visible = IsWindowVisible(hwnd).as_bool();
                if visible {
                    DefWindowProcW(hwnd, WM_SETREDRAW, WPARAM(0), LPARAM(0));
                }
                let result = DefWindowProcW(hwnd, message, wparam, lparam);
                if visible {
                    DefWindowProcW(hwnd, WM_SETREDRAW, WPARAM(1), LPARAM(0));
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                Some(result)
            }
            _ => None,
        }
    }
}

pub fn calculate_client(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    unsafe {
        // Keep the normal overlapped-window styles: Windows still owns the system
        // menu, resizing, double-click maximize, taskbar, and caption commands.
        if IsZoomed(hwnd).as_bool() {
            let dpi = GetDpiForWindow(hwnd);
            let inset = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
            let client = &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0];
            client.left += inset;
            client.top += inset;
            client.right -= inset;
            client.bottom -= inset;
        }
    }
    LRESULT(0)
}

pub fn hit_test(hwnd: HWND, lparam: LPARAM, dpi: u32) -> LRESULT {
    let mut point = POINT {
        x: (lparam.0 as u32 & 0xffff) as i16 as i32,
        y: ((lparam.0 as u32 >> 16) & 0xffff) as i16 as i32,
    };
    let mut client = RECT::default();
    unsafe {
        let _ = ScreenToClient(hwnd, &mut point);
        let _ = GetClientRect(hwnd, &mut client);
        LRESULT(hit_test_at(
            client.right,
            client.bottom,
            point.x,
            point.y,
            dpi,
            IsZoomed(hwnd).as_bool(),
        ) as isize)
    }
}

fn hit_test_at(w: i32, h: i32, x: i32, y: i32, dpi: u32, maximized: bool) -> u32 {
    let border = dpi::scale(6, dpi);
    if !maximized {
        let left = x < border;
        let right = x >= w - border;
        let top = y < border;
        let bottom = y >= h - border;
        match (left, right, top, bottom) {
            (true, _, true, _) => return HTTOPLEFT,
            (_, true, true, _) => return HTTOPRIGHT,
            (true, _, _, true) => return HTBOTTOMLEFT,
            (_, true, _, true) => return HTBOTTOMRIGHT,
            (true, _, _, _) => return HTLEFT,
            (_, true, _, _) => return HTRIGHT,
            (_, _, true, _) => return HTTOP,
            (_, _, _, true) => return HTBOTTOM,
            _ => {}
        }
    }
    if y < dpi::scale(layout::TITLE_HEIGHT, dpi) {
        let button = dpi::scale(layout::CAPTION_BUTTON_WIDTH, dpi);
        if x >= w - 2 * button && x < w - button {
            // The native maximize hit target preserves the Windows 11 Snap flyout.
            HTMAXBUTTON
        } else if x < w - 3 * button {
            HTCAPTION
        } else {
            HTCLIENT
        }
    } else {
        HTCLIENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetWindowTextW, LoadIconW, SendMessageW, ICON_BIG,
        IDI_APPLICATION, WM_GETICON, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
        WS_VISIBLE,
    };

    #[test]
    fn frame_redraw_guard_preserves_native_title_icon_and_callers_visibility() {
        for visible in [false, true] {
            unsafe {
                // An off-screen, nonactivating native fixture needs no desktop
                // input and cannot cover the user's windows.
                let hwnd = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("original"),
                    if visible {
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE
                    } else {
                        WS_OVERLAPPEDWINDOW
                    },
                    -32000,
                    -32000,
                    200,
                    120,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let icon = LoadIconW(None, IDI_APPLICATION).unwrap();
                custom_frame_message(
                    hwnd,
                    WM_SETICON,
                    WPARAM(ICON_BIG as usize),
                    LPARAM(icon.0 as isize),
                );
                assert_eq!(
                    SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_BIG as usize)), None).0,
                    icon.0 as isize
                );
                assert_eq!(IsWindowVisible(hwnd).as_bool(), visible);

                let set_title = || {
                    assert_eq!(
                        custom_frame_message(
                            hwnd,
                            WM_SETTEXT,
                            WPARAM(0),
                            LPARAM(w!("updated title").as_ptr() as isize),
                        ),
                        Some(LRESULT(1))
                    );
                    let mut title = [0_u16; 64];
                    let len = GetWindowTextW(hwnd, &mut title) as usize;
                    assert_eq!(String::from_utf16_lossy(&title[..len]), "updated title");
                };
                set_title();
                assert_eq!(IsWindowVisible(hwnd).as_bool(), visible);

                // Preserve a caller's existing redraw lock. Unlocking here would
                // both expose a hidden window and permit native caption drawing.
                DefWindowProcW(hwnd, WM_SETREDRAW, WPARAM(0), LPARAM(0));
                set_title();
                assert!(!IsWindowVisible(hwnd).as_bool());
                DestroyWindow(hwnd).unwrap();
            }
        }
    }

    #[test]
    fn caption_retains_drag_resize_and_native_maximize_targets() {
        for dpi in [96, 144, 192] {
            let s = |v| dpi::scale(v, dpi);
            let hit = |x, y| hit_test_at(s(1280), s(800), s(x), s(y), dpi, false);
            assert_eq!(hit(1, 1), HTTOPLEFT);
            assert_eq!(hit(1279, 799), HTBOTTOMRIGHT);
            assert_eq!(hit(300, 25), HTCAPTION);
            assert_eq!(hit(1150, 25), HTCLIENT);
            assert_eq!(hit(1210, 25), HTMAXBUTTON);
            assert_eq!(hit(1260, 25), HTCLIENT);
            assert_eq!(hit(300, 90), HTCLIENT);
            assert_eq!(hit_test_at(s(1280), s(800), 0, 0, dpi, true), HTCAPTION);
        }
    }
}
