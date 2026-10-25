use std::ffi::c_void;

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, SYS_COLOR_INDEX,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemThemeColors {
    pub window: COLORREF,
    pub terminal: COLORREF,
    pub toolbar: COLORREF,
    pub sidebar: COLORREF,
    pub surface: COLORREF,
    pub selected: COLORREF,
    pub text_primary: COLORREF,
    pub text_secondary: COLORREF,
    pub text_muted: COLORREF,
    pub brand: COLORREF,
    pub on_primary: COLORREF,
    pub status_success: COLORREF,
    pub status_error: COLORREF,
}

impl Default for SystemThemeColors {
    fn default() -> Self {
        dark_colors()
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(((b as u32) << 16) | ((g as u32) << 8) | (r as u32))
}

pub fn is_high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut HIGHCONTRASTW as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
    };
    ok && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

pub fn dark_colors() -> SystemThemeColors {
    SystemThemeColors {
        window: rgb(0x08, 0x0D, 0x0E),
        terminal: rgb(0x03, 0x06, 0x07),
        toolbar: rgb(0x0C, 0x11, 0x12),
        sidebar: rgb(0x0C, 0x14, 0x14),
        surface: rgb(0x0B, 0x11, 0x12),
        selected: rgb(0x1A, 0x39, 0x2D),
        text_primary: rgb(0xE6, 0xEE, 0xEB),
        text_secondary: rgb(0xAC, 0xBB, 0xB4),
        text_muted: rgb(0x8D, 0x9E, 0x96),
        brand: rgb(0x24, 0xE6, 0x9C),
        on_primary: rgb(0x04, 0x14, 0x0D),
        status_success: rgb(0x44, 0xD7, 0x9E),
        status_error: rgb(0xFF, 0x64, 0x75),
    }
}

fn sys_color(index: SYS_COLOR_INDEX) -> COLORREF {
    unsafe { COLORREF(GetSysColor(index)) }
}

pub fn high_contrast_colors() -> SystemThemeColors {
    SystemThemeColors {
        window: sys_color(COLOR_WINDOW),
        terminal: sys_color(COLOR_WINDOW),
        toolbar: sys_color(COLOR_BTNFACE),
        sidebar: sys_color(COLOR_BTNFACE),
        surface: sys_color(COLOR_BTNFACE),
        selected: sys_color(COLOR_BTNFACE),
        text_primary: sys_color(COLOR_WINDOWTEXT),
        text_secondary: sys_color(COLOR_GRAYTEXT),
        text_muted: sys_color(COLOR_GRAYTEXT),
        brand: sys_color(COLOR_HIGHLIGHT),
        on_primary: sys_color(COLOR_HIGHLIGHTTEXT),
        status_success: sys_color(COLOR_WINDOWTEXT),
        status_error: sys_color(COLOR_WINDOWTEXT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_colors_bgr() {
        let colors = dark_colors();
        assert_eq!(colors.window.0, 0x000E0D08);
        assert_eq!(colors.terminal.0, 0x00070603);
        assert_eq!(colors.toolbar.0, 0x0012110C);
        assert_eq!(colors.sidebar.0, 0x0014140C);
        assert_eq!(colors.surface.0, 0x0012110B);
        assert_eq!(colors.selected.0, 0x002D391A);
        assert_eq!(colors.text_primary.0, 0x00EBEEE6);
        assert_eq!(colors.text_secondary.0, 0x00B4BBAC);
        assert_eq!(colors.text_muted.0, 0x00969E8D);
        assert_eq!(colors.brand.0, 0x009CE624);
        assert_eq!(colors.on_primary.0, 0x000D1404);
        assert_eq!(colors.status_success.0, 0x009ED744);
        assert_eq!(colors.status_error.0, 0x007564FF);
    }
}
