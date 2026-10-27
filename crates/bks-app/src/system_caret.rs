//! Keep the native text caret available to accessibility without painting it.
//!
//! HideCaret maintains a counter. Only hide a currently visible caret, so
//! repeated polling cannot require multiple ShowCaret calls to restore it.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    GetGUIThreadInfo, HideCaret, GUITHREADINFO, GUI_CARETBLINKING,
};

/// Hide the calling UI thread's visible caret only when `hwnd` owns it.
///
/// The caret object and its position remain available to accessibility.
/// Returns whether HideCaret succeeded; an already hidden or unrelated caret
/// is left untouched. Call on the window's UI thread after native EDIT updates.
/// If Win32 currently reports no visible caret, a later poll can retry.
pub fn hide_visible(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_err()
            || info.hwndCaret != hwnd
            || !info.flags.contains(GUI_CARETBLINKING)
        {
            return false;
        }
        HideCaret(Some(hwnd)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateCaret, CreateWindowExW, DestroyCaret, DestroyWindow, SetCaretPos, ShowCaret,
        WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
    };

    struct HiddenEdit {
        parent: HWND,
        edit: HWND,
    }

    impl HiddenEdit {
        fn new() -> Self {
            unsafe {
                let parent = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    120,
                    80,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD,
                    0,
                    0,
                    100,
                    60,
                    Some(parent),
                    None,
                    None,
                    None,
                );
                match edit {
                    Ok(edit) => Self { parent, edit },
                    Err(error) => {
                        let _ = DestroyWindow(parent);
                        panic!("create private EDIT: {error}");
                    }
                }
            }
        }
    }

    impl Drop for HiddenEdit {
        fn drop(&mut self) {
            unsafe {
                // Only the private test thread owns this caret and window.
                let _ = DestroyCaret();
                let _ = DestroyWindow(self.parent);
            }
        }
    }

    fn state() -> GUITHREADINFO {
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info).unwrap() };
        info
    }

    fn visible(info: &GUITHREADINFO) -> bool {
        info.flags.contains(GUI_CARETBLINKING)
    }

    #[test]
    fn native_caret_hiding_preserves_object_position_and_show_counter() {
        // A dedicated UI thread and never-shown windows isolate the native
        // caret. No SetFocus, activation, input, or user window is involved.
        std::thread::spawn(|| unsafe {
            let fixture = HiddenEdit::new();
            assert!(!hide_visible(fixture.edit));
            CreateCaret(fixture.edit, None, 2, 17).unwrap();
            SetCaretPos(13, 7).unwrap();
            ShowCaret(Some(fixture.edit)).unwrap();
            let before = state();
            assert_eq!(before.hwndCaret, fixture.edit);
            assert!(visible(&before));

            // A different HWND must not hide this thread's visible caret.
            assert!(!hide_visible(fixture.parent));
            assert!(!hide_visible(HWND::default()));
            assert!(visible(&state()));

            assert!(hide_visible(fixture.edit));
            let hidden = state();
            assert!(!visible(&hidden));
            assert_eq!(hidden.hwndCaret, before.hwndCaret);
            assert_eq!(hidden.rcCaret, before.rcCaret);
            for _ in 0..16 {
                assert!(!hide_visible(fixture.edit));
            }
            let repeated = state();
            assert_eq!(repeated.hwndCaret, before.hwndCaret);
            assert_eq!(repeated.rcCaret, before.rcCaret);

            // One ShowCaret suffices after arbitrarily many hide polls.
            ShowCaret(Some(fixture.edit)).unwrap();
            assert!(visible(&state()));
            assert!(hide_visible(fixture.edit));
            assert!(!visible(&state()));

            // Native EDIT can replace the caret after font/size changes.
            CreateCaret(fixture.edit, None, 3, 21).unwrap();
            SetCaretPos(29, 11).unwrap();
            ShowCaret(Some(fixture.edit)).unwrap();
            let recreated = state();
            assert!(visible(&recreated));
            assert_eq!(recreated.hwndCaret, fixture.edit);
            assert_ne!(recreated.rcCaret, before.rcCaret);
            assert!(hide_visible(fixture.edit));
            let hidden_again = state();
            assert!(!visible(&hidden_again));
            assert_eq!(hidden_again.hwndCaret, recreated.hwndCaret);
            assert_eq!(hidden_again.rcCaret, recreated.rcCaret);
        })
        .join()
        .unwrap();
    }
}
