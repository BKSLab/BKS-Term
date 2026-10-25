use crate::dpi::scale;

pub const TITLE_HEIGHT: i32 = 36;
pub const CAPTION_BUTTON_WIDTH: i32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowLayout {
    pub title_bar: Rect,
    pub sidebar: Rect,
    pub session_header: Rect,
    pub output: Rect,
    pub input_label: Rect,
    pub input: Rect,
    pub execute_button: Rect,
    pub status_bar: Rect,
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect {
        x,
        y,
        width: width.max(0),
        height: height.max(0),
    }
}

pub fn calculate(width: i32, height: i32, dpi: u32, sidebar_expanded: bool) -> WindowLayout {
    let w = width.max(0);
    let h = height.max(0);
    let title_h = scale(TITLE_HEIGHT, dpi).min(h);
    let sidebar_w = scale(if sidebar_expanded { 144 } else { 56 }, dpi).min(w);
    let margin = scale(16, dpi).min((w - sidebar_w) / 2);
    let gap = scale(8, dpi);
    let header_h = scale(52, dpi).min(h - title_h);
    let status_h = scale(40, dpi).min((h - title_h - header_h).max(0));
    let status_y = h - status_h;
    let content_top = title_h + header_h;
    let content_x = sidebar_w + margin;
    let content_w = (w - sidebar_w - 2 * margin).max(0);
    let input_bottom = (status_y - gap).max(content_top);
    let input_h = scale(44, dpi).min((input_bottom - content_top).max(0));
    let input_y = input_bottom - input_h;
    let label_h = scale(20, dpi).min((input_y - content_top).max(0));
    let label_y = input_y - label_h;
    let output_top = (content_top + scale(16, dpi)).min(label_y);
    let output_bottom = (label_y - gap).max(output_top);
    let button_w = scale(104, dpi).min(content_w);
    let button_gap = gap.min((content_w - button_w).max(0));
    let input_w = content_w - button_w - button_gap;
    let button_h = scale(36, dpi).min(input_h);

    WindowLayout {
        title_bar: rect(0, 0, w, title_h),
        sidebar: rect(0, title_h, sidebar_w, h - title_h),
        session_header: rect(sidebar_w, title_h, w - sidebar_w, header_h),
        output: rect(content_x, output_top, content_w, output_bottom - output_top),
        input_label: rect(content_x, label_y, content_w, label_h),
        input: rect(content_x, input_y, input_w, input_h),
        execute_button: rect(
            content_x + input_w + button_gap,
            input_y + (input_h - button_h) / 2,
            button_w,
            button_h,
        ),
        status_bar: rect(sidebar_w, status_y, w - sidebar_w, status_h),
    }
}

pub struct TabPlacement {
    pub index: usize,
    pub tab: Rect,
    pub close: Rect,
}

pub struct TabStrip {
    pub first_visible: usize,
    pub tabs: Vec<TabPlacement>,
    pub new_tab: Rect,
}

pub fn tabs(header: Rect, dpi: u32, count: usize, active: usize, scroll: usize) -> TabStrip {
    let inset = scale(12, dpi).min(header.width / 2);
    let gap = scale(8, dpi);
    let action_w = scale(40, dpi).min(header.width - 2 * inset);
    let available = (header.width - 2 * inset - action_w - gap).max(0);
    let min_width = scale(136, dpi).max(1);
    let visible = count.min((available / min_width).max(1) as usize);
    let tab_width = if visible == 0 {
        0
    } else {
        scale(208, dpi).min((available / visible as i32 - gap).max(0))
    };
    let first_visible = scroll
        .min(active)
        .max(active.saturating_add(1).saturating_sub(visible))
        .min(count.saturating_sub(visible));
    let tab_h = scale(44, dpi).min(header.height);
    let tab_y = header.y + header.height - tab_h;
    let mut placements = Vec::new();
    for slot in 0..visible {
        let x = header.x + inset + slot as i32 * (tab_width + gap);
        let close_w = scale(32, dpi).min(tab_width);
        let close_h = close_w.min(tab_h);
        placements.push(TabPlacement {
            index: first_visible + slot,
            tab: rect(x, tab_y, tab_width, tab_h),
            close: rect(
                (x + tab_width - close_w - scale(4, dpi)).max(x),
                tab_y + (tab_h - close_h) / 2,
                close_w,
                close_h,
            ),
        });
    }
    let action_x = header.x + inset + visible as i32 * (tab_width + gap);
    let action_y = header.y + (header.height - action_w) / 2;
    TabStrip {
        first_visible,
        tabs: placements,
        new_tab: rect(action_x, action_y, action_w, action_w.min(header.height)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within(r: Rect, w: i32, h: i32) {
        assert!(
            r.x >= 0 && r.y >= 0 && r.width >= 0 && r.height >= 0,
            "{r:?}"
        );
        assert!(
            r.x + r.width <= w && r.y + r.height <= h,
            "{r:?} outside {w}x{h}"
        );
    }

    #[test]
    fn layout_fits_compact_and_large_windows_at_supported_scales() {
        for dpi in [96, 144, 192] {
            for (width, height) in [(480, 320), (800, 600), (1280, 800)] {
                let (w, h) = (scale(width, dpi), scale(height, dpi));
                for expanded in [false, true] {
                    let l = calculate(w, h, dpi, expanded);
                    for r in [
                        l.title_bar,
                        l.sidebar,
                        l.session_header,
                        l.output,
                        l.input_label,
                        l.input,
                        l.execute_button,
                        l.status_bar,
                    ] {
                        within(r, w, h);
                    }
                    assert_eq!(l.sidebar.y, l.title_bar.height);
                    assert!(l.output.y >= l.session_header.y + l.session_header.height);
                    assert!(l.output.y + l.output.height <= l.input_label.y);
                    assert!(l.input.x + l.input.width <= l.execute_button.x);
                    assert!(l.input.y + l.input.height <= l.status_bar.y);
                }
            }
        }
    }

    #[test]
    fn overflowing_tabs_keep_active_tab_and_actions_inside_strip() {
        for dpi in [96, 144, 192] {
            for width in [424, 640, 1136] {
                let header = rect(
                    scale(56, dpi),
                    scale(TITLE_HEIGHT, dpi),
                    scale(width, dpi),
                    scale(52, dpi),
                );
                for count in [1, 3, 30] {
                    for active in [0, count / 2, count - 1] {
                        let strip = tabs(header, dpi, count, active, 0);
                        assert!(strip.tabs.iter().any(|tab| tab.index == active));
                        for tab in &strip.tabs {
                            assert!(tab.close.x >= tab.tab.x);
                            assert!(tab.close.x + tab.close.width <= tab.tab.x + tab.tab.width);
                            assert!(tab.tab.x + tab.tab.width <= strip.new_tab.x);
                        }
                        assert!(strip.new_tab.x + strip.new_tab.width <= header.x + header.width);
                    }
                }
            }
        }
    }

    #[test]
    fn tiny_and_zero_windows_stay_in_bounds() {
        for (w, h) in [(0, 0), (1, 1), (10, 10), (100, 50)] {
            let l = calculate(w, h, 96, false);
            for r in [
                l.title_bar,
                l.sidebar,
                l.session_header,
                l.output,
                l.input_label,
                l.input,
                l.execute_button,
                l.status_bar,
            ] {
                within(r, w, h);
            }
        }
    }
}
