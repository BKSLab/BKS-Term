//! Native rich text with canonical CRLF/UTF-16 offsets at the application edge.
//! RichEdit/TOM positions count each CRLF as one character. No UIA messages are
//! intercepted or reinterpreted: native clients retain the native coordinate system.
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

// Production RichEdit/TOM/UIA controls share one UI thread. Native font and
// UIA fixtures must also isolate their full window/font lifetime: concurrent
// short-lived controls can invalidate RichEdit's process-wide native caches.
#[cfg(test)]
pub(crate) static NATIVE_TEXT_FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

use bks_terminal::StyleSpan;
use windows::core::{w, IUnknown, Interface, Result};
use windows::Win32::Foundation::{COLORREF, E_FAIL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::UI::Controls::RichEdit::{
    ITextDocument, CHARRANGE, EM_EXGETSEL, EM_EXLIMITTEXT, EM_EXSETSEL, EM_GETOLEINTERFACE,
    EM_SETBKGNDCOLOR,
};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageW, SetWindowTextW, WM_COPY, WM_NCDESTROY, WM_SETFONT, WM_THEMECHANGED,
};

use crate::text_style::{resolve, ResolvedStyle, TuiSurface};
use crate::theme::Theme;

const SUBCLASS: usize = 0x424b5343;

#[derive(Default, Debug)]
struct OffsetMap {
    crlf: Vec<u32>,
    length: u32,
}

impl OffsetMap {
    fn new(text: &str) -> Self {
        let mut result = Self::default();
        result.extend(text, false);
        result
    }

    fn extend(&mut self, text: &str, mut previous_cr: bool) {
        for character in text.chars() {
            if character == '\n' && previous_cr {
                self.crlf.push(self.length - 1);
            }
            previous_cr = character == '\r';
            self.length += character.len_utf16() as u32;
        }
    }

    fn native(&self, offset: u32) -> i32 {
        let offset = offset.min(self.length);
        (offset - self.crlf.partition_point(|start| *start < offset) as u32) as i32
    }

    fn canonical(&self, offset: i32) -> u32 {
        let offset = offset.max(0) as u32;
        let mut low = 0;
        let mut high = self.crlf.len();
        while low < high {
            let middle = low + (high - low) / 2;
            if self.crlf[middle] - (middle as u32) < offset {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        (offset + low as u32).min(self.length)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeSpan {
    start: u32,
    end: u32,
    style: ResolvedStyle,
}

struct Control {
    source: String,
    offsets: OffsetMap,
    document: ITextDocument,
    styles: Vec<NativeSpan>,
    background: Option<COLORREF>,
}

thread_local! {
    static CONTROLS: RefCell<HashMap<usize, Control>> = RefCell::new(HashMap::new());
}

pub fn load() -> Result<()> {
    static LOADED: OnceLock<std::result::Result<(), windows::core::HRESULT>> = OnceLock::new();
    let result = LOADED.get_or_init(|| unsafe {
        LoadLibraryExW(w!("msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            .map(|_| ())
            .map_err(|error| error.code())
    });
    result.map_err(windows::core::Error::from)
}

pub fn register(hwnd: HWND, source: &str) -> Result<()> {
    let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETOLEINTERFACE,
            None,
            Some(LPARAM(&mut raw as *mut _ as isize)),
        );
        if raw.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let unknown = IUnknown::from_raw(raw);
        let document: ITextDocument = unknown.cast()?;
        CONTROLS.with(|controls| {
            controls.borrow_mut().insert(
                hwnd.0 as usize,
                Control {
                    source: source.to_owned(),
                    offsets: OffsetMap::new(source),
                    document,
                    styles: Vec::new(),
                    background: None,
                },
            );
        });
        SendMessageW(hwnd, EM_EXLIMITTEXT, None, Some(LPARAM(0x7fff_fffe)));
        if !SetWindowSubclass(hwnd, Some(control_proc), SUBCLASS, 0).as_bool() {
            CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as usize)));
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

pub fn text(hwnd: HWND) -> Option<String> {
    CONTROLS.with(|controls| {
        controls
            .borrow()
            .get(&(hwnd.0 as usize))
            .map(|control| control.source.clone())
    })
}

pub fn length(hwnd: HWND) -> Option<u32> {
    CONTROLS.with(|controls| {
        controls
            .borrow()
            .get(&(hwnd.0 as usize))
            .map(|control| control.offsets.length)
    })
}

pub fn native_offset(hwnd: HWND, offset: u32) -> u32 {
    CONTROLS.with(|controls| {
        controls
            .borrow()
            .get(&(hwnd.0 as usize))
            .map_or(offset, |control| control.offsets.native(offset) as u32)
    })
}

pub fn canonical_offset(hwnd: HWND, offset: u32) -> u32 {
    CONTROLS.with(|controls| {
        controls
            .borrow()
            .get(&(hwnd.0 as usize))
            .map_or(offset, |control| control.offsets.canonical(offset as i32))
    })
}

pub fn selection(hwnd: HWND) -> (u32, u32) {
    if length(hwnd).is_some() {
        let mut range = CHARRANGE::default();
        unsafe {
            SendMessageW(
                hwnd,
                EM_EXGETSEL,
                None,
                Some(LPARAM(&mut range as *mut _ as isize)),
            );
        }
        (
            canonical_offset(hwnd, range.cpMin.max(0) as u32),
            canonical_offset(hwnd, range.cpMax.max(0) as u32),
        )
    } else {
        let (mut start, mut end) = (0_u32, 0_u32);
        unsafe {
            SendMessageW(
                hwnd,
                EM_GETSEL,
                Some(WPARAM(&mut start as *mut _ as usize)),
                Some(LPARAM(&mut end as *mut _ as isize)),
            );
        }
        (start, end)
    }
}

pub fn select(hwnd: HWND, start: u32, end: u32) {
    if length(hwnd).is_some() {
        let range = CHARRANGE {
            cpMin: native_offset(hwnd, start) as i32,
            cpMax: native_offset(hwnd, end) as i32,
        };
        unsafe {
            SendMessageW(
                hwnd,
                EM_EXSETSEL,
                None,
                Some(LPARAM(&range as *const _ as isize)),
            );
        }
    } else {
        unsafe {
            SendMessageW(
                hwnd,
                EM_SETSEL,
                Some(WPARAM(start as usize)),
                Some(LPARAM(end as isize)),
            );
        }
    }
}

pub fn set_text(hwnd: HWND, source: &str) {
    let wide: Vec<_> = source.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide.as_ptr()));
    }
    CONTROLS.with(|controls| {
        if let Some(control) = controls.borrow_mut().get_mut(&(hwnd.0 as usize)) {
            control.source = source.to_owned();
            control.offsets = OffsetMap::new(source);
            control.styles.clear();
        }
    });
}

/// Replace using canonical positions, then atomically publish the new offset map.
pub fn replace(hwnd: HWND, start: u32, end: u32, replacement: &str, source: &str) {
    replace_inner(hwnd, start, end, replacement, Some(source));
}

pub fn append(hwnd: HWND, text: &str) {
    if text.is_empty() {
        return;
    }
    let end = length(hwnd).unwrap_or(0);
    replace_inner(hwnd, end, end, text, None);
}

fn byte_offset(source: &str, offset: u32) -> usize {
    let mut units = 0;
    for (byte, character) in source.char_indices() {
        if units >= offset {
            return byte;
        }
        units += character.len_utf16() as u32;
    }
    source.len()
}

fn replace_inner(hwnd: HWND, start: u32, end: u32, replacement: &str, source: Option<&str>) {
    let context = CONTROLS.with(|controls| {
        let controls = controls.borrow();
        let control = controls.get(&(hwnd.0 as usize))?;
        let start_byte = if start == control.offsets.length {
            control.source.len()
        } else {
            byte_offset(&control.source, start)
        };
        let end_byte = if end == control.offsets.length {
            control.source.len()
        } else {
            byte_offset(&control.source, end)
        };
        // RichEdit normalizes CRLF during each individual replacement. Include
        // adjacent CR/LF when an update splits or joins a pair across chunks.
        let left_cr = control.source[..start_byte].ends_with('\r');
        let right_lf = control.source[end_byte..].starts_with('\n');
        let mut native_text = String::with_capacity(replacement.len() + 2);
        if left_cr {
            native_text.push('\r');
        }
        native_text.push_str(replacement);
        if right_lf {
            native_text.push('\n');
        }
        Some((start_byte, end_byte, left_cr, right_lf, native_text))
    });
    let (native_start, native_end, native_text) = context.as_ref().map_or(
        (start, end, replacement),
        |(_, _, left_cr, right_lf, native_text)| {
            (
                start - u32::from(*left_cr),
                end + u32::from(*right_lf),
                native_text.as_str(),
            )
        },
    );
    select(hwnd, native_start, native_end);
    let wide: Vec<_> = native_text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        SendMessageW(
            hwnd,
            EM_REPLACESEL,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
    CONTROLS.with(|controls| {
        if let Some(control) = controls.borrow_mut().get_mut(&(hwnd.0 as usize)) {
            rebase_styles(
                &mut control.styles,
                native_start,
                native_end,
                native_text.encode_utf16().count() as u32,
            );
            let (start_byte, end_byte, _, _, _) = context.as_ref().unwrap();
            if start == end && end == control.offsets.length {
                let previous_cr = control.source.ends_with('\r');
                control.source.push_str(replacement);
                control.offsets.extend(replacement, previous_cr);
            } else {
                control
                    .source
                    .replace_range(*start_byte..*end_byte, replacement);
                control.offsets = OffsetMap::new(&control.source);
            }
            if let Some(source) = source {
                debug_assert_eq!(control.source, source);
            }
        }
    });
}

fn rebase_styles(spans: &mut Vec<NativeSpan>, start: u32, end: u32, inserted: u32) {
    let delta = i64::from(inserted) - i64::from(end - start);
    let mut retained = Vec::with_capacity(spans.len() + 1);
    for span in spans.iter().copied() {
        if span.start < start {
            retained.push(NativeSpan {
                end: span.end.min(start),
                ..span
            });
        }
        if span.end > end {
            retained.push(NativeSpan {
                start: (i64::from(span.start.max(end)) + delta) as u32,
                end: (i64::from(span.end) + delta) as u32,
                ..span
            });
        }
    }
    *spans = retained;
}

fn changed_spans(old: &[NativeSpan], desired: &[NativeSpan]) -> Vec<NativeSpan> {
    let mut result: Vec<NativeSpan> = Vec::new();
    let mut index = 0;
    for span in desired {
        let mut start = span.start;
        while start < span.end {
            while index < old.len() && old[index].end <= start {
                index += 1;
            }
            let previous = old.get(index);
            let end = previous.map_or(span.end, |old| {
                if old.start > start {
                    old.start.min(span.end)
                } else {
                    old.end.min(span.end)
                }
            });
            if !previous.is_some_and(|old| old.start <= start && old.style == span.style) {
                if let Some(last) = result
                    .last_mut()
                    .filter(|last| last.end == start && last.style == span.style)
                {
                    last.end = end;
                } else {
                    result.push(NativeSpan {
                        start,
                        end,
                        style: span.style,
                    });
                }
            }
            start = end;
        }
    }
    result
}

pub fn format(hwnd: HWND, styles: &[StyleSpan], theme: &Theme) -> Result<usize> {
    format_surface(hwnd, styles, theme, None)
}

pub fn format_tui(
    hwnd: HWND,
    styles: &[StyleSpan],
    theme: &Theme,
    live_start: u32,
    surface: Option<TuiSurface>,
) -> Result<usize> {
    format_surface(
        hwnd,
        styles,
        theme,
        surface.or_else(|| TuiSurface::detect(styles, live_start)),
    )
}

fn format_surface(
    hwnd: HWND,
    styles: &[StyleSpan],
    theme: &Theme,
    surface: Option<TuiSurface>,
) -> Result<usize> {
    let desired: Vec<_> = styles
        .iter()
        .map(|span| NativeSpan {
            start: span.start,
            end: span.end,
            style: surface.map_or_else(
                || resolve(span.style, theme),
                |surface| surface.resolve(span.style, theme),
            ),
        })
        .collect();
    let Some((document, changes, background_changed)) = CONTROLS.with(|controls| {
        let controls = controls.borrow();
        controls.get(&(hwnd.0 as usize)).map(|control| {
            (
                control.document.clone(),
                changed_spans(&control.styles, &desired),
                control.background != Some(theme.colors.terminal),
            )
        })
    }) else {
        return Ok(0);
    };
    if changes.is_empty() && !background_changed {
        return Ok(0);
    }
    struct Frozen(ITextDocument);
    impl Drop for Frozen {
        fn drop(&mut self) {
            unsafe {
                let _ = self.0.Unfreeze();
            }
        }
    }
    unsafe {
        document.Freeze()?;
    }
    let _frozen = Frozen(document.clone());
    use windows::Win32::UI::Controls::RichEdit::{
        CFE_BOLD, CFE_ITALIC, CFE_STRIKEOUT, CFE_UNDERLINE, CFM_BACKCOLOR, CFM_BOLD, CFM_COLOR,
        CFM_ITALIC, CFM_STRIKEOUT, CFM_UNDERLINE, CHARFORMAT2W, EM_GETEVENTMASK, EM_GETSCROLLPOS,
        EM_SETCHARFORMAT, EM_SETEVENTMASK, EM_SETSCROLLPOS, SCF_SELECTION,
    };
    // EM_SETCHARFORMAT permits programmatic formatting of ES_READONLY text.
    // TOM font setters instead return E_ACCESSDENIED on this same HWND.
    struct Restore {
        hwnd: HWND,
        selection: CHARRANGE,
        scroll: POINT,
        mask: isize,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                SendMessageW(
                    self.hwnd,
                    EM_EXSETSEL,
                    None,
                    Some(LPARAM(&self.selection as *const _ as isize)),
                );
                SendMessageW(
                    self.hwnd,
                    EM_SETSCROLLPOS,
                    None,
                    Some(LPARAM(&self.scroll as *const _ as isize)),
                );
                SendMessageW(self.hwnd, EM_SETEVENTMASK, None, Some(LPARAM(self.mask)));
            }
        }
    }
    let mut saved = Restore {
        hwnd,
        selection: CHARRANGE::default(),
        scroll: POINT::default(),
        mask: 0,
    };
    unsafe {
        SendMessageW(
            hwnd,
            EM_EXGETSEL,
            None,
            Some(LPARAM(&mut saved.selection as *mut _ as isize)),
        );
        SendMessageW(
            hwnd,
            EM_GETSCROLLPOS,
            None,
            Some(LPARAM(&mut saved.scroll as *mut _ as isize)),
        );
        saved.mask = SendMessageW(hwnd, EM_GETEVENTMASK, None, None).0;
        SendMessageW(hwnd, EM_SETEVENTMASK, None, Some(LPARAM(0)));
        if background_changed {
            SendMessageW(
                hwnd,
                EM_SETBKGNDCOLOR,
                None,
                Some(LPARAM(theme.colors.terminal.0 as isize)),
            );
        }
        for span in &changes {
            let range = CHARRANGE {
                cpMin: native_offset(hwnd, span.start) as i32,
                cpMax: native_offset(hwnd, span.end) as i32,
            };
            SendMessageW(
                hwnd,
                EM_EXSETSEL,
                None,
                Some(LPARAM(&range as *const _ as isize)),
            );
            let mut format = CHARFORMAT2W::default();
            format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            format.Base.dwMask =
                CFM_COLOR | CFM_BACKCOLOR | CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_STRIKEOUT;
            if span.style.bold {
                format.Base.dwEffects |= CFE_BOLD;
            }
            if span.style.italic {
                format.Base.dwEffects |= CFE_ITALIC;
            }
            if span.style.underline {
                format.Base.dwEffects |= CFE_UNDERLINE;
            }
            if span.style.strike {
                format.Base.dwEffects |= CFE_STRIKEOUT;
            }
            format.Base.crTextColor = span.style.foreground;
            format.crBackColor = span.style.background;
            if SendMessageW(
                hwnd,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM(&format as *const _ as isize)),
            )
            .0 == 0
            {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Native character formatting failed",
                ));
            }
        }
    }
    drop(saved);
    CONTROLS.with(|controls| {
        if let Some(control) = controls.borrow_mut().get_mut(&(hwnd.0 as usize)) {
            control.styles = desired;
            control.background = Some(theme.colors.terminal);
        }
    });
    Ok(changes.len())
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_SETFONT | WM_THEMECHANGED => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            // RichEdit can reset character colors while applying a native font
            // or theme, even when our desired resolved styles have not changed.
            CONTROLS.with(|controls| {
                if let Some(control) = controls.borrow_mut().get_mut(&(hwnd.0 as usize)) {
                    control.styles.clear();
                    control.background = None;
                }
            });
            result
        }
        WM_COPY => {
            let (start, end) = selection(hwnd);
            if let Some(source) = text(hwnd) {
                let units: Vec<_> = source.encode_utf16().collect();
                if let Some(selected) = units.get(start as usize..end as usize) {
                    let _ = crate::clipboard::write(hwnd, &String::from_utf16_lossy(selected));
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            CONTROLS.with(|controls| controls.borrow_mut().remove(&(hwnd.0 as usize)));
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(control_proc), SUBCLASS);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_positions_roundtrip_unicode_crlf_and_many_lines() {
        for text in [
            "Ж🙂\r\ne\u{301}\r\nlast",
            "\r\n\r\n\r\n",
            "one\rtwo\nthree",
            "",
        ] {
            let map = OffsetMap::new(text);
            for offset in 0..=map.length {
                let inside = map.crlf.contains(&offset.saturating_sub(1)) && offset > 0;
                assert_eq!(
                    map.canonical(map.native(offset)),
                    if inside { offset - 1 } else { offset },
                    "{text:?} at {offset}"
                );
            }
        }
    }

    #[test]
    fn formatting_delta_and_patch_touch_only_inserted_or_changed_ranges() {
        let theme = Theme::default();
        let plain = resolve(bks_terminal::TextStyle::default(), &theme);
        let mut spans = vec![NativeSpan {
            start: 0,
            end: 10000,
            style: plain,
        }];
        rebase_styles(&mut spans, 5000, 5000, 2);
        let desired = vec![NativeSpan {
            start: 0,
            end: 10002,
            style: plain,
        }];
        assert_eq!(
            changed_spans(&spans, &desired),
            vec![NativeSpan {
                start: 5000,
                end: 5002,
                style: plain
            }]
        );
        assert!(changed_spans(&desired, &desired).is_empty());
    }
}

#[cfg(test)]
#[path = "accessible_text_native_tests.rs"]
mod native_tests;
