use std::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{WM_MOUSEMOVE, WM_NCDESTROY};

thread_local! {
    static HOT_CONTROL: Cell<HWND> = Cell::new(HWND::default());
}

pub fn is_hot(hwnd: HWND) -> bool {
    HOT_CONTROL.with(|current| current.get() == hwnd)
}

pub fn install(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(button_proc), 1, 0);
    }
}

unsafe extern "system" fn button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        WM_MOUSEMOVE if !is_hot(hwnd) => {
            HOT_CONTROL.with(|current| {
                let previous = current.replace(hwnd);
                if !previous.0.is_null() {
                    let _ = InvalidateRect(Some(previous), None, false);
                }
            });
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                ..Default::default()
            };
            let _ = TrackMouseEvent(&mut tracking);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        WM_MOUSELEAVE | WM_NCDESTROY => {
            HOT_CONTROL.with(|current| {
                if current.get() == hwnd {
                    current.set(HWND::default());
                    if message != WM_NCDESTROY {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            });
            if message == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(button_proc), 1);
            }
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}
