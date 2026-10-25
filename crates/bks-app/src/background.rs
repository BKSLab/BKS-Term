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
}

pub fn paint(hdc: HDC, bounds: RECT, canvas: RECT, surface: Surface, theme: &Theme) {
    let brush = match surface {
        Surface::Title => theme.window,
        Surface::Sidebar => theme.sidebar,
    };
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return;
    }
    unsafe {
        if theme.high_contrast || canvas.right <= canvas.left || canvas.bottom <= canvas.top {
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
        };
        let stops: &[i32] = match surface {
            Surface::Title => &[0, 500, 1000],
            Surface::Sidebar => &[0, 400, 850, 1000],
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
            Surface::Title => layout.title_bar,
            Surface::Sidebar => layout.sidebar,
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
        let radius = dpi::scale(8, dpi);
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
