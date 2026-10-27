//! Native GDI rendering of the visible VT grid. Offsets refer to cells, not UTF-16.

use std::ops::Range;

use bks_terminal::{
    CellSnapshot, Color, Flags, NamedColor, ReadingCell, ReadingDocument, TerminalSnapshot,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, DeleteDC,
    DeleteObject, ExtTextOutW, FillRect, FrameRect, GetDC, GetObjectW, GetStockObject,
    GetTextMetricsW, ReleaseDC, RestoreDC, SaveDC, SelectObject, SetBkMode, SetDCBrushColor,
    SetTextAlign, SetTextColor, DC_BRUSH, ETO_CLIPPED, FW_BOLD, HBITMAP, HBRUSH, HDC, HFONT,
    HGDIOBJ, LOGFONTW, SRCCOPY, TA_LEFT, TA_NOUPDATECP, TA_TOP, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::ansi_palette::{AnsiPalette, DARK_PALETTE};
use crate::search::SearchMatch;
use crate::theme::Theme;

pub struct SearchOverlay<'a> {
    pub document: &'a ReadingDocument,
    /// SearchState results have nondecreasing UTF-16 starts and ends, including overlaps.
    pub matches: &'a [SearchMatch],
    pub current: Option<SearchMatch>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SearchHighlight {
    #[default]
    None,
    Match,
    Current,
}

fn visible_reading_cells(
    document: &ReadingDocument,
    display_offset: usize,
    rows: usize,
) -> &[ReadingCell] {
    let top = -(display_offset.min(i32::MAX as usize) as i64);
    let bottom = top + rows as i64;
    let start = document
        .cells
        .partition_point(|cell| i64::from(cell.line) < top);
    let end = document
        .cells
        .partition_point(|cell| i64::from(cell.line) < bottom);
    &document.cells[start..end]
}

/// Restrict the snapshot copy to results intersecting the visible glyphs.
/// SearchState ranges have nondecreasing starts and ends, including overlaps.
pub fn visible_matches<'a>(
    document: &ReadingDocument,
    frame: &TerminalSnapshot,
    matches: &'a [SearchMatch],
) -> &'a [SearchMatch] {
    if document.alternate_screen != frame.alternate_screen {
        return &matches[..0];
    }
    let cells = visible_reading_cells(document, frame.display_offset, frame.rows);
    let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
        return &matches[..0];
    };
    let start = matches.partition_point(|found| found.end <= first.start);
    let end = matches.partition_point(|found| found.start < last.end);
    &matches[start..end]
}

fn overlaps(cell: &ReadingCell, found: SearchMatch) -> bool {
    found.start < found.end && cell.start < found.end && found.start < cell.end
}

fn search_highlights(
    frame: &TerminalSnapshot,
    overlay: &SearchOverlay<'_>,
    visible_cols: usize,
    visible_rows: usize,
) -> Vec<SearchHighlight> {
    let rows = visible_rows.min(frame.rows);
    let cols = visible_cols.min(frame.cols);
    let mut highlights = vec![SearchHighlight::None; rows.saturating_mul(frame.cols)];
    if overlay.document.alternate_screen != frame.alternate_screen {
        return highlights;
    }
    let cells = visible_reading_cells(overlay.document, frame.display_offset, rows);
    let Some(first) = cells.first() else {
        return highlights;
    };
    let mut candidate = overlay
        .matches
        .partition_point(|found| found.end <= first.start);
    let offset = frame.display_offset.min(i32::MAX as usize) as i64;
    for cell in cells {
        if cell.column >= cols {
            continue;
        }
        while overlay
            .matches
            .get(candidate)
            .is_some_and(|found| found.end <= cell.start)
        {
            candidate += 1;
        }
        let highlight = if overlay.current.is_some_and(|found| overlaps(cell, found)) {
            SearchHighlight::Current
        } else if overlay.matches[candidate..]
            .iter()
            .take_while(|found| found.start < cell.end)
            .any(|found| overlaps(cell, *found))
        {
            SearchHighlight::Match
        } else {
            continue;
        };
        let row = (i64::from(cell.line) + offset) as usize;
        for column in cell.column..cell.column.saturating_add(cell.width).min(cols) {
            highlights[row * frame.cols + column] = highlight;
        }
    }
    highlights
}

/// The same cell size must be used for painting, ConPTY resize, and mouse selection.
pub fn cell_metrics(hwnd: HWND, font: HFONT) -> (i32, i32) {
    unsafe {
        let hdc = GetDC(Some(hwnd));
        if hdc.0.is_null() {
            return (8, 16);
        }
        let old_font = SelectObject(hdc, font.into());
        let mut metrics = TEXTMETRICW::default();
        let measured = GetTextMetricsW(hdc, &mut metrics).as_bool();
        let _ = SelectObject(hdc, old_font);
        let _ = ReleaseDC(Some(hwnd), hdc);
        if measured && metrics.tmAveCharWidth > 0 && metrics.tmHeight > 0 {
            (metrics.tmAveCharWidth, metrics.tmHeight)
        } else {
            (8, 16)
        }
    }
}

/// Return the cell under the pointer, clamped to the viewport (zero for an empty grid).
pub fn hit_test(hwnd: HWND, font: HFONT, x: i32, y: i32, cols: usize, rows: usize) -> usize {
    cell_at(x, y, cols, rows, cell_metrics(hwnd, font))
}

fn cell_at(x: i32, y: i32, cols: usize, rows: usize, metrics: (i32, i32)) -> usize {
    if cols == 0 || rows == 0 {
        return 0;
    }
    let column = (x.max(0) / metrics.0.max(1)) as usize;
    let row = (y.max(0) / metrics.1.max(1)) as usize;
    row.min(rows - 1)
        .saturating_mul(cols)
        .saturating_add(column.min(cols - 1))
}

fn selection_range(
    frame: &TerminalSnapshot,
    selection: Option<(usize, usize)>,
) -> Option<Range<usize>> {
    let (anchor, end) = selection?;
    let len = frame.cells.len().min(frame.cols.saturating_mul(frame.rows));
    let mut start = anchor.min(end).min(len);
    let mut end = anchor.max(end).min(len);
    if start == end || frame.cols == 0 {
        return None;
    }

    // Selecting either half of a wide glyph selects the complete glyph.
    if start % frame.cols != 0
        && frame.cells[start].flags.contains(Flags::WIDE_CHAR_SPACER)
        && frame.cells[start - 1].flags.contains(Flags::WIDE_CHAR)
    {
        start -= 1;
    }
    if end < len && end % frame.cols != 0 && frame.cells[end - 1].flags.contains(Flags::WIDE_CHAR) {
        end += 1;
    }
    Some(start..end)
}

/// Copy visible cells, preserving combining marks and joining soft-wrapped lines.
pub fn selected_text(frame: &TerminalSnapshot, selection: Option<(usize, usize)>) -> String {
    let Some(range) = selection_range(frame, selection) else {
        return String::new();
    };
    let mut result = String::new();
    let first_row = range.start / frame.cols;
    let last_row = (range.end - 1) / frame.cols;
    for row in first_row..=last_row {
        let start = range.start.max(row * frame.cols);
        let end = range.end.min((row + 1) * frame.cols);
        let mut line = String::new();
        for cell in &frame.cells[start..end] {
            if !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                line.push_str(&cell.text);
            }
        }
        let wrapped = end % frame.cols == 0 && frame.cells[end - 1].flags.contains(Flags::WRAPLINE);
        if end % frame.cols == 0 && !wrapped {
            line.truncate(line.trim_end_matches(' ').len());
        }
        result.push_str(&line);
        if row != last_row && !wrapped {
            result.push_str("\r\n");
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellColors {
    fg: COLORREF,
    bg: COLORREF,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

fn blend(first: COLORREF, second: COLORREF, second_percent: u32) -> COLORREF {
    let channel = |shift: u32| {
        let first = (first.0 >> shift) & 0xffu32;
        let second = (second.0 >> shift) & 0xffu32;
        ((first * (100 - second_percent) + second * second_percent) / 100) as u8
    };
    rgb(channel(0), channel(8), channel(16))
}

fn indexed_color(index: u8, palette: &AnsiPalette) -> COLORREF {
    match index {
        0..=15 => palette.colors[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let level = |component: u8| {
                if component == 0 {
                    0
                } else {
                    55 + 40 * component
                }
            };
            rgb(level(index / 36), level(index / 6 % 6), level(index % 6))
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            rgb(level, level, level)
        }
    }
}

fn resolve_color(color: Color, palette: &AnsiPalette) -> COLORREF {
    match color {
        Color::Spec(color) => rgb(color.r, color.g, color.b),
        Color::Indexed(index) => indexed_color(index, palette),
        Color::Named(name) => match name {
            NamedColor::Foreground => palette.default_fg,
            NamedColor::Background => palette.default_bg,
            NamedColor::Cursor => palette.cursor,
            NamedColor::BrightForeground => palette.colors[15],
            NamedColor::DimForeground => blend(palette.default_fg, palette.default_bg, 34),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => blend(
                palette.colors[name as usize - NamedColor::DimBlack as usize],
                palette.default_bg,
                34,
            ),
            _ => palette.colors[name as usize],
        },
    }
}

fn cell_colors(cell: &CellSnapshot, theme: &Theme, selected: bool, active_row: bool) -> CellColors {
    let mut colors = if theme.high_contrast {
        CellColors {
            fg: theme.colors.text_primary,
            bg: theme.colors.terminal,
        }
    } else {
        CellColors {
            fg: resolve_color(cell.fg, &DARK_PALETTE),
            bg: resolve_color(cell.bg, &DARK_PALETTE),
        }
    };
    if cell.flags.contains(Flags::DIM) && !theme.high_contrast {
        colors.fg = blend(colors.fg, colors.bg, 34);
    }
    if cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut colors.fg, &mut colors.bg);
    }
    if selected {
        if theme.high_contrast {
            colors.fg = theme.colors.on_primary;
            colors.bg = theme.colors.brand;
        } else {
            colors.fg = DARK_PALETTE.default_fg;
            colors.bg = DARK_PALETTE.selection_bg;
        }
    } else if active_row
        && !theme.high_contrast
        && !cell.flags.contains(Flags::INVERSE)
        && cell.bg == Color::Named(NamedColor::Background)
    {
        colors.bg = blend(colors.bg, DARK_PALETTE.cursor, 4);
    }
    colors
}

fn colors_with_search(
    cell: &CellSnapshot,
    theme: &Theme,
    selected: bool,
    active_row: bool,
    highlight: SearchHighlight,
) -> CellColors {
    if selected || highlight == SearchHighlight::None {
        return cell_colors(cell, theme, selected, active_row);
    }
    if theme.high_contrast {
        if highlight == SearchHighlight::Current {
            CellColors {
                fg: theme.colors.terminal,
                bg: theme.colors.text_primary,
            }
        } else {
            CellColors {
                fg: theme.colors.on_primary,
                bg: theme.colors.brand,
            }
        }
    } else if highlight == SearchHighlight::Current {
        CellColors {
            fg: DARK_PALETTE.default_bg,
            bg: DARK_PALETTE.colors[11],
        }
    } else {
        CellColors {
            fg: DARK_PALETTE.default_fg,
            bg: blend(DARK_PALETTE.default_bg, DARK_PALETTE.colors[3], 28),
        }
    }
}

/// At most three derived fonts per paint; the theme owns the regular font.
struct Fonts {
    handles: [HFONT; 4],
}

impl Fonts {
    fn new(base: HFONT) -> Self {
        let mut fonts = Self {
            handles: [base, HFONT::default(), HFONT::default(), HFONT::default()],
        };
        let mut source = LOGFONTW::default();
        unsafe {
            if GetObjectW(
                base.into(),
                std::mem::size_of::<LOGFONTW>() as i32,
                Some(&mut source as *mut LOGFONTW as *mut _),
            ) == 0
            {
                return fonts;
            }
            for style in 1..4 {
                let mut font = source;
                if style & 1 != 0 {
                    font.lfWeight = FW_BOLD.0 as i32;
                }
                font.lfItalic = u8::from(style & 2 != 0);
                font.lfUnderline = 0;
                font.lfStrikeOut = 0;
                fonts.handles[style] = CreateFontIndirectW(&font);
            }
        }
        fonts
    }

    fn for_flags(&self, flags: Flags) -> HFONT {
        let index = usize::from(flags.contains(Flags::BOLD))
            | (usize::from(flags.contains(Flags::ITALIC)) << 1);
        let font = self.handles[index];
        if font.0.is_null() {
            self.handles[0]
        } else {
            font
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in &self.handles[1..] {
            if !font.0.is_null() {
                unsafe {
                    let _ = DeleteObject((*font).into());
                }
            }
        }
    }
}

/// Compose backgrounds, glyphs, and the cursor before changing the window surface.
/// The paint DC keeps its update-region clip when the finished bitmap is copied.
struct BackBuffer {
    hdc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        unsafe {
            let hdc = CreateCompatibleDC(Some(target));
            if hdc.0.is_null() {
                return None;
            }
            // Use the target DC, since a new memory DC initially has a monochrome bitmap.
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.0.is_null() {
                let _ = DeleteDC(hdc);
                return None;
            }
            let previous_bitmap = SelectObject(hdc, bitmap.into());
            if previous_bitmap.0.is_null() || previous_bitmap.0 as isize == -1 {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(hdc);
                return None;
            }
            Some(Self {
                hdc,
                bitmap,
                previous_bitmap,
                width,
                height,
            })
        }
    }

    fn present(&self, target: HDC) {
        unsafe {
            let _ = BitBlt(
                target,
                0,
                0,
                self.width,
                self.height,
                Some(self.hdc),
                0,
                0,
                SRCCOPY,
            );
        }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.hdc, self.previous_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.hdc);
        }
    }
}

struct Painter {
    hdc: HDC,
    brush: HBRUSH,
}

impl Painter {
    fn fill(&self, rect: &RECT, color: COLORREF) {
        unsafe {
            let _ = SetDCBrushColor(self.hdc, color);
            FillRect(self.hdc, rect, self.brush);
        }
    }

    fn text(&self, cell: &CellSnapshot, rect: &RECT, color: COLORREF, fonts: &Fonts) {
        if cell
            .flags
            .intersects(Flags::HIDDEN | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            return;
        }
        if !cell.text.is_empty() && cell.text != " " {
            let text: Vec<u16> = cell.text.encode_utf16().collect();
            unsafe {
                let _ = SelectObject(self.hdc, fonts.for_flags(cell.flags).into());
                let _ = SetTextColor(self.hdc, color);
                let _ = ExtTextOutW(
                    self.hdc,
                    rect.left,
                    rect.top,
                    ETO_CLIPPED,
                    Some(rect),
                    PCWSTR(text.as_ptr()),
                    text.len() as u32,
                    None,
                );
            }
        }
        let thickness = ((rect.bottom - rect.top) / 16).max(1);
        if cell.flags.intersects(Flags::ALL_UNDERLINES) {
            let y = rect.bottom - thickness - 1;
            self.rule(rect, y, thickness, color);
            if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
                self.rule(rect, y - thickness * 2, thickness, color);
            }
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            self.rule(rect, (rect.top + rect.bottom) / 2, thickness, color);
        }
    }

    fn rule(&self, rect: &RECT, top: i32, thickness: i32, color: COLORREF) {
        self.fill(
            &RECT {
                left: rect.left,
                top: top.max(rect.top),
                right: rect.right,
                bottom: (top + thickness).min(rect.bottom),
            },
            color,
        );
    }
}

fn pixel(index: usize, size: i32) -> i32 {
    index.saturating_mul(size as usize).min(i32::MAX as usize) as i32
}

fn cell_rect(row: usize, column: usize, width: usize, metrics: (i32, i32)) -> RECT {
    RECT {
        left: pixel(column, metrics.0),
        top: pixel(row, metrics.1),
        right: pixel(column.saturating_add(width), metrics.0),
        bottom: pixel(row.saturating_add(1), metrics.1),
    }
}

pub fn paint(
    hwnd: HWND,
    hdc: HDC,
    frame: &TerminalSnapshot,
    theme: &Theme,
    selection: Option<(usize, usize)>,
    focused: bool,
) {
    paint_with_search(hwnd, hdc, frame, theme, selection, focused, None);
}

pub fn paint_with_search(
    hwnd: HWND,
    hdc: HDC,
    frame: &TerminalSnapshot,
    theme: &Theme,
    selection: Option<(usize, usize)>,
    focused: bool,
    overlay: Option<&SearchOverlay<'_>>,
) {
    let metrics = cell_metrics(hwnd, theme.terminal_font);
    let mut client = RECT::default();
    unsafe {
        if GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
    }
    if client.right <= 0 || client.bottom <= 0 {
        return;
    }
    let target = hdc;
    // Fall back to direct painting if GDI cannot allocate a compatible bitmap.
    let buffer = BackBuffer::new(target, client.right, client.bottom);
    let hdc = buffer.as_ref().map_or(target, |buffer| buffer.hdc);
    let saved = unsafe { SaveDC(hdc) };
    let old_font = unsafe { SelectObject(hdc, theme.terminal_font.into()) };
    let fonts = Fonts::new(theme.terminal_font);
    let painter = Painter {
        hdc,
        brush: HBRUSH(unsafe { GetStockObject(DC_BRUSH) }.0),
    };
    unsafe {
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextAlign(hdc, TA_LEFT | TA_TOP | TA_NOUPDATECP);
    }
    painter.fill(
        &client,
        if theme.high_contrast {
            theme.colors.terminal
        } else {
            DARK_PALETTE.default_bg
        },
    );

    let visible_cols = (client.right.max(0) as usize)
        .div_ceil(metrics.0 as usize)
        .min(frame.cols);
    let visible_rows = (client.bottom.max(0) as usize)
        .div_ceil(metrics.1 as usize)
        .min(frame.rows);
    let selection = selection_range(frame, selection);
    let is_selected = |index| {
        selection
            .as_ref()
            .is_some_and(|range| range.contains(&index))
    };
    let search =
        overlay.map(|overlay| search_highlights(frame, overlay, visible_cols, visible_rows));
    let search_at = |index| {
        search
            .as_ref()
            .and_then(|highlights| highlights.get(index))
            .copied()
            .unwrap_or_default()
    };

    // Paint all backgrounds first: a spacer must not erase its wide glyph.
    for row in 0..visible_rows {
        let active_row = focused
            && frame
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.row == row);
        let mut column = 0;
        while column < visible_cols {
            let index = row * frame.cols + column;
            let Some(cell) = frame.cells.get(index) else {
                break;
            };
            let color = colors_with_search(
                cell,
                theme,
                is_selected(index),
                active_row,
                search_at(index),
            )
            .bg;
            let start = column;
            column += 1;
            while column < visible_cols {
                let index = row * frame.cols + column;
                let Some(cell) = frame.cells.get(index) else {
                    break;
                };
                if colors_with_search(
                    cell,
                    theme,
                    is_selected(index),
                    active_row,
                    search_at(index),
                )
                .bg != color
                {
                    break;
                }
                column += 1;
            }
            painter.fill(&cell_rect(row, start, column - start, metrics), color);
        }
    }
    for row in 0..visible_rows {
        let active_row = focused
            && frame
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.row == row);
        for column in 0..visible_cols {
            let index = row * frame.cols + column;
            let Some(cell) = frame.cells.get(index) else {
                break;
            };
            let width = if cell.flags.contains(Flags::WIDE_CHAR) && column + 1 < frame.cols {
                2
            } else {
                1
            };
            let highlight = search_at(index);
            let colors = colors_with_search(cell, theme, is_selected(index), active_row, highlight);
            painter.text(
                cell,
                &cell_rect(row, column, width, metrics),
                colors.fg,
                &fonts,
            );
            if highlight == SearchHighlight::Current && !is_selected(index) {
                // Keep the current result distinct even when a High Contrast
                // theme uses the same highlight and window-text colors.
                let rect = cell_rect(row, column, width, metrics);
                painter.rule(&rect, rect.bottom - 2, 1, colors.fg);
            }
        }
    }

    if let Some(cursor) = frame.cursor.as_ref() {
        if cursor.row < visible_rows && cursor.column < visible_cols {
            let mut column = cursor.column;
            let mut index = cursor.row * frame.cols + column;
            if column > 0
                && frame
                    .cells
                    .get(index)
                    .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            {
                column -= 1;
                index -= 1;
            }
            if let Some(cell) = frame.cells.get(index) {
                let width = if cell.flags.contains(Flags::WIDE_CHAR) && column + 1 < frame.cols {
                    2
                } else {
                    1
                };
                let rect = cell_rect(cursor.row, column, width, metrics);
                let (cursor_color, text_color) = if theme.high_contrast {
                    (theme.colors.text_primary, theme.colors.terminal)
                } else {
                    (DARK_PALETTE.cursor, DARK_PALETTE.default_bg)
                };
                if focused {
                    painter.fill(&rect, cursor_color);
                    painter.text(cell, &rect, text_color, &fonts);
                } else {
                    unsafe {
                        let _ = SetDCBrushColor(hdc, cursor_color);
                        FrameRect(hdc, &rect, painter.brush);
                    }
                }
            }
        }
    }

    // Deselect every owned font before Fonts deletes it, even if SaveDC failed.
    unsafe {
        let _ = SelectObject(hdc, old_font);
        if saved != 0 {
            let _ = RestoreDC(hdc, saved);
        }
    }
    if let Some(buffer) = buffer {
        buffer.present(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{Rgb, TerminalModel};

    #[test]
    fn back_buffer_keeps_partial_paint_off_the_target_surface() {
        use windows::Win32::Graphics::Gdi::GetPixel;

        let screen = unsafe { GetDC(None) };
        assert!(!screen.0.is_null());
        let target = BackBuffer::new(screen, 4, 4);
        let buffer = BackBuffer::new(screen, 4, 4);
        unsafe {
            let _ = ReleaseDC(None, screen);
        }
        let target = target.expect("target bitmap");
        let buffer = buffer.expect("back buffer bitmap");
        let brush = HBRUSH(unsafe { GetStockObject(DC_BRUSH) }.0);
        let old_color = rgb(10, 20, 30);
        let background = rgb(40, 50, 60);
        let glyph = rgb(70, 80, 90);
        let client = RECT {
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        };
        Painter {
            hdc: target.hdc,
            brush,
        }
        .fill(&client, old_color);
        let painter = Painter {
            hdc: buffer.hdc,
            brush,
        };
        painter.fill(&client, background);
        painter.fill(
            &RECT {
                right: 1,
                bottom: 1,
                ..client
            },
            glyph,
        );
        // Neither a background clear nor subsequent drawing reaches the target early.
        assert_eq!(unsafe { GetPixel(target.hdc, 0, 0) }, old_color);
        assert_eq!(unsafe { GetPixel(target.hdc, 3, 3) }, old_color);
        buffer.present(target.hdc);
        assert_eq!(unsafe { GetPixel(target.hdc, 0, 0) }, glyph);
        assert_eq!(unsafe { GetPixel(target.hdc, 3, 3) }, background);
        assert!(BackBuffer::new(target.hdc, 0, 4).is_none());
        assert!(BackBuffer::new(target.hdc, 4, 0).is_none());
    }

    #[test]
    fn indexed_palette_and_cube_boundaries() {
        for index in 0..16 {
            assert_eq!(
                indexed_color(index, &DARK_PALETTE),
                DARK_PALETTE.colors[index as usize]
            );
        }
        for (index, color) in [
            (16, rgb(0, 0, 0)),
            (21, rgb(0, 0, 255)),
            (46, rgb(0, 255, 0)),
            (196, rgb(255, 0, 0)),
            (208, rgb(255, 135, 0)),
            (231, rgb(255, 255, 255)),
            (232, rgb(8, 8, 8)),
            (255, rgb(238, 238, 238)),
        ] {
            assert_eq!(indexed_color(index, &DARK_PALETTE), color, "index {index}");
        }
    }

    #[test]
    fn named_defaults_and_truecolor() {
        assert_eq!(
            resolve_color(Color::Named(NamedColor::Foreground), &DARK_PALETTE),
            DARK_PALETTE.default_fg
        );
        assert_eq!(
            resolve_color(Color::Named(NamedColor::Background), &DARK_PALETTE),
            crate::theme::BG_TERMINAL
        );
        assert_eq!(
            resolve_color(Color::Named(NamedColor::BrightRed), &DARK_PALETTE),
            DARK_PALETTE.colors[9]
        );
        assert_eq!(
            resolve_color(
                Color::Spec(Rgb {
                    r: 17,
                    g: 34,
                    b: 51
                }),
                &DARK_PALETTE
            ),
            COLORREF(0x0033_2211)
        );
    }

    #[test]
    fn hit_testing_uses_cells_and_clamps_edges() {
        assert_eq!(cell_at(23, 39, 10, 3, (8, 20)), 12);
        assert_eq!(cell_at(-1, -30, 10, 3, (8, 20)), 0);
        assert_eq!(cell_at(800, 600, 10, 3, (8, 20)), 29);
        assert_eq!(cell_at(10, 10, 0, 0, (8, 20)), 0);
    }

    #[test]
    fn selection_preserves_combining_marks_and_wide_glyphs() {
        let mut model = TerminalModel::new(12, 2);
        model.push_bytes("Aé界e\u{301}🙂Z".as_bytes());
        let frame = model.snapshot();
        assert_eq!(selected_text(&frame, Some((0, 8))), "Aé界e\u{301}🙂Z");
        assert_eq!(selected_text(&frame, Some((3, 4))), "界");
        assert_eq!(selected_text(&frame, Some((2, 3))), "界");
        assert_eq!(selected_text(&frame, Some((8, 0))), "Aé界e\u{301}🙂Z");
        assert_eq!(selected_text(&frame, Some((4, 4))), "");
        assert_eq!(selected_text(&frame, None), "");
    }

    #[test]
    fn copying_joins_wraps_and_preserves_hard_breaks() {
        let mut model = TerminalModel::new(4, 3);
        model.push_bytes(b"abcdef\r\nxy");
        assert_eq!(
            selected_text(&model.snapshot(), Some((0, 10))),
            "abcdef\r\nxy"
        );
        assert_eq!(selected_text(&model.snapshot(), Some((4, 6))), "ef");
        assert_eq!(selected_text(&model.snapshot(), Some((100, 120))), "");
    }

    #[test]
    fn highlight_preserves_explicit_background_and_inverse() {
        let mut model = TerminalModel::new(8, 1);
        model.push_bytes(b"A\x1b[48;2;10;20;30mB\x1b[0;7mC");
        let frame = model.snapshot();
        let theme = Theme::default();
        assert_ne!(
            cell_colors(&frame.cells[0], &theme, false, true).bg,
            DARK_PALETTE.default_bg
        );
        assert_eq!(
            cell_colors(&frame.cells[1], &theme, false, true).bg,
            rgb(10, 20, 30)
        );
        let inverse = cell_colors(&frame.cells[2], &theme, false, true);
        assert_eq!(inverse.fg, DARK_PALETTE.default_bg);
        assert_eq!(inverse.bg, DARK_PALETTE.default_fg);
        assert_eq!(
            cell_colors(&frame.cells[1], &theme, true, true).bg,
            DARK_PALETTE.selection_bg
        );
    }

    #[test]
    fn high_contrast_uses_system_colors_for_text_and_selection() {
        let mut model = TerminalModel::new(4, 1);
        model.push_bytes(b"\x1b[31;42mX");
        let frame = model.snapshot();
        let mut theme = Theme {
            high_contrast: true,
            ..Theme::default()
        };
        theme.colors.text_primary = rgb(255, 255, 0);
        theme.colors.terminal = rgb(0, 0, 0);
        theme.colors.brand = rgb(0, 0, 255);
        theme.colors.on_primary = rgb(255, 255, 255);
        assert_eq!(
            cell_colors(&frame.cells[0], &theme, false, true),
            CellColors {
                fg: theme.colors.text_primary,
                bg: theme.colors.terminal
            }
        );
        assert_eq!(
            cell_colors(&frame.cells[0], &theme, true, true),
            CellColors {
                fg: theme.colors.on_primary,
                bg: theme.colors.brand
            }
        );
    }

    #[test]
    fn search_maps_surrogates_wide_cells_and_combining_marks() {
        let mut model = TerminalModel::new(12, 2);
        model.push_bytes("A😀界e\u{301}Z".as_bytes());
        let frame = model.snapshot();
        let document = model.reading_document();
        let matches = [
            SearchMatch { start: 1, end: 3 },
            SearchMatch { start: 3, end: 4 },
            SearchMatch { start: 5, end: 6 },
        ];
        let overlay = SearchOverlay {
            document: &document,
            matches: &matches,
            current: Some(matches[2]),
        };
        let highlights = search_highlights(&frame, &overlay, frame.cols, frame.rows);
        assert_eq!(
            &highlights[..7],
            &[
                SearchHighlight::None,
                SearchHighlight::Match,
                SearchHighlight::Match,
                SearchHighlight::Match,
                SearchHighlight::Match,
                SearchHighlight::Current,
                SearchHighlight::None,
            ]
        );
        // A clipped wide glyph still marks its visible leading column only.
        let clipped = search_highlights(&frame, &overlay, 4, frame.rows);
        assert_eq!(clipped[3], SearchHighlight::Match);
        assert_eq!(clipped[4], SearchHighlight::None);
    }

    #[test]
    fn search_uses_history_offset_and_clips_match_snapshot() {
        let mut model = TerminalModel::new(8, 2);
        model.push_bytes(b"old\r\nmid\r\nnew");
        let document = model.reading_document();
        let matches = [
            SearchMatch { start: 0, end: 3 },
            SearchMatch { start: 10, end: 13 },
        ];
        let overlay = SearchOverlay {
            document: &document,
            matches: &matches,
            current: Some(matches[0]),
        };
        let bottom = model.snapshot();
        assert_eq!(visible_matches(&document, &bottom, &matches), &matches[1..]);
        let bottom_marks = search_highlights(&bottom, &overlay, bottom.cols, bottom.rows);
        assert!(bottom_marks[..8]
            .iter()
            .all(|mark| *mark == SearchHighlight::None));
        assert_eq!(&bottom_marks[8..11], &[SearchHighlight::Match; 3]);
        model.scroll_display(1);
        let history = model.snapshot();
        assert_eq!(
            visible_matches(&document, &history, &matches),
            &matches[..1]
        );
        let history_marks = search_highlights(&history, &overlay, history.cols, history.rows);
        assert_eq!(&history_marks[..3], &[SearchHighlight::Current; 3]);
        assert!(history_marks[3..]
            .iter()
            .all(|mark| *mark == SearchHighlight::None));
    }

    #[test]
    fn search_visible_cell_slice_is_bounded_by_the_viewport() {
        let mut model = TerminalModel::new(4, 2);
        for _ in 0..400 {
            model.push_bytes(b"ab\r\n");
        }
        let document = model.reading_document();
        assert!(document.cells.len() > 700);
        let cells = visible_reading_cells(&document, 20, 2);
        assert!(cells.len() <= 8);
        assert!(cells.iter().all(|cell| (-20..-18).contains(&cell.line)));
    }

    #[test]
    fn search_ignores_crlf_and_uses_exclusive_end_offsets() {
        let mut model = TerminalModel::new(8, 2);
        model.push_bytes(b"ab\r\ncd");
        let frame = model.snapshot();
        let document = model.reading_document();
        let matches = [
            SearchMatch { start: 0, end: 1 },
            SearchMatch { start: 2, end: 4 },
        ];
        let overlay = SearchOverlay {
            document: &document,
            matches: &matches,
            current: None,
        };
        let highlights = search_highlights(&frame, &overlay, frame.cols, frame.rows);
        assert_eq!(highlights[0], SearchHighlight::Match);
        assert!(highlights[1..]
            .iter()
            .all(|mark| *mark == SearchHighlight::None));
    }

    #[test]
    fn selection_wins_and_closing_search_restores_ansi_without_mutation() {
        let mut model = TerminalModel::new(8, 2);
        model.push_bytes(b"\x1b[1;38;2;17;34;51;48;2;10;20;30mX");
        let frame = model.snapshot();
        let document = model.reading_document();
        let before_frame = frame.clone();
        let before_document = document.clone();
        let matches = [SearchMatch { start: 0, end: 1 }];
        let overlay = SearchOverlay {
            document: &document,
            matches: &matches,
            current: Some(matches[0]),
        };
        let highlights = search_highlights(&frame, &overlay, frame.cols, frame.rows);
        let cell = &frame.cells[0];
        let theme = Theme::default();
        let ansi = cell_colors(cell, &theme, false, false);
        let selected = cell_colors(cell, &theme, true, false);
        assert_ne!(
            colors_with_search(cell, &theme, false, false, SearchHighlight::Match),
            ansi
        );
        assert_ne!(
            colors_with_search(cell, &theme, false, false, highlights[0]),
            ansi
        );
        assert_eq!(
            colors_with_search(cell, &theme, true, false, highlights[0]),
            selected
        );
        assert_eq!(
            colors_with_search(cell, &theme, false, false, SearchHighlight::None),
            ansi
        );
        assert_eq!(
            ansi,
            CellColors {
                fg: rgb(17, 34, 51),
                bg: rgb(10, 20, 30)
            }
        );
        assert_eq!(frame, before_frame);
        assert_eq!(document, before_document);
    }

    #[test]
    fn search_high_contrast_uses_system_pairs_and_ignores_stale_screen() {
        let mut model = TerminalModel::new(8, 2);
        model.push_bytes(b"X");
        let frame = model.snapshot();
        let mut document = model.reading_document();
        let theme = Theme {
            high_contrast: true,
            ..Theme::default()
        };
        let normal = colors_with_search(
            &frame.cells[0],
            &theme,
            false,
            false,
            SearchHighlight::Match,
        );
        let current = colors_with_search(
            &frame.cells[0],
            &theme,
            false,
            false,
            SearchHighlight::Current,
        );
        assert_eq!(
            normal,
            CellColors {
                fg: theme.colors.on_primary,
                bg: theme.colors.brand
            }
        );
        assert_eq!(
            current,
            CellColors {
                fg: theme.colors.terminal,
                bg: theme.colors.text_primary
            }
        );
        assert_ne!(normal, current);
        let matches = [SearchMatch { start: 0, end: 1 }];
        document.alternate_screen = !frame.alternate_screen;
        let overlay = SearchOverlay {
            document: &document,
            matches: &matches,
            current: Some(matches[0]),
        };
        assert!(visible_matches(&document, &frame, &matches).is_empty());
        assert!(search_highlights(&frame, &overlay, frame.cols, frame.rows)
            .iter()
            .all(|mark| *mark == SearchHighlight::None));
    }
}
