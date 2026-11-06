//! Paint the native vertical scrollbar without replacing its HWND, range,
//! hit targets or accessibility. Its track shares the terminal's glass.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{Error, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, GetStockObject, GetWindowDC,
    IntersectClipRect, Polyline, ReleaseDC, RestoreDC, RoundRect, SaveDC, SelectObject, HDC,
    NULL_PEN, PS_SOLID,
};
use windows::Win32::UI::Controls::EM_REPLACESEL;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetScrollBarInfo, GetWindowRect, OBJID_VSCROLL, PRF_NONCLIENT, SCROLLBARINFO,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCPAINT, WM_PAINT, WM_PRINT, WM_SETFONT, WM_SETREDRAW,
    WM_SETTEXT, WM_SIZE, WM_THEMECHANGED, WM_VSCROLL,
};

const SUBCLASS: usize = 0x424b5353;
const INVISIBLE_OR_OFFSCREEN: u32 = 0x8000 | 0x10000;
const UNAVAILABLE: u32 = 1;

thread_local! {
    static REDRAW: RefCell<HashMap<usize, bool>> = RefCell::new(HashMap::new());
}

pub fn attach(hwnd: HWND) -> Result<()> {
    if !unsafe { SetWindowSubclass(hwnd, Some(scroll_proc), SUBCLASS, 0).as_bool() } {
        return Err(Error::from_thread());
    }
    REDRAW.with_borrow_mut(|states| states.insert(hwnd.0 as usize, true));
    Ok(())
}

unsafe extern "system" fn scroll_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        REDRAW.with_borrow_mut(|states| states.remove(&(hwnd.0 as usize)));
        let _ = RemoveWindowSubclass(hwnd, Some(scroll_proc), SUBCLASS);
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }
    if message == WM_SETREDRAW {
        REDRAW.with_borrow_mut(|states| states.insert(hwnd.0 as usize, wparam.0 != 0));
    }
    let enabled = !crate::theme().high_contrast;
    let redraw =
        REDRAW.with_borrow(|states| states.get(&(hwnd.0 as usize)).copied().unwrap_or(true));
    if message == WM_NCPAINT && enabled && (!redraw || paint(hwnd, None)) {
        return LRESULT(0);
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if enabled && redraw {
        if message == WM_PRINT && lparam.0 & PRF_NONCLIENT as isize != 0 {
            paint(hwnd, Some(HDC(wparam.0 as *mut _)));
        } else if matches!(
            message,
            WM_PAINT
                | WM_SIZE
                | WM_VSCROLL
                | WM_MOUSEWHEEL
                | WM_NCMOUSEMOVE
                | WM_NCMOUSELEAVE
                | WM_NCLBUTTONDOWN
                | WM_NCLBUTTONUP
                | WM_LBUTTONUP
                | WM_SETTEXT
                | WM_SETFONT
                | WM_THEMECHANGED
                | WM_SETREDRAW
                | EM_REPLACESEL
        ) || (message == WM_MOUSEMOVE && wparam.0 & 1 != 0)
        {
            paint(hwnd, None);
        }
    }
    result
}

fn paint(hwnd: HWND, target: Option<HDC>) -> bool {
    unsafe {
        let mut info = SCROLLBARINFO {
            cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        let mut window = RECT::default();
        if GetScrollBarInfo(hwnd, OBJID_VSCROLL, &mut info).is_err()
            || GetWindowRect(hwnd, &mut window).is_err()
            || info.rgstate[0] & INVISIBLE_OR_OFFSCREEN != 0
        {
            return false;
        }
        let screen = info.rcScrollBar;
        let rect = RECT {
            left: screen.left - window.left,
            top: screen.top - window.top,
            right: screen.right - window.left,
            bottom: screen.bottom - window.top,
        };
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return false;
        }
        let mut cursor = POINT::default();
        let hot = GetCursorPos(&mut cursor).is_ok()
            && cursor.x >= screen.left
            && cursor.x < screen.right
            && cursor.y >= screen.top
            && cursor.y < screen.bottom;
        let dc = target.unwrap_or_else(|| GetWindowDC(Some(hwnd)));
        if dc.0.is_null() {
            return false;
        }
        let saved = SaveDC(dc);
        IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
        let painted = if let Some(mut buffer) = crate::glass_buffer::GlassBuffer::new(dc, rect) {
            let theme = crate::theme();
            FillRect(buffer.hdc(), &rect, theme.terminal);
            if crate::backdrop::active() {
                buffer.seal_background(crate::theme::TERMINAL_GLASS_ALPHA);
            }
            draw_thumb(buffer.hdc(), rect, &info, GetDpiForWindow(hwnd), hot);
            buffer.finish()
        } else {
            false
        };
        if saved != 0 {
            let _ = RestoreDC(dc, saved);
        }
        if target.is_none() {
            ReleaseDC(Some(hwnd), dc);
        }
        painted
    }
}

fn draw_thumb(dc: HDC, rect: RECT, info: &SCROLLBARINFO, dpi: u32, hot: bool) {
    let available = info.rgstate[1] & UNAVAILABLE == 0 || info.rgstate[5] & UNAVAILABLE == 0;
    if !available {
        return;
    }
    let scale = |v| crate::dpi::scale(v, dpi).max(1);
    let color = if hot {
        crate::theme::FOCUS_RING
    } else {
        crate::theme::TEXT_MUTED
    };
    let center = (rect.left + rect.right) / 2;
    let width = scale(if hot { 6 } else { 4 }).min(rect.right - rect.left);
    let top = (rect.top + info.xyThumbTop).clamp(rect.top, rect.bottom);
    let bottom = (rect.top + info.xyThumbBottom).clamp(top, rect.bottom);
    unsafe {
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(dc, brush.into());
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        if bottom > top {
            let _ = RoundRect(
                dc,
                center - width / 2,
                top,
                center - width / 2 + width,
                bottom,
                width,
                width,
            );
        }
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(brush.into());
        // Arrow hit targets remain native. Show their small chevrons on hover;
        // the idle surface contains only the slim thumb, with no black rail.
        if hot {
            let pen = CreatePen(PS_SOLID, scale(1), color);
            let previous = SelectObject(dc, pen.into());
            for (index, y, direction) in [
                (1, rect.top + info.dxyLineButton / 2, -1),
                (5, rect.bottom - info.dxyLineButton / 2, 1),
            ] {
                if info.rgstate[index] & UNAVAILABLE == 0 {
                    let _ = Polyline(
                        dc,
                        &[
                            POINT {
                                x: center - scale(3),
                                y: y - direction * scale(1),
                            },
                            POINT {
                                x: center,
                                y: y + direction * scale(2),
                            },
                            POINT {
                                x: center + scale(3),
                                y: y - direction * scale(1),
                            },
                        ],
                    );
                }
            }
            SelectObject(dc, previous);
            let _ = DeleteObject(pen.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Controls::EM_GETFIRSTVISIBLELINE;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetScrollInfo, GetWindowLongPtrW, SendMessageW,
        SetWindowTextW, ES_AUTOVSCROLL, ES_MULTILINE, GWL_STYLE, HTVSCROLL, SB_LINEDOWN, SB_VERT,
        SCROLLINFO, SIF_ALL, WINDOW_STYLE, WM_NCHITTEST, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_POPUP, WS_VISIBLE, WS_VSCROLL,
    };

    #[test]
    fn native_scroll_range_hit_target_and_line_scroll_survive_styling() {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("EDIT"),
                w!(""),
                WS_POPUP
                    | WS_VISIBLE
                    | WS_VSCROLL
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
                -32000,
                -32000,
                320,
                120,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let text: Vec<u16> = (0..120)
                .map(|line| format!("Line {line}\r\n"))
                .collect::<String>()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            SetWindowTextW(hwnd, PCWSTR(text.as_ptr())).unwrap();
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let mut before = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            GetScrollInfo(hwnd, SB_VERT, &mut before).unwrap();
            attach(hwnd).unwrap();
            SendMessageW(hwnd, WM_NCPAINT, Some(WPARAM(1)), None);
            let mut after = before;
            GetScrollInfo(hwnd, SB_VERT, &mut after).unwrap();
            assert_eq!(
                (before.nMin, before.nMax, before.nPage, before.nPos),
                (after.nMin, after.nMax, after.nPage, after.nPos)
            );
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
            let mut bar = SCROLLBARINFO {
                cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
                ..Default::default()
            };
            GetScrollBarInfo(hwnd, OBJID_VSCROLL, &mut bar).unwrap();
            let x = (bar.rcScrollBar.left + bar.rcScrollBar.right) / 2;
            let y = (bar.rcScrollBar.top + bar.rcScrollBar.bottom) / 2;
            let point = ((y as u16 as u32) << 16) | x as u16 as u32;
            assert_eq!(
                SendMessageW(hwnd, WM_NCHITTEST, None, Some(LPARAM(point as isize))).0,
                HTVSCROLL as isize
            );
            let first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
            SendMessageW(hwnd, WM_VSCROLL, Some(WPARAM(SB_LINEDOWN.0 as usize)), None);
            assert!(SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0 > first);
            DestroyWindow(hwnd).unwrap();
            assert!(!REDRAW.with_borrow(|states| states.contains_key(&(hwnd.0 as usize))));
        }
    }
}
