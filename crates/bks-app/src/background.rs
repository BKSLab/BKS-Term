use crate::{dpi, layout, theme};
use theme::Theme;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, ExtSelectClipRgn, FillRect, GradientFill, IntersectClipRect,
    RestoreDC, SaveDC, ScreenToClient, GRADIENT_FILL_TRIANGLE, GRADIENT_TRIANGLE, HDC, RGN_AND,
    TRIVERTEX,
};
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetParent, GetWindowRect};

#[derive(Clone, Copy)]
pub enum Surface {
    Title,
    Sidebar,
    Status,
    Terminal,
}

pub fn paint(hdc: HDC, bounds: RECT, canvas: RECT, surface: Surface, theme: &Theme) {
    if crate::backdrop::active() && !theme.high_contrast {
        if let Some(mut buffer) = crate::glass_buffer::GlassBuffer::new(hdc, bounds) {
            paint_opaque(buffer.hdc(), bounds, canvas, surface, theme);
            // Desktop Acrylic already blurs and tints the backdrop. Keep our
            // tint light so it does not turn that material into a solid panel.
            buffer.seal_background(match surface {
                Surface::Title => 80,
                Surface::Sidebar => 112,
                Surface::Status => 80,
                Surface::Terminal => theme::TERMINAL_GLASS_ALPHA,
            });
            buffer.finish();
            return;
        }
    }
    paint_opaque(hdc, bounds, canvas, surface, theme);
}

fn paint_opaque(hdc: HDC, bounds: RECT, canvas: RECT, surface: Surface, theme: &Theme) {
    let brush = match surface {
        Surface::Title => theme.window,
        Surface::Sidebar => theme.sidebar,
        Surface::Status => theme.surface,
        Surface::Terminal => theme.terminal,
    };
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return;
    }
    unsafe {
        if theme.high_contrast
            || matches!(surface, Surface::Terminal)
            || canvas.right <= canvas.left
            || canvas.bottom <= canvas.top
        {
            FillRect(hdc, &bounds, brush);
            return;
        }
        let saved = SaveDC(hdc);
        if saved == 0 {
            FillRect(hdc, &bounds, brush);
            return;
        }
        IntersectClipRect(hdc, bounds.left, bounds.top, bounds.right, bounds.bottom);
        let colors: &[[[u8; 3]; 3]] = match surface {
            Surface::Title => &theme::TITLE_GRADIENT,
            Surface::Sidebar => &theme::SIDEBAR_GRADIENT,
            Surface::Status => &theme::STATUS_GRADIENT,
            Surface::Terminal => unreachable!(),
        };
        let stops: &[i32] = match surface {
            Surface::Title | Surface::Status => &[0, 500, 1000],
            Surface::Sidebar => &[0, 400, 850, 1000],
            Surface::Terminal => unreachable!(),
        };
        let mut vertices = Vec::with_capacity(colors.len() * 3);
        for (row, stops) in colors.iter().zip(stops) {
            for (column, color) in row.iter().enumerate() {
                vertices.push(vertex(
                    canvas.left + (canvas.right - canvas.left) * column as i32 / 2,
                    canvas.top + (canvas.bottom - canvas.top) * stops / 1000,
                    *color,
                ));
            }
        }
        let mut triangles = Vec::with_capacity((colors.len() - 1) * 4);
        for row in 0..colors.len() - 1 {
            for column in 0..2 {
                let top = (row * 3 + column) as u32;
                triangles.push(GRADIENT_TRIANGLE {
                    Vertex1: top,
                    Vertex2: top + 1,
                    Vertex3: top + 3,
                });
                triangles.push(GRADIENT_TRIANGLE {
                    Vertex1: top + 1,
                    Vertex2: top + 4,
                    Vertex3: top + 3,
                });
            }
        }
        if !GradientFill(
            hdc,
            &vertices,
            triangles.as_ptr().cast(),
            triangles.len() as u32,
            GRADIENT_FILL_TRIANGLE,
        )
        .as_bool()
        {
            FillRect(hdc, &bounds, brush);
        }
        let _ = RestoreDC(hdc, saved);
    }
}

pub fn control(item: &DRAWITEMSTRUCT, surface: Surface, theme: &Theme, dpi: u32) {
    unsafe {
        let parent = GetParent(item.hwndItem).unwrap_or_default();
        let mut client = RECT::default();
        let mut child = RECT::default();
        let _ = GetClientRect(parent, &mut client);
        let _ = GetWindowRect(item.hwndItem, &mut child);
        let mut origin = POINT {
            x: child.left,
            y: child.top,
        };
        let _ = ScreenToClient(parent, &mut origin);
        let layout = layout::calculate(
            client.right,
            client.bottom,
            dpi,
            crate::sidebar_expanded_for_width(client.right),
        );
        let canvas = match surface {
            Surface::Title => layout::Rect {
                height: layout.title_bar.height + layout.session_header.height,
                ..layout.title_bar
            },
            Surface::Sidebar => layout.sidebar,
            Surface::Status => layout.status_bar,
            Surface::Terminal => layout::Rect {
                x: 0,
                y: 0,
                width: client.right,
                height: client.bottom,
            },
        };
        paint(
            item.hDC,
            item.rcItem,
            RECT {
                left: canvas.x - origin.x,
                top: canvas.y - origin.y,
                right: canvas.x + canvas.width - origin.x,
                bottom: canvas.y + canvas.height - origin.y,
            },
            surface,
            theme,
        );
    }
}

pub fn selected(hdc: HDC, rect: RECT, theme: &Theme, dpi: u32) {
    unsafe {
        if theme.high_contrast {
            FillRect(hdc, &rect, theme.selected);
            return;
        }
        let saved = SaveDC(hdc);
        if saved == 0 {
            return;
        }
        let radius = dpi::scale(12, dpi);
        let region = CreateRoundRectRgn(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        ExtSelectClipRgn(hdc, Some(region), RGN_AND);
        let colors = theme::SELECTED_GRADIENT;
        let vertices = [
            vertex(rect.left, rect.top, colors[0]),
            vertex(rect.right, rect.top, colors[1]),
            vertex(rect.left, rect.bottom, colors[2]),
            vertex(rect.right, rect.bottom, colors[3]),
        ];
        let triangles = [
            GRADIENT_TRIANGLE {
                Vertex1: 0,
                Vertex2: 1,
                Vertex3: 2,
            },
            GRADIENT_TRIANGLE {
                Vertex1: 1,
                Vertex2: 3,
                Vertex3: 2,
            },
        ];
        let _ = GradientFill(
            hdc,
            &vertices,
            triangles.as_ptr().cast(),
            2,
            GRADIENT_FILL_TRIANGLE,
        );
        let _ = RestoreDC(hdc, saved);
        let _ = DeleteObject(region.into());
    }
}

/// One vertical material ramp shared by a tab and its separate native close button.
pub fn tab(
    hdc: HDC,
    bounds: RECT,
    canvas: RECT,
    active: bool,
    theme: &Theme,
    dpi: u32,
    rounded: bool,
) {
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return;
    }
    unsafe {
        if theme.high_contrast {
            FillRect(
                hdc,
                &bounds,
                if active {
                    theme.selected
                } else {
                    theme.toolbar
                },
            );
            return;
        }
        let saved = SaveDC(hdc);
        if saved == 0 {
            return;
        }
        IntersectClipRect(hdc, bounds.left, bounds.top, bounds.right, bounds.bottom);
        let region = rounded.then(|| {
            CreateRoundRectRgn(
                canvas.left,
                canvas.top,
                canvas.right,
                canvas.bottom,
                dpi::scale(24, dpi),
                dpi::scale(24, dpi),
            )
        });
        if let Some(region) = region {
            ExtSelectClipRgn(hdc, Some(region), RGN_AND);
        }
        let base = if active {
            theme::TAB_ACTIVE
        } else {
            theme::TAB_INACTIVE
        };
        let [r, g, b] = [base.0 as u8, (base.0 >> 8) as u8, (base.0 >> 16) as u8];
        let (top, bottom) = if active {
            ([r + 9, g + 8, b + 3], [r - 12, g - 19, b - 38])
        } else {
            ([r + 5, g + 4, b - 1], [r - 7, g - 10, b - 17])
        };
        let vertices = [
            vertex(canvas.left, canvas.top, top),
            vertex(canvas.right, canvas.top, top),
            vertex(canvas.left, canvas.bottom, bottom),
            vertex(canvas.right, canvas.bottom, bottom),
        ];
        let triangles = [
            GRADIENT_TRIANGLE {
                Vertex1: 0,
                Vertex2: 1,
                Vertex3: 2,
            },
            GRADIENT_TRIANGLE {
                Vertex1: 1,
                Vertex2: 3,
                Vertex3: 2,
            },
        ];
        let _ = GradientFill(
            hdc,
            &vertices,
            triangles.as_ptr().cast(),
            2,
            GRADIENT_FILL_TRIANGLE,
        );
        let _ = RestoreDC(hdc, saved);
        if let Some(region) = region {
            let _ = DeleteObject(region.into());
        }
    }
}

fn vertex(x: i32, y: i32, color: [u8; 3]) -> TRIVERTEX {
    TRIVERTEX {
        x,
        y,
        Red: u16::from(color[0]) << 8,
        Green: u16::from(color[1]) << 8,
        Blue: u16::from(color[2]) << 8,
        Alpha: 0,
    }
}
