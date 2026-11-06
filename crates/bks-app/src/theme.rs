use crate::system_colors::{
    dark_colors, high_contrast_colors, is_high_contrast, SystemThemeColors,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject,
    DrawTextW, FillRect, GetSysColorBrush, GetTextFaceW, RoundRect, SelectObject, SetBkMode,
    SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_GRAYTEXT,
    COLOR_HIGHLIGHT, COLOR_WINDOW, DEFAULT_CHARSET, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    FF_MODERN, FIXED_PITCH, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT, OUT_DEFAULT_PRECIS, PS_SOLID,
    TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

// Neutral frosted glass: environmental color comes from DWM, not purple fills.
// Blue is reserved for small controls and focus; ANSI remains independent.
pub const BG_WINDOW: COLORREF = rgb(0x17, 0x19, 0x1C);
pub const BG_TERMINAL: COLORREF = rgb(0x15, 0x18, 0x1B);
/// A light graphite veil over Acrylic; glyphs retain their own opacity.
pub const TERMINAL_GLASS_ALPHA: u8 = 144;
pub const BG_TOOLBAR: COLORREF = rgb(0x2C, 0x2F, 0x33);
pub const BG_SIDEBAR: COLORREF = rgb(0x25, 0x28, 0x2C);
pub const BG_SURFACE: COLORREF = rgb(0x30, 0x34, 0x3A);
pub const BRAND_PRIMARY: COLORREF = rgb(0x5E, 0x8C, 0xFF);
pub const TEXT_PRIMARY: COLORREF = rgb(0xF1, 0xF3, 0xF6);
pub const TEXT_SECONDARY: COLORREF = rgb(0xC6, 0xCB, 0xD3);
pub const TEXT_MUTED: COLORREF = rgb(0xAD, 0xB4, 0xBE);
pub const FOCUS_RING: COLORREF = rgb(0x99, 0xC4, 0xFF);
pub const TAB_ACTIVE: COLORREF = rgb(0x3B, 0x50, 0x6F);
pub const TAB_INACTIVE: COLORREF = rgb(0x34, 0x39, 0x41);
pub const BORDER: COLORREF = rgb(0x63, 0x6D, 0x7A);
pub const BORDER_SUBTLE: COLORREF = rgb(0x45, 0x4B, 0x54);
pub const HOVER: COLORREF = rgb(0x41, 0x49, 0x54);
pub const SELECTED: COLORREF = TAB_ACTIVE;
pub const CARET_ROW_TINT: u32 = 8;
pub const TEXT_SELECTION_TINT: u32 = 22;
pub const INACTIVE_SELECTION_TINT: u32 = 14;
pub const TEXT_SELECTION_BG: COLORREF = rgb(0x25, 0x31, 0x4D);

// Neutral reflections, continuous across title and session header.
pub const TITLE_GRADIENT: [[[u8; 3]; 3]; 3] = [
    [[55, 58, 62], [51, 54, 58], [55, 56, 59]],
    [[44, 47, 51], [42, 45, 49], [46, 47, 50]],
    [[34, 37, 41], [35, 38, 42], [38, 39, 42]],
];
pub const SIDEBAR_GRADIENT: [[[u8; 3]; 3]; 4] = [
    [[38, 41, 45], [36, 39, 43], [34, 37, 41]],
    [[30, 33, 37], [31, 34, 38], [29, 32, 36]],
    [[31, 34, 38], [32, 35, 39], [29, 32, 36]],
    [[32, 35, 39], [33, 36, 40], [30, 33, 37]],
];
pub const STATUS_GRADIENT: [[[u8; 3]; 3]; 3] = [
    [[37, 40, 44], [39, 41, 45], [36, 40, 45]],
    [[31, 34, 38], [33, 36, 40], [30, 34, 39]],
    [[28, 31, 35], [30, 33, 37], [27, 31, 36]],
];
pub const SELECTED_GRADIENT: [[u8; 3]; 4] =
    [[59, 78, 108], [65, 80, 109], [42, 57, 80], [48, 60, 83]];

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
    pub status_font: HFONT,
    pub colors: SystemThemeColors,
    pub high_contrast: bool,
}

impl Theme {
    pub fn new(dpi: u32) -> Self {
        let (ui_font, title_font, terminal_font, status_font) = create_fonts(dpi);
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
                    status_font,
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
                    status_font,
                    colors,
                    high_contrast,
                }
            }
        }
    }

    pub fn update_fonts(&mut self, dpi: u32) {
        let (ui_font, title_font, terminal_font, status_font) = create_fonts(dpi);
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
            if !self.status_font.0.is_null() {
                let _ = DeleteObject(self.status_font.into());
            }
        }
        self.ui_font = ui_font;
        self.title_font = title_font;
        self.terminal_font = terminal_font;
        self.status_font = status_font;
    }
}

fn create_fonts(dpi: u32) -> (HFONT, HFONT, HFONT, HFONT) {
    (
        create_font(
            dpi,
            14,
            FW_NORMAL.0 as i32,
            false,
            "Segoe UI Variable Text",
            "Segoe UI",
        ),
        create_font(
            dpi,
            16,
            FW_SEMIBOLD.0 as i32,
            false,
            "Segoe UI Variable Display",
            "Segoe UI",
        ),
        create_font(
            dpi,
            17,
            FW_NORMAL.0 as i32,
            true,
            "Cascadia Mono",
            "Consolas",
        ),
        create_font(
            dpi,
            12,
            FW_NORMAL.0 as i32,
            false,
            "Segoe UI Variable Text",
            "Segoe UI",
        ),
    )
}

fn create_font(
    dpi: u32,
    size: i32,
    weight: i32,
    mono: bool,
    preferred: &str,
    fallback: &str,
) -> HFONT {
    let make = |name: &str| {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unsafe {
            CreateFontW(
                crate::dpi::font_height(size, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                if mono {
                    (FIXED_PITCH.0 | FF_MODERN.0) as u32
                } else {
                    0
                },
                PCWSTR(name.as_ptr()),
            )
        }
    };
    let font = make(preferred);
    // The generic "Segoe UI Variable" name silently maps to Arial in GDI.
    // Verify the realized face so older Windows versions use our explicit fallback.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return font;
        }
        let previous = SelectObject(dc, font.into());
        let mut name = [0_u16; 128];
        let count = GetTextFaceW(dc, Some(&mut name));
        SelectObject(dc, previous);
        let _ = DeleteDC(dc);
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        if count > 0 && String::from_utf16_lossy(&name[..end]).eq_ignore_ascii_case(preferred) {
            font
        } else {
            let _ = DeleteObject(font.into());
            make(fallback)
        }
    }
}

pub fn draw_execute_button(item: &DRAWITEMSTRUCT, theme: &Theme, text: &str) {
    draw_button(item, theme, text, true);
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
        (theme.toolbar, theme.colors.text_primary)
    } else if hot {
        (theme.hover, theme.colors.text_primary)
    } else {
        (
            if primary {
                theme.selected
            } else {
                theme.surface
            },
            if primary {
                theme.colors.text_primary
            } else {
                theme.colors.text_secondary
            },
        )
    };

    let border_color = if focus {
        if theme.high_contrast {
            theme.colors.brand
        } else {
            FOCUS_RING
        }
    } else if primary {
        if theme.high_contrast {
            theme.colors.brand
        } else {
            BORDER
        }
    } else {
        if theme.high_contrast {
            theme.colors.text_secondary
        } else {
            BORDER_SUBTLE
        }
    };

    unsafe {
        let dpi = match GetDpiForWindow(item.hwndItem) {
            0 => 96,
            value => value,
        };
        let radius = crate::dpi::scale(10, dpi);

        FillRect(
            hdc,
            &item.rcItem,
            if primary {
                theme.terminal
            } else {
                theme.window
            },
        );
        let old_brush = SelectObject(hdc, background.into());
        let pen = CreatePen(
            PS_SOLID,
            crate::dpi::scale(if focus { 2 } else { 1 }, dpi).max(1),
            border_color,
        );
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
        assert_eq!(BG_WINDOW.0, 0x001C1917);
        assert_eq!(BG_TERMINAL.0, 0x001B1815);
        assert_eq!(BG_TOOLBAR.0, 0x00332F2C);
        assert_eq!(BG_SURFACE.0, 0x003A3430);
        assert_eq!(BRAND_PRIMARY.0, 0x00FF8C5E);
    }
}
