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
    use crate::theme;
    SystemThemeColors {
        window: theme::BG_WINDOW,
        terminal: theme::BG_TERMINAL,
        toolbar: theme::BG_TOOLBAR,
        sidebar: theme::BG_SIDEBAR,
        surface: theme::BG_SURFACE,
        selected: theme::SELECTED,
        text_primary: theme::TEXT_PRIMARY,
        text_secondary: theme::TEXT_SECONDARY,
        text_muted: theme::TEXT_MUTED,
        brand: theme::BRAND_PRIMARY,
        on_primary: theme::TEXT_PRIMARY,
        status_success: rgb(0x49, 0xE4, 0x7E),
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
    fn ui_text_and_focus_keep_contrast_on_opaque_surfaces() {
        let colors = dark_colors();
        fn luminance(color: COLORREF) -> f64 {
            let channel = |shift: u32| {
                let v = ((color.0 >> shift) & 255_u32) as f64 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
        }
        for background in [
            colors.window,
            colors.toolbar,
            colors.sidebar,
            colors.surface,
        ] {
            for foreground in [
                colors.text_primary,
                colors.text_secondary,
                colors.text_muted,
            ] {
                assert!((luminance(foreground) + 0.05) / (luminance(background) + 0.05) >= 4.5);
            }
        }
        assert!(
            (luminance(crate::theme::FOCUS_RING) + 0.05) / (luminance(colors.selected) + 0.05)
                >= 3.0
        );
    }
}
