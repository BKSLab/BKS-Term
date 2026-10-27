use std::ffi::c_void;

use crate::accessibility::set_accessible_name;
use crate::dpi::scale;
use crate::layout::Rect;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_ENDELLIPSIS};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowLongW, GetWindowRect, MoveWindow, SendMessageW,
    ShowWindow, BS_OWNERDRAW, ES_AUTOHSCROLL, GWL_STYLE, HMENU, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_SETFONT, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

pub const ID_QUERY: usize = 1020;
pub const ID_PREVIOUS: usize = 1021;
pub const ID_NEXT: usize = 1022;
pub const ID_CLOSE: usize = 1023;
pub const ID_STATUS: usize = 1024;
pub const ID_LABEL: usize = 1025;
pub const HEIGHT_DIP: i32 = 36;

#[derive(Clone, Copy, Default)]
pub struct SearchBar {
    pub label: HWND,
    pub query: HWND,
    pub previous: HWND,
    pub next: HWND,
    pub close: HWND,
    pub status: HWND,
}

/// Create hidden native controls. The parent owns their lifetime and painting.
pub fn create(parent: HWND, instance: HINSTANCE) -> Result<SearchBar> {
    let mut bar = SearchBar::default();
    let result: Result<()> = (|| {
        // Native EDIT UIA proxies also derive Name from the preceding STATIC.
        bar.label = control(
            parent,
            instance,
            w!("STATIC"),
            w!("Поиск по экрану терминала"),
            WS_CHILD | WINDOW_STYLE(SS_CENTERIMAGE.0 | SS_ENDELLIPSIS.0),
            ID_LABEL,
        )?;
        bar.query = control(
            parent,
            instance,
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            ID_QUERY,
        )?;
        for (handle, caption, id, name) in [
            (
                &mut bar.previous,
                w!("Назад"),
                ID_PREVIOUS,
                "Предыдущее совпадение",
            ),
            (&mut bar.next, w!("Далее"), ID_NEXT, "Следующее совпадение"),
            (&mut bar.close, w!("Закрыть"), ID_CLOSE, "Закрыть поиск"),
        ] {
            *handle = control(
                parent,
                instance,
                w!("BUTTON"),
                caption,
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                id,
            )?;
            crate::control_state::install(*handle);
            set_accessible_name(*handle, name)?;
        }
        bar.status = control(
            parent,
            instance,
            w!("STATIC"),
            w!("Введите текст"),
            WS_CHILD | WINDOW_STYLE(SS_CENTERIMAGE.0 | SS_ENDELLIPSIS.0),
            ID_STATUS,
        )?;
        set_accessible_name(bar.query, "Поиск по экрану терминала")?;
        // Keep status Name native: SetWindowTextW must expose the new count,
        // rather than a fixed accessibility annotation overriding its text.
        unsafe {
            let theme = if crate::system_colors::is_high_contrast() {
                PCWSTR::null()
            } else {
                w!("DarkMode_Explorer")
            };
            let _ = SetWindowTheme(bar.query, theme, PCWSTR::null());
        }
        Ok(())
    })();
    if let Err(error) = result {
        for handle in bar.handles() {
            if !handle.0.is_null() {
                unsafe {
                    let _ = DestroyWindow(handle);
                }
            }
        }
        return Err(error);
    }
    Ok(bar)
}

fn control(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    caption: PCWSTR,
    style: WINDOW_STYLE,
    id: usize,
) -> Result<HWND> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            caption,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut c_void)),
            Some(instance),
            None,
        )
    }
}

impl SearchBar {
    fn handles(&self) -> [HWND; 6] {
        [
            self.label,
            self.query,
            self.previous,
            self.next,
            self.close,
            self.status,
        ]
    }

    pub fn show(&self, visible: bool) {
        for handle in self.handles() {
            let allocated = if handle == self.label || handle == self.status {
                let mut rect = RECT::default();
                (unsafe { GetWindowRect(handle, &mut rect).is_ok() })
                    && rect.right > rect.left
                    && rect.bottom > rect.top
            } else {
                true
            };
            unsafe {
                let _ = ShowWindow(
                    handle,
                    if visible && allocated {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
    }

    pub fn layout(&self, rect: Rect, dpi: u32) {
        let layout = geometry(rect, dpi);
        unsafe {
            // Read the control's own style so layout also works before its
            // parent is visible and independently of show/layout call order.
            let visible = GetWindowLongW(self.query, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0;
            for (handle, rect) in self.handles().into_iter().zip(layout) {
                let _ = MoveWindow(handle, rect.x, rect.y, rect.width, rect.height, true);
                if handle == self.label || handle == self.status {
                    let _ = ShowWindow(
                        handle,
                        if visible && rect.width > 0 && rect.height > 0 {
                            SW_SHOW
                        } else {
                            SW_HIDE
                        },
                    );
                }
            }
        }
    }

    pub fn apply_font(&self, font: HFONT) {
        for handle in self.handles() {
            unsafe {
                SendMessageW(
                    handle,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
    }
}

// Order matches SearchBar::handles: label, query, previous, next, close, status.
fn geometry(rect: Rect, dpi: u32) -> [Rect; 6] {
    let width = rect.width.max(0);
    let height = rect.height.max(0).min(scale(HEIGHT_DIP, dpi));
    let label = if width >= scale(640, dpi) {
        scale(176, dpi)
    } else {
        0
    };
    let desired_buttons = [scale(64, dpi), scale(64, dpi), scale(72, dpi)];
    let button_total: i32 = desired_buttons.into_iter().sum();
    let min_query = scale(100, dpi).min(width);
    let desired_status = scale(if width >= scale(456, dpi) { 128 } else { 64 }, dpi);
    let usual_gap = scale(6, dpi);
    let status_room =
        width - label - min_query - button_total - usual_gap * (4 + i32::from(label > 0));
    let status = if status_room >= scale(48, dpi) {
        status_room.min(desired_status)
    } else {
        0
    };
    let gap_count = 3 + i32::from(label > 0) + i32::from(status > 0);
    let gap = usual_gap.min(width / gap_count);
    let button_budget = (width - label - status - gap * gap_count - min_query).max(0);
    let buttons = if button_budget < button_total {
        let previous = desired_buttons[0] * button_budget / button_total;
        let next = desired_buttons[1] * button_budget / button_total;
        [previous, next, button_budget - previous - next]
    } else {
        desired_buttons
    };
    let query = width - label - status - buttons.into_iter().sum::<i32>() - gap * gap_count;
    let widths = [label, query, buttons[0], buttons[1], buttons[2], status];
    let mut x = rect.x;
    let mut result = [Rect::default(); 6];
    for (index, width) in widths.into_iter().enumerate() {
        let item_height = if index == 1 {
            height.min(scale(30, dpi))
        } else {
            height
        };
        result[index] = Rect {
            x,
            y: rect.y + (height - item_height) / 2,
            width,
            height: item_height,
        };
        x += width;
        if (index < 4 || (index == 4 && status > 0)) && (index != 0 || label > 0) {
            x += gap;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_row_keeps_query_and_buttons_usable_at_each_scale() {
        for dpi in [96, 144, 192] {
            let row = Rect {
                x: 24,
                y: 80,
                width: scale(392, dpi),
                height: scale(HEIGHT_DIP, dpi),
            };
            let layout = geometry(row, dpi);
            assert_eq!(layout[0].width, 0);
            assert!(layout[1].width >= scale(100, dpi));
            assert!(layout[2].width >= scale(64, dpi));
            assert!(layout[3].width >= scale(64, dpi));
            assert!(layout[4].width >= scale(72, dpi));
            assert!(layout[5].width >= scale(48, dpi));
            assert_eq!(layout[5].x + layout[5].width, row.x + row.width);
        }
    }

    #[test]
    fn row_fits_its_bounds_from_zero_width_to_large_windows() {
        for dpi in [96, 144, 192] {
            for width in [0, 80, 240, 392, 640, 1000] {
                let row = Rect {
                    x: 27,
                    y: 88,
                    width: scale(width, dpi),
                    height: scale(36, dpi),
                };
                let layout = geometry(row, dpi);
                let mut previous_right = row.x;
                for item in layout {
                    assert!(item.width >= 0 && item.height >= 0, "{item:?}");
                    assert!(item.x >= previous_right, "overlap: {item:?}");
                    assert!(item.x + item.width <= row.x + row.width, "{item:?}");
                    assert!(item.y >= row.y && item.y + item.height <= row.y + row.height);
                    previous_right = item.x + item.width;
                }
                if width >= 640 {
                    assert!(layout[0].width > 0);
                    assert!(layout[1].width >= scale(100, dpi));
                }
            }
        }
    }
}
