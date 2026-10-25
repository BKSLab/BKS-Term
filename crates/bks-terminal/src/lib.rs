use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, Processor};

struct TerminalDimensions {
    columns: usize,
    screen_lines: usize,
}

impl TerminalDimensions {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for TerminalDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSnapshot {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

pub struct TerminalModel {
    term: Term<VoidListener>,
    parser: Processor,
    cols: usize,
    rows: usize,
}

impl TerminalModel {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let term = Term::new(
            Config::default(),
            &TerminalDimensions::new(cols, rows),
            VoidListener,
        );
        Self {
            term,
            parser: Processor::new(),
            cols,
            rows,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        self.term.resize(TerminalDimensions::new(cols, rows));
        self.cols = cols;
        self.rows = rows;
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn visible_cells(&self) -> Vec<CellSnapshot> {
        self.term
            .grid()
            .display_iter()
            .map(|item| CellSnapshot {
                c: item.c,
                fg: item.fg,
                bg: item.bg,
                flags: item.flags,
            })
            .collect()
    }

    pub fn visible_text(&self) -> String {
        let grid = self.term.grid();
        let mut lines = Vec::with_capacity(self.rows);
        for row in 0..self.rows {
            let mut line = String::with_capacity(self.cols);
            for column in 0..self.cols {
                line.push(grid[Line(row as i32)][Column(column)].c);
            }
            lines.push(line.trim_end_matches(' ').to_string());
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::{NamedColor, Rgb};

    fn is_red(color: Color) -> bool {
        matches!(color, Color::Named(NamedColor::Red)) || color == Color::Indexed(1)
    }

    fn is_green(color: Color) -> bool {
        matches!(color, Color::Named(NamedColor::Green)) || color == Color::Indexed(2)
    }

    #[test]
    fn plain_ascii_and_split_utf8() {
        let mut model = TerminalModel::new(20, 5);
        model.push_bytes(b"Hello");
        model.push_bytes(&[0xC3]);
        model.push_bytes(&[0xA9]);
        assert!(model.visible_text().starts_with("Helloé"));
    }

    #[test]
    fn sgr_colors_bold_reset() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"\x1b[31mR\x1b[32mG\x1b[1mB\x1b[0mN");
        let cells = model.visible_cells();
        assert!(is_red(cells[0].fg));
        assert!(is_green(cells[1].fg));
        assert!(cells[2].flags.contains(Flags::BOLD));
    }

    #[test]
    fn indexed_256_color() {
        let mut model = TerminalModel::new(20, 2);
        model.push_bytes(b"\x1b[38;5;208mX");
        assert_eq!(model.visible_cells()[0].fg, Color::Indexed(208));
    }

    #[test]
    fn truecolor_foreground_and_background() {
        let mut model = TerminalModel::new(20, 2);
        model.push_bytes(b"\x1b[38;2;17;34;51mY\x1b[48;2;10;20;30mZ");
        let cells = model.visible_cells();
        assert!(matches!(
            cells[0].fg,
            Color::Spec(Rgb {
                r: 17,
                g: 34,
                b: 51
            })
        ));
        assert!(matches!(
            cells[1].bg,
            Color::Spec(Rgb {
                r: 10,
                g: 20,
                b: 30
            })
        ));
    }

    #[test]
    fn clear_screen() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"Hello");
        assert!(!model.visible_text().trim().is_empty());
        model.push_bytes(b"\x1b[2J");
        assert!(model.visible_text().trim().is_empty());
    }

    #[test]
    fn alternate_screen() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"main");
        model.push_bytes(b"\x1b[?1049h");
        model.push_bytes(b"alt");
        assert!(model.visible_text().contains("alt"));
        assert!(!model.visible_text().contains("main"));
        model.push_bytes(b"\x1b[?1049l");
        assert!(model.visible_text().contains("main"));
        assert!(!model.visible_text().contains("alt"));
    }

    #[test]
    fn resize_preserves_text() {
        let mut model = TerminalModel::new(20, 5);
        model.push_bytes(b"hello");
        model.resize(30, 6);
        assert_eq!(model.size(), (30, 6));
        assert!(model.visible_text().starts_with("hello"));
    }

    #[test]
    fn reset_colors_after_sgr0() {
        let fresh = TerminalModel::new(10, 1);
        let default_fg = fresh.visible_cells()[0].fg;
        let mut model = TerminalModel::new(20, 2);
        model.push_bytes(b"\x1b[31mA\x1b[0mB");
        let cells = model.visible_cells();
        assert!(is_red(cells[0].fg));
        assert_eq!(cells[1].fg, default_fg);
    }
}
