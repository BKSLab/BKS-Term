//! Native RichEdit/EDIT presentation over opaque or translucent backgrounds.
//!
//! RichEdit continues to own text, formatting, selection, scrolling and its
//! accessibility provider. Only WM_PAINT is redirected through a BGRA buffer;
//! native WM_PRINTCLIENT draws the same text over a denser glass background.
//! Ordinary printing and all accessibility messages keep their native route.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{Error, IUnknown, Interface, Result};
use windows::Win32::Foundation::{COLORREF, E_FAIL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GdiFlush, GetCurrentObject, GetObjectW, GetStockObject,
    InvalidateRect, SetDCBrushColor, DC_BRUSH, DIBSECTION, HBRUSH, HDC, OBJ_BITMAP, PAINTSTRUCT,
};
use windows::Win32::UI::Controls::RichEdit::{
    ITextDocument, EM_EXSETSEL, EM_GETOLEINTERFACE, EM_HIDESELECTION, EM_REDO, EM_SETBKGNDCOLOR,
    EM_SETCHARFORMAT, EM_SETPARAFORMAT, EM_SETSCROLLPOS, EM_SETTEXTEX, EM_SETZOOM, EM_STREAMIN,
};
use windows::Win32::UI::Controls::{
    EM_LINESCROLL, EM_REPLACESEL, EM_SCROLL, EM_SCROLLCARET, EM_SETRECT, EM_SETRECTNP, EM_SETSEL,
    EM_UNDO,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, PRF_CLIENT, WM_CHAR,
    WM_CLEAR, WM_CUT, WM_ERASEBKGND, WM_HSCROLL, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY,
    WM_PAINT, WM_PASTE, WM_PRINTCLIENT, WM_SETFOCUS, WM_SETFONT, WM_SETREDRAW, WM_SETTEXT, WM_SIZE,
    WM_SYSKEYDOWN, WM_UNDO, WM_VSCROLL, WS_EX_TRANSPARENT,
};

use crate::glass_buffer::GlassBuffer;
use crate::text_decorations::Decorations;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;

const SUBCLASS: usize = 0x424b5347;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    RichEdit,
    Edit,
}

#[derive(Clone)]
struct Control {
    kind: ControlKind,
    originally_transparent: bool,
    enabled: bool,
    redraw: bool,
    painting: bool,
    background: COLORREF,
    document: Option<ITextDocument>,
    frozen: bool,
}

thread_local! {
    static CONTROLS: RefCell<HashMap<usize, Control>> = RefCell::new(HashMap::new());
}

/// Attach once, on the HWND's owning UI thread, after creating a RichEdit.
/// The native HWND and its accessible identity are never replaced.
pub fn attach(hwnd: HWND) -> Result<()> {
    let theme = crate::theme();
    attach_with_policy(
        hwnd,
        ControlKind::RichEdit,
        !theme.high_contrast,
        theme.colors.terminal,
    )
}

/// Attach the plain Win32 input EDIT. Its standard background is converted in
/// the private DIB; HWND hit testing and native text opacity are unchanged.
pub fn attach_edit(hwnd: HWND) -> Result<()> {
    let theme = crate::theme();
    attach_with_policy(
        hwnd,
        ControlKind::Edit,
        !theme.high_contrast,
        theme.colors.terminal,
    )
}

/// Refresh after changing the backdrop or the theme. Disabling restores the
/// original transparency style while preserving unrelated extended styles.
pub fn refresh(hwnd: HWND) {
    let theme = crate::theme();
    configure(hwnd, !theme.high_contrast, theme.colors.terminal);
}

fn attach_with_policy(
    hwnd: HWND,
    kind: ControlKind,
    enabled: bool,
    background: COLORREF,
) -> Result<()> {
    if state(hwnd).is_none() {
        let document = if kind == ControlKind::RichEdit {
            Some(native_document(hwnd)?)
        } else {
            None
        };
        let originally_transparent =
            unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize != 0 };
        CONTROLS.with_borrow_mut(|controls| {
            controls.insert(
                hwnd.0 as usize,
                Control {
                    kind,
                    originally_transparent,
                    enabled: false,
                    redraw: true,
                    painting: false,
                    background,
                    document,
                    frozen: false,
                },
            );
        });
        if !unsafe { SetWindowSubclass(hwnd, Some(control_proc), SUBCLASS, 0).as_bool() } {
            CONTROLS.with_borrow_mut(|controls| controls.remove(&(hwnd.0 as usize)));
            return Err(Error::from_thread());
        }
    }
    configure(hwnd, enabled, background);
    Ok(())
}

fn state(hwnd: HWND) -> Option<Control> {
    CONTROLS.with_borrow(|controls| controls.get(&(hwnd.0 as usize)).cloned())
}

fn native_document(hwnd: HWND) -> Result<ITextDocument> {
    let mut raw = std::ptr::null_mut::<std::ffi::c_void>();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            hwnd,
            EM_GETOLEINTERFACE,
            None,
            Some(LPARAM(&mut raw as *mut _ as isize)),
        );
        if raw.is_null() {
            return Err(Error::new(E_FAIL, "Native RichEdit has no TOM document"));
        }
        IUnknown::from_raw(raw).cast()
    }
}

fn configure(hwnd: HWND, enabled: bool, background: COLORREF) {
    let Some(previous) = state(hwnd) else {
        return;
    };
    CONTROLS.with_borrow_mut(|controls| {
        if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
            control.enabled = enabled;
            control.background = background;
        }
    });
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let transparent =
            (enabled && previous.kind == ControlKind::RichEdit) || previous.originally_transparent;
        let desired = if transparent {
            style | WS_EX_TRANSPARENT.0 as isize
        } else {
            style & !(WS_EX_TRANSPARENT.0 as isize)
        };
        if style != desired {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired);
        }
        // Do not assume a style change succeeded: preserve native painting if
        // the control cannot accept the transparency mode used by the buffer.
        if enabled
            && previous.kind == ControlKind::RichEdit
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize == 0
        {
            CONTROLS.with_borrow_mut(|controls| {
                if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                    control.enabled = false;
                }
            });
        }
        if previous.enabled != enabled || previous.background != background || style != desired {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn paint_contents(
    target: HDC,
    bounds: RECT,
    kind: ControlKind,
    background: COLORREF,
    decorations: Option<&Decorations>,
    draw_native: impl FnOnce(HDC),
) -> bool {
    let Some(mut buffer) = GlassBuffer::new(target, bounds) else {
        return false;
    };
    unsafe {
        SetDCBrushColor(buffer.hdc(), background);
        FillRect(buffer.hdc(), &bounds, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
    let alpha = decorations.map_or(crate::theme::TERMINAL_GLASS_ALPHA, |style| style.alpha);
    buffer.seal_background(alpha);
    draw_native(buffer.hdc());
    if kind == ControlKind::Edit && !convert_edit_background(buffer.hdc(), background, alpha) {
        return false;
    }
    if let Some(decorations) = decorations {
        buffer.map_native_pixels(|pixels, width| decorations.apply(pixels, width, background));
    }
    // Native glyphs and explicit ANSI backgrounds stay opaque. The paint-only
    // accents retain their premultiplied matte alpha.
    buffer.finish()
}

fn convert_edit_background(dc: HDC, background: COLORREF, alpha: u8) -> bool {
    // This DC belongs to GlassBuffer, so its selected bitmap is our private
    // 32-bit DIB. Inspect its actual stride rather than assuming tight rows.
    unsafe {
        let bitmap = GetCurrentObject(dc, OBJ_BITMAP);
        let mut dib = DIBSECTION::default();
        if GetObjectW(
            bitmap,
            std::mem::size_of::<DIBSECTION>() as i32,
            Some((&mut dib as *mut DIBSECTION).cast()),
        ) != std::mem::size_of::<DIBSECTION>() as i32
            || dib.dsBm.bmBits.is_null()
            || dib.dsBm.bmBitsPixel != 32
            || dib.dsBm.bmWidth <= 0
            || dib.dsBm.bmHeight <= 0
            || dib.dsBm.bmWidthBytes < dib.dsBm.bmWidth.saturating_mul(4)
        {
            return false;
        }
        let color = background.0;
        let rgb = ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff);
        let alpha = u32::from(alpha);
        let channel = |shift: u32| (((rgb >> shift) & 0xff) * alpha + 127) / 255;
        let matte = (alpha << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0);
        let _ = GdiFlush();
        for y in 0..dib.dsBm.bmHeight as usize {
            let row = dib
                .dsBm
                .bmBits
                .cast::<u8>()
                .add(y * dib.dsBm.bmWidthBytes as usize)
                .cast::<u32>();
            for pixel in std::slice::from_raw_parts_mut(row, dib.dsBm.bmWidth as usize) {
                if *pixel & 0x00ff_ffff == rgb {
                    *pixel = matte;
                }
            }
        }
    }
    true
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        CONTROLS.with_borrow_mut(|controls| controls.remove(&(hwnd.0 as usize)));
        let _ = RemoveWindowSubclass(hwnd, Some(control_proc), SUBCLASS);
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }
    let Some(control) = state(hwnd) else {
        return DefSubclassProc(hwnd, message, wparam, lparam);
    };
    if message == WM_SETREDRAW {
        if let Some(document) = control
            .document
            .as_ref()
            .filter(|_| control.enabled || control.frozen)
        {
            let redraw = wparam.0 != 0;
            if !redraw && control.redraw {
                if document.Freeze().is_ok() {
                    CONTROLS.with_borrow_mut(|controls| {
                        if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                            control.redraw = false;
                            control.frozen = true;
                        }
                    });
                    return LRESULT(0);
                }
                configure(hwnd, false, control.background);
            } else {
                // Native WM_SETREDRAW removes WS_VISIBLE during a batch. TOM
                // suspends native drawing without making the control vanish
                // from the parent's clipping or from accessibility clients.
                CONTROLS.with_borrow_mut(|controls| {
                    if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                        control.redraw = redraw;
                        control.frozen = !redraw && control.frozen;
                    }
                });
                if redraw && control.frozen {
                    let _ = document.Unfreeze();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                return LRESULT(0);
            }
        }
        CONTROLS.with_borrow_mut(|controls| {
            if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                control.redraw = wparam.0 != 0;
            }
        });
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if control.enabled && wparam.0 != 0 {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return result;
    }
    if !control.enabled {
        return DefSubclassProc(hwnd, message, wparam, lparam);
    }
    if message == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if message == WM_PAINT && !control.redraw {
        // Reentrant native/COM work can dispatch paint while a batch is
        // frozen. Keep the last complete frame; unfreeze schedules one new
        // buffered paint after text and formatting have both been updated.
        let mut paint = PAINTSTRUCT::default();
        BeginPaint(hwnd, &mut paint);
        let _ = EndPaint(hwnd, &paint);
        return LRESULT(0);
    }
    if message == WM_PAINT && control.redraw && !control.painting {
        let mut bounds = RECT::default();
        if GetClientRect(hwnd, &mut bounds).is_err()
            || bounds.right <= bounds.left
            || bounds.bottom <= bounds.top
        {
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
        CONTROLS.with_borrow_mut(|controls| {
            if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                control.painting = true;
            }
        });
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        let decorations = Decorations::native(
            hwnd,
            control.document.as_ref(),
            bounds,
            if crate::backdrop::active() {
                crate::theme::TERMINAL_GLASS_ALPHA
            } else {
                255
            },
            GetFocus() == hwnd,
        );
        let painted = paint_contents(
            dc,
            bounds,
            control.kind,
            control.background,
            Some(&decorations),
            |dc| {
                DefSubclassProc(
                    hwnd,
                    WM_PRINTCLIENT,
                    WPARAM(dc.0 as usize),
                    LPARAM(PRF_CLIENT as isize),
                );
            },
        );
        let _ = EndPaint(hwnd, &paint);
        CONTROLS.with_borrow_mut(|controls| {
            if let Some(control) = controls.get_mut(&(hwnd.0 as usize)) {
                control.painting = false;
            }
        });
        if painted {
            return LRESULT(0);
        }
        // Allocation/presentation failure must not leave a transparent native
        // HWND drawing unbuffered GDI text. Restore its normal opaque route.
        configure(hwnd, false, control.background);
        let _ = InvalidateRect(Some(hwnd), None, true);
        return DefSubclassProc(hwnd, WM_PAINT, wparam, lparam);
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    // RichEdit sometimes copies pixels directly while scrolling/selecting.
    // Repaint the complete client after those operations so the next frame
    // again contains the matte background. Never force synchronous paints:
    // native Freeze/WM_SETREDRAW batches must retain their normal behaviour.
    if control.redraw && changes_pixels(message, wparam) {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    result
}

fn changes_pixels(message: u32, wparam: WPARAM) -> bool {
    matches!(
        message,
        WM_SETTEXT
            | WM_SETFONT
            | WM_SIZE
            | WM_SETFOCUS
            | WM_KILLFOCUS
            | WM_VSCROLL
            | WM_HSCROLL
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_KEYDOWN
            | WM_SYSKEYDOWN
            | WM_CHAR
            | WM_CUT
            | WM_PASTE
            | WM_CLEAR
            | WM_UNDO
            | EM_REPLACESEL
            | EM_SETSEL
            | EM_EXSETSEL
            | EM_SCROLL
            | EM_SCROLLCARET
            | EM_LINESCROLL
            | EM_SETSCROLLPOS
            | EM_SETCHARFORMAT
            | EM_SETPARAFORMAT
            | EM_SETTEXTEX
            | EM_STREAMIN
            | EM_REDO
            | EM_UNDO
            | EM_SETRECT
            | EM_SETRECTNP
            | EM_SETZOOM
            | EM_SETBKGNDCOLOR
            | EM_HIDESELECTION
    ) || (message == WM_MOUSEMOVE && wparam.0 & 1 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject,
        SetBkColor, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
        HGDIOBJ,
    };
    use windows::Win32::UI::Controls::RichEdit::{
        CFM_BACKCOLOR, CFM_COLOR, CFM_SIZE, CHARFORMAT2W, CHARRANGE, EM_EXGETSEL, SCF_ALL,
        SCF_SELECTION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsWindowVisible, SendMessageW, SetWindowTextW,
        ES_MULTILINE, ES_NOHIDESEL, ES_READONLY, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CTLCOLOREDIT,
        WM_CTLCOLORSTATIC, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY, WS_VISIBLE,
    };

    const WIDTH: i32 = 480;
    const HEIGHT: i32 = 180;
    const BACKGROUND: COLORREF = COLORREF(0x0020_2020);
    const FOREGROUND: COLORREF = COLORREF(0x00ee_eeee);
    const ANSI_BACKGROUND: COLORREF = COLORREF(0x0010_1060);

    struct Fixture {
        kind: ControlKind,
        parent: HWND,
        edit: HWND,
        dc: HDC,
        bitmap: HBITMAP,
        old_bitmap: HGDIOBJ,
        pixels: *const u32,
        _apartment: crate::accessibility::ComApartment,
        _native_fixture: std::sync::MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn new(style: WINDOW_EX_STYLE) -> Self {
            Self::new_kind(ControlKind::RichEdit, style)
        }

        fn new_kind(kind: ControlKind, style: WINDOW_EX_STYLE) -> Self {
            let native_fixture = crate::accessible_text::NATIVE_TEXT_FIXTURE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let apartment = crate::accessibility::ComApartment::initialize().unwrap();
            crate::accessible_text::load().unwrap();
            unsafe {
                let parent = CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("BKS private terminal glass fixture"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    WIDTH,
                    HEIGHT,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                assert!(SetWindowSubclass(parent, Some(parent_color_proc), SUBCLASS, 0).as_bool());
                let edit = CreateWindowExW(
                    style,
                    if kind == ControlKind::RichEdit {
                        w!("RICHEDIT50W")
                    } else {
                        w!("EDIT")
                    },
                    w!("Native RichEdit text\r\nSelection caret and ANSI style"),
                    WS_CHILD
                        | WS_VISIBLE
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_NOHIDESEL) as u32),
                    0,
                    0,
                    WIDTH,
                    HEIGHT,
                    Some(parent),
                    None,
                    None,
                    None,
                )
                .unwrap();
                if kind == ControlKind::RichEdit {
                    SendMessageW(
                        edit,
                        EM_SETBKGNDCOLOR,
                        None,
                        Some(LPARAM(BACKGROUND.0 as isize)),
                    );
                    let mut format = CHARFORMAT2W::default();
                    format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
                    format.Base.dwMask = CFM_COLOR | CFM_SIZE | CFM_BACKCOLOR;
                    format.Base.crTextColor = FOREGROUND;
                    // The real terminal supplies an explicit default color
                    // for every ANSI span. Matching the control background
                    // must still let the matte show through between glyphs.
                    format.crBackColor = BACKGROUND;
                    format.Base.yHeight = 300;
                    assert_ne!(
                        SendMessageW(
                            edit,
                            EM_SETCHARFORMAT,
                            Some(WPARAM(SCF_ALL as usize)),
                            Some(LPARAM(&format as *const _ as isize)),
                        )
                        .0,
                        0
                    );
                }
                let dc = CreateCompatibleDC(None);
                assert!(!dc.0.is_null());
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: WIDTH,
                        biHeight: -HEIGHT,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut pixels = std::ptr::null_mut();
                let bitmap =
                    CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut pixels, None, 0).unwrap();
                let old_bitmap = SelectObject(dc, bitmap.into());
                assert!(!pixels.is_null());
                assert!(!IsWindowVisible(parent).as_bool());
                attach_with_policy(edit, kind, true, BACKGROUND).unwrap();
                Self {
                    kind,
                    parent,
                    edit,
                    dc,
                    bitmap,
                    old_bitmap,
                    pixels: pixels.cast(),
                    _apartment: apartment,
                    _native_fixture: native_fixture,
                }
            }
        }

        fn paint(&self) -> Vec<u32> {
            self.paint_background(BACKGROUND)
        }

        fn paint_background(&self, background: COLORREF) -> Vec<u32> {
            self.paint_decorations(background, None)
        }

        fn paint_decorations(
            &self,
            background: COLORREF,
            decorations: Option<&Decorations>,
        ) -> Vec<u32> {
            let bounds = RECT {
                left: 0,
                top: 0,
                right: WIDTH,
                bottom: HEIGHT,
            };
            assert!(paint_contents(
                self.dc,
                bounds,
                self.kind,
                background,
                decorations,
                |dc| unsafe {
                    SendMessageW(
                        self.edit,
                        WM_PRINTCLIENT,
                        Some(WPARAM(dc.0 as usize)),
                        Some(LPARAM(PRF_CLIENT as isize)),
                    );
                }
            ));
            unsafe {
                let _ = GdiFlush();
                std::slice::from_raw_parts(self.pixels, (WIDTH * HEIGHT) as usize).to_vec()
            }
        }

        fn select(&self, start: i32, end: i32) {
            if self.kind == ControlKind::Edit {
                unsafe {
                    SendMessageW(
                        self.edit,
                        EM_SETSEL,
                        Some(WPARAM(start as usize)),
                        Some(LPARAM(end as isize)),
                    );
                }
                return;
            }
            let range = CHARRANGE {
                cpMin: start,
                cpMax: end,
            };
            unsafe {
                SendMessageW(
                    self.edit,
                    EM_EXSETSEL,
                    None,
                    Some(LPARAM(&range as *const _ as isize)),
                );
            }
        }

        fn selection(&self) -> (i32, i32) {
            if self.kind == ControlKind::Edit {
                let (mut start, mut end) = (0_i32, 0_i32);
                unsafe {
                    SendMessageW(
                        self.edit,
                        windows::Win32::UI::Controls::EM_GETSEL,
                        Some(WPARAM(&mut start as *mut i32 as usize)),
                        Some(LPARAM(&mut end as *mut i32 as isize)),
                    );
                }
                return (start, end);
            }
            let mut range = CHARRANGE::default();
            unsafe {
                SendMessageW(
                    self.edit,
                    EM_EXGETSEL,
                    None,
                    Some(LPARAM(&mut range as *mut _ as isize)),
                );
            }
            (range.cpMin, range.cpMax)
        }
    }

    unsafe extern "system" fn parent_color_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if matches!(message, WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC) {
            let dc = HDC(wparam.0 as *mut _);
            SetTextColor(dc, FOREGROUND);
            SetBkColor(dc, BACKGROUND);
            SetDCBrushColor(dc, BACKGROUND);
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.parent);
                SelectObject(self.dc, self.old_bitmap);
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
            assert!(state(self.edit).is_none());
        }
    }

    #[test]
    fn native_caret_row_uses_real_line_geometry_and_follows_selection_without_mutation() {
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let control = state(fixture.edit).unwrap();
        let bounds = RECT {
            left: 0,
            top: 0,
            right: WIDTH,
            bottom: HEIGHT,
        };
        fixture.select(0, 0);
        let first = Decorations::native(fixture.edit, control.document.as_ref(), bounds, 144, true);
        let row = first.row.expect("native caret line has visible geometry");
        assert!(first.selection.is_empty());
        let original = fixture.paint();
        let highlighted = fixture.paint_decorations(BACKGROUND, Some(&first));
        assert_eq!(fixture.selection(), (0, 0));
        assert!(original
            .iter()
            .zip(&highlighted)
            .any(|(before, after)| before != after));
        for (index, (before, after)) in original.iter().zip(&highlighted).enumerate() {
            let y = index as i32 / WIDTH;
            if y < row.top || y >= row.bottom || before >> 24 == 255 {
                assert_eq!(before, after);
            }
        }
        save_preview("caret-row", &highlighted);
        fixture.select(25, 25);
        let next = Decorations::native(fixture.edit, control.document.as_ref(), bounds, 144, true);
        assert!(next.row.unwrap().top > row.top);
        let next_pixels = fixture.paint_decorations(BACKGROUND, Some(&next));
        assert_eq!(fixture.selection(), (25, 25));
        assert!(next_pixels
            .iter()
            .zip(&highlighted)
            .any(|(before, after)| before != after));
        let unfocused =
            Decorations::native(fixture.edit, control.document.as_ref(), bounds, 144, false);
        assert!(unfocused.row.is_none());
    }

    #[test]
    fn native_rich_selection_is_soft_while_text_ranges_and_ansi_format_stay_native() {
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        fixture.select(0, 30);
        let control = state(fixture.edit).unwrap();
        let document = control.document.as_ref().unwrap();
        let range = unsafe { document.Range(0, 30).unwrap() };
        let before = unsafe { range.GetText().unwrap().to_string() };
        let font_background = unsafe { range.GetFont().unwrap().GetBackColor().unwrap() };
        let decorations = Decorations::native(
            fixture.edit,
            Some(document),
            RECT {
                left: 0,
                top: 0,
                right: WIDTH,
                bottom: HEIGHT,
            },
            144,
            true,
        );
        assert!(decorations.row.is_none() && decorations.selection.len() >= 2);
        let pixels = fixture.paint_decorations(BACKGROUND, Some(&decorations));
        let target = crate::text_style::blend(
            BACKGROUND,
            crate::theme::BRAND_PRIMARY,
            crate::theme::TEXT_SELECTION_TINT,
        );
        let a = 144u32;
        let channel = |shift: u32| (((target.0 >> shift) & 255) * a + 127) / 255;
        let matte = (a << 24) | (channel(0) << 16) | (channel(8) << 8) | channel(16);
        assert!(
            pixels.contains(&matte),
            "native selection receives the soft theme fill"
        );
        assert_eq!(fixture.selection(), (0, 30));
        assert_eq!(unsafe { range.GetText().unwrap().to_string() }, before);
        assert_eq!(
            unsafe { range.GetFont().unwrap().GetBackColor().unwrap() },
            font_background
        );
        save_preview("text-selection", &pixels);
    }

    #[test]
    fn native_plain_input_selection_uses_the_same_soft_palette_without_glass() {
        let fixture = Fixture::new_kind(ControlKind::Edit, WINDOW_EX_STYLE::default());
        fixture.select(0, 6);
        let decorations = Decorations::native(
            fixture.edit,
            None,
            RECT {
                left: 0,
                top: 0,
                right: WIDTH,
                bottom: HEIGHT,
            },
            255,
            true,
        );
        assert!(decorations.row.is_none() && !decorations.selection.is_empty());
        let pixels = fixture.paint_decorations(BACKGROUND, Some(&decorations));
        let target = crate::text_style::blend(
            BACKGROUND,
            crate::theme::BRAND_PRIMARY,
            crate::theme::TEXT_SELECTION_TINT,
        )
        .0;
        let target =
            0xff000000 | ((target & 255) << 16) | (target & 0xff00) | ((target >> 16) & 255);
        assert!(pixels.contains(&target));
        assert!(pixels.iter().all(|pixel| pixel >> 24 == 255));
        assert_eq!(fixture.selection(), (0, 6));
        save_preview("input-selection-opaque", &pixels);
    }

    fn save_preview(name: &str, pixels: &[u32]) {
        let mut bytes = Vec::with_capacity(54 + pixels.len() * 4);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(54u32 + pixels.len() as u32 * 4).to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&WIDTH.to_le_bytes());
        bytes.extend_from_slice(&(-HEIGHT).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        for &value in pixels {
            // Preview over a neutral backdrop; production keeps DWM's backdrop.
            let alpha = value >> 24;
            let channel =
                |shift: u32| (((value >> shift) & 255) + 24 * (255 - alpha) / 255).min(255);
            let value = 0xff000000 | (channel(16) << 16) | (channel(8) << 8) | channel(0);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/text-highlights");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.bmp")), bytes).unwrap();
    }

    #[test]
    fn native_glass_keeps_opaque_text_selection_and_ansi_backgrounds() {
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        fixture.select(0, 6);
        let pixels = fixture.paint();
        assert_eq!(fixture.selection(), (0, 6));
        assert!(pixels.iter().all(|pixel| matches!(pixel >> 24, 144 | 255)));
        assert!(pixels.iter().filter(|pixel| **pixel >> 24 == 144).count() > pixels.len() / 2);
        assert!(pixels.contains(&0xffee_eeee), "opaque native glyph cores");
        assert!(pixels.iter().filter(|pixel| **pixel >> 24 == 255).count() > 500);

        fixture.select(7, 15);
        unsafe {
            let mut format = CHARFORMAT2W::default();
            format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            format.Base.dwMask = CFM_BACKCOLOR;
            format.crBackColor = ANSI_BACKGROUND;
            assert_ne!(
                SendMessageW(
                    fixture.edit,
                    EM_SETCHARFORMAT,
                    Some(WPARAM(SCF_SELECTION as usize)),
                    Some(LPARAM(&format as *const _ as isize)),
                )
                .0,
                0
            );
        }
        fixture.select(0, 0);
        let pixels = fixture.paint();
        assert!(
            pixels.contains(&0xff60_1010),
            "opaque explicit ANSI background"
        );
        assert_eq!(fixture.selection(), (0, 0));
    }

    #[test]
    fn native_glass_redraw_clears_old_text_completely() {
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let before = fixture.paint();
        assert!(before.iter().any(|pixel| pixel >> 24 == 255));
        unsafe {
            SetWindowTextW(fixture.edit, w!("")).unwrap();
        }
        let after = fixture.paint();
        // 32 * 144 / 255, rounded, is 18 in each premultiplied channel.
        assert!(after.iter().all(|pixel| *pixel == 0x9012_1212));
    }

    #[test]
    fn native_glass_repaints_unchanged_glyphs_into_each_fresh_buffer() {
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let before = fixture.paint();
        for frame in 0..16 {
            assert_eq!(fixture.paint(), before, "complete native paint {frame}");
        }
    }

    #[test]
    fn native_glass_disable_restores_style_without_replacing_window() {
        for originally_transparent in [false, true] {
            let original = if originally_transparent {
                WS_EX_TRANSPARENT
            } else {
                WINDOW_EX_STYLE::default()
            };
            let fixture = Fixture::new(original);
            unsafe {
                let style = GetWindowLongPtrW(fixture.edit, GWL_EXSTYLE);
                assert_ne!(style & WS_EX_TRANSPARENT.0 as isize, 0);
                SetWindowLongPtrW(
                    fixture.edit,
                    GWL_EXSTYLE,
                    style | WS_EX_NOPARENTNOTIFY.0 as isize,
                );
            }
            fixture.select(3, 9);
            configure(fixture.edit, false, BACKGROUND);
            let restored = unsafe { GetWindowLongPtrW(fixture.edit, GWL_EXSTYLE) };
            assert_eq!(
                restored & WS_EX_TRANSPARENT.0 as isize != 0,
                originally_transparent
            );
            assert_ne!(restored & WS_EX_NOPARENTNOTIFY.0 as isize, 0);
            assert_eq!(fixture.selection(), (3, 9));
            assert!(!state(fixture.edit).unwrap().enabled);
        }
    }

    #[test]
    fn native_edit_glass_keeps_opaque_text_selection_and_native_hit_style() {
        let fixture = Fixture::new_kind(ControlKind::Edit, WINDOW_EX_STYLE::default());
        let style = unsafe { GetWindowLongPtrW(fixture.edit, GWL_EXSTYLE) };
        assert_eq!(style & WS_EX_TRANSPARENT.0 as isize, 0);
        fixture.select(0, 6);
        let pixels = fixture.paint();
        assert_eq!(fixture.selection(), (0, 6));
        assert!(pixels.iter().all(|pixel| matches!(pixel >> 24, 144 | 255)));
        assert!(pixels.iter().filter(|pixel| **pixel >> 24 == 144).count() > pixels.len() / 2);
        assert!(pixels.contains(&0xffee_eeee), "opaque EDIT text");
        assert!(pixels.iter().filter(|pixel| **pixel >> 24 == 255).count() > 100);
        unsafe {
            SetWindowTextW(fixture.edit, w!("")).unwrap();
        }
        let after = fixture.paint();
        assert!(after.iter().all(|pixel| *pixel == 0x9012_1212));
    }

    #[test]
    fn native_streaming_batches_retain_visibility_and_balance_nested_freeze() {
        use windows::Win32::UI::WindowsAndMessaging::GWL_STYLE;
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let document = native_document(fixture.edit).unwrap();
        unsafe {
            let mut text = String::new();
            for step in 0..48 {
                assert_ne!(
                    GetWindowLongPtrW(fixture.edit, GWL_STYLE) & WS_VISIBLE.0 as isize,
                    0
                );
                SendMessageW(fixture.edit, WM_SETREDRAW, Some(WPARAM(0)), None);
                assert_ne!(
                    GetWindowLongPtrW(fixture.edit, GWL_STYLE) & WS_VISIBLE.0 as isize,
                    0,
                    "streaming batch {step} must not hide its accessible native HWND"
                );
                // Formatting already nests TOM Freeze inside the outer text
                // update. Neither repeated WM_SETREDRAW nor the inner thaw
                // may release that outer batch prematurely.
                SendMessageW(fixture.edit, WM_SETREDRAW, Some(WPARAM(0)), None);
                assert_eq!(document.Freeze().unwrap(), 2);
                text.push_str(&format!("token {step} "));
                let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
                SetWindowTextW(fixture.edit, windows::core::PCWSTR(wide.as_ptr())).unwrap();
                assert_eq!(document.Unfreeze().unwrap(), 1);
                assert!(!state(fixture.edit).unwrap().redraw);
                SendMessageW(fixture.edit, WM_PAINT, None, None);
                SendMessageW(fixture.edit, WM_SETREDRAW, Some(WPARAM(1)), None);
                assert_eq!(
                    document.Freeze().unwrap(),
                    1,
                    "outer freeze must be balanced"
                );
                assert_eq!(document.Unfreeze().unwrap(), 0);
            }
            assert_eq!(
                document
                    .Range(0, text.len() as i32)
                    .unwrap()
                    .GetText()
                    .unwrap()
                    .to_string(),
                text
            );
        }
    }

    #[test]
    fn native_frozen_update_accepts_viewport_restore_before_thaw() {
        use windows::Win32::UI::Controls::EM_GETFIRSTVISIBLELINE;
        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let text = "Stable first line\r\n".repeat(24);
        let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            SetWindowTextW(fixture.edit, windows::core::PCWSTR(wide.as_ptr())).unwrap();
            for step in 0..32 {
                SendMessageW(fixture.edit, WM_SETREDRAW, Some(WPARAM(0)), None);
                fixture.select(-1, -1);
                let token: Vec<_> = format!("token {step}\r\n")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                SendMessageW(
                    fixture.edit,
                    EM_REPLACESEL,
                    None,
                    Some(LPARAM(token.as_ptr() as isize)),
                );
                // A TUI update temporarily selects its changed tail. Restore
                // the reading position while native screen updates are still
                // frozen, before Unfreeze can expose that temporary viewport.
                fixture.select(0, 0);
                let current = SendMessageW(fixture.edit, EM_GETFIRSTVISIBLELINE, None, None).0;
                SendMessageW(fixture.edit, EM_LINESCROLL, None, Some(LPARAM(-current)));
                SendMessageW(fixture.edit, WM_SETREDRAW, Some(WPARAM(1)), None);
                assert_eq!(
                    SendMessageW(fixture.edit, EM_GETFIRSTVISIBLELINE, None, None).0,
                    0,
                    "native viewport must already be restored when batch {step} thaws"
                );
                assert_eq!(fixture.selection(), (0, 0));
                assert!(fixture.paint().contains(&0xffee_eeee));
            }
        }
    }

    #[test]
    fn native_tui_surface_keeps_glass_canvas_text_panels_and_exact_reader_colors() {
        use bks_terminal::{Color, Flags, NamedColor, Rgb, StyleSpan, TextStyle};

        let fixture = Fixture::new(WINDOW_EX_STYLE::default());
        let theme = crate::theme::Theme::default();
        let gray = |value| {
            Color::Spec(Rgb {
                r: value,
                g: value,
                b: value,
            })
        };
        let rows = [
            (
                "Canvas readable text             \r\n",
                gray(9),
                Flags::empty(),
            ),
            ("Dim readable text                \r\n", gray(9), Flags::DIM),
            (
                "Canvas remaining space           \r\n",
                gray(9),
                Flags::empty(),
            ),
            ("Panel one   ", gray(17), Flags::empty()),
            ("Panel two\r\n", gray(30), Flags::empty()),
            (
                "Blue ANSI   ",
                Color::Named(NamedColor::Blue),
                Flags::empty(),
            ),
            (
                "Red ANSI\r\n",
                Color::Spec(Rgb { r: 64, g: 8, b: 8 }),
                Flags::empty(),
            ),
            ("Inverse", gray(9), Flags::INVERSE),
        ];
        let mut source = String::new();
        let mut styles = Vec::new();
        let mut offset = 0;
        for (text, background, flags) in rows {
            let end = offset + text.encode_utf16().count() as u32;
            styles.push(StyleSpan {
                start: offset,
                end,
                style: TextStyle::new(gray(238), background, flags),
            });
            source.push_str(text);
            offset = end;
        }
        crate::accessible_text::register(fixture.edit, "").unwrap();
        crate::accessible_text::set_text(fixture.edit, &source);
        configure(fixture.edit, true, theme.colors.terminal);
        let document = native_document(fixture.edit).unwrap();
        let colors = || {
            styles
                .iter()
                .map(|span| unsafe {
                    let start =
                        crate::accessible_text::native_offset(fixture.edit, span.start) as i32;
                    let font = document.Range(start, start + 1).unwrap().GetFont().unwrap();
                    (
                        font.GetForeColor().unwrap() as u32,
                        font.GetBackColor().unwrap() as u32,
                    )
                })
                .collect::<Vec<_>>()
        };
        let opaque = |color: u32| {
            0xff00_0000 | ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
        };

        crate::accessible_text::format(fixture.edit, &styles, &theme).unwrap();
        let reader_colors = colors();
        let reader_pixels = fixture.paint_background(theme.colors.terminal);
        assert!(
            crate::accessible_text::format_tui(fixture.edit, &styles, &theme, 0, None).unwrap() > 0
        );
        let tui_colors = colors();
        let tui_pixels = fixture.paint_background(theme.colors.terminal);
        for (before, after) in reader_colors.iter().zip(&tui_colors) {
            assert_eq!(
                before.0, after.0,
                "native foreground, including DIM, is unchanged"
            );
        }
        for index in [0, 1, 2] {
            assert_eq!(tui_colors[index].1, theme.colors.terminal.0);
        }
        assert!(
            reader_pixels
                .iter()
                .zip(&tui_pixels)
                .filter(|(before, after)| { **before == opaque(0x090909) && **after >> 24 == 144 })
                .count()
                > 500,
            "the former opaque canvas becomes actual glass"
        );
        assert!(
            tui_pixels.contains(&opaque(reader_colors[0].0)),
            "opaque readable glyph cores"
        );
        assert!(
            tui_pixels.contains(&opaque(reader_colors[1].0)),
            "DIM glyph cores retain their source color"
        );
        assert_ne!(tui_colors[3].1, theme.colors.terminal.0);
        assert_ne!(
            tui_colors[4].1, tui_colors[3].1,
            "secondary panels remain distinct"
        );
        for index in [3, 4, 5, 6, 7] {
            assert!(
                tui_pixels.contains(&opaque(tui_colors[index].1)),
                "native opaque panel/ANSI background {index}"
            );
        }
        for index in [5, 6, 7] {
            assert_eq!(
                tui_colors[index], reader_colors[index],
                "blue/red ANSI and inverse retain exact colors"
            );
        }

        fixture.select(0, 6);
        let selected = fixture.paint_background(theme.colors.terminal);
        assert_eq!(fixture.selection(), (0, 6));
        assert!(
            selected
                .iter()
                .zip(&tui_pixels)
                .filter(|(after, before)| { **after >> 24 == 255 && **before >> 24 == 144 })
                .count()
                > 100,
            "native selection remains opaque over the glass canvas"
        );
        fixture.select(0, 0);
        assert!(crate::accessible_text::format(fixture.edit, &styles, &theme).unwrap() > 0);
        assert_eq!(
            colors(),
            reader_colors,
            "same source styles restore exact reader colors"
        );
        assert_eq!(
            fixture.paint_background(theme.colors.terminal),
            reader_pixels
        );
        assert_eq!(
            crate::accessible_text::text(fixture.edit).as_deref(),
            Some(source.as_str())
        );
    }
}
