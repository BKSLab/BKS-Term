//! Paint-only caret/selection accents over native text. Coordinates and ranges
//! are read from the control; text, formatting and selection are never changed.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetSysColor, GetTextMetricsW, ReleaseDC, SelectObject, COLOR_BTNFACE, COLOR_HIGHLIGHT,
    COLOR_HIGHLIGHTTEXT, HGDIOBJ, TA_BOTTOM, TEXTMETRICW,
};
use windows::Win32::UI::Controls::RichEdit::{
    tomAllowOffClient, tomClientCoord, tomConstants, tomStart, ITextDocument,
};
use windows::Win32::UI::Controls::{
    EM_GETFIRSTVISIBLELINE, EM_GETRECT, EM_GETSEL, EM_LINEFROMCHAR, EM_POSFROMCHAR,
};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_GETFONT, WM_GETTEXTLENGTH};

use crate::text_style::blend;
use crate::theme;

#[derive(Default)]
pub struct Decorations {
    pub row: Option<RECT>,
    pub selection: Vec<RECT>,
    pub alpha: u8,
    focused: bool,
}

impl Decorations {
    pub fn native(
        hwnd: HWND,
        document: Option<&ITextDocument>,
        bounds: RECT,
        alpha: u8,
        focused: bool,
    ) -> Self {
        let mut result = Self {
            alpha,
            focused,
            ..Self::default()
        };
        unsafe {
            let (start, end, first, last) = if let Some(document) = document {
                let Ok(selection) = document.GetSelection() else {
                    return result;
                };
                let (Ok(start), Ok(end)) = (selection.GetStart(), selection.GetEnd()) else {
                    return result;
                };
                if start == end && !focused {
                    return result;
                }
                let first = rich_line(document, start);
                let last = if start == end {
                    first
                } else {
                    rich_line(document, end)
                };
                (start, end, first, last)
            } else {
                let (mut start, mut end) = (0u32, 0u32);
                SendMessageW(
                    hwnd,
                    EM_GETSEL,
                    Some(windows::Win32::Foundation::WPARAM(
                        &mut start as *mut _ as usize,
                    )),
                    Some(windows::Win32::Foundation::LPARAM(
                        &mut end as *mut _ as isize,
                    )),
                );
                if start == end {
                    return result;
                }
                let height = edit_line_height(hwnd);
                (
                    start as i32,
                    end as i32,
                    edit_line(hwnd, start, height),
                    edit_line(hwnd, end, height),
                )
            };
            let (Some(first), Some(last)) = (first, last) else {
                return result;
            };
            if start == end {
                if focused {
                    result.row = clipped(
                        RECT {
                            left: bounds.left,
                            right: bounds.right,
                            top: first.0.y,
                            bottom: first.1,
                        },
                        bounds,
                    );
                }
            } else {
                result.selection = selection_rectangles(bounds, first, last);
            }
        }
        result
    }

    pub fn apply(&self, pixels: &mut [u32], width: usize, background: COLORREF) {
        if width == 0 {
            return;
        }
        let height = pixels.len() / width;
        let matte = pixel(background, self.alpha);
        let opaque = pixel(background, 255);
        if let Some(row) = self.row {
            let color = pixel(
                blend(background, theme::BRAND_PRIMARY, theme::CARET_ROW_TINT),
                self.alpha,
            );
            visit(pixels, width, height, row, |value| {
                if *value == matte || *value == opaque {
                    *value = color;
                }
            });
        }
        if !self.selection.is_empty() {
            let native = unsafe {
                [
                    pixel(COLORREF(GetSysColor(COLOR_HIGHLIGHT)), 255),
                    pixel(COLORREF(GetSysColor(COLOR_BTNFACE)), 255),
                ]
            };
            let foreground = unsafe { pixel(COLORREF(GetSysColor(COLOR_HIGHLIGHTTEXT)), 255) };
            let tint = if self.focused {
                theme::TEXT_SELECTION_TINT
            } else {
                theme::INACTIVE_SELECTION_TINT
            };
            let color = pixel(blend(background, theme::BRAND_PRIMARY, tint), self.alpha);
            for &rect in &self.selection {
                visit(pixels, width, height, rect, |value| {
                    if native.contains(value) {
                        *value = color;
                    } else if let Some(edge) = selection_edge(*value, native[0], foreground, color)
                    {
                        *value = edge;
                    }
                });
            }
        }
    }
}

unsafe fn rich_line(document: &ITextDocument, position: i32) -> Option<(POINT, i32)> {
    let range = document.Range(position, position).ok()?;
    let flags = tomStart.0 | tomClientCoord.0 | tomAllowOffClient.0;
    let (mut x, mut top, mut bottom) = (0, 0, 0);
    range.GetPoint(tomConstants(flags), &mut x, &mut top).ok()?;
    let mut bottom_x = 0;
    range
        .GetPoint(
            tomConstants(flags | TA_BOTTOM.0 as i32),
            &mut bottom_x,
            &mut bottom,
        )
        .ok()?;
    (bottom > top).then_some((POINT { x, y: top }, bottom))
}

unsafe fn edit_line_height(hwnd: HWND) -> i32 {
    let dc = GetDC(Some(hwnd));
    let font = SendMessageW(hwnd, WM_GETFONT, None, None);
    let previous = (font.0 != 0).then(|| SelectObject(dc, HGDIOBJ(font.0 as *mut _)));
    let mut metrics = TEXTMETRICW::default();
    let _ = GetTextMetricsW(dc, &mut metrics);
    if let Some(previous) = previous {
        SelectObject(dc, previous);
    }
    ReleaseDC(Some(hwnd), dc);
    (metrics.tmHeight + metrics.tmExternalLeading).max(1)
}

unsafe fn edit_line(hwnd: HWND, position: u32, height: i32) -> Option<(POINT, i32)> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    let point = SendMessageW(hwnd, EM_POSFROMCHAR, Some(WPARAM(position as usize)), None);
    let top = if point.0 != -1 {
        POINT {
            x: point.0 as u16 as i16 as i32,
            y: ((point.0 as u32 >> 16) as u16 as i16) as i32,
        }
    } else {
        let length = SendMessageW(hwnd, WM_GETTEXTLENGTH, None, None).0;
        if i64::from(position) != length as i64 {
            return None;
        }
        let first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0 as i32;
        let line =
            SendMessageW(hwnd, EM_LINEFROMCHAR, Some(WPARAM(position as usize)), None).0 as i32;
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            EM_GETRECT,
            None,
            Some(LPARAM(&mut rect as *mut _ as isize)),
        );
        POINT {
            x: rect.right,
            y: rect
                .top
                .saturating_add((line - first).saturating_mul(height)),
        }
    };
    Some((top, top.y.saturating_add(height)))
}

fn clipped(rect: RECT, bounds: RECT) -> Option<RECT> {
    let rect = RECT {
        left: rect.left.max(bounds.left),
        top: rect.top.max(bounds.top),
        right: rect.right.min(bounds.right),
        bottom: rect.bottom.min(bounds.bottom),
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

fn selection_rectangles(bounds: RECT, first: (POINT, i32), last: (POINT, i32)) -> Vec<RECT> {
    let mut result = Vec::with_capacity(3);
    let mut push = |rect| {
        if let Some(rect) = clipped(rect, bounds) {
            result.push(rect);
        }
    };
    if first.0.y == last.0.y {
        push(RECT {
            left: first.0.x.min(last.0.x),
            right: first.0.x.max(last.0.x),
            top: first.0.y,
            bottom: first.1.max(last.1),
        });
    } else {
        push(RECT {
            left: first.0.x,
            right: bounds.right,
            top: first.0.y,
            bottom: first.1,
        });
        push(RECT {
            left: bounds.left,
            right: bounds.right,
            top: first.1,
            bottom: last.0.y,
        });
        push(RECT {
            left: bounds.left,
            right: last.0.x,
            top: last.0.y,
            bottom: last.1,
        });
    }
    result
}

fn visit(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    rect: RECT,
    mut change: impl FnMut(&mut u32),
) {
    let Some(rect) = clipped(
        rect,
        RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
    ) else {
        return;
    };
    for y in rect.top as usize..rect.bottom as usize {
        for value in &mut pixels[y * width + rect.left as usize..y * width + rect.right as usize] {
            change(value);
        }
    }
}

fn pixel(color: COLORREF, alpha: u8) -> u32 {
    let a = u32::from(alpha);
    let channel = |shift: u32| (((color.0 >> shift) & 255u32) * a + 127) / 255;
    (a << 24) | (channel(0) << 16) | (channel(8) << 8) | channel(16)
}

/// Reconstruct native antialias coverage over the softer background. A flat
/// fill replacement alone leaves blue ClearType fringes around white glyphs.
fn selection_edge(value: u32, background: u32, foreground: u32, target: u32) -> Option<u32> {
    if value >> 24 != 255 {
        return None;
    }
    let mut result = 0;
    let mut coverage = [0; 3];
    let mut shared = [false; 3];
    let mut maximum = 0;
    for (index, shift) in [0, 8, 16].into_iter().enumerate() {
        let channel = |pixel: u32| ((pixel >> shift) & 255) as i32;
        let (v, b, f) = (channel(value), channel(background), channel(foreground));
        if v < b.min(f) || v > b.max(f) {
            return None;
        }
        if b == f {
            shared[index] = true;
        } else {
            coverage[index] = (v - b).abs() * 255 / (f - b).abs();
            maximum = maximum.max(coverage[index]);
        }
    }
    for (index, shift) in [0, 8, 16].into_iter().enumerate() {
        let coverage = if shared[index] {
            maximum
        } else {
            coverage[index]
        };
        let foreground = ((foreground >> shift) & 255) as i32;
        let target = ((target >> shift) & 255) as i32;
        let component = (target * (255 - coverage) + foreground * coverage + 127) / 255;
        result |= (component as u32) << shift;
    }
    let alpha = (target >> 24) as i32;
    result |= (((alpha * (255 - maximum) + 255 * maximum + 127) / 255) as u32) << 24;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_row_changes_only_backdrop_pixels_in_its_clipped_band() {
        let background = theme::BG_TERMINAL;
        let matte = pixel(background, 144);
        let foreground = pixel(theme::TEXT_PRIMARY, 255);
        let ansi = pixel(COLORREF(0x00102080), 255);
        let mut pixels = vec![matte; 24];
        pixels[9] = foreground;
        pixels[11] = ansi;
        let original = pixels.clone();
        Decorations {
            row: Some(RECT {
                left: -10,
                top: 1,
                right: 99,
                bottom: 2,
            }),
            alpha: 144,
            focused: true,
            ..Default::default()
        }
        .apply(&mut pixels, 8, background);
        assert_eq!(&pixels[..8], &original[..8]);
        assert_eq!(&pixels[16..], &original[16..]);
        assert_ne!(pixels[8], matte);
        assert_eq!(pixels[8] >> 24, 144);
        assert_eq!(pixels[9], foreground);
        assert_eq!(pixels[11], ansi);
    }

    #[test]
    fn selection_softens_native_fill_inside_the_range_without_recoloring_glyphs() {
        let background = theme::BG_TERMINAL;
        let native = unsafe { pixel(COLORREF(GetSysColor(COLOR_HIGHLIGHT)), 255) };
        let white = pixel(COLORREF(0x00ffffff), 255);
        let bounds = RECT {
            left: 2,
            top: 1,
            right: 6,
            bottom: 2,
        };
        for alpha in [144, 255] {
            let mut pixels = vec![pixel(background, alpha); 24];
            for index in [0, 10, 11, 12, 13] {
                pixels[index] = native;
            }
            pixels[11] = white;
            let native_color = unsafe { COLORREF(GetSysColor(COLOR_HIGHLIGHT)) };
            pixels[12] = pixel(blend(native_color, COLORREF(0x00ffffff), 40), 255);
            Decorations {
                selection: vec![bounds],
                alpha,
                focused: true,
                ..Default::default()
            }
            .apply(&mut pixels, 8, background);
            assert_eq!(
                pixels[0], native,
                "unselected truecolor pixels keep their color"
            );
            assert_eq!(
                pixels[11], white,
                "native selected glyph stays opaque and sharp"
            );
            assert_eq!(pixels[10], pixel(theme::TEXT_SELECTION_BG, alpha));
            assert_eq!(pixels[13] >> 24, u32::from(alpha));
            assert_ne!(pixels[12], native);
            for value in &pixels {
                for shift in [0, 8, 16] {
                    assert!((value >> shift) & 255 <= value >> 24);
                }
            }
        }
    }

    #[test]
    fn antialias_reconstruction_handles_saturated_system_selection_colors() {
        let target = pixel(theme::TEXT_SELECTION_BG, 144);
        for background in [0xff0078d7, 0xff3399ff, 0xff0000ff] {
            let mut edge = 0xff000000;
            for shift in [0, 8, 16] {
                edge |= ((((background >> shift) & 255) * 3 + 255 * 2) / 5) << shift;
            }
            let recolored = selection_edge(edge, background, 0xffffffff, target).unwrap();
            assert_ne!(recolored, edge);
            assert!((145..255).contains(&(recolored >> 24)));
            for shift in [0, 8, 16] {
                assert!((recolored >> shift) & 255 <= recolored >> 24);
            }
            assert_eq!(
                selection_edge(0xffffffff, background, 0xffffffff, target),
                Some(0xffffffff)
            );
        }
    }

    #[test]
    fn multiline_selection_clips_offscreen_endpoints_to_visible_native_rows() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 60,
        };
        let rectangles = selection_rectangles(
            bounds,
            (POINT { x: 45, y: -20 }, 0),
            (POINT { x: 35, y: 40 }, 60),
        );
        assert_eq!(
            rectangles,
            [
                RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 40
                },
                RECT {
                    left: 0,
                    top: 40,
                    right: 35,
                    bottom: 60
                }
            ]
        );
    }
}
