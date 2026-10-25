pub const BASE_DPI: u32 = 96;

fn scaled(value: i64, dpi: u32) -> i64 {
    let dpi = i64::from(if dpi == 0 { BASE_DPI } else { dpi });
    let den = i64::from(BASE_DPI);
    let num = value * dpi;
    if num >= 0 {
        (num + den / 2) / den
    } else {
        (num - den / 2) / den
    }
}

pub fn scale(value: i32, dpi: u32) -> i32 {
    scaled(i64::from(value), dpi).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub fn font_height(dip: i32, dpi: u32) -> i32 {
    if dip == 0 {
        0
    } else {
        -(scaled(i64::from(dip).abs(), dpi).min(i64::from(i32::MAX)) as i32)
    }
}

pub fn min_track_size(dpi: u32) -> (i32, i32) {
    (scale(480, dpi), scale(320, dpi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_at_base_dpi() {
        assert_eq!(scale(100, 96), 100);
        assert_eq!(scale(0, 96), 0);
        assert_eq!(scale(-100, 96), -100);
    }

    #[test]
    fn scale_at_higher_dpi() {
        assert_eq!(scale(100, 144), 150);
        assert_eq!(scale(100, 192), 200);
        assert_eq!(scale(-100, 144), -150);
        assert_eq!(scale(-100, 192), -200);
    }

    #[test]
    fn scale_rounds_to_nearest() {
        assert_eq!(scale(1, 144), 2);
        assert_eq!(scale(-1, 144), -2);
        assert_eq!(scale(1, 96), 1);
        assert_eq!(scale(3, 96), 3);
    }

    #[test]
    fn scale_zero_dpi_falls_back() {
        assert_eq!(scale(100, 0), 100);
        assert_eq!(scale(-100, 0), -100);
    }

    #[test]
    fn scale_saturates() {
        assert_eq!(scale(i32::MAX, u32::MAX), i32::MAX);
        assert_eq!(scale(i32::MIN, u32::MAX), i32::MIN);
    }

    #[test]
    fn font_height_basic() {
        assert_eq!(font_height(16, 96), -16);
        assert_eq!(font_height(16, 144), -24);
        assert_eq!(font_height(16, 192), -32);
        assert_eq!(font_height(0, 144), 0);
    }

    #[test]
    fn font_height_negative_dip() {
        assert_eq!(font_height(-16, 144), -24);
        assert_eq!(font_height(-16, 96), -16);
    }

    #[test]
    fn font_height_min_dip() {
        assert_eq!(font_height(i32::MIN, 96), -i32::MAX);
    }

    #[test]
    fn min_track_size_scales() {
        assert_eq!(min_track_size(96), (480, 320));
        assert_eq!(min_track_size(144), (720, 480));
        assert_eq!(min_track_size(192), (960, 640));
        assert_eq!(min_track_size(0), (480, 320));
    }
}
