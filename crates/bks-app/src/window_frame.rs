use crate::{dpi, layout};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsZoomed, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
    HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER,
    SM_CXSIZEFRAME,
};

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
