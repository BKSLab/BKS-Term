//! Native output geometry. Measurements use the output's resolved font and an
//! independent, never-shown RichEdit; no display text, focus or input is changed.
//! Grid formatting changes native presentation only. Reflow retains logical
//! paragraphs and never imposes the live VT cell spacing on archived answers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;

use bks_terminal::{Flags, ReadingDocument, StyleSpan};
use windows::core::{w, Error, IUnknown, Interface, Result, BSTR};
use windows::Win32::Foundation::{
    E_FAIL, E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetDC, GetGlyphIndicesW, GetObjectW, GetTextFaceW,
    GetTextMetricsW, ReleaseDC, SelectObject, GGI_MARK_NONEXISTING_GLYPHS, HFONT, LOGFONTW,
    TEXTMETRICW,
};
use windows::Win32::UI::Controls::RichEdit::{
    ITextDocument, ITextFont2, CFE_BOLD, CFE_ITALIC, CFM_BOLD, CFM_ITALIC, CFM_SPACING,
    CHARFORMAT2W, CHARRANGE, EM_EXGETSEL, EM_EXSETSEL, EM_GETEVENTMASK, EM_GETOLEINTERFACE,
    EM_GETSCROLLPOS, EM_SETCHARFORMAT, EM_SETEVENTMASK, EM_SETPARAFORMAT, EM_SETSCROLLPOS,
    EM_SETTARGETDEVICE, PARAFORMAT2, PFM_LINESPACING, SCF_SELECTION,
};
use windows::Win32::UI::Controls::{EM_GETRECT, EM_POSFROMCHAR, EM_REPLACESEL, EM_SETREADONLY};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetWindowLongW, GetWindowRect, SendMessageW,
    SetWindowTextW, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWL_STYLE, SM_CXVSCROLL,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCDESTROY, WM_SETFONT, WM_SETTEXT, WS_CHILD,
    WS_EX_NOACTIVATE, WS_VSCROLL,
};

const SAMPLE: i32 = 16;
const MAX_GLYPH_UNITS: usize = 64;
const SUBCLASS: usize = 0x424b534d;

thread_local! {
    static EPOCHS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    static NEXT_EPOCH: Cell<u64> = const { Cell::new(0) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Grid,
    Reflow,
}

/// Pixel metrics and stable insets. `grid_size` takes the original output slot
/// from layout, before a popup/search pane reduces the visible control height.
pub struct Metrics {
    pub cell_width: i32,
    pub cell_height: i32,
    pub dpi: u32,
    pub face: String,
    pub horizontal_inset: i32,
    pub vertical_inset: i32,
    font: HFONT,
    descriptor: LOGFONTW,
    probe_origin: (i32, i32),
    _thread: PhantomData<Rc<()>>,
    glyphs: RefCell<HashMap<GlyphKey, Correction>>,
    applied: RefCell<HashMap<usize, Applied>>,
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Metrics")
            .field("cell_width", &self.cell_width)
            .field("cell_height", &self.cell_height)
            .field("dpi", &self.dpi)
            .field("face", &self.face)
            .field("horizontal_inset", &self.horizontal_inset)
            .field("vertical_inset", &self.vertical_inset)
            .finish_non_exhaustive()
    }
}

impl Metrics {
    pub fn grid_size(&self, outer_width: i32, outer_height: i32) -> Option<(u16, u16)> {
        let width = outer_width.checked_sub(self.horizontal_inset)?;
        let height = outer_height.checked_sub(self.vertical_inset)?;
        if width < self.cell_width * 2 || height < self.cell_height {
            return None;
        }
        Some((
            (width / self.cell_width).min(i16::MAX as i32) as u16,
            (height / self.cell_height).min(i16::MAX as i32) as u16,
        ))
    }
}

pub fn measure(reference: HWND, font: HFONT) -> Result<Metrics> {
    if font.0.is_null() {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let (face, text_metrics, mut descriptor) = font_description(reference, font)?;
    for (to, from) in descriptor
        .lfFaceName
        .iter_mut()
        .zip(face.encode_utf16().chain(Some(0)))
    {
        *to = from;
    }
    let mut reference_bounds = RECT::default();
    unsafe {
        GetWindowRect(reference, &mut reference_bounds)?;
    }
    let probe_origin = (reference_bounds.left, reference_bounds.top);
    let mut probe = Probe::new(&descriptor, probe_origin)?;
    let baseline = probe.advance(&[b'M' as u16], Flags::empty(), None, 0)?;
    let cell_width = baseline / SAMPLE;
    if cell_width <= 0 || baseline != cell_width * SAMPLE || text_metrics.tmHeight <= 0 {
        return Err(Error::new(
            E_FAIL,
            "Native output font has no stable cell advance",
        ));
    }
    for glyph in *b"i " {
        if probe.advance(&[u16::from(glyph)], Flags::empty(), None, 0)? != baseline {
            return Err(Error::new(E_FAIL, "Native output font is not monospaced"));
        }
    }
    let mut outer = RECT::default();
    let mut client = RECT::default();
    let mut format = RECT::default();
    unsafe {
        GetWindowRect(reference, &mut outer)?;
        GetClientRect(reference, &mut client)?;
        SendMessageW(
            reference,
            EM_GETRECT,
            None,
            Some(LPARAM(&mut format as *mut _ as isize)),
        );
    }
    let dpi = unsafe { GetDpiForWindow(reference) }.max(96);
    let scrollbar = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) }.max(0);
    let visible_scrollbar =
        unsafe { GetWindowLongW(reference, GWL_STYLE) } as u32 & WS_VSCROLL.0 != 0;
    let nonclient_width = (outer.right - outer.left - client.right + client.left).max(0);
    let allocated_scrollbar = if visible_scrollbar && nonclient_width >= scrollbar {
        scrollbar
    } else {
        0
    };
    let border_width = (nonclient_width - allocated_scrollbar).max(0);
    let horizontal_inset = border_width
        + scrollbar
        + (format.left - client.left).max(0)
        + (client.right - format.right).max(0);
    let vertical_inset = (outer.bottom - outer.top - client.bottom + client.top).max(0)
        + (format.top - client.top).max(0)
        + (client.bottom - format.bottom).max(0);
    Ok(Metrics {
        cell_width,
        cell_height: text_metrics.tmHeight,
        dpi,
        face,
        horizontal_inset,
        vertical_inset,
        font,
        descriptor,
        probe_origin,
        _thread: PhantomData,
        glyphs: RefCell::new(HashMap::new()),
        applied: RefCell::new(HashMap::new()),
    })
}

fn font_description(hwnd: HWND, font: HFONT) -> Result<(String, TEXTMETRICW, LOGFONTW)> {
    unsafe {
        let dc = GetDC(Some(hwnd));
        if dc.0.is_null() {
            return Err(Error::from_thread());
        }
        let old = SelectObject(dc, font.into());
        let mut metrics = TEXTMETRICW::default();
        let mut descriptor = LOGFONTW::default();
        let mut name = [0u16; 128];
        let valid = GetTextMetricsW(dc, &mut metrics).as_bool()
            && GetObjectW(
                font.into(),
                std::mem::size_of::<LOGFONTW>() as i32,
                Some(&mut descriptor as *mut _ as *mut c_void),
            ) > 0
            && GetTextFaceW(dc, Some(&mut name)) > 0;
        SelectObject(dc, old);
        ReleaseDC(Some(hwnd), dc);
        if !valid {
            return Err(Error::new(E_FAIL, "Native output font measurement failed"));
        }
        let end = name
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(name.len());
        Ok((String::from_utf16_lossy(&name[..end]), metrics, descriptor))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Correction {
    face: bool,
    spacing: i16,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    text: Vec<u16>,
    width: usize,
    flags: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Run {
    start: u32,
    end: u32,
    correction: Correction,
}

struct Applied {
    text: String,
    styles: Vec<StyleSpan>,
    runs: Vec<Run>,
    mode: LayoutMode,
    epoch: u64,
}

/// Call after source text/style synchronization. Explicit CRLF and source
/// UTF-16 offsets stay unchanged. Both modes retain native paragraph wrapping:
/// ReadingDocument joins VT soft wraps, and those logical lines must remain
/// visible through vertical scrolling. Grid adds cell spacing/fixed height;
/// Reflow applies only graphics font fixes and keeps natural line height.
pub fn format_grid(
    hwnd: HWND,
    document: &ReadingDocument,
    font: HFONT,
    metrics: &Metrics,
    mode: LayoutMode,
) -> Result<usize> {
    if font != metrics.font || crate::accessible_text::text(hwnd).as_deref() != Some(&document.text)
    {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let units: Vec<_> = document.text.encode_utf16().collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut style_index = 0;
    let graphic_lines: Vec<_> = document
        .lines
        .iter()
        .map(|line| {
            units
                .get(line.start as usize..line.end as usize)
                .is_some_and(|text| text.iter().any(|unit| (0x2500..=0x259f).contains(unit)))
        })
        .collect();
    let mut line_index = 0;
    for cell in &document.cells {
        let Some(glyph) = units.get(cell.start as usize..cell.end as usize) else {
            return Err(Error::from_hresult(E_INVALIDARG));
        };
        while document
            .styles
            .get(style_index)
            .is_some_and(|span| span.end <= cell.start)
        {
            style_index += 1;
        }
        let flags = document
            .styles
            .get(style_index)
            .map_or(Flags::empty(), |span| span.style.flags);
        while document
            .lines
            .get(line_index)
            .is_some_and(|line| line.end < cell.start)
        {
            line_index += 1;
        }
        let graphics_line = graphic_lines.get(line_index).copied().unwrap_or(false);
        if glyph.is_empty() || (mode == LayoutMode::Reflow && !graphics_line) {
            continue;
        }
        let correction = if glyph.len() == 1 && (0x20..=0x7e).contains(&glyph[0]) && cell.width == 1
        {
            // Spaces beside box glyphs inherit RichEdit's symbol repertoire
            // too; normalizing just the visible border leaves 3px padding.
            Correction {
                face: true,
                spacing: 0,
            }
        } else {
            metrics.correction(glyph, cell.width, flags)?
        };
        let correction = if mode == LayoutMode::Reflow {
            Correction {
                face: correction.face,
                spacing: 0,
            }
        } else {
            correction
        };
        if correction.face || correction.spacing != 0 {
            if let Some(last) = runs
                .last_mut()
                .filter(|last| last.end == cell.start && last.correction == correction)
            {
                last.end = cell.end;
            } else {
                runs.push(Run {
                    start: cell.start,
                    end: cell.end,
                    correction,
                });
            }
        }
    }
    let key = hwnd.0 as usize;
    let epoch = native_epoch(hwnd)?;
    if metrics.applied.borrow().get(&key).is_some_and(|previous| {
        previous.text == document.text
            && previous.styles == document.styles
            && previous.runs == runs
            && previous.mode == mode
            && previous.epoch == epoch
    }) {
        return Ok(0);
    }
    let native = native_document(hwnd)?;
    let _restore = Restore::new(hwnd, native.clone())?;
    unsafe {
        SendMessageW(hwnd, EM_SETTARGETDEVICE, None, Some(LPARAM(0)));
    }
    let all_end = crate::accessible_text::native_offset(hwnd, units.len() as u32) as i32;
    select_native(hwnd, 0, all_end);
    character_spacing(hwnd, 0)?;
    let mut paragraph = PARAFORMAT2::default();
    paragraph.Base.cbSize = std::mem::size_of::<PARAFORMAT2>() as u32;
    paragraph.Base.dwMask = PFM_LINESPACING;
    if mode == LayoutMode::Grid {
        paragraph.bLineSpacingRule = 4;
        paragraph.dyLineSpacing =
            (i64::from(metrics.cell_height) * 1440 / i64::from(metrics.dpi)) as i32;
    }
    unsafe {
        SendMessageW(
            hwnd,
            EM_SETPARAFORMAT,
            None,
            Some(LPARAM(&paragraph as *const _ as isize)),
        );
    }
    for run in &runs {
        let start = crate::accessible_text::native_offset(hwnd, run.start) as i32;
        let end = crate::accessible_text::native_offset(hwnd, run.end) as i32;
        if run.correction.face {
            force_graphics_face(&native, start, end, &metrics.face)?;
        }
        select_native(hwnd, start, end);
        character_spacing(hwnd, run.correction.spacing)?;
    }
    let changed = runs.len() + 1;
    metrics.applied.borrow_mut().insert(
        key,
        Applied {
            text: document.text.clone(),
            styles: document.styles.clone(),
            runs,
            mode,
            epoch,
        },
    );
    Ok(changed)
}

fn bump_epoch(hwnd: HWND) -> u64 {
    let epoch = NEXT_EPOCH.with(|next| {
        let epoch = next.get().wrapping_add(1);
        next.set(epoch);
        epoch
    });
    EPOCHS.with(|epochs| epochs.borrow_mut().insert(hwnd.0 as usize, epoch));
    epoch
}

fn native_epoch(hwnd: HWND) -> Result<u64> {
    if let Some(epoch) = EPOCHS.with(|epochs| epochs.borrow().get(&(hwnd.0 as usize)).copied()) {
        return Ok(epoch);
    }
    if !unsafe { SetWindowSubclass(hwnd, Some(metrics_proc), SUBCLASS, 0).as_bool() } {
        return Err(Error::from_thread());
    }
    Ok(bump_epoch(hwnd))
}

unsafe extern "system" fn metrics_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_SETTEXT | EM_REPLACESEL | WM_SETFONT => {
            bump_epoch(hwnd);
        }
        WM_NCDESTROY => {
            EPOCHS.with(|epochs| epochs.borrow_mut().remove(&(hwnd.0 as usize)));
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(metrics_proc), SUBCLASS);
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl Metrics {
    fn correction(&self, glyph: &[u16], width: usize, flags: Flags) -> Result<Correction> {
        if glyph.len() > MAX_GLYPH_UNITS || !(1..=2).contains(&width) {
            return Ok(Correction {
                face: false,
                spacing: 0,
            });
        }
        let key = GlyphKey {
            text: glyph.to_vec(),
            width,
            flags: u8::from(flags.contains(Flags::BOLD))
                | (u8::from(flags.contains(Flags::ITALIC)) << 1),
        };
        if let Some(correction) = self.glyphs.borrow().get(&key) {
            return Ok(*correction);
        }
        let mut probe = Probe::new(&self.descriptor, self.probe_origin)?;
        let face = probe.supports(glyph);
        let forced_face = face.then_some(self.face.as_str());
        let target = self.cell_width * width as i32 * SAMPLE;
        let base = probe.advance(glyph, flags, forced_face, 0)?;
        let mut spacing = 0;
        if base != target {
            let trial = 120;
            let shifted = probe.advance(glyph, flags, forced_face, trial)?;
            if shifted > base {
                let desired = ((target - base) as f64 * f64::from(trial)
                    / f64::from(shifted - base))
                .round() as i32;
                let mut best = (i32::MAX, 0);
                for candidate in desired.saturating_sub(2)..=desired.saturating_add(2) {
                    let Ok(candidate) = i16::try_from(candidate) else {
                        continue;
                    };
                    let advance = probe.advance(glyph, flags, forced_face, candidate)?;
                    let error = advance.abs_diff(target) as i32;
                    if error < best.0 {
                        best = (error, candidate);
                    }
                }
                // Keep unknown/complex shaping natural instead of inventing a
                // correction which the actual native range failed to verify.
                if best.0 <= 1 {
                    // Twips scale with the window DPI, while the borrowed
                    // HFONT describes the same pixel-sized face in both HWNDs.
                    let probe_dpi = unsafe { GetDpiForWindow(probe.edit) }.max(1);
                    spacing = (f64::from(best.1) * f64::from(probe_dpi) / f64::from(self.dpi))
                        .round() as i16;
                }
            }
        }
        let correction = Correction { face, spacing };
        self.glyphs.borrow_mut().insert(key, correction);
        Ok(correction)
    }
}

fn native_document(hwnd: HWND) -> Result<ITextDocument> {
    let mut raw = std::ptr::null_mut::<c_void>();
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETOLEINTERFACE,
            None,
            Some(LPARAM(&mut raw as *mut _ as isize)),
        );
        if raw.is_null() {
            return Err(Error::new(E_FAIL, "Native output has no TOM document"));
        }
        let unknown = IUnknown::from_raw(raw);
        unknown.cast()
    }
}

fn force_graphics_face(document: &ITextDocument, start: i32, end: i32, face: &str) -> Result<()> {
    unsafe {
        let font = document.Range(start, end)?.GetFont()?;
        let font2: ITextFont2 = font.cast()?;
        // RichEdit otherwise keeps its internal symbol repertoire (52) even
        // after a successful CFM_FACE. This tag changes no Unicode codepoint.
        font2.SetCharRep(0)?;
        font.SetName(&BSTR::from(face))
    }
}

fn select_native(hwnd: HWND, start: i32, end: i32) {
    let range = CHARRANGE {
        cpMin: start,
        cpMax: end,
    };
    unsafe {
        SendMessageW(
            hwnd,
            EM_EXSETSEL,
            None,
            Some(LPARAM(&range as *const _ as isize)),
        );
    }
}

fn character_spacing(hwnd: HWND, spacing: i16) -> Result<()> {
    let mut format = CHARFORMAT2W::default();
    format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
    format.Base.dwMask = CFM_SPACING;
    format.sSpacing = spacing;
    if unsafe {
        SendMessageW(
            hwnd,
            EM_SETCHARFORMAT,
            Some(WPARAM(SCF_SELECTION as usize)),
            Some(LPARAM(&format as *const _ as isize)),
        )
        .0
    } == 0
    {
        return Err(Error::new(E_FAIL, "Native output spacing failed"));
    }
    Ok(())
}

struct Restore {
    hwnd: HWND,
    document: ITextDocument,
    selection: CHARRANGE,
    scroll: POINT,
    event_mask: isize,
    readonly: bool,
}

impl Restore {
    fn new(hwnd: HWND, document: ITextDocument) -> Result<Self> {
        unsafe {
            document.Freeze()?;
        }
        let mut result = Self {
            hwnd,
            document,
            selection: CHARRANGE::default(),
            scroll: POINT::default(),
            event_mask: 0,
            readonly: false,
        };
        unsafe {
            SendMessageW(
                hwnd,
                EM_EXGETSEL,
                None,
                Some(LPARAM(&mut result.selection as *mut _ as isize)),
            );
            SendMessageW(
                hwnd,
                EM_GETSCROLLPOS,
                None,
                Some(LPARAM(&mut result.scroll as *mut _ as isize)),
            );
            result.event_mask = SendMessageW(hwnd, EM_GETEVENTMASK, None, None).0;
            result.readonly = GetWindowLongW(hwnd, GWL_STYLE) as u32 & ES_READONLY as u32 != 0;
            SendMessageW(hwnd, EM_SETEVENTMASK, None, Some(LPARAM(0)));
            if result.readonly {
                SendMessageW(hwnd, EM_SETREADONLY, Some(WPARAM(0)), None);
            }
        }
        Ok(result)
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        unsafe {
            if self.readonly {
                SendMessageW(self.hwnd, EM_SETREADONLY, Some(WPARAM(1)), None);
            }
            SendMessageW(
                self.hwnd,
                EM_EXSETSEL,
                None,
                Some(LPARAM(&self.selection as *const _ as isize)),
            );
            let _ = self.document.Unfreeze();
            SendMessageW(
                self.hwnd,
                EM_SETSCROLLPOS,
                None,
                Some(LPARAM(&self.scroll as *const _ as isize)),
            );
            SendMessageW(
                self.hwnd,
                EM_SETEVENTMASK,
                None,
                Some(LPARAM(self.event_mask)),
            );
        }
    }
}

struct Probe {
    parent: HWND,
    edit: HWND,
    font: HFONT,
    document: Option<ITextDocument>,
    _thread: PhantomData<Rc<()>>,
}

impl Probe {
    fn new(descriptor: &LOGFONTW, origin: (i32, i32)) -> Result<Self> {
        crate::accessible_text::load()?;
        let parent = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("BKS private text metrics"),
                WINDOW_STYLE::default(),
                origin.0,
                origin.1,
                800,
                400,
                None,
                None,
                None,
                None,
            )?
        };
        let font = unsafe { CreateFontIndirectW(descriptor) };
        if font.0.is_null() {
            let error = Error::from_thread();
            unsafe {
                let _ = DestroyWindow(parent);
            }
            return Err(error);
        }
        let edit = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("RICHEDIT50W"),
                w!(""),
                WS_CHILD | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
                0,
                0,
                16000,
                400,
                Some(parent),
                None,
                None,
                None,
            )
        } {
            Ok(edit) => edit,
            Err(error) => {
                unsafe {
                    let _ = DestroyWindow(parent);
                    let _ = DeleteObject(font.into());
                }
                return Err(error);
            }
        };
        unsafe {
            SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            SendMessageW(edit, EM_SETTARGETDEVICE, None, Some(LPARAM(1)));
        }
        let document = match native_document(edit) {
            Ok(document) => document,
            Err(error) => {
                unsafe {
                    let _ = DestroyWindow(parent);
                    let _ = DeleteObject(font.into());
                }
                return Err(error);
            }
        };
        Ok(Self {
            parent,
            edit,
            font,
            document: Some(document),
            _thread: PhantomData,
        })
    }

    fn supports(&self, glyph: &[u16]) -> bool {
        unsafe {
            let dc = GetDC(Some(self.edit));
            if dc.0.is_null() {
                return false;
            }
            let old = SelectObject(dc, self.font.into());
            let mut indices = vec![0; glyph.len()];
            let result = GetGlyphIndicesW(
                dc,
                windows::core::PCWSTR(glyph.as_ptr()),
                glyph.len() as i32,
                indices.as_mut_ptr(),
                GGI_MARK_NONEXISTING_GLYPHS,
            );
            SelectObject(dc, old);
            ReleaseDC(Some(self.edit), dc);
            result != u32::MAX && indices.iter().all(|glyph| *glyph != u16::MAX)
        }
    }

    fn advance(
        &mut self,
        glyph: &[u16],
        flags: Flags,
        face: Option<&str>,
        spacing: i16,
    ) -> Result<i32> {
        let mut text = glyph.repeat(SAMPLE as usize);
        let end = text.len() as i32;
        text.extend([b'|' as u16, 0]);
        unsafe {
            SetWindowTextW(self.edit, windows::core::PCWSTR(text.as_ptr()))?;
        }
        select_native(self.edit, 0, end);
        let mut format = CHARFORMAT2W::default();
        format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
        format.Base.dwMask = CFM_BOLD | CFM_ITALIC | CFM_SPACING;
        if flags.contains(Flags::BOLD) {
            format.Base.dwEffects |= CFE_BOLD;
        }
        if flags.contains(Flags::ITALIC) {
            format.Base.dwEffects |= CFE_ITALIC;
        }
        format.sSpacing = spacing;
        unsafe {
            SendMessageW(
                self.edit,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_SELECTION as usize)),
                Some(LPARAM(&format as *const _ as isize)),
            );
        }
        if let Some(face) = face {
            force_graphics_face(
                self.document
                    .as_ref()
                    .ok_or_else(|| Error::from_hresult(E_FAIL))?,
                0,
                end,
                face,
            )?;
        }
        let first = native_point(self.edit, 0);
        let last = native_point(self.edit, end);
        if first.y != last.y {
            return Err(Error::new(
                E_FAIL,
                "Native calibration unexpectedly wrapped",
            ));
        }
        Ok(last.x - first.x)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.document.take();
        unsafe {
            let _ = DestroyWindow(self.parent);
            let _ = DeleteObject(self.font.into());
        }
    }
}

fn native_point(hwnd: HWND, offset: i32) -> POINT {
    let mut point = POINT::default();
    unsafe {
        SendMessageW(
            hwnd,
            EM_POSFROMCHAR,
            Some(WPARAM(&mut point as *mut _ as usize)),
            Some(LPARAM(offset as isize)),
        );
    }
    point
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;
    use std::sync::MutexGuard;
    use windows::Win32::UI::Controls::{EM_GETFIRSTVISIBLELINE, EM_GETLINECOUNT, EM_LINESCROLL};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, MoveWindow, SB_BOTTOM, WM_VSCROLL, WS_VISIBLE,
    };

    struct HiddenOutput {
        parent: HWND,
        edit: HWND,
        theme: crate::theme::Theme,
        _com: crate::accessibility::ComApartment,
        _font_fixture: MutexGuard<'static, ()>,
    }

    impl HiddenOutput {
        fn new(dpi: u32, width: i32, height: i32) -> Self {
            let font_fixture = crate::accessible_text::NATIVE_TEXT_FIXTURE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let com = crate::accessibility::ComApartment::initialize().unwrap();
            crate::accessible_text::load().unwrap();
            let theme = crate::theme::Theme::new(dpi);
            let parent = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("BKS private native grid test"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    width,
                    height,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            let edit = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("RICHEDIT50W"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
                    0,
                    0,
                    width,
                    height,
                    Some(parent),
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            crate::accessible_text::register(edit, "").unwrap();
            unsafe {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(theme.terminal_font.0 as usize)),
                    Some(LPARAM(0)),
                );
            }
            Self {
                parent,
                edit,
                theme,
                _com: com,
                _font_fixture: font_fixture,
            }
        }

        fn set_document(&self, document: &ReadingDocument) {
            crate::accessible_text::set_text(self.edit, &document.text);
            crate::accessible_text::format(self.edit, &document.styles, &self.theme).unwrap();
        }

        fn point(&self, canonical: u32) -> POINT {
            native_point(
                self.edit,
                crate::accessible_text::native_offset(self.edit, canonical) as i32,
            )
        }

        fn log_fonts(&self, document: &ReadingDocument, dpi: u32, phase: &str) {
            let native = native_document(self.edit).unwrap();
            for line in document.lines.iter().take(3) {
                let start = crate::accessible_text::native_offset(self.edit, line.start) as i32;
                let font = unsafe { native.Range(start, start + 1).unwrap().GetFont().unwrap() };
                let font2: ITextFont2 = font.cast().unwrap();
                eprintln!(
                    "native logo fonts: dpi={dpi} phase={phase} row={} face={} rep={} size={} spacing={} scaling={} width={}",
                    line.start,
                    unsafe { font.GetName().unwrap() },
                    unsafe { font2.GetCharRep().unwrap() },
                    unsafe { font.GetSize().unwrap() },
                    unsafe { font.GetSpacing().unwrap() },
                    unsafe { font2.GetScaling().unwrap() },
                    self.point(line.start + 10).x - self.point(line.start).x,
                );
            }
        }
    }

    impl Drop for HiddenOutput {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.parent);
                for font in [
                    self.theme.ui_font,
                    self.theme.title_font,
                    self.theme.terminal_font,
                ] {
                    let _ = DeleteObject(font.into());
                }
                if !self.theme.high_contrast {
                    for brush in [
                        self.theme.window,
                        self.theme.terminal,
                        self.theme.toolbar,
                        self.theme.sidebar,
                        self.theme.surface,
                        self.theme.hover,
                        self.theme.selected,
                        self.theme.border,
                        self.theme.border_subtle,
                        self.theme.brand,
                    ] {
                        let _ = DeleteObject(brush.into());
                    }
                }
            }
        }
    }

    fn document(text: &str, columns: usize) -> ReadingDocument {
        let mut terminal = TerminalModel::new(columns, 12);
        terminal.push_bytes(text.as_bytes());
        terminal.reading_document()
    }

    #[test]
    fn native_fullscreen_visual_rows_keep_logo_columns_and_page_top_at_every_dpi() {
        let mut terminal = TerminalModel::new(110, 36);
        terminal.push_bytes(&include_bytes!("../tests/fixtures/gutter-panel-menu.vt")[..13_222]);
        let frame = terminal.live_snapshot();
        let source =
            crate::tui_projection::project(&frame, &terminal.reading_document(), None).output;
        let original = source.clone();
        let visual = crate::tui_projection::visual_document(&frame, &source, &[(18, 22)]);
        assert_eq!(visual.document.lines.len(), frame.rows - 5);
        for dpi in [96, 120, 144, 192] {
            let output = HiddenOutput::new(dpi, 1500, 900);
            let focus = unsafe { GetFocus() };
            let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
            // ConPTY includes the shared input area; the upper output can be
            // shorter than its physical page. Following EOF would hide its top.
            unsafe {
                MoveWindow(
                    output.edit,
                    0,
                    0,
                    frame.cols as i32 * metrics.cell_width + metrics.horizontal_inset,
                    22 * metrics.cell_height + metrics.vertical_inset,
                    false,
                )
                .unwrap();
            }
            output.set_document(&visual.document);
            format_grid(
                output.edit,
                &visual.document,
                output.theme.terminal_font,
                &metrics,
                LayoutMode::Grid,
            )
            .unwrap();
            let end = visual.document.text.encode_utf16().count() as u32;
            crate::restore_native_reading(output.edit, (end, end), visual.live_start(), false);
            let origin = output.point(visual.live_start());
            assert_eq!(
                unsafe { SendMessageW(output.edit, EM_GETFIRSTVISIBLELINE, None, None).0 },
                0
            );
            assert_eq!(
                unsafe { SendMessageW(output.edit, EM_GETLINECOUNT, None, None).0 },
                visual.document.lines.len() as isize
            );
            let units: Vec<_> = visual.document.text.encode_utf16().collect();
            for cell in &visual.document.cells {
                if units[cell.start as usize..cell.end as usize]
                    .iter()
                    .all(|unit| *unit == 32)
                {
                    continue;
                }
                let row = visual
                    .document
                    .lines
                    .iter()
                    .position(|line| line.line == cell.line)
                    .unwrap();
                let point = output.point(cell.start);
                assert_eq!(
                    point.x,
                    origin.x + cell.column as i32 * metrics.cell_width,
                    "dpi={dpi} row={} column={}",
                    cell.line,
                    cell.column
                );
                assert_eq!(
                    point.y,
                    origin.y + row as i32 * metrics.cell_height,
                    "dpi={dpi} row={} column={}",
                    cell.line,
                    cell.column
                );
            }
            let padding = visual
                .document
                .cells
                .iter()
                .find(|cell| cell.line == 12 && cell.column == 109)
                .unwrap();
            let span = visual
                .document
                .styles
                .iter()
                .find(|span| span.start <= padding.start && padding.start < span.end)
                .unwrap();
            let expected = crate::text_style::resolve(span.style, &output.theme);
            let native = native_document(output.edit).unwrap();
            let at = crate::accessible_text::native_offset(output.edit, padding.start) as i32;
            let font = unsafe { native.Range(at, at + 1).unwrap().GetFont().unwrap() };
            assert_eq!(
                unsafe { font.GetBackColor().unwrap() },
                expected.background.0 as i32
            );
            assert_eq!(
                unsafe { font.GetForeColor().unwrap() },
                expected.foreground.0 as i32
            );
            assert_eq!(
                crate::accessible_text::text(output.edit).as_deref(),
                Some(visual.document.text.as_str())
            );
            assert_eq!(unsafe { GetFocus() }, focus);
            assert!(!unsafe { IsWindowVisible(output.parent).as_bool() });
        }
        assert_eq!(source, original);
    }

    #[test]
    fn native_visual_reader_handoff_keeps_selection_menu_and_unicode_source_offsets() {
        let mut terminal = TerminalModel::new(40, 9);
        terminal.push_bytes(
            "\x1b[48;2;21;32;43m\x1b[2J\x1b[2;6HMenu\x1b[4;9H/agents 界🙂e\u{301}\x1b[7;1Hfooter"
                .as_bytes(),
        );
        let frame = terminal.live_snapshot();
        let source = terminal.reading_document();
        let original = source.clone();
        let visual = crate::tui_projection::visual_document(&frame, &source, &[]);
        let selected = "/agents 界🙂e\u{301}";
        let start = source.text[..source.text.find(selected).unwrap()]
            .encode_utf16()
            .count() as u32;
        let end = start + selected.encode_utf16().count() as u32;
        let output = HiddenOutput::new(96, 640, 300);
        let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
        output.set_document(&visual.document);
        format_grid(
            output.edit,
            &visual.document,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        crate::accessible_text::select(
            output.edit,
            visual.visual_offset(start),
            visual.visual_offset(end),
        );
        let displayed_selection = crate::accessible_text::selection(output.edit);
        let canonical = (
            visual.source_offset(displayed_selection.0),
            visual.source_offset(displayed_selection.1),
        );
        assert_eq!(canonical, (start, end));
        let mut state = crate::reading_state::ReadingState::default();
        state.capture(
            &source,
            canonical,
            visual.source_offset(visual.live_start()),
            false,
        );
        output.set_document(&source);
        format_grid(
            output.edit,
            &source,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        let saved = state.restore(&source);
        crate::restore_native_reading(
            output.edit,
            saved.selection,
            saved.viewport_offset,
            saved.follow,
        );
        assert_eq!(crate::accessible_text::selection(output.edit), (start, end));
        let native = native_document(output.edit).unwrap();
        let native_start = crate::accessible_text::native_offset(output.edit, start) as i32;
        let native_end = crate::accessible_text::native_offset(output.edit, end) as i32;
        assert_eq!(
            unsafe {
                native
                    .Range(native_start, native_end)
                    .unwrap()
                    .GetText()
                    .unwrap()
                    .to_string()
            },
            selected
        );
        state.capture(&source, (start, end), saved.viewport_offset, false);
        output.set_document(&visual.document);
        format_grid(
            output.edit,
            &visual.document,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        let saved = state.restore(&source);
        crate::restore_native_reading(
            output.edit,
            (
                visual.visual_offset(saved.selection.0),
                visual.visual_offset(saved.selection.1),
            ),
            visual.live_start(),
            false,
        );
        assert_eq!(
            crate::accessible_text::selection(output.edit),
            displayed_selection
        );
        let native = native_document(output.edit).unwrap();
        let native_start =
            crate::accessible_text::native_offset(output.edit, displayed_selection.0) as i32;
        let native_end =
            crate::accessible_text::native_offset(output.edit, displayed_selection.1) as i32;
        assert_eq!(
            unsafe {
                native
                    .Range(native_start, native_end)
                    .unwrap()
                    .GetText()
                    .unwrap()
                    .to_string()
            },
            selected
        );
        assert_eq!(source, original);
    }

    #[test]
    fn actual_font_grid_fits_all_box_right_edges_with_bold_wide_and_combining_glyphs() {
        for dpi in [96, 144] {
            let output = HiddenOutput::new(dpi, 760, 260);
            let focus = unsafe { GetFocus() };
            let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
            let (columns, _) = metrics.grid_size(760, 260).unwrap();
            let columns = usize::from(columns);
            for bold in [false, true] {
                let start = if bold { "\x1b[1m" } else { "" };
                let top = format!("┌{}┐", "─".repeat(columns - 2));
                let empty = format!("│{}│", " ".repeat(columns - 2));
                let content = format!("│Ж界e\u{301}😀{}│", " ".repeat(columns - 8));
                let bottom = format!("└{}┘", "─".repeat(columns - 2));
                let source = format!("{start}{top}\r\n{empty}\r\n{content}\r\n{bottom}\x1b[0m");
                let document = document(&source, columns);
                let original = document.clone();
                output.set_document(&document);
                crate::accessible_text::select(output.edit, 3, 8);
                let saved = crate::accessible_text::selection(output.edit);
                format_grid(
                    output.edit,
                    &document,
                    output.theme.terminal_font,
                    &metrics,
                    LayoutMode::Grid,
                )
                .unwrap();
                assert_eq!(
                    crate::accessible_text::text(output.edit).as_deref(),
                    Some(document.text.as_str())
                );
                assert_eq!(document, original);
                assert_eq!(crate::accessible_text::selection(output.edit), saved);
                assert_ne!(
                    unsafe { GetWindowLongW(output.edit, GWL_STYLE) } & ES_READONLY,
                    0
                );
                assert_eq!(unsafe { GetFocus() }, focus);
                assert!(!unsafe { IsWindowVisible(output.parent).as_bool() });
                let right_edges: Vec<_> = document
                    .cells
                    .iter()
                    .filter(|cell| cell.column == columns - 1)
                    .map(|cell| output.point(cell.start))
                    .collect();
                assert_eq!(right_edges.len(), 4);
                for (row, point) in right_edges.iter().enumerate() {
                    assert_eq!(point.x, right_edges[0].x, "dpi={dpi} bold={bold} row={row}");
                    assert_eq!(point.y, right_edges[0].y + row as i32 * metrics.cell_height);
                }
                let mut format = RECT::default();
                unsafe {
                    SendMessageW(
                        output.edit,
                        EM_GETRECT,
                        None,
                        Some(LPARAM(&mut format as *mut _ as isize)),
                    );
                }
                assert!(right_edges[0].x + metrics.cell_width <= format.right);
                let native = native_document(output.edit).unwrap();
                let font = unsafe { native.Range(0, 1).unwrap().GetFont().unwrap() };
                assert_eq!(unsafe { font.GetName().unwrap().to_string() }, metrics.face);
                assert_eq!(unsafe { font.GetBold().unwrap() } != 0, bold);
                eprintln!("native grid: dpi={dpi} bold={bold} face={} cell={}x{} inset={} cols={columns} right={} limit={}", metrics.face, metrics.cell_width, metrics.cell_height, metrics.horizontal_inset, right_edges[0].x + metrics.cell_width, format.right);
                assert_eq!(
                    format_grid(
                        output.edit,
                        &document,
                        output.theme.terminal_font,
                        &metrics,
                        LayoutMode::Grid
                    )
                    .unwrap(),
                    0
                );
            }
        }
    }

    #[test]
    fn native_block_logo_uses_the_resolved_grid_font_without_font_fallback_drift() {
        for dpi in [96, 144] {
            let output = HiddenOutput::new(dpi, 640, 200);
            let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
            let document = document("AAAAAAAAAA\r\n▀▀▀▀▀▀▀▀▀▀\r\n          Z", 80);
            output.set_document(&document);
            format_grid(
                output.edit,
                &document,
                output.theme.terminal_font,
                &metrics,
                LayoutMode::Grid,
            )
            .unwrap();
            let starts: Vec<_> = document.lines.iter().map(|line| line.start).collect();
            let widths: Vec<_> = starts
                .iter()
                .take(3)
                .map(|start| output.point(*start + 10).x - output.point(*start).x)
                .collect();
            if widths.iter().any(|width| *width != metrics.cell_width * 10) {
                output.log_fonts(&document, dpi, "initial-bad-width");
            }
            for width in widths {
                assert_eq!(width, metrics.cell_width * 10);
            }
            // A full source sync can reinstall native font bindings even if
            // the logical text/styles equal the previous display document.
            output.set_document(&document);
            assert!(
                format_grid(
                    output.edit,
                    &document,
                    output.theme.terminal_font,
                    &metrics,
                    LayoutMode::Grid
                )
                .unwrap()
                    > 0
            );
            let width = output.point(starts[1] + 10).x - output.point(starts[1]).x;
            if width != metrics.cell_width * 10 {
                output.log_fonts(&document, dpi, "repeat-bad-width");
            }
            assert_eq!(width, metrics.cell_width * 10);
            eprintln!(
                "native logo: dpi={dpi} ten cells={}px",
                metrics.cell_width * 10
            );
        }
    }

    #[test]
    fn usable_width_reserves_scrollbar_before_it_appears_and_ignores_popup_height() {
        let output = HiddenOutput::new(96, 640, 240);
        let empty = measure(output.edit, output.theme.terminal_font).unwrap();
        let expected = empty.grid_size(640, 240);
        let source = (0..150)
            .map(|row| format!("row {row}\r\n"))
            .collect::<String>();
        output.set_document(&document(&source, 80));
        let scrolling = measure(output.edit, output.theme.terminal_font).unwrap();
        assert_eq!(scrolling.horizontal_inset, empty.horizontal_inset);
        assert_eq!(scrolling.grid_size(640, 240), expected);
        unsafe {
            MoveWindow(output.edit, 0, 0, 640, 48, false).unwrap();
        }
        let popup = measure(output.edit, output.theme.terminal_font).unwrap();
        assert_eq!(popup.grid_size(640, 240), expected);
        assert_eq!(popup.vertical_inset, empty.vertical_inset);
        assert!(popup.grid_size(1, 1).is_none());
        assert_eq!(
            popup.grid_size(i32::MAX, i32::MAX),
            Some((i16::MAX as u16, i16::MAX as u16))
        );
    }

    #[test]
    fn both_modes_wrap_paragraphs_without_changing_canonical_text_or_selection() {
        let output = HiddenOutput::new(96, 320, 180);
        let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
        let source = "A logical paragraph with Unicode Ж界e\u{301}😀 and words. ".repeat(25);
        let document = document(&source, 2000);
        let original = document.clone();
        output.set_document(&document);
        crate::accessible_text::select(output.edit, 5, 20);
        let saved = crate::accessible_text::selection(output.edit);
        format_grid(
            output.edit,
            &document,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        assert!(unsafe { SendMessageW(output.edit, EM_GETLINECOUNT, None, None).0 } > 1);
        format_grid(
            output.edit,
            &document,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Reflow,
        )
        .unwrap();
        assert!(unsafe { SendMessageW(output.edit, EM_GETLINECOUNT, None, None).0 } > 1);
        assert_eq!(crate::accessible_text::selection(output.edit), saved);
        assert_eq!(
            crate::accessible_text::text(output.edit).as_deref(),
            Some(document.text.as_str())
        );
        assert_eq!(document, original);
    }

    #[test]
    fn grid_soft_wrapped_vt_paragraph_is_visible_by_vertical_scrolling_and_copies_unchanged() {
        let output = HiddenOutput::new(96, 320, 180);
        let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
        let source = "Word Ж界e\u{301}😀 value ".repeat(80) + "BKS_SOFTWRAP_END";
        let document = document(&source, 40);
        assert_eq!(
            document.lines.len(),
            1,
            "the VT soft wraps form one canonical paragraph"
        );
        assert_eq!(document.text, source);
        let first_line = document.cells.first().unwrap().line;
        assert!(document.cells.iter().any(|cell| cell.line != first_line));
        let original = document.clone();
        output.set_document(&document);
        crate::accessible_text::select(output.edit, 5, 20);
        let saved = crate::accessible_text::selection(output.edit);
        format_grid(
            output.edit,
            &document,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        assert_eq!(crate::accessible_text::selection(output.edit), saved);
        let visual_lines = unsafe { SendMessageW(output.edit, EM_GETLINECOUNT, None, None).0 };
        assert!(visual_lines > 10);
        let tail = document.cells.last().unwrap();
        let before_scroll = output.point(tail.start);
        let mut format = RECT::default();
        unsafe {
            SendMessageW(
                output.edit,
                EM_GETRECT,
                None,
                Some(LPARAM(&mut format as *mut _ as isize)),
            );
        }
        assert!(
            before_scroll.y >= format.bottom,
            "the complete answer extends below the first page"
        );
        assert!(
            before_scroll.x >= format.left && before_scroll.x + metrics.cell_width <= format.right
        );
        unsafe {
            SendMessageW(
                output.edit,
                WM_VSCROLL,
                Some(WPARAM(SB_BOTTOM.0 as usize)),
                None,
            );
        }
        let after_scroll = output.point(tail.start);
        assert!(
            after_scroll.y >= format.top && after_scroll.y + metrics.cell_height <= format.bottom
        );
        assert_eq!(after_scroll.x, before_scroll.x);
        let native = native_document(output.edit).unwrap();
        let end = crate::accessible_text::native_offset(
            output.edit,
            source.encode_utf16().count() as u32,
        );
        let copied = unsafe {
            native
                .Range(0, end as i32)
                .unwrap()
                .GetText()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            copied, source,
            "native visual wraps insert no copied line breaks"
        );
        assert_eq!(
            crate::accessible_text::text(output.edit).as_deref(),
            Some(source.as_str())
        );
        assert_eq!(document, original);
        eprintln!("native soft-wrap: logical_lines=1 visual_lines={visual_lines} tail_before={before_scroll:?} tail_after={after_scroll:?} limit={format:?}");
    }

    #[test]
    fn native_style_changes_and_appends_do_not_reuse_stale_spacing_or_move_history() {
        let output = HiddenOutput::new(96, 640, 160);
        let metrics = measure(output.edit, output.theme.terminal_font).unwrap();
        let source = (0..60)
            .map(|row| format!("row{row}: 界😀 ─▀\r\n"))
            .collect::<String>();
        let first = document(&source, 100);
        output.set_document(&first);
        format_grid(
            output.edit,
            &first,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        crate::accessible_text::select(output.edit, 4, 7);
        unsafe {
            SendMessageW(output.edit, EM_LINESCROLL, None, Some(LPARAM(8)));
        }
        let saved = crate::accessible_text::selection(output.edit);
        let mut scroll = POINT::default();
        unsafe {
            SendMessageW(
                output.edit,
                EM_GETSCROLLPOS,
                None,
                Some(LPARAM(&mut scroll as *mut _ as isize)),
            );
        }
        let next = document(&(source + "\x1b[1mwide 界😀 block ▀\x1b[0m"), 100);
        crate::accessible_text::append(output.edit, &next.text[first.text.len()..]);
        // Appending owns its caret update; formatting must preserve the
        // reading bookmark present when this separate operation begins.
        crate::accessible_text::select(output.edit, saved.0, saved.1);
        unsafe {
            SendMessageW(
                output.edit,
                EM_SETSCROLLPOS,
                None,
                Some(LPARAM(&scroll as *const _ as isize)),
            );
        }
        crate::accessible_text::format(output.edit, &next.styles, &output.theme).unwrap();
        format_grid(
            output.edit,
            &next,
            output.theme.terminal_font,
            &metrics,
            LayoutMode::Grid,
        )
        .unwrap();
        let mut after = POINT::default();
        unsafe {
            SendMessageW(
                output.edit,
                EM_GETSCROLLPOS,
                None,
                Some(LPARAM(&mut after as *mut _ as isize)),
            );
        }
        assert_eq!(crate::accessible_text::selection(output.edit), saved);
        assert_eq!(after, scroll);
        assert_eq!(
            crate::accessible_text::text(output.edit).as_deref(),
            Some(next.text.as_str())
        );
    }
}
