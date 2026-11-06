use windows::Win32::Foundation::{COLORREF, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, Ellipse, FillRect, GetStockObject,
    GetTextExtentPoint32W, Polyline, RestoreDC, RoundRect, SaveDC, SelectObject, SetBkMode,
    SetTextColor, DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, HBRUSH, HDC, HFONT, NULL_BRUSH, NULL_PEN, PS_SOLID, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetDlgCtrlID, GetDlgItem, GetParent, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
};

use crate::background::{self, Surface};
use crate::dpi::scale;
use crate::theme::Theme;

#[derive(Clone, Copy)]
pub enum SidebarButton {
    Toggle,
    Terminals,
    Profiles,
}

#[derive(Clone, Copy)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

pub fn draw_title_bar(hdc: HDC, rect: RECT, theme: &Theme, dpi: u32) {
    if !has_area(rect) {
        return;
    }
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    background::paint(
        hdc,
        rect,
        RECT {
            bottom: rect.bottom + scale(52, dpi),
            ..rect
        },
        Surface::Title,
        theme,
    );

    let icon_size = scale(24, dpi);
    let icon_left = rect.left + scale(12, dpi);
    let icon_top = rect.top + (rect.bottom - rect.top - icon_size) / 2;
    crate::profile_icons::draw_brand(
        hdc,
        RECT {
            left: icon_left,
            top: icon_top,
            right: icon_left + icon_size,
            bottom: icon_top + icon_size,
        },
    );

    let text_left = icon_left + icon_size + scale(14, dpi);
    draw_text(
        hdc,
        RECT {
            left: text_left,
            top: rect.top,
            right: (rect.right - scale(156, dpi)).max(text_left),
            bottom: rect.bottom,
        },
        "BKS Term",
        theme.title_font,
        theme.colors.text_primary,
    );
}

pub fn draw_tab_background(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32) {
    let rc = item.rcItem;
    let line = RECT {
        left: rc.left,
        top: (rc.bottom - scale(1, dpi).max(1)).max(rc.top),
        right: rc.right,
        bottom: rc.bottom,
    };
    unsafe {
        background::control(item, Surface::Title, theme, dpi);
        FillRect(item.hDC, &line, theme.border_subtle);
    }
}

pub fn draw_connection_status(
    item: &DRAWITEMSTRUCT,
    theme: &Theme,
    dpi: u32,
    connected: Option<bool>,
) {
    draw_status(item, theme, dpi, Some(connected));
}

pub fn draw_status(
    item: &DRAWITEMSTRUCT,
    theme: &Theme,
    dpi: u32,
    connection: Option<Option<bool>>,
) {
    let rc = item.rcItem;
    if !has_area(rc) {
        return;
    }
    let hdc = item.hDC;
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    background::control(item, Surface::Status, theme, dpi);
    let text = control_text(item);
    if text.is_empty() {
        return;
    }
    let mut text_size = windows::Win32::Foundation::SIZE::default();
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let previous = SelectObject(hdc, theme.status_font.into());
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut text_size);
        SelectObject(hdc, previous);
    }
    let pill = RECT {
        left: rc.left + scale(2, dpi),
        top: rc.top + scale(5, dpi),
        right: (rc.left + text_size.cx + scale(if connection.is_some() { 38 } else { 20 }, dpi))
            .min(rc.right - scale(2, dpi)),
        bottom: rc.bottom - scale(5, dpi),
    };
    rounded_fill(hdc, pill, theme.surface, scale(11, dpi));
    rounded_outline(
        hdc,
        pill,
        if theme.high_contrast {
            theme.colors.text_secondary
        } else {
            crate::theme::BORDER_SUBTLE
        },
        scale(1, dpi).max(1),
        scale(11, dpi),
    );
    let color = match connection.flatten() {
        Some(true) => theme.colors.status_success,
        Some(false) => theme.colors.status_error,
        None => theme.colors.text_muted,
    };
    let radius = scale(4, dpi).max(1);
    let x = rc.left + scale(12, dpi);
    let y = rc.top + (rc.bottom - rc.top) / 2;
    unsafe {
        if connection.is_some() {
            let brush = CreateSolidBrush(color);
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let _ = Ellipse(hdc, x - radius, y - radius, x + radius + 1, y + radius + 1);
            let _ = SelectObject(hdc, old_pen);
            let _ = SelectObject(hdc, old_brush);
            let _ = DeleteObject(brush.into());
        }
    }
    draw_text(
        hdc,
        RECT {
            left: (rc.left + scale(if connection.is_some() { 26 } else { 10 }, dpi)).min(rc.right),
            top: rc.top,
            right: (rc.right - scale(8, dpi)).max(rc.left),
            bottom: rc.bottom,
        },
        &text,
        theme.status_font,
        theme.colors.text_secondary,
    );
}

pub fn draw_tab_button(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32, active: bool) {
    let rc = item.rcItem;
    if !has_area(rc) {
        return;
    }
    let hdc = item.hDC;
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let hot = item.itemState.0 & (ODS_HOTLIGHT.0 | ODS_SELECTED.0) != 0;
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let thickness = scale(1, dpi).max(1);
    background::control(item, Surface::Title, theme, dpi);
    let capsule = inset(rc, scale(2, dpi));
    background::tab(hdc, capsule, capsule, active, theme, dpi, true);
    rounded_outline(
        hdc,
        capsule,
        if theme.high_contrast {
            theme.colors.brand
        } else if active {
            crate::theme::BRAND_PRIMARY
        } else if hot {
            crate::theme::BORDER
        } else {
            crate::theme::BORDER_SUBTLE
        },
        thickness,
        scale(12, dpi),
    );

    let text = control_text(item);
    let icon_size = scale(20, dpi);
    let icon_rect = RECT {
        left: rc.left + scale(12, dpi),
        top: rc.top + (rc.bottom - rc.top - icon_size) / 2,
        right: rc.left + scale(12, dpi) + icon_size,
        bottom: rc.top + (rc.bottom - rc.top - icon_size) / 2 + icon_size,
    };
    draw_profile_icon(hdc, icon_rect, &text, theme, dpi, disabled);
    let text_color = if disabled {
        theme.colors.text_muted
    } else if active || hot || focused {
        theme.colors.text_primary
    } else {
        theme.colors.text_secondary
    };
    draw_text(
        hdc,
        RECT {
            left: (rc.left + scale(42, dpi)).min(rc.right),
            top: rc.top,
            right: (rc.right - scale(36, dpi)).max(rc.left),
            bottom: rc.bottom,
        },
        &text,
        theme.ui_font,
        text_color,
    );
    if focused {
        rounded_outline(
            hdc,
            inset(rc, scale(3, dpi)),
            focus_color(theme),
            scale(2, dpi).max(1),
            scale(10, dpi),
        );
    }
}

pub fn draw_tab_close(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32, active: bool) {
    let rc = item.rcItem;
    if !has_area(rc) {
        return;
    }
    let hdc = item.hDC;
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let hot = item.itemState.0 & ODS_HOTLIGHT.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let mut canvas = rc;
    unsafe {
        let parent = GetParent(item.hwndItem).unwrap_or_default();
        let id = GetDlgCtrlID(item.hwndItem) - 1000;
        if let Ok(tab) = GetDlgItem(Some(parent), id) {
            let mut tab_rc = RECT::default();
            let mut close_rc = RECT::default();
            let _ = GetWindowRect(tab, &mut tab_rc);
            let _ = GetWindowRect(item.hwndItem, &mut close_rc);
            canvas = RECT {
                left: tab_rc.left - close_rc.left,
                top: tab_rc.top - close_rc.top,
                right: tab_rc.right - close_rc.left,
                bottom: tab_rc.bottom - close_rc.top,
            };
        }
    }
    background::tab(
        hdc,
        rc,
        inset(canvas, scale(2, dpi)),
        active,
        theme,
        dpi,
        false,
    );
    if !disabled && (hot || pressed) {
        rounded_fill(hdc, inset(rc, scale(2, dpi)), theme.hover, scale(5, dpi));
    }
    let color = if disabled {
        theme.colors.text_muted
    } else if hot || pressed || focused {
        theme.colors.text_primary
    } else {
        theme.colors.text_secondary
    };
    draw_cross(hdc, rc, scale(5, dpi), color, scale(1, dpi).max(1));
    if focused {
        rounded_outline(
            hdc,
            inset(rc, scale(2, dpi)),
            focus_color(theme),
            scale(1, dpi).max(1),
            scale(5, dpi),
        );
    }
}

pub fn draw_new_tab(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32) {
    let Some(_state) = DcState::save(item.hDC) else {
        return;
    };
    background::control(item, Surface::Title, theme, dpi);
    let rc = inset(item.rcItem, scale(3, dpi));
    let hot = item.itemState.0 & ODS_HOTLIGHT.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    rounded_fill(
        item.hDC,
        rc,
        if pressed {
            theme.toolbar
        } else if hot {
            theme.hover
        } else {
            theme.surface
        },
        scale(11, dpi),
    );
    rounded_outline(
        item.hDC,
        rc,
        if focused {
            focus_color(theme)
        } else if theme.high_contrast {
            theme.colors.text_secondary
        } else {
            crate::theme::BORDER_SUBTLE
        },
        scale(if focused { 2 } else { 1 }, dpi).max(1),
        scale(11, dpi),
    );
    let x = (rc.left + rc.right) / 2;
    let y = (rc.top + rc.bottom) / 2;
    let half = scale(6, dpi);
    let width = scale(1, dpi).max(1);
    stroke(
        item.hDC,
        [(x - half, y), (x + half + 1, y)],
        theme.colors.text_primary,
        width,
    );
    stroke(
        item.hDC,
        [(x, y - half), (x, y + half + 1)],
        theme.colors.text_primary,
        width,
    );
}

pub fn draw_input_label(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32) {
    background::control(item, Surface::Terminal, theme, dpi);
    draw_text(
        item.hDC,
        item.rcItem,
        &control_text(item),
        theme.ui_font,
        theme.colors.text_secondary,
    );
}

pub fn draw_input_frame(hdc: HDC, rect: RECT, theme: &Theme, dpi: u32, focused: bool) {
    if !theme.high_contrast {
        // The native caret identifies focus on the continuous terminal canvas.
        // No secondary underline or partial frame belongs around the editor.
        return;
    }
    let frame = RECT {
        left: rect.left - scale(3, dpi),
        top: rect.top - scale(3, dpi),
        right: rect.right + scale(3, dpi),
        bottom: rect.bottom + scale(3, dpi),
    };
    rounded_outline(
        hdc,
        frame,
        if focused {
            focus_color(theme)
        } else if theme.high_contrast {
            theme.colors.text_secondary
        } else {
            crate::theme::BORDER
        },
        scale(if focused { 2 } else { 1 }, dpi).max(1),
        scale(10, dpi),
    );
}

pub fn draw_sidebar_button(
    item: &DRAWITEMSTRUCT,
    theme: &Theme,
    dpi: u32,
    kind: SidebarButton,
    expanded: bool,
) {
    let hdc = item.hDC;
    let rc = item.rcItem;
    if !has_area(rc) {
        return;
    }
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let hot = item.itemState.0 & ODS_HOTLIGHT.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let terminals = matches!(kind, SidebarButton::Terminals);
    let expanded = expanded && rc.right - rc.left >= scale(80, dpi);
    background::control(item, Surface::Sidebar, theme, dpi);
    if terminals && !hot && !pressed {
        background::selected(hdc, rc, theme, dpi);
    } else if hot || pressed {
        rounded_fill(
            hdc,
            rc,
            if hot || pressed {
                theme.hover
            } else {
                theme.selected
            },
            scale(8, dpi),
        );
    }
    if terminals {
        rounded_fill(
            hdc,
            RECT {
                left: rc.left + scale(2, dpi),
                top: rc.top + scale(7, dpi),
                right: rc.left + scale(4, dpi),
                bottom: (rc.bottom - scale(7, dpi)).max(rc.top),
            },
            theme.brand,
            scale(1, dpi).max(1),
        );
    }

    let icon_size = scale(24, dpi);
    let icon_left = if expanded {
        rc.left + scale(12, dpi)
    } else {
        rc.left + (rc.right - rc.left - icon_size) / 2
    };
    let center_y = rc.top + (rc.bottom - rc.top) / 2;
    let icon_rect = RECT {
        left: icon_left,
        top: center_y - icon_size / 2,
        right: icon_left + icon_size,
        bottom: center_y - icon_size / 2 + icon_size,
    };
    let color = if disabled {
        theme.colors.text_muted
    } else if terminals {
        theme.colors.brand
    } else {
        theme.colors.text_secondary
    };
    if terminals {
        draw_chevron(hdc, icon_rect, color, scale(2, dpi).max(1));
    } else if matches!(kind, SidebarButton::Profiles) {
        let inset = scale(2, dpi);
        stroke(
            hdc,
            [
                (icon_rect.left + inset, icon_rect.top + scale(5, dpi)),
                (
                    icon_rect.left + scale(10, dpi),
                    icon_rect.top + scale(5, dpi),
                ),
                (
                    icon_rect.left + scale(13, dpi),
                    icon_rect.top + scale(8, dpi),
                ),
                (icon_rect.right - inset, icon_rect.top + scale(8, dpi)),
                (icon_rect.right - inset, icon_rect.bottom - scale(3, dpi)),
                (icon_rect.left + inset, icon_rect.bottom - scale(3, dpi)),
                (icon_rect.left + inset, icon_rect.top + scale(5, dpi)),
            ],
            color,
            scale(2, dpi).max(1),
        );
    } else {
        for offset in [-6, 0, 6] {
            let y = center_y + scale(offset, dpi);
            stroke(
                hdc,
                [
                    (icon_rect.left + scale(1, dpi), y),
                    (icon_rect.right - scale(1, dpi), y),
                ],
                color,
                scale(1, dpi).max(1),
            );
        }
    }

    if !matches!(kind, SidebarButton::Toggle) && expanded {
        draw_text(
            hdc,
            RECT {
                left: (rc.left + scale(44, dpi)).min(rc.right),
                top: rc.top,
                right: (rc.right - scale(6, dpi)).max(rc.left),
                bottom: rc.bottom,
            },
            if terminals {
                "Терминалы"
            } else {
                "Профили"
            },
            theme.ui_font,
            if disabled {
                theme.colors.text_muted
            } else {
                theme.colors.text_primary
            },
        );
    }
    if focused {
        rounded_outline(
            hdc,
            inset(rc, scale(1, dpi).max(1)),
            theme.colors.brand,
            scale(1, dpi).max(1),
            scale(7, dpi),
        );
    }
}

pub fn draw_window_button(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32, kind: WindowButton) {
    let hdc = item.hDC;
    let rc = item.rcItem;
    if !has_area(rc) {
        return;
    }
    let Some(_state) = DcState::save(hdc) else {
        return;
    };
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let hot = item.itemState.0 & (ODS_HOTLIGHT.0 | ODS_SELECTED.0) != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let close_hover = !disabled && hot && matches!(kind, WindowButton::Close);
    if close_hover {
        unsafe {
            let brush = CreateSolidBrush(theme.colors.status_error);
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
        }
    } else if !disabled && hot {
        unsafe { FillRect(hdc, &rc, theme.hover) };
    } else {
        background::control(item, Surface::Title, theme, dpi);
    }
    let color = if disabled {
        theme.colors.text_muted
    } else if close_hover {
        theme.colors.on_primary
    } else {
        theme.colors.text_secondary
    };
    let x = rc.left + (rc.right - rc.left) / 2;
    let y = rc.top + (rc.bottom - rc.top) / 2;
    let half = scale(6, dpi);
    let width = scale(1, dpi).max(1);
    match kind {
        WindowButton::Minimize => stroke(hdc, [(x - half, y), (x + half, y)], color, width),
        WindowButton::Maximize => stroke(
            hdc,
            [
                (x - half, y - half),
                (x + half, y - half),
                (x + half, y + half),
                (x - half, y + half),
                (x - half, y - half),
            ],
            color,
            width,
        ),
        WindowButton::Restore => {
            let offset = scale(3, dpi);
            stroke(
                hdc,
                [
                    (x - half + offset, y - half),
                    (x + half, y - half),
                    (x + half, y + half - offset),
                ],
                color,
                width,
            );
            stroke(
                hdc,
                [
                    (x - half, y - half + offset),
                    (x + half - offset, y - half + offset),
                    (x + half - offset, y + half),
                    (x - half, y + half),
                    (x - half, y - half + offset),
                ],
                color,
                width,
            );
        }
        WindowButton::Close => draw_cross(hdc, rc, half, color, width),
    }
    if focused {
        rounded_outline(
            hdc,
            inset(rc, scale(4, dpi)),
            if close_hover {
                theme.colors.on_primary
            } else {
                theme.colors.brand
            },
            width,
            scale(4, dpi),
        );
    }
}

fn focus_color(theme: &Theme) -> COLORREF {
    if theme.high_contrast {
        theme.colors.brand
    } else {
        crate::theme::FOCUS_RING
    }
}

fn control_text(item: &DRAWITEMSTRUCT) -> String {
    unsafe {
        let length = GetWindowTextLengthW(item.hwndItem).max(0) as usize;
        let mut buffer = vec![0_u16; length + 1];
        let copied = GetWindowTextW(item.hwndItem, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }
}

fn draw_profile_icon(hdc: HDC, rect: RECT, text: &str, theme: &Theme, dpi: u32, disabled: bool) {
    if !disabled && crate::profile_icons::draw(hdc, rect, text) {
        return;
    }
    let color = if disabled {
        theme.colors.text_muted
    } else {
        theme.colors.brand
    };
    draw_chevron(hdc, rect, color, scale(2, dpi).max(1));
}

fn draw_chevron(hdc: HDC, rect: RECT, color: COLORREF, width: i32) {
    let x = rect.left + (rect.right - rect.left) / 2;
    let y = rect.top + (rect.bottom - rect.top) / 2;
    let half_w = ((rect.right - rect.left) * 3 / 20).max(1);
    let half_h = ((rect.bottom - rect.top) * 2 / 5).max(1);
    stroke(
        hdc,
        [
            (x - half_w, y - half_h),
            (x + half_w, y),
            (x - half_w, y + half_h),
        ],
        color,
        width,
    );
}

fn draw_cross(hdc: HDC, rect: RECT, half: i32, color: COLORREF, width: i32) {
    let x = rect.left + (rect.right - rect.left) / 2;
    let y = rect.top + (rect.bottom - rect.top) / 2;
    stroke(
        hdc,
        [(x - half, y - half), (x + half, y + half)],
        color,
        width,
    );
    stroke(
        hdc,
        [(x + half, y - half), (x - half, y + half)],
        color,
        width,
    );
}

fn draw_text(hdc: HDC, rect: RECT, text: &str, font: HFONT, color: COLORREF) {
    if !has_area(rect) {
        return;
    }
    let mut text: Vec<u16> = text.encode_utf16().collect();
    let mut rect = rect;
    let flags: DRAW_TEXT_FORMAT =
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
    unsafe {
        let old_font = SelectObject(hdc, font.into());
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, color);
        if !crate::backdrop::active()
            || !crate::glass_buffer::draw_text(hdc, &text, rect, flags, color)
        {
            let _ = DrawTextW(hdc, &mut text, &mut rect, flags);
        }
        let _ = SelectObject(hdc, old_font);
    }
}

fn rounded_fill(hdc: HDC, rect: RECT, brush: HBRUSH, radius: i32) {
    if !has_area(rect) {
        return;
    }
    unsafe {
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        let _ = SelectObject(hdc, old_pen);
        let _ = SelectObject(hdc, old_brush);
    }
}

fn rounded_outline(hdc: HDC, rect: RECT, color: COLORREF, width: i32, radius: i32) {
    if !has_area(rect) {
        return;
    }
    unsafe {
        let pen = CreatePen(PS_SOLID, width, color);
        let old_pen = SelectObject(hdc, pen.into());
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen.into());
    }
}

fn stroke<const N: usize>(hdc: HDC, points: [(i32, i32); N], color: COLORREF, width: i32) {
    let points = points.map(|(x, y)| POINT { x, y });
    unsafe {
        let pen = CreatePen(PS_SOLID, width, color);
        let old_pen = SelectObject(hdc, pen.into());
        let _ = Polyline(hdc, &points);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen.into());
    }
}

fn inset(rect: RECT, amount: i32) -> RECT {
    RECT {
        left: rect.left + amount,
        top: rect.top + amount,
        right: rect.right - amount,
        bottom: rect.bottom - amount,
    }
}

fn has_area(rect: RECT) -> bool {
    rect.right > rect.left && rect.bottom > rect.top
}

struct DcState {
    hdc: HDC,
    saved: i32,
}

impl DcState {
    fn save(hdc: HDC) -> Option<Self> {
        let saved = unsafe { SaveDC(hdc) };
        if saved == 0 {
            None
        } else {
            Some(Self { hdc, saved })
        }
    }
}

impl Drop for DcState {
    fn drop(&mut self) {
        unsafe {
            let _ = RestoreDC(self.hdc, self.saved);
        }
    }
}
