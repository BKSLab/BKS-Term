use crate::system_colors::{
    dark_colors, high_contrast_colors, is_high_contrast, SystemThemeColors,
};
use windows::core::w;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, GetSysColorBrush,
    RoundRect, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_WINDOW, DEFAULT_CHARSET, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FF_MODERN, FIXED_PITCH, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT,
    OUT_DEFAULT_PRECIS, PS_SOLID, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

pub const BG_WINDOW: COLORREF = rgb(0x08, 0x0D, 0x0E);
pub const BG_TERMINAL: COLORREF = rgb(0x03, 0x06, 0x07);
pub const BG_TOOLBAR: COLORREF = rgb(0x0C, 0x11, 0x12);
pub const BG_SIDEBAR: COLORREF = rgb(0x0C, 0x14, 0x14);
pub const BG_SURFACE: COLORREF = rgb(0x0B, 0x11, 0x12);
pub const BRAND_PRIMARY: COLORREF = rgb(0x24, 0xE6, 0x9C);

// Sampled from the approved PNG: black work surface, subtle cool chrome,
// and a localized green tint in the lower part of the sidebar.
pub const TITLE_GRADIENT: [[[u8; 3]; 3]; 3] = [
    [[14, 30, 24], [11, 22, 19], [11, 17, 23]],
    [[10, 20, 16], [7, 12, 13], [8, 13, 18]],
    [[4, 8, 7], [4, 7, 8], [6, 10, 13]],
];
pub const SIDEBAR_GRADIENT: [[[u8; 3]; 3]; 4] = [
    [[12, 21, 20], [12, 20, 19], [12, 19, 19]],
    [[14, 26, 24], [13, 23, 22], [13, 22, 22]],
    [[19, 40, 34], [19, 38, 32], [14, 25, 23]],
    [[16, 28, 25], [16, 27, 24], [13, 22, 22]],
];
pub const SELECTED_GRADIENT: [[u8; 3]; 4] =
    [[31, 59, 50], [27, 49, 42], [28, 52, 44], [24, 44, 38]];

const HOVER: COLORREF = rgb(0x1B, 0x2B, 0x26);
const SELECTED: COLORREF = rgb(0x1A, 0x39, 0x2D);
const BORDER: COLORREF = rgb(0x27, 0x3B, 0x34);
const BORDER_SUBTLE: COLORREF = rgb(0x1D, 0x2B, 0x26);
const BRAND_HOVER: COLORREF = rgb(0x49, 0xF3, 0xB0);
const BRAND_PRESSED: COLORREF = rgb(0x19, 0xBF, 0x7E);

#[derive(Clone, Copy, Default)]
pub struct Theme {
    pub window: HBRUSH,
    pub terminal: HBRUSH,
    pub toolbar: HBRUSH,
    pub sidebar: HBRUSH,
    pub surface: HBRUSH,
    pub hover: HBRUSH,
    pub selected: HBRUSH,
    pub border: HBRUSH,
    pub border_subtle: HBRUSH,
    pub brand: HBRUSH,
    pub ui_font: HFONT,
    pub title_font: HFONT,
    pub terminal_font: HFONT,
    pub colors: SystemThemeColors,
    pub high_contrast: bool,
}

impl Theme {
    pub fn new(dpi: u32) -> Self {
        let (ui_font, title_font, terminal_font) = create_fonts(dpi);
        let high_contrast = is_high_contrast();
        let colors = if high_contrast {
            high_contrast_colors()
        } else {
            dark_colors()
        };
        unsafe {
            if high_contrast {
                Self {
                    window: GetSysColorBrush(COLOR_WINDOW),
                    terminal: GetSysColorBrush(COLOR_WINDOW),
                    toolbar: GetSysColorBrush(COLOR_BTNFACE),
                    sidebar: GetSysColorBrush(COLOR_BTNFACE),
                    surface: GetSysColorBrush(COLOR_BTNFACE),
                    hover: GetSysColorBrush(COLOR_BTNFACE),
                    selected: GetSysColorBrush(COLOR_BTNFACE),
                    border: GetSysColorBrush(COLOR_GRAYTEXT),
                    border_subtle: GetSysColorBrush(COLOR_GRAYTEXT),
                    brand: GetSysColorBrush(COLOR_HIGHLIGHT),
                    ui_font,
                    title_font,
                    terminal_font,
                    colors,
                    high_contrast,
                }
            } else {
                Self {
                    window: CreateSolidBrush(BG_WINDOW),
                    terminal: CreateSolidBrush(BG_TERMINAL),
                    toolbar: CreateSolidBrush(BG_TOOLBAR),
                    sidebar: CreateSolidBrush(BG_SIDEBAR),
                    surface: CreateSolidBrush(BG_SURFACE),
                    hover: CreateSolidBrush(HOVER),
                    selected: CreateSolidBrush(SELECTED),
                    border: CreateSolidBrush(BORDER),
                    border_subtle: CreateSolidBrush(BORDER_SUBTLE),
                    brand: CreateSolidBrush(BRAND_PRIMARY),
                    ui_font,
                    title_font,
                    terminal_font,
                    colors,
                    high_contrast,
                }
            }
        }
    }

    pub fn update_fonts(&mut self, dpi: u32) {
        let (ui_font, title_font, terminal_font) = create_fonts(dpi);
        unsafe {
            if !self.ui_font.0.is_null() {
                let _ = DeleteObject(self.ui_font.into());
            }
            if !self.terminal_font.0.is_null() {
                let _ = DeleteObject(self.terminal_font.into());
            }
            if !self.title_font.0.is_null() {
                let _ = DeleteObject(self.title_font.into());
            }
        }
        self.ui_font = ui_font;
        self.title_font = title_font;
        self.terminal_font = terminal_font;
    }
}

fn create_fonts(dpi: u32) -> (HFONT, HFONT, HFONT) {
    unsafe {
        let ui_font = CreateFontW(
            crate::dpi::font_height(14, dpi),
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
            w!("Segoe UI Variable"),
        );
        let terminal_font = CreateFontW(
            crate::dpi::font_height(15, dpi),
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
        let title_font = CreateFontW(
            crate::dpi::font_height(14, dpi),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI Variable"),
        );
        (ui_font, title_font, terminal_font)
    }
}

pub fn draw_execute_button(item: &DRAWITEMSTRUCT, theme: &Theme) {
    draw_button(item, theme, "Выполнить", true);
}

pub fn draw_mode_button(item: &DRAWITEMSTRUCT, theme: &Theme, text: &str) {
    draw_button(item, theme, text, false);
}

fn draw_button(item: &DRAWITEMSTRUCT, theme: &Theme, text: &str, primary: bool) {
    let hdc = item.hDC;
    let state = item.itemState.0;

    let disabled = state & ODS_DISABLED.0 != 0;
    let selected = state & ODS_SELECTED.0 != 0;
    let hot = state & ODS_HOTLIGHT.0 != 0;
    let focus = state & ODS_FOCUS.0 != 0;

    let (background, text_color) = if disabled {
        (theme.surface, theme.colors.text_muted)
    } else if selected {
        (
            theme.selected,
            if primary && !theme.high_contrast {
                BRAND_PRESSED
            } else {
                theme.colors.text_primary
            },
        )
    } else if hot {
        (
            theme.hover,
            if primary && !theme.high_contrast {
                BRAND_HOVER
            } else {
                theme.colors.text_primary
            },
        )
    } else {
        (
            theme.surface,
            if primary {
                theme.colors.brand
            } else {
                theme.colors.text_secondary
            },
        )
    };

    let border_color = if focus || primary {
        theme.colors.brand
    } else {
        BORDER_SUBTLE
    };

    unsafe {
        let dpi = match GetDpiForWindow(item.hwndItem) {
            0 => 96,
            value => value,
        };
        let radius = crate::dpi::scale(8, dpi);

        FillRect(hdc, &item.rcItem, theme.window);
        let old_brush = SelectObject(hdc, background.into());
        let pen = CreatePen(PS_SOLID, 1, border_color);
        let old_pen = SelectObject(hdc, pen.into());
        let old_font = SelectObject(hdc, theme.ui_font.into());

        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, text_color);

        let _ = RoundRect(
            hdc,
            item.rcItem.left,
            item.rcItem.top,
            item.rcItem.right,
            item.rcItem.bottom,
            radius * 2,
            radius * 2,
        );

        let mut text: Vec<u16> = text.encode_utf16().collect();
        let mut rect = item.rcItem;
        let _ = DrawTextW(
            hdc,
            text.as_mut_slice(),
            &mut rect as *mut _,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );

        let _ = SelectObject(hdc, old_font);
        let _ = SelectObject(hdc, old_pen);
        let _ = SelectObject(hdc, old_brush);
        let _ = DeleteObject(pen.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_uses_bgr_layout() {
        assert_eq!(rgb(0x12, 0x34, 0x56).0, 0x00563412);
    }

    #[test]
    fn constants_match_hex_values() {
        assert_eq!(BG_WINDOW.0, 0x000E0D08);
        assert_eq!(BG_TERMINAL.0, 0x00070603);
        assert_eq!(BG_TOOLBAR.0, 0x0012110C);
        assert_eq!(BG_SURFACE.0, 0x0012110B);
        assert_eq!(BRAND_PRIMARY.0, 0x009CE624);
    }
}
