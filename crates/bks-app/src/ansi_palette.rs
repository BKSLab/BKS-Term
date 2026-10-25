use windows::Win32::Foundation::COLORREF;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiPalette {
    pub colors: [COLORREF; 16],
    pub default_fg: COLORREF,
    pub default_bg: COLORREF,
    pub cursor: COLORREF,
    pub selection_bg: COLORREF,
}

pub const DARK_PALETTE: AnsiPalette = AnsiPalette {
    colors: [
        rgb(0x10, 0x17, 0x15),
        rgb(0xD8, 0x5A, 0x65),
        rgb(0x36, 0xC9, 0x92),
        rgb(0xDD, 0xBB, 0x67),
        rgb(0x6B, 0x9F, 0xDB),
        rgb(0xC1, 0x8B, 0xDD),
        rgb(0x62, 0xBD, 0xD0),
        rgb(0xD6, 0xE1, 0xDC),
        rgb(0x71, 0x82, 0x7A),
        rgb(0xFF, 0x64, 0x75),
        rgb(0x51, 0xE7, 0xAB),
        rgb(0xF5, 0xCA, 0x65),
        rgb(0x82, 0xB8, 0xF5),
        rgb(0xD7, 0xAC, 0xFF),
        rgb(0x80, 0xDA, 0xED),
        rgb(0xF7, 0xFA, 0xF8),
    ],
    default_fg: rgb(0xE6, 0xEE, 0xEB),
    default_bg: rgb(0x07, 0x0B, 0x0A),
    cursor: rgb(0x24, 0xE6, 0x9C),
    selection_bg: rgb(0x25, 0x57, 0x44),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_len_is_16() {
        assert_eq!(DARK_PALETTE.colors.len(), 16);
    }

    #[test]
    fn characteristic_bgr_values() {
        assert_eq!(DARK_PALETTE.colors[0], COLORREF(0x0015_1710));
        assert_eq!(DARK_PALETTE.colors[1], COLORREF(0x0065_5AD8));
        assert_eq!(DARK_PALETTE.colors[7], COLORREF(0x00DC_E1D6));
        assert_eq!(DARK_PALETTE.colors[15], COLORREF(0x00F8_FAF7));
    }

    #[test]
    fn defaults_and_accents() {
        assert_eq!(DARK_PALETTE.default_fg, COLORREF(0x00EB_EEE6));
        assert_eq!(DARK_PALETTE.default_bg, COLORREF(0x000A_0B07));
        assert_eq!(DARK_PALETTE.cursor, COLORREF(0x009C_E624));
        assert_eq!(DARK_PALETTE.selection_bg, COLORREF(0x0044_5725));
    }
}
