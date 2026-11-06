//! Consistent profile marks, rasterized from small vectors at the requested DPI.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, DrawIconEx, DI_NORMAL, HICON, ICONINFO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum IconKind {
    Brand,
    PowerShell,
    GitBash,
    CommandPrompt,
    Wsl,
    Ubuntu,
    Ssh,
    Shell,
}

type IconCache = RefCell<HashMap<(IconKind, i32), Option<Icon>>>;

thread_local! {
    // Rasterize once per physical size; HICON ownership stays on the UI thread.
    static ICONS: IconCache = RefCell::new(HashMap::new());
}

struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

struct Icon(HICON);

impl Icon {
    fn new(kind: IconKind, size: i32) -> Option<Self> {
        let pixels = rasterize(kind, size as usize);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = Bitmap(unsafe {
            CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?
        });
        if bits.is_null() {
            return None;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        }
        // A 32-bit icon uses premultiplied alpha; keep its required AND mask clear.
        // Monochrome bitmap rows are word-aligned, including odd DPI sizes.
        let mask_bits = vec![0_u8; (size as usize).div_ceil(16) * 2 * size as usize];
        let mask =
            Bitmap(unsafe { CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast())) });
        if mask.0.is_invalid() {
            return None;
        }
        // CreateIconIndirect copies both bitmaps; their temporary owners can drop.
        unsafe {
            CreateIconIndirect(&ICONINFO {
                fIcon: true.into(),
                hbmMask: mask.0,
                hbmColor: color.0,
                ..Default::default()
            })
            .ok()
            .map(Self)
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// Draw a profile mark centered inside `rect`, with transparent antialiased edges.
///
/// Profile names select a decorative mark only; they do not imply session state.
/// Unknown names use a neutral terminal mark. Returns `false` on an empty rect or
/// resource/drawing failure, so the caller can retain its simple fallback.
pub fn draw(hdc: HDC, rect: RECT, name: &str) -> bool {
    draw_kind(hdc, rect, icon_kind(name))
}

/// Draw the existing terminal brand mark in the indigo chrome palette.
pub fn draw_brand(hdc: HDC, rect: RECT) -> bool {
    draw_kind(hdc, rect, IconKind::Brand)
}

/// Borrow a brand icon for the title bar/taskbar at a physical pixel size.
///
/// The returned handle stays owned by this UI thread until it exits. Callers
/// must not destroy it. Profile-cache eviction never removes borrowed brand icons.
pub fn brand_icon(size: i32) -> Option<HICON> {
    if size <= 0 {
        return None;
    }
    cached_icon(IconKind::Brand, size)
}

fn cached_icon(kind: IconKind, size: i32) -> Option<HICON> {
    // Chrome normally needs 16-48 px; bound accidental large raster allocations.
    let size = size.min(128);
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 64 && !cache.contains_key(&(kind, size)) {
            cache.retain(|(kind, _), _| *kind == IconKind::Brand);
        }
        cache
            .entry((kind, size))
            .or_insert_with(|| Icon::new(kind, size))
            .as_ref()
            .map(|icon| icon.0)
    })
}

fn draw_kind(hdc: HDC, rect: RECT, kind: IconKind) -> bool {
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width <= 0 || height <= 0 {
        return false;
    }
    let size = width.min(height);
    let Some(icon) = cached_icon(kind, size) else {
        return false;
    };
    unsafe {
        DrawIconEx(
            hdc,
            rect.left + (width - size) / 2,
            rect.top + (height - size) / 2,
            icon,
            size,
            size,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok()
    }
}

fn icon_kind(name: &str) -> IconKind {
    let name = name.trim().to_ascii_lowercase();
    if name == "ssh" || name.starts_with("ssh ") || name.starts_with("ssh:") {
        IconKind::Ssh
    } else if name.contains("powershell") || name.contains("pwsh") {
        IconKind::PowerShell
    } else if name.contains("git bash") || name == "git-bash" {
        IconKind::GitBash
    } else if name.starts_with("ubuntu") {
        IconKind::Ubuntu
    } else if name == "wsl" || name.starts_with("wsl ") || name.starts_with("wsl(") {
        IconKind::Wsl
    } else if name.starts_with("command prompt") || name == "cmd" || name == "cmd.exe" {
        IconKind::CommandPrompt
    } else {
        IconKind::Shell
    }
}

type Point = (f32, f32);
type Bounds = (f32, f32, f32, f32);

// Geometry uses a 24-unit view box. Coverage is resolved before creating HICON,
// so GDI never has to draw aliased diagonals or rescale a 16-pixel source image.
const SAMPLES: usize = 4;

struct Canvas {
    side: usize,
    scale: f32,
    pixels: Vec<u32>,
}

impl Canvas {
    fn new(size: usize) -> Self {
        let side = size * SAMPLES;
        Self {
            side,
            scale: side as f32 / 24.0,
            pixels: vec![0; side * side],
        }
    }

    fn paint(&mut self, bounds: Bounds, color: u32, inside: impl Fn(f32, f32) -> bool) {
        let start_x = (bounds.0 * self.scale).floor().max(0.0) as usize;
        let start_y = (bounds.1 * self.scale).floor().max(0.0) as usize;
        let end_x = ((bounds.2 * self.scale).ceil() as usize).min(self.side);
        let end_y = ((bounds.3 * self.scale).ceil() as usize).min(self.side);
        for y in start_y..end_y {
            for x in start_x..end_x {
                if inside((x as f32 + 0.5) / self.scale, (y as f32 + 0.5) / self.scale) {
                    self.pixels[y * self.side + x] = 0xff00_0000 | color;
                }
            }
        }
    }

    fn circle(&mut self, (cx, cy): Point, radius: f32, color: u32) {
        self.paint(
            (cx - radius, cy - radius, cx + radius, cy + radius),
            color,
            |x, y| (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius,
        );
    }

    fn stroke(&mut self, points: &[Point], width: f32, color: u32) {
        let radius = width * 0.5;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = dx * dx + dy * dy;
            self.paint(
                (
                    a.0.min(b.0) - radius,
                    a.1.min(b.1) - radius,
                    a.0.max(b.0) + radius,
                    a.1.max(b.1) + radius,
                ),
                color,
                |x, y| {
                    let t = (((x - a.0) * dx + (y - a.1) * dy) / length).clamp(0.0, 1.0);
                    (x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2) <= radius * radius
                },
            );
        }
    }

    fn polygon(&mut self, points: &[Point], color: u32) {
        self.paint((0.0, 0.0, 24.0, 24.0), color, |x, y| {
            let mut inside = false;
            let mut last = points[points.len() - 1];
            for &point in points {
                if (point.1 > y) != (last.1 > y)
                    && x < (last.0 - point.0) * (y - point.1) / (last.1 - point.1) + point.0
                {
                    inside = !inside;
                }
                last = point;
            }
            inside
        });
    }

    fn panel(&mut self, bounds: Bounds, radius: f32, color: u32) {
        self.paint(bounds, color, |x, y| rounded_contains(x, y, bounds, radius));
    }

    fn outline(&mut self, bounds: Bounds, radius: f32, width: f32, color: u32) {
        let inner = (
            bounds.0 + width,
            bounds.1 + width,
            bounds.2 - width,
            bounds.3 - width,
        );
        self.paint(bounds, color, |x, y| {
            rounded_contains(x, y, bounds, radius)
                && !rounded_contains(x, y, inner, (radius - width).max(0.0))
        });
    }

    fn finish(self) -> Vec<u32> {
        let size = self.side / SAMPLES;
        let mut pixels = vec![0; size * size];
        for y in 0..size {
            for x in 0..size {
                let mut channels = [0; 4];
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let sample = self.pixels[(y * SAMPLES + sy) * self.side + x * SAMPLES + sx];
                        for (channel, value) in channels.iter_mut().enumerate() {
                            *value += (sample >> (channel * 8)) & 0xff;
                        }
                    }
                }
                // Transparent samples are zero. Averaging the color and alpha
                // together yields premultiplied BGRA without dark edge halos.
                pixels[y * size + x] = channels
                    .iter()
                    .enumerate()
                    .map(|(channel, value)| (value / (SAMPLES * SAMPLES) as u32) << (channel * 8))
                    .sum();
            }
        }
        pixels
    }
}

fn rounded_contains(x: f32, y: f32, bounds: Bounds, radius: f32) -> bool {
    let dx = (bounds.0 + radius - x).max(x - bounds.2 + radius).max(0.0);
    let dy = (bounds.1 + radius - y).max(y - bounds.3 + radius).max(0.0);
    dx * dx + dy * dy <= radius * radius
        && x >= bounds.0
        && x <= bounds.2
        && y >= bounds.1
        && y <= bounds.3
}

fn rasterize(kind: IconKind, size: usize) -> Vec<u32> {
    let mut c = Canvas::new(size);
    match kind {
        IconKind::Brand => {
            c.panel((1.6, 1.6, 22.4, 22.4), 5.0, 0x5e8cff);
            c.panel((3.1, 3.1, 20.9, 20.9), 3.6, 0x233e83);
            c.panel((4.0, 4.8, 20.0, 20.0), 3.0, 0x182b62);
            c.stroke(&[(6.0, 3.2), (17.8, 3.2)], 0.7, 0x9abbff);
            c.stroke(&[(7.2, 7.0), (11.3, 12.0), (7.2, 17.0)], 2.2, 0xebf3ff);
            c.stroke(&[(13.3, 17.0), (17.5, 17.0)], 1.9, 0xbfd7ff);
        }
        IconKind::PowerShell => {
            c.polygon(
                &[(5.1, 3.3), (22.0, 3.3), (18.9, 20.7), (2.0, 20.7)],
                0xb8d8ff,
            );
            c.polygon(
                &[(6.2, 4.7), (20.3, 4.7), (17.8, 19.3), (3.7, 19.3)],
                0x436fae,
            );
            c.stroke(&[(8.4, 7.2), (12.0, 11.8), (6.8, 16.5)], 2.1, 0xf1f6ff);
            c.stroke(&[(12.6, 16.5), (16.4, 16.5)], 1.7, 0xf1f6ff);
        }
        IconKind::GitBash => {
            c.paint((1.0, 1.0, 23.0, 23.0), 0xf2674a, |x, y| {
                let u = (x + y - 24.0) * std::f32::consts::FRAC_1_SQRT_2;
                let v = (y - x) * std::f32::consts::FRAC_1_SQRT_2;
                rounded_contains(u, v, (-7.6, -7.6, 7.6, 7.6), 1.2)
            });
            let ink = 0x402b37;
            c.stroke(&[(7.7, 5.0), (17.0, 14.3)], 1.65, ink);
            c.stroke(&[(10.0, 7.3), (10.0, 17.0)], 1.65, ink);
            for point in [(10.0, 7.3), (17.0, 14.3), (10.0, 17.0)] {
                c.circle(point, 1.65, ink);
            }
        }
        IconKind::Ubuntu => {
            let orange = 0xf47440;
            let heads = [(3.5, 12.0), (16.4, 4.4), (16.4, 19.6)];
            c.paint((4.4, 4.4, 19.6, 19.6), orange, |x, y| {
                let (dx, dy) = (x - 12.0, y - 12.0);
                let r2 = dx * dx + dy * dy;
                (5.2 * 5.2..=7.6 * 7.6).contains(&r2)
                    && ![(1.0, 0.0), (-0.5, 0.866_025_4), (-0.5, -0.866_025_4)]
                        .iter()
                        .any(|&(vx, vy)| dx * vx + dy * vy > 0.0 && (dx * vy - dy * vx).abs() < 0.8)
                    && !heads
                        .iter()
                        .any(|&(cx, cy)| (x - cx).powi(2) + (y - cy).powi(2) < 2.65 * 2.65)
            });
            for point in heads {
                c.circle(point, 1.9, orange);
            }
        }
        IconKind::Ssh => {
            for top in [2.8, 9.5, 16.2] {
                c.outline((2.8, top, 21.2, top + 5.0), 1.55, 1.3, 0xc4e4f7);
                c.stroke(&[(5.7, top + 2.5), (13.5, top + 2.5)], 1.05, 0x92bcd5);
                c.circle((18.2, top + 2.5), 0.7, 0xd5efff);
            }
        }
        IconKind::CommandPrompt | IconKind::Wsl | IconKind::Shell => {
            let (edge, face) = match kind {
                IconKind::CommandPrompt => (0xb7c4d9, 0x273248),
                IconKind::Wsl => (0x8fc8eb, 0x244963),
                _ => (0xabc3e6, 0x32456a),
            };
            c.panel((1.9, 3.7, 22.1, 20.3), 2.4, edge);
            c.panel((3.2, 5.0, 20.8, 19.0), 1.15, face);
            if kind == IconKind::CommandPrompt {
                c.stroke(&[(3.2, 8.1), (20.8, 8.1)], 1.1, edge);
                c.stroke(&[(6.2, 11.2), (9.0, 13.7), (6.2, 16.2)], 1.5, 0xe3eaf6);
                c.stroke(&[(12.0, 16.1), (16.0, 16.1)], 1.4, 0xe3eaf6);
            } else if kind == IconKind::Wsl {
                for (x, y) in [(5.3, 7.1), (8.8, 7.1), (5.3, 10.6), (8.8, 10.6)] {
                    c.panel((x, y, x + 2.5, y + 2.5), 0.15, 0x9bd9f4);
                }
                c.stroke(&[(13.1, 11.5), (15.8, 14.0), (13.1, 16.5)], 1.5, 0xe3f5ff);
                c.stroke(&[(17.7, 16.5), (19.0, 16.5)], 1.4, 0xe3f5ff);
            } else {
                c.stroke(&[(6.2, 8.4), (10.0, 12.0), (6.2, 15.6)], 1.8, 0xe5efff);
                c.stroke(&[(12.7, 15.6), (17.4, 15.6)], 1.6, 0xe5efff);
            }
        }
    }
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [IconKind; 8] = [
        IconKind::Brand,
        IconKind::PowerShell,
        IconKind::GitBash,
        IconKind::CommandPrompt,
        IconKind::Wsl,
        IconKind::Ubuntu,
        IconKind::Ssh,
        IconKind::Shell,
    ];

    #[test]
    fn profile_names_select_marks_without_borrowing_a_remote_distro_icon() {
        for name in ["PowerShell", "Windows PowerShell", "PowerShell 7", "pwsh"] {
            assert_eq!(icon_kind(name), IconKind::PowerShell);
        }
        assert_eq!(icon_kind(" Git Bash "), IconKind::GitBash);
        assert_eq!(icon_kind("git-bash"), IconKind::GitBash);
        assert_eq!(icon_kind("Ubuntu (WSL)"), IconKind::Ubuntu);
        assert_eq!(icon_kind("WSL"), IconKind::Wsl);
        assert_eq!(icon_kind("Command Prompt"), IconKind::CommandPrompt);
        assert_eq!(icon_kind("CMD.EXE"), IconKind::CommandPrompt);
        for name in ["SSH", "SSH prod", "SSH: Ubuntu", "SSH PowerShell"] {
            assert_eq!(icon_kind(name), IconKind::Ssh);
        }
        assert_eq!(icon_kind("Custom local shell"), IconKind::Shell);
    }

    #[test]
    fn dpi_rasters_have_premultiplied_edges_and_transparent_margins() {
        for size in [16, 20, 24, 25, 30, 32, 40, 48] {
            for kind in KINDS {
                let pixels = rasterize(kind, size);
                assert_eq!(pixels.len(), size * size);
                assert!(
                    pixels.iter().any(|&pixel| pixel >> 24 >= 128),
                    "empty {kind:?} at {size}px"
                );
                assert!(pixels
                    .iter()
                    .any(|&pixel| (1..255).contains(&(pixel >> 24))));
                for (index, &pixel) in pixels.iter().enumerate() {
                    let alpha = pixel >> 24;
                    for shift in [0, 8, 16] {
                        assert!((pixel >> shift) & 0xff <= alpha);
                    }
                    if index / size == 0
                        || index / size == size - 1
                        || index % size == 0
                        || index % size == size - 1
                    {
                        assert_eq!(pixel, 0, "clipped {kind:?} at {size}px");
                    }
                }
            }
        }
    }

    #[test]
    fn native_icons_are_created_at_fractional_dpi_sizes() {
        for size in [20, 25, 30, 40] {
            for kind in KINDS {
                assert!(Icon::new(kind, size).is_some(), "{kind:?} at {size}px");
            }
        }
    }

    #[test]
    fn borrowed_brand_handle_survives_profile_cache_eviction() {
        let brand = brand_icon(24).expect("brand icon");
        for size in 16..90 {
            assert!(cached_icon(IconKind::Shell, size).is_some());
        }
        assert_eq!(brand_icon(24), Some(brand));
        assert!(brand_icon(0).is_none());
    }
}
