//! Pixel regressions for the actual GDI path used by raw TUI input.

use super::*;
use bks_terminal::{TerminalModel, TextStyle};
use windows::core::w;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, CreateFontW, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, FF_MODERN,
    FIXED_PITCH, FW_NORMAL, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_EX_STYLE,
    WS_POPUP,
};

struct Canvas {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *const u32,
    width: i32,
    height: i32,
    metrics: (i32, i32),
    theme: Theme,
}

impl Canvas {
    fn new(frame: &TerminalSnapshot) -> Self {
        unsafe {
            let font = CreateFontW(
                -22,
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
                (FIXED_PITCH.0 | FF_MODERN.0) as u32,
                w!("Cascadia Mono"),
            );
            assert!(!font.0.is_null());
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("BKS private VT paint fixture"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let metrics = cell_metrics(hwnd, font);
            let width = frame.cols as i32 * metrics.0;
            let height = frame.rows as i32 * metrics.1;
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .unwrap();
            let dc = CreateCompatibleDC(None);
            assert!(!dc.0.is_null());
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
            let mut pixels = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0).unwrap();
            let previous = SelectObject(dc, bitmap.into());
            Self {
                hwnd,
                dc,
                bitmap,
                previous,
                pixels: pixels.cast(),
                width,
                height,
                metrics,
                theme: Theme {
                    terminal_font: font,
                    ..Default::default()
                },
            }
        }
    }

    fn paint(&self, frame: &TerminalSnapshot) {
        paint(self.hwnd, self.dc, frame, &self.theme, None, false);
        unsafe {
            let _ = GdiFlush();
        }
    }

    fn assert_cell_background(
        &self,
        index: usize,
        columns: usize,
        expected: COLORREF,
        phase: &str,
    ) {
        let pixels =
            unsafe { std::slice::from_raw_parts(self.pixels, (self.width * self.height) as usize) };
        let color = expected.0;
        let rgb = ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255);
        let x = (index % columns) as i32 * self.metrics.0;
        let y = (index / columns) as i32 * self.metrics.1;
        for row in y..y + self.metrics.1 {
            for column in x..x + self.metrics.0 {
                assert_eq!(pixels[(row * self.width + column) as usize] & 0xffffff, rgb,
                    "{phase}: blank cell {index}, pixel ({column},{row}) must use the reader's palette without old glyphs");
            }
        }
    }

    fn save(&self, name: &str) {
        let pixels =
            unsafe { std::slice::from_raw_parts(self.pixels, (self.width * self.height) as usize) };
        let mut bytes = Vec::with_capacity(54 + pixels.len() * 4);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(54u32 + pixels.len() as u32 * 4).to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&(-self.height).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        for &pixel in pixels {
            bytes.extend_from_slice(&(pixel | 0xff000000).to_le_bytes());
        }
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-followup/menu");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.bmp")), bytes).unwrap();
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
            let _ = DeleteObject(self.theme.terminal_font.into());
        }
    }
}

#[test]
fn captured_model_menu_pixels_follow_the_shared_palette_after_navigation_close_and_resize() {
    let bytes = include_bytes!("../tests/fixtures/tui-opencode-model-menu.vt");
    let mut model = TerminalModel::new(110, 36);
    let mut offset = 0;
    let mut target: Option<Canvas> = None;
    for (name, end, resized) in [
        ("models", 19_449, false),
        ("models-down", 19_702, false),
        ("closed", 25_319, false),
        ("narrow-models", 35_897, true),
        ("narrow-closed", bytes.len(), false),
    ] {
        if resized {
            model.resize(80, 24);
        }
        model.push_bytes(&bytes[offset..end]);
        offset = end;
        let frame = model.live_snapshot();
        let original = frame.clone();
        let surface = crate::text_style::TuiSurface::detect_grid(&frame).unwrap();
        if target.as_ref().is_none_or(|canvas| {
            canvas.width != frame.cols as i32 * canvas.metrics.0
                || canvas.height != frame.rows as i32 * canvas.metrics.1
        }) {
            target = Some(Canvas::new(&frame));
        }
        // Reuse the painted target across selection movement and closing.
        // Checking fresh surfaces alone would miss stale menu/glyph pixels.
        let canvas = target.as_ref().unwrap();
        canvas.paint(&frame);
        canvas.save(name);
        let mut checked = 0;
        for (index, cell) in frame.cells.iter().enumerate() {
            if cell.text != " "
                || cell
                    .flags
                    .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
                || frame
                    .cursor
                    .is_some_and(|cursor| cursor.row * frame.cols + cursor.column == index)
            {
                continue;
            }
            let style = TextStyle::new(cell.fg, cell.bg, cell.flags);
            let expected = surface.resolve(style, &canvas.theme).background;
            canvas.assert_cell_background(index, frame.cols, expected, name);
            checked += 1;
        }
        assert!(checked > frame.cells.len() / 2);
        assert_eq!(
            model.live_snapshot(),
            original,
            "painting must not change VT"
        );
    }
}

#[test]
fn model_submenus_keep_the_material_over_chat_panels_without_black_repaints() {
    let panelled = crate::text_style::tests::panelled_menu_model().live_snapshot();
    let mut model = TerminalModel::new(100, 29);
    model.push_bytes(&include_bytes!("../tests/fixtures/tui-opencode-menu-stack.vt")[..17_493]);
    let variant = model.live_snapshot();
    let canvas = Canvas::new(&panelled);
    for (name, frame) in [
        ("panelled-models", panelled),
        ("variant", variant),
        (
            "error-model-menu",
            crate::text_style::tests::error_menu_frame(),
        ),
    ] {
        let surface = crate::text_style::TuiSurface::detect_grid(&frame)
            .expect("floating submenu uses material");
        canvas.paint(&frame);
        canvas.save(name);
        for (index, cell) in frame.cells.iter().enumerate() {
            if cell.text != " "
                || cell
                    .flags
                    .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
                || frame
                    .cursor
                    .is_some_and(|cursor| cursor.row * frame.cols + cursor.column == index)
            {
                continue;
            }
            let source = TextStyle::new(cell.fg, cell.bg, cell.flags);
            canvas.assert_cell_background(
                index,
                frame.cols,
                surface.resolve(source, &canvas.theme).background,
                name,
            );
        }
    }
}
