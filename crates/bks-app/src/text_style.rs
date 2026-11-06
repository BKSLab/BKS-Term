use crate::ansi_palette::{AnsiPalette, DARK_PALETTE};
use crate::theme::Theme;
use bks_terminal::{Color, Flags, NamedColor, StyleSpan, TerminalSnapshot, TextStyle};
use windows::Win32::Foundation::COLORREF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedStyle {
    pub foreground: COLORREF,
    pub background: COLORREF,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// A truecolor TUI canvas can share the terminal material without changing
/// source VT attributes or semantic history. A proven floating menu can use
/// the application's softer selection palette in its presentation only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiSurface {
    base: COLORREF,
    modal_accent: Option<(Color, Color)>,
}

impl TuiSurface {
    pub fn detect(styles: &[StyleSpan], live_start: u32) -> Option<Self> {
        Self::detect_weighted(styles.iter().map(|span| {
            (
                span.style,
                u64::from(span.end.saturating_sub(span.start.max(live_start))),
            )
        }))
    }

    /// The raw VT canvas and its native reader use the same presentation.
    /// This is visual evidence only; it never recognizes or invokes actions.
    pub fn detect_grid(frame: &TerminalSnapshot) -> Option<Self> {
        if !frame.alternate_screen
            || frame.display_offset != 0
            || frame.cols == 0
            || frame.cells.len() != frame.rows.checked_mul(frame.cols)?
        {
            return None;
        }
        // A large modal and the message panels beneath it can split the
        // neutral canvas into several shades. Its outside edge remains a
        // stable reference even when no single shade covers half the grid.
        let mut surface = Self::detect_perimeter(frame).or_else(|| {
            Self::detect_weighted(
                frame
                    .cells
                    .iter()
                    .map(|cell| (TextStyle::new(cell.fg, cell.bg, cell.flags), 1)),
            )
        })?;
        surface.modal_accent = modal_accent(frame, surface.base);
        Some(surface)
    }

    fn detect_perimeter(frame: &TerminalSnapshot) -> Option<Self> {
        let edge = frame.cells.iter().enumerate().filter_map(|(index, cell)| {
            let (row, column) = (index / frame.cols, index % frame.cols);
            (row == 0 || row + 1 == frame.rows || column == 0 || column + 1 == frame.cols)
                .then_some((TextStyle::new(cell.fg, cell.bg, cell.flags), 1))
        });
        let surface = Self::detect_weighted(edge)?;
        let (mut canvas, mut neutral) = (0, 0);
        for cell in &frame.cells {
            if !cell.flags.contains(Flags::INVERSE) {
                if let Some(color) = dark_neutral(cell.bg) {
                    neutral += 1;
                    canvas += usize::from(color == surface.base);
                }
            }
        }
        // A thin border around colored output is not an application canvas.
        (neutral > frame.cells.len() / 2 && canvas >= frame.cells.len().div_ceil(8))
            .then_some(surface)
    }

    fn detect_weighted(styles: impl IntoIterator<Item = (TextStyle, u64)>) -> Option<Self> {
        let mut colors = std::collections::BTreeMap::<u32, u64>::new();
        let mut total = 0_u64;
        for (style, length) in styles {
            total += length;
            if !style.flags.contains(Flags::INVERSE) {
                if let Some(color) = dark_neutral(style.background) {
                    *colors.entry(color.0).or_default() += length;
                }
            }
        }
        let (color, count) = colors.into_iter().max_by_key(|(_, count)| *count)?;
        // A small neutral highlight is not evidence of an application canvas.
        (count > total / 2).then_some(Self {
            base: COLORREF(color),
            modal_accent: None,
        })
    }

    pub fn is_canvas(self, source: TextStyle) -> bool {
        !source.flags.contains(Flags::INVERSE)
            && source.foreground != source.background
            && dark_neutral(source.background) == Some(self.base)
    }

    pub fn resolve(self, source: TextStyle, theme: &Theme) -> ResolvedStyle {
        let mut style = resolve(source, theme);
        if theme.high_contrast || source.flags.contains(Flags::INVERSE) {
            return style;
        }
        if let Some((accent, panel)) = self.modal_accent {
            if source.background == accent {
                let panel = self.resolve(
                    TextStyle::new(source.foreground, panel, source.flags),
                    theme,
                );
                style.background = blend(
                    panel.background,
                    crate::theme::BRAND_PRIMARY,
                    crate::theme::TEXT_SELECTION_TINT,
                );
                style.foreground = if source.foreground == source.background {
                    style.background
                } else if dark_neutral(source.foreground).is_some() {
                    theme.colors.text_primary
                } else {
                    style.foreground
                };
                return style;
            }
        }
        let Some(background) = dark_neutral(source.background) else {
            return style;
        };
        // Equal foreground/background can deliberately conceal glyphs. Keep
        // their original contrast rather than revealing them on the glass.
        if style.foreground == style.background {
            return style;
        }
        style.background = if background == self.base {
            theme.colors.terminal
        } else {
            // Secondary panels remain matte, with the canvas's neutral tint.
            // ANSI colors and inverse/selected cells never enter this path.
            let level = |color: COLORREF| {
                ((color.0 & 255) + ((color.0 >> 8) & 255) + ((color.0 >> 16) & 255)) / 3
            };
            if level(background) <= level(self.base) {
                return style;
            }
            let lift = 12 + level(background) - level(self.base);
            let channel =
                |shift: u32| (((theme.colors.terminal.0 >> shift) & 255_u32) + lift).min(255) as u8;
            rgb(channel(0), channel(8), channel(16))
        };
        style
    }
}

/// A single bright strip enclosed by a neutral floating panel, surrounded by
/// the dominant dark canvas. Ordinary ANSI output and inline colored spans do
/// not establish this shape. Ambiguous, clipped or multiple strips stay native.
fn modal_accent(frame: &TerminalSnapshot, canvas: COLORREF) -> Option<(Color, Color)> {
    let mut found = None;
    if frame.rows < 5 || frame.cols < 18 {
        return None;
    }
    for row in 2..frame.rows - 2 {
        let cells = &frame.cells[row * frame.cols..(row + 1) * frame.cols];
        let mut column = 0;
        while column < cells.len() {
            let start = column;
            let accent = cells[start].bg;
            column += 1;
            while column < cells.len() && cells[column].bg == accent {
                column += 1;
            }
            let end = column;
            let Color::Spec(color) = accent else { continue };
            if color.r.max(color.g).max(color.b) < 128
                || end - start < 12
                || start == 0
                || end == cells.len()
            {
                continue;
            }
            let panel = cells[start - 1].bg;
            let Some(neutral) = dark_neutral(panel) else {
                continue;
            };
            if neutral.0 <= canvas.0
                || cells[end].bg != panel
                || cells[start..end]
                    .iter()
                    .any(|cell| cell.flags.intersects(Flags::INVERSE | Flags::HIDDEN))
            {
                continue;
            }
            let left = cells[..start]
                .iter()
                .rposition(|cell| cell.bg != panel)
                .map_or(0, |index| index + 1);
            let right = end
                + cells[end..]
                    .iter()
                    .position(|cell| cell.bg != panel)
                    .unwrap_or(cells.len() - end);
            if left == 0 || right == cells.len() || start - left > 4 || right - end > 4 {
                continue;
            }
            if cells[..left]
                .iter()
                .chain(&cells[right..])
                .any(|cell| dark_neutral(cell.bg).is_none_or(|outside| outside.0 >= neutral.0))
            {
                continue;
            }
            if [row - 2, row - 1, row + 1, row + 2]
                .into_iter()
                .any(|other| {
                    frame.cells[other * frame.cols + left..other * frame.cols + right]
                        .iter()
                        .any(|cell| cell.bg != panel)
                })
            {
                continue;
            }
            if frame.cells.iter().filter(|cell| cell.bg == accent).count() != end - start {
                continue;
            }
            if found.replace((accent, panel)).is_some() {
                return None;
            }
        }
    }
    found
}

fn dark_neutral(color: Color) -> Option<COLORREF> {
    let Color::Spec(color) = color else {
        // Named/indexed colors often encode ANSI meaning (diffs, errors, etc.).
        return None;
    };
    let max = color.r.max(color.g).max(color.b);
    let min = color.r.min(color.g).min(color.b);
    (max <= 48 && max - min <= 8).then(|| rgb(color.r, color.g, color.b))
}

pub fn resolve(style: TextStyle, theme: &Theme) -> ResolvedStyle {
    let (mut foreground, mut background) = if theme.high_contrast {
        (theme.colors.text_primary, theme.colors.terminal)
    } else {
        (
            resolve_color(style.foreground, &DARK_PALETTE),
            resolve_color(style.background, &DARK_PALETTE),
        )
    };
    if style.flags.contains(Flags::DIM) && !theme.high_contrast {
        foreground = blend(foreground, background, 34);
    }
    if style.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }
    ResolvedStyle {
        foreground,
        background,
        bold: style.flags.contains(Flags::BOLD),
        italic: style.flags.contains(Flags::ITALIC),
        underline: style.flags.intersects(Flags::ALL_UNDERLINES),
        strike: style.flags.contains(Flags::STRIKEOUT),
    }
}

pub fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

pub fn blend(first: COLORREF, second: COLORREF, second_percent: u32) -> COLORREF {
    let channel = |shift: u32| {
        let first = (first.0 >> shift) & 0xffu32;
        let second = (second.0 >> shift) & 0xffu32;
        ((first * (100 - second_percent) + second * second_percent) / 100) as u8
    };
    rgb(channel(0), channel(8), channel(16))
}

pub fn indexed_color(index: u8, palette: &AnsiPalette) -> COLORREF {
    match index {
        0..=15 => palette.colors[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let level = |component: u8| {
                if component == 0 {
                    0
                } else {
                    55 + 40 * component
                }
            };
            rgb(level(index / 36), level(index / 6 % 6), level(index % 6))
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            rgb(level, level, level)
        }
    }
}

pub fn resolve_color(color: Color, palette: &AnsiPalette) -> COLORREF {
    match color {
        Color::Spec(color) => rgb(color.r, color.g, color.b),
        Color::Indexed(index) => indexed_color(index, palette),
        Color::Named(name) => match name {
            NamedColor::Foreground => palette.default_fg,
            NamedColor::Background => palette.default_bg,
            NamedColor::Cursor => palette.cursor,
            NamedColor::BrightForeground => palette.colors[15],
            NamedColor::DimForeground => blend(palette.default_fg, palette.default_bg, 34),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => blend(
                palette.colors[name as usize - NamedColor::DimBlack as usize],
                palette.default_bg,
                34,
            ),
            _ => palette.colors[name as usize],
        },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bks_terminal::Rgb;

    fn style(background: Color, flags: Flags) -> TextStyle {
        TextStyle::new(
            Color::Spec(Rgb {
                r: 220,
                g: 210,
                b: 190,
            }),
            background,
            flags,
        )
    }

    fn gray(value: u8) -> Color {
        Color::Spec(Rgb {
            r: value,
            g: value,
            b: value,
        })
    }

    #[test]
    fn tui_canvas_uses_glass_and_panels_keep_hierarchy_without_changing_text_colors() {
        let theme = Theme::default();
        let source = style(gray(9), Flags::DIM | Flags::ITALIC);
        let spans = [StyleSpan {
            start: 0,
            end: 100,
            style: source,
        }];
        let surface = TuiSurface::detect(&spans, 0).unwrap();
        let adapted = surface.resolve(source, &theme);
        assert_eq!(adapted.background, theme.colors.terminal);
        assert_eq!(adapted.foreground, resolve(source, &theme).foreground);
        assert!(adapted.italic);
        let panel = surface.resolve(style(gray(17), Flags::empty()), &theme);
        let selected = surface.resolve(style(gray(30), Flags::BOLD), &theme);
        assert_ne!(panel.background, adapted.background);
        assert_ne!(selected.background, panel.background);
        assert!(selected.bold);
        assert_ne!(
            surface
                .resolve(style(gray(45), Flags::empty()), &theme)
                .background,
            surface
                .resolve(style(gray(48), Flags::empty()), &theme)
                .background,
        );
        let lighter_canvas = TuiSurface {
            base: rgb(32, 32, 32),
            modal_accent: None,
        };
        for background in [gray(9), gray(17)] {
            let source = style(background, Flags::empty());
            assert_eq!(
                lighter_canvas.resolve(source, &theme),
                resolve(source, &theme)
            );
        }
        assert_eq!(spans[0].style, source);
    }

    #[test]
    fn tui_surface_preserves_ansi_inverse_high_contrast_and_hidden_contrast() {
        let theme = Theme::default();
        let surface = TuiSurface {
            base: rgb(9, 9, 9),
            modal_accent: None,
        };
        let cases = [
            style(Color::Named(NamedColor::Black), Flags::empty()),
            style(Color::Indexed(232), Flags::empty()),
            style(Color::Spec(Rgb { r: 64, g: 8, b: 8 }), Flags::empty()),
            style(gray(240), Flags::empty()),
            style(gray(9), Flags::INVERSE),
            TextStyle::new(gray(9), gray(9), Flags::empty()),
        ];
        for source in cases {
            assert_eq!(surface.resolve(source, &theme), resolve(source, &theme));
        }
        let high_contrast = Theme {
            high_contrast: true,
            ..theme
        };
        let source = style(gray(9), Flags::empty());
        assert_eq!(
            surface.resolve(source, &high_contrast),
            resolve(source, &high_contrast)
        );
    }

    #[test]
    fn small_highlights_and_retained_history_do_not_define_the_live_canvas() {
        let styles = [
            StyleSpan {
                start: 0,
                end: 1000,
                style: style(gray(9), Flags::empty()),
            },
            StyleSpan {
                start: 1000,
                end: 1200,
                style: TextStyle::default(),
            },
            StyleSpan {
                start: 1200,
                end: 1220,
                style: style(gray(17), Flags::empty()),
            },
        ];
        assert!(TuiSurface::detect(&styles, 1000).is_none());
        assert!(TuiSurface::detect(&styles, 1220).is_none());
        assert!(TuiSurface::detect(&styles, 0).is_some());
    }

    #[test]
    fn captured_fullscreen_canvas_adapts_without_changing_source_attributes() {
        let mut terminal = bks_terminal::TerminalModel::new(110, 36);
        terminal.push_bytes(&include_bytes!("../tests/fixtures/gutter-panel-menu.vt")[..13_222]);
        let frame = terminal.live_snapshot();
        let source =
            crate::tui_projection::project(&frame, &terminal.reading_document(), None).output;
        let visual = crate::tui_projection::visual_document(&frame, &source, &[(18, 22)]);
        let surface = TuiSurface::detect(&visual.document.styles, visual.live_start()).unwrap();
        let theme = Theme::default();
        let mut glass_units = 0;
        for span in &visual.document.styles {
            let original = resolve(span.style, &theme);
            let adapted = surface.resolve(span.style, &theme);
            assert_eq!(adapted.foreground, original.foreground);
            if adapted.background == theme.colors.terminal
                && original.background != adapted.background
            {
                glass_units += span.end - span.start;
            }
        }
        assert!(
            glass_units > 2000,
            "large explicit canvas uses the terminal material"
        );
        assert_eq!(frame, terminal.live_snapshot());
    }

    fn model_menu_frame() -> TerminalSnapshot {
        let mut terminal = bks_terminal::TerminalModel::new(110, 36);
        terminal
            .push_bytes(&include_bytes!("../tests/fixtures/tui-opencode-model-menu.vt")[..19_449]);
        terminal.live_snapshot()
    }

    pub(crate) fn panelled_menu_model() -> bks_terminal::TerminalModel {
        use std::fmt::Write;
        let mut terminal = bks_terminal::TerminalModel::new(100, 29);
        terminal
            .push_bytes(&include_bytes!("../tests/fixtures/tui-opencode-menu-stack.vt")[..37_931]);
        // Source-equivalent message panels around a byte-exact captured menu.
        // VT writes keep the source document, styles and live grid consistent.
        let mut panels = String::from("\x1b7\x1b[38;2;215;215;215;48;2;12;12;12m");
        for row in (2..7).chain(24..28) {
            write!(&mut panels, "\x1b[{};2H{}", row + 1, " ".repeat(98)).unwrap();
        }
        panels.push_str("\x1b8");
        terminal.push_bytes(panels.as_bytes());
        terminal
    }

    pub(crate) fn error_menu_frame() -> TerminalSnapshot {
        let mut model = bks_terminal::TerminalModel::new(100, 29);
        model
            .push_bytes(&include_bytes!("../tests/fixtures/tui-opencode-error-menu.vt")[..116_007]);
        model.live_snapshot()
    }

    #[test]
    fn captured_model_menu_softens_only_the_floating_accent_and_preserves_source_colors() {
        let frame = model_menu_frame();
        let original = frame.clone();
        let surface = TuiSurface::detect_grid(&frame).unwrap();
        let (accent, panel) = surface.modal_accent.expect("enclosed source menu strip");
        let theme = Theme::default();
        let mut accented = 0;
        let mut readable_accent_glyphs = 0;
        for cell in &frame.cells {
            let source = TextStyle::new(cell.fg, cell.bg, cell.flags);
            let styled = surface.resolve(source, &theme);
            if cell.bg == accent {
                accented += 1;
                let panel = surface.resolve(TextStyle::new(cell.fg, panel, cell.flags), &theme);
                assert_eq!(
                    styled.background,
                    blend(
                        panel.background,
                        crate::theme::BRAND_PRIMARY,
                        crate::theme::TEXT_SELECTION_TINT
                    )
                );
                if dark_neutral(source.foreground).is_some() {
                    readable_accent_glyphs += usize::from(cell.text != " ");
                    assert_eq!(styled.foreground, theme.colors.text_primary);
                } else {
                    assert_eq!(styled.foreground, resolve(source, &theme).foreground);
                }
            } else {
                assert_eq!(styled.foreground, resolve(source, &theme).foreground);
            }
            assert_eq!(styled.bold, cell.flags.contains(Flags::BOLD));
        }
        assert_eq!(accented, 58);
        assert!(readable_accent_glyphs > 10);
        for background in [
            Color::Named(NamedColor::Red),
            Color::Indexed(232),
            Color::Spec(bks_terminal::Rgb {
                r: 180,
                g: 20,
                b: 30,
            }),
        ] {
            let source = style(background, Flags::empty());
            assert_eq!(surface.resolve(source, &theme), resolve(source, &theme));
        }
        let concealed = TextStyle::new(accent, accent, Flags::empty());
        let resolved = surface.resolve(concealed, &theme);
        assert_eq!(resolved.foreground, resolved.background);
        let high_contrast = Theme {
            high_contrast: true,
            ..theme
        };
        for cell in &frame.cells {
            let source = TextStyle::new(cell.fg, cell.bg, cell.flags);
            assert_eq!(
                surface.resolve(source, &high_contrast),
                resolve(source, &high_contrast)
            );
        }
        assert_eq!(frame, original);
    }

    #[test]
    fn ambiguous_clipped_and_inverse_highlights_keep_their_source_palette() {
        let frame = model_menu_frame();
        let (accent, _) = TuiSurface::detect_grid(&frame)
            .unwrap()
            .modal_accent
            .unwrap();
        let first = frame
            .cells
            .iter()
            .position(|cell| cell.bg == accent)
            .unwrap();
        let mut duplicate = frame.clone();
        duplicate.cells[0].bg = accent;
        assert!(TuiSurface::detect_grid(&duplicate)
            .unwrap()
            .modal_accent
            .is_none());
        let mut inverse = frame.clone();
        inverse.cells[first].flags.insert(Flags::INVERSE);
        assert!(TuiSurface::detect_grid(&inverse)
            .unwrap()
            .modal_accent
            .is_none());
        let mut clipped = frame;
        let start = first / clipped.cols * clipped.cols;
        for cell in &mut clipped.cells[start..first] {
            cell.bg = accent;
        }
        assert!(TuiSurface::detect_grid(&clipped)
            .unwrap()
            .modal_accent
            .is_none());
    }

    #[test]
    fn grid_surface_is_limited_to_a_complete_live_alternate_screen() {
        let frame = model_menu_frame();
        assert!(TuiSurface::detect_grid(&frame).is_some());
        let mut scrolled = frame.clone();
        scrolled.display_offset = 1;
        assert!(TuiSurface::detect_grid(&scrolled).is_none());
        let mut shell = frame.clone();
        shell.alternate_screen = false;
        assert!(TuiSurface::detect_grid(&shell).is_none());
        let mut broken = frame;
        broken.cells.pop();
        assert!(TuiSurface::detect_grid(&broken).is_none());
    }

    #[test]
    fn floating_menu_keeps_the_canvas_when_underlying_panels_split_its_majority() {
        let terminal = panelled_menu_model();
        let frame = terminal.live_snapshot();
        let original = frame.clone();
        let surface = TuiSurface::detect_grid(&frame)
            .expect("the menu must not restore black backgrounds over neutral message panels");
        assert_eq!(surface.base, rgb(4, 4, 4));
        assert!(surface.modal_accent.is_some());
        assert_eq!(frame, original);
    }

    #[test]
    fn a_thin_neutral_border_around_ansi_output_does_not_establish_a_canvas() {
        let mut frame = model_menu_frame();
        for (index, cell) in frame.cells.iter_mut().enumerate() {
            let (row, column) = (index / frame.cols, index % frame.cols);
            cell.bg =
                if row == 0 || row + 1 == frame.rows || column == 0 || column + 1 == frame.cols {
                    gray(4)
                } else {
                    Color::Named(NamedColor::Green)
                };
        }
        assert!(TuiSurface::detect_grid(&frame).is_none());
    }

    #[test]
    fn captured_error_context_keeps_its_text_colors_when_a_model_popup_opens() {
        let frame = error_menu_frame();
        let original = frame.clone();
        let surface =
            TuiSurface::detect_grid(&frame).expect("error context remains a styled TUI canvas");
        let (accent, _) = surface
            .modal_accent
            .expect("real model popup uses a soft accent");
        let theme = Theme::default();
        for cell in &frame.cells {
            if cell.bg != accent {
                let source = TextStyle::new(cell.fg, cell.bg, cell.flags);
                assert_eq!(
                    surface.resolve(source, &theme).foreground,
                    resolve(source, &theme).foreground
                );
            }
        }
        assert_eq!(frame, original);
    }
}
