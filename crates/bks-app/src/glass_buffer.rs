//! Alpha-preserving GDI buffering for the DWM-extended chrome.
//!
//! Draw a plain RGB gradient, call `seal_background`, then draw foreground GDI
//! text and shapes. Alternatively, nested buffers can supply premultiplied
//! backgrounds; `preserve_background` records them without multiplying twice.
//! `finish` repairs GDI's missing alpha and copies BGRA bytes with SRCCOPY.
//! This is for the application's MM_TEXT screen DCs, not printing or transforms.

use std::cell::RefCell;
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetBkColor,
    GetBkMode, GetCurrentObject, GetMapMode, GetTextAlign, GetTextColor, GetViewportOrgEx,
    SelectObject, SetBkColor, SetBkMode, SetTextAlign, SetTextColor, SetViewportOrgEx,
    BACKGROUND_MODE, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DRAW_TEXT_FORMAT,
    HBITMAP, HDC, HGDIOBJ, MM_TEXT, OBJ_BITMAP, OBJ_BRUSH, OBJ_FONT, OBJ_PEN, SRCCOPY,
};
use windows::Win32::UI::Controls::{
    CloseThemeData, DrawThemeTextEx, OpenThemeData, DTTOPTS, DTT_COMPOSITED, DTT_TEXTCOLOR,
};

const ALPHA: u32 = 0xff00_0000;
const RGB: u32 = 0x00ff_ffff;

#[derive(Clone, Copy)]
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    pixels: *mut u32,
    width: i32,
    height: i32,
}

thread_local! {
    // GetObjectW reports positive DIB heights even for top-down storage. Only
    // buffers created here have the orientation required by DTT_COMPOSITED.
    static SURFACES: RefCell<Vec<Surface>> = const { RefCell::new(Vec::new()) };
}

pub struct GlassBuffer {
    target: HDC,
    bounds: RECT,
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u32,
    pixel_count: usize,
    background: Option<Vec<u32>>,
    pixels_ready: bool,
}

impl GlassBuffer {
    /// Allocate a top-down BGRA DIB. Drawing coordinates remain those of the
    /// target DC, including nonzero/negative `bounds` origins. The destination
    /// DC's clipping is honored when presenting. Paint the full buffer bounds.
    /// On allocation failure, the caller should use its ordinary opaque paint.
    pub fn new(target: HDC, bounds: RECT) -> Option<Self> {
        let width = bounds.right.checked_sub(bounds.left)?;
        let height = bounds.bottom.checked_sub(bounds.top)?;
        if width <= 0 || height <= 0 || target.0.is_null() {
            return None;
        }
        let viewport_x = bounds.left.checked_neg()?;
        let viewport_y = bounds.top.checked_neg()?;
        let pixel_count = (width as usize).checked_mul(height as usize)?;
        if pixel_count > isize::MAX as usize / std::mem::size_of::<u32>() {
            return None;
        }
        unsafe {
            let dc = CreateCompatibleDC(Some(target));
            if dc.0.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = std::ptr::null_mut::<c_void>();
            let bitmap =
                match CreateDIBSection(Some(target), &info, DIB_RGB_COLORS, &mut pixels, None, 0) {
                    Ok(bitmap) if !pixels.is_null() => bitmap,
                    Ok(bitmap) => {
                        let _ = DeleteObject(bitmap.into());
                        let _ = DeleteDC(dc);
                        return None;
                    }
                    Err(_) => {
                        let _ = DeleteDC(dc);
                        return None;
                    }
                };
            let old_bitmap = SelectObject(dc, bitmap.into());
            if old_bitmap.0.is_null() || old_bitmap.0 as isize == -1 {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
                return None;
            }
            let buffer = Self {
                target,
                bounds,
                dc,
                bitmap,
                old_bitmap,
                pixels: pixels.cast::<u32>(),
                pixel_count,
                background: None,
                pixels_ready: false,
            };
            if !SetViewportOrgEx(dc, viewport_x, viewport_y, None).as_bool() {
                return None;
            }
            // Copy the relevant drawing defaults; borrowed GDI objects remain
            // owned by the caller and are never destroyed by this buffer.
            for object_type in [OBJ_FONT, OBJ_BRUSH, OBJ_PEN] {
                let object = GetCurrentObject(target, object_type);
                if !object.0.is_null() {
                    SelectObject(dc, object);
                }
            }
            SetBkColor(dc, GetBkColor(target));
            SetBkMode(dc, BACKGROUND_MODE(GetBkMode(target) as u32));
            SetTextColor(dc, GetTextColor(target));
            SetTextAlign(dc, GetTextAlign(target));
            // Unpainted pixels have a defined, opaque fallback.
            std::slice::from_raw_parts_mut(buffer.pixels, pixel_count).fill(ALPHA);
            SURFACES.with_borrow_mut(|surfaces| {
                surfaces.push(Surface {
                    dc,
                    bitmap,
                    pixels: buffer.pixels,
                    width,
                    height,
                });
            });
            Some(buffer)
        }
    }

    pub fn hdc(&self) -> HDC {
        self.dc
    }

    /// Convert a completed straight-RGB GDI background to premultiplied BGRA.
    /// Call once per freshly painted RGB background, before foreground text.
    /// A nested backdrop should use alpha > 0 so an outer buffer can distinguish
    /// it from ordinary GDI output, which commonly resets alpha to zero.
    pub fn seal_background(&mut self, alpha: u8) {
        unsafe {
            let _ = GdiFlush();
            for pixel in std::slice::from_raw_parts_mut(self.pixels, self.pixel_count) {
                *pixel = premultiply(*pixel, alpha);
            }
        }
        self.preserve_background();
    }

    /// Snapshot an already premultiplied background without altering its RGB or
    /// alpha. Call immediately before foreground GDI drawing. This also catches
    /// drawing operations that change RGB while leaving the old alpha intact.
    /// Do not call after ordinary GDI text/fills whose alpha still needs repair.
    pub fn preserve_background(&mut self) {
        self.pixels_ready = false;
        unsafe {
            let _ = GdiFlush();
            self.background =
                Some(std::slice::from_raw_parts(self.pixels, self.pixel_count).to_vec());
        }
    }

    /// Present and release the buffer. Without an explicit snapshot, nonzero
    /// alpha from nested buffers is preserved and zero-alpha GDI is made opaque.
    /// Dropping without `finish` only releases resources; it never presents.
    /// Apply paint-only accents after all native GDI drawing. The callback
    /// receives valid premultiplied pixels and must preserve that format.
    pub fn map_native_pixels(&mut self, change: impl FnOnce(&mut [u32], usize)) {
        unsafe {
            let _ = GdiFlush();
            let pixels = std::slice::from_raw_parts_mut(self.pixels, self.pixel_count);
            repair_alpha(pixels, self.background.as_deref());
            change(pixels, (self.bounds.right - self.bounds.left) as usize);
            self.pixels_ready = true;
        }
    }

    pub fn finish(self) -> bool {
        unsafe {
            let _ = GdiFlush();
            let pixels = std::slice::from_raw_parts_mut(self.pixels, self.pixel_count);
            if !self.pixels_ready {
                repair_alpha(pixels, self.background.as_deref());
            }
            BitBlt(
                self.target,
                self.bounds.left,
                self.bounds.top,
                self.bounds.right - self.bounds.left,
                self.bounds.bottom - self.bounds.top,
                Some(self.dc),
                self.bounds.left,
                self.bounds.top,
                SRCCOPY,
            )
            .is_ok()
        }
    }
}

impl Drop for GlassBuffer {
    fn drop(&mut self) {
        SURFACES.with_borrow_mut(|surfaces| surfaces.retain(|surface| surface.dc != self.dc));
        unsafe {
            // GDI can batch commands that still reference the selected bitmap.
            let _ = GdiFlush();
            SelectObject(self.dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Draw antialiased text directly into a premultiplied glass surface. Ordinary
/// GDI text discards alpha, leaving dark opaque fringes on a light backdrop.
/// Screen/printer DCs and unavailable themes retain the caller's GDI fallback.
pub fn draw_text(
    dc: HDC,
    text: &[u16],
    mut rect: RECT,
    flags: DRAW_TEXT_FORMAT,
    color: COLORREF,
) -> bool {
    let Some(surface) =
        SURFACES.with_borrow(|surfaces| surfaces.iter().find(|surface| surface.dc == dc).copied())
    else {
        return false;
    };
    unsafe {
        if GetMapMode(dc) != MM_TEXT || GetCurrentObject(dc, OBJ_BITMAP) != surface.bitmap.into() {
            return false;
        }
        let mut origin = POINT::default();
        if !GetViewportOrgEx(dc, &mut origin).as_bool() {
            return false;
        }
        let theme = OpenThemeData(None, w!("CompositedWindow::Window"));
        if theme.0 == 0 {
            return false;
        }
        // GDI pill/tab fills have straight RGB and zero alpha until finish().
        // Restore those pixels BEFORE themed text blends with them. Existing
        // nonzero glass alpha must be preserved, including previous glyphs.
        let _ = GdiFlush();
        let width = surface.width;
        let height = surface.height;
        let left = rect.left.saturating_add(origin.x).clamp(0, width);
        let right = rect.right.saturating_add(origin.x).clamp(0, width);
        let top = rect.top.saturating_add(origin.y).clamp(0, height);
        let bottom = rect.bottom.saturating_add(origin.y).clamp(0, height);
        for y in top..bottom {
            let row = surface.pixels.add(y as usize * width as usize);
            for x in left..right {
                let pixel = &mut *row.add(x as usize);
                if *pixel & ALPHA == 0 {
                    *pixel |= ALPHA;
                }
            }
        }
        let options = DTTOPTS {
            dwSize: std::mem::size_of::<DTTOPTS>() as u32,
            dwFlags: DTT_COMPOSITED | DTT_TEXTCOLOR,
            crText: color,
            ..Default::default()
        };
        let result = DrawThemeTextEx(theme, dc, 0, 0, text, flags, &mut rect, Some(&options));
        let _ = CloseThemeData(theme);
        result.is_ok()
    }
}

fn premultiply(pixel: u32, alpha: u8) -> u32 {
    let alpha = u32::from(alpha);
    let channel = |shift: u32| (((pixel >> shift) & 0xff_u32) * alpha + 127) / 255;
    (alpha << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn repair_alpha(pixels: &mut [u32], background: Option<&[u32]>) {
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let previous = background
            .and_then(|background| background.get(index))
            .copied();
        if previous == Some(*pixel) {
            continue;
        }
        let rgb_changed_without_alpha = previous.is_some_and(|previous| {
            previous & RGB != *pixel & RGB && previous & ALPHA == *pixel & ALPHA
        });
        if *pixel & ALPHA == 0 || rgb_changed_without_alpha {
            *pixel |= ALPHA;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{draw_text, premultiply, repair_alpha, GlassBuffer};
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, FillRect,
        GdiFlush, SelectObject, SetBkMode, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
        DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HDC, HFONT, OUT_DEFAULT_PRECIS,
        TRANSPARENT,
    };

    // Nonzero coordinates exercise the helper's viewport-to-DIB translation.
    const NATIVE_BOUNDS: RECT = RECT {
        left: 13,
        top: 9,
        right: 333,
        bottom: 73,
    };
    const NATIVE_WIDTH: usize = 320;
    const BACKGROUND: u32 = 0x0037_3a3e;
    const TEXT: u32 = 0xfff1_f3f6;

    struct NativeCanvas {
        target: HDC,
        font: HFONT,
        output: Option<GlassBuffer>,
    }

    impl NativeCanvas {
        fn new() -> Self {
            unsafe {
                let target = CreateCompatibleDC(None);
                assert!(!target.0.is_null());
                let font = CreateFontW(
                    -24,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                assert!(!font.0.is_null());
                let output = GlassBuffer::new(target, NATIVE_BOUNDS).expect("output DIB");
                SelectObject(output.hdc(), font.into());
                Self {
                    target,
                    font,
                    output: Some(output),
                }
            }
        }

        fn glass_frame(&self) -> GlassBuffer {
            let frame = GlassBuffer::new(self.output.as_ref().unwrap().hdc(), NATIVE_BOUNDS)
                .expect("foreground DIB");
            let mut background =
                GlassBuffer::new(frame.hdc(), NATIVE_BOUNDS).expect("background DIB");
            fill_native(background.hdc(), NATIVE_BOUNDS);
            background.seal_background(80);
            assert!(background.finish());
            frame
        }

        fn pixels(&self) -> Vec<u32> {
            let output = self.output.as_ref().unwrap();
            unsafe {
                let _ = GdiFlush();
                std::slice::from_raw_parts(output.pixels, output.pixel_count).to_vec()
            }
        }
    }

    impl Drop for NativeCanvas {
        fn drop(&mut self) {
            drop(self.output.take());
            unsafe {
                let _ = DeleteObject(self.font.into());
                let _ = DeleteDC(self.target);
            }
        }
    }

    fn fill_native(dc: HDC, rect: RECT) {
        unsafe {
            let brush = CreateSolidBrush(COLORREF(0x003e_3a37));
            assert!(!brush.0.is_null());
            let filled = FillRect(dc, &rect, brush);
            let _ = DeleteObject(brush.into());
            assert_ne!(filled, 0);
        }
    }

    fn draw_native_text(dc: HDC, rect: RECT) {
        let text: Vec<u16> = "BKS Term | Text".encode_utf16().collect();
        unsafe {
            SetBkMode(dc, TRANSPARENT);
        }
        assert!(
            draw_text(
                dc,
                &text,
                rect,
                DT_LEFT | DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER,
                COLORREF(0x00f6_f3f1),
            ),
            "themed drawing must accept the registered top-down GlassBuffer"
        );
    }

    fn assert_valid_premultiplied(pixels: &[u32]) {
        for (index, &pixel) in pixels.iter().enumerate() {
            for shift in [0, 8, 16] {
                assert!(
                    (pixel >> shift) & 255 <= pixel >> 24,
                    "invalid premultiplied pixel {index}: {pixel:08x}"
                );
            }
        }
    }

    #[test]
    fn native_pixel_accents_preserve_translucency_after_gdi_alpha_is_repaired() {
        let canvas = NativeCanvas::new();
        let mut frame = canvas.glass_frame();
        draw_native_text(frame.hdc(), NATIVE_BOUNDS);
        let accent = 0x500c1826;
        frame.map_native_pixels(|pixels, width| {
            assert_eq!(width, NATIVE_WIDTH);
            assert_valid_premultiplied(pixels);
            pixels[0] = accent;
        });
        assert!(frame.finish());
        let pixels = canvas.pixels();
        assert_eq!(pixels[0], accent);
        assert!(pixels.contains(&TEXT));
        assert_valid_premultiplied(&pixels);
    }

    #[test]
    fn native_themed_text_preserves_glass_background_and_antialiased_alpha() {
        let canvas = NativeCanvas::new();
        let frame = canvas.glass_frame();
        draw_native_text(
            frame.hdc(),
            RECT {
                left: NATIVE_BOUNDS.left + 12,
                top: NATIVE_BOUNDS.top + 8,
                right: NATIVE_BOUNDS.right - 12,
                bottom: NATIVE_BOUNDS.bottom - 8,
            },
        );
        assert!(frame.finish());
        let pixels = canvas.pixels();
        let background = premultiply(BACKGROUND, 80);
        assert_eq!(pixels[0], background);
        assert_eq!(*pixels.last().unwrap(), background);
        assert!(pixels.iter().all(|pixel| pixel >> 24 >= 80));
        assert!(pixels.contains(&TEXT), "glyph cores must remain opaque");
        assert!(
            pixels
                .iter()
                .any(|pixel| (81..255).contains(&(pixel >> 24))),
            "glyph edges must retain antialiased alpha after finish"
        );
        assert_valid_premultiplied(&pixels);
    }

    #[test]
    fn native_themed_text_repairs_opaque_gdi_panel_before_compositing() {
        let canvas = NativeCanvas::new();
        let frame = canvas.glass_frame();
        let panel = RECT {
            left: NATIVE_BOUNDS.left + 8,
            top: NATIVE_BOUNDS.top + 8,
            right: NATIVE_BOUNDS.right - 8,
            bottom: NATIVE_BOUNDS.bottom - 8,
        };
        fill_native(frame.hdc(), panel);
        unsafe {
            let _ = GdiFlush();
            assert_eq!(*frame.pixels.add(8 * NATIVE_WIDTH + 8) >> 24, 0);
        }
        draw_native_text(
            frame.hdc(),
            RECT {
                left: panel.left + 8,
                right: panel.right - 8,
                ..panel
            },
        );
        assert!(frame.finish());
        let pixels = canvas.pixels();
        assert_eq!(pixels[0], premultiply(BACKGROUND, 80));
        assert_eq!(pixels[8 * NATIVE_WIDTH + 8], BACKGROUND | 0xff00_0000);
        assert!(pixels.contains(&TEXT), "glyph cores must remain opaque");
        for y in 8..56 {
            for x in 8..312 {
                assert_eq!(
                    pixels[y * NATIVE_WIDTH + x] >> 24,
                    255,
                    "opaque panel became transparent at {x},{y}"
                );
            }
        }
        assert_valid_premultiplied(&pixels);
    }

    #[test]
    fn tint_is_premultiplied_and_text_remains_opaque() {
        let background = [premultiply(0x0020_4080, 128); 3];
        assert_eq!(background[0], 0x8010_2040);
        let mut pixels = [background[0], 0x00f1_f3ff, 0x805e_8cff];
        repair_alpha(&mut pixels, Some(&background));
        assert_eq!(pixels, [0x8010_2040, 0xfff1_f3ff, 0xff5e_8cff]);
    }

    #[test]
    fn nested_background_keeps_its_alpha_and_is_not_premultiplied_twice() {
        let mut pixels = [0x8010_2040, 0x00f1_f3ff, 0x0000_0000, 0xff12_3456];
        repair_alpha(&mut pixels, None);
        assert_eq!(pixels, [0x8010_2040, 0xfff1_f3ff, 0xff00_0000, 0xff12_3456]);
    }

    #[test]
    fn explicitly_snapshotted_transparency_stays_transparent() {
        let background = [premultiply(0x00ff_ffff, 0), 0x8020_3040];
        let mut pixels = background;
        repair_alpha(&mut pixels, Some(&background));
        assert_eq!(pixels, background);
    }
}
