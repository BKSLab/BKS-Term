mod input_modes;
mod reading_document;
mod style_tracker;
mod text_style;

pub use reading_document::{ReadingCell, ReadingDocument, ReadingLine};
pub use style_tracker::StyleTracker;
pub use text_style::{StyleSpan, TextStyle};

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode, MIN_COLUMNS, MIN_SCREEN_LINES};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};

pub use alacritty_terminal::term::cell::Flags;
pub use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

const SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Default)]
struct ReplyListener {
    responses: Arc<Mutex<Vec<u8>>>,
    default_colors: Arc<Mutex<Option<[Rgb; 3]>>>,
}

impl ReplyListener {
    fn drain(&self) -> Vec<u8> {
        let mut responses = self.responses.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *responses)
    }
}

impl EventListener for ReplyListener {
    fn send_event(&self, event: Event) {
        // Only VT protocol replies may be sent back to the PTY. In particular,
        // clipboard, title and other OSC events never perform external actions.
        let response = match event {
            Event::PtyWrite(response) => Some(response),
            Event::ColorRequest(index, format) => {
                let slot = if index == NamedColor::Foreground as usize {
                    Some(0)
                } else if index == NamedColor::Background as usize {
                    Some(1)
                } else if index == NamedColor::Cursor as usize {
                    Some(2)
                } else {
                    None
                };
                let color = slot.and_then(|slot| {
                    self.default_colors
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .as_ref()
                        .map(|colors| colors[slot])
                });
                color.map(|color| format(color))
            }
            _ => None,
        };
        if let Some(response) = response {
            let mut responses = self.responses.lock().unwrap_or_else(|err| err.into_inner());
            responses.extend_from_slice(response.as_bytes());
        }
    }
}

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
    /// Concealed glyphs are exported as spaces, including this legacy field.
    pub c: char,
    /// Base character and its combining characters; empty for wide spacers.
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

impl From<&Cell> for CellSnapshot {
    fn from(cell: &Cell) -> Self {
        let mut text = String::new();
        if !cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            if cell.flags.contains(Flags::HIDDEN) {
                // Preserve the UTF-16 span without exporting the base glyph
                // or its combining marks. Grid width remains in the flags.
                let units = cell.c.len_utf16()
                    + cell
                        .zerowidth()
                        .into_iter()
                        .flatten()
                        .map(|c| c.len_utf16())
                        .sum::<usize>();
                text.extend(std::iter::repeat_n(' ', units));
            } else {
                text.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth);
                }
            }
        }
        Self {
            c: if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            },
            text,
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    /// Zero-based row in the visible viewport.
    pub row: usize,
    /// Zero-based column, adjusted to the first cell of a wide character.
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub cols: usize,
    pub rows: usize,
    /// Exactly `rows * cols` cells in row-major order, including wide spacers.
    pub cells: Vec<CellSnapshot>,
    /// Absent when the cursor is hidden or outside the visible viewport.
    pub cursor: Option<CursorSnapshot>,
    pub display_offset: usize,
    pub history_size: usize,
    pub alternate_screen: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub win32_input: bool,
}

pub struct TerminalModel {
    term: Term<ReplyListener>,
    parser: Processor,
    replies: ReplyListener,
    cols: usize,
    rows: usize,
    reading_revision: u64,
    input_modes: input_modes::InputModeTracker,
}

impl TerminalModel {
    /// Create a terminal with at most 10,000 history rows and OSC 52 disabled.
    /// Dimensions are clamped to the VT engine's minimum of two columns, one row.
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(MIN_COLUMNS);
        let rows = rows.max(MIN_SCREEN_LINES);
        let replies = ReplyListener::default();
        let term = Term::new(
            Config {
                scrolling_history: SCROLLBACK_LINES,
                osc52: Osc52::Disabled,
                ..Config::default()
            },
            &TerminalDimensions::new(cols, rows),
            replies.clone(),
        );
        Self {
            term,
            parser: Processor::new(),
            replies,
            cols,
            rows,
            reading_revision: 0,
            input_modes: input_modes::InputModeTracker::default(),
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(&mut self.term, bytes);
        self.input_modes.push_bytes(bytes);
        self.reading_revision = self.reading_revision.wrapping_add(1);
    }

    /// Consume up to the first complete synchronized redraw, or the whole
    /// slice when there is none. The parser's actual sync state is the proof;
    /// matching a byte sequence inside OSC/DCS text is not a frame boundary.
    pub fn push_bytes_until_frame(&mut self, bytes: &[u8]) -> (usize, bool) {
        if bytes.is_empty() {
            return (0, false);
        }
        let mut start = 0;
        let mut pending = self.sync_pending();
        for (index, byte) in bytes.iter().enumerate() {
            // Both DECSET and DECRST must be observed, including when either
            // sequence was split across PTY reads. Ordinary text stays batched.
            if !matches!(byte, b'h' | b'l') {
                continue;
            }
            self.parser.advance(&mut self.term, &bytes[start..=index]);
            self.input_modes.push_bytes(&bytes[start..=index]);
            start = index + 1;
            let next = self.sync_pending();
            if pending && !next {
                self.reading_revision = self.reading_revision.wrapping_add(1);
                return (start, true);
            }
            pending = next;
        }
        if start < bytes.len() {
            self.parser.advance(&mut self.term, &bytes[start..]);
            self.input_modes.push_bytes(&bytes[start..]);
        }
        self.reading_revision = self.reading_revision.wrapping_add(1);
        (bytes.len(), false)
    }

    /// Publish the host's actual default colors for OSC 10/11/12 queries.
    /// This only answers VT protocol requests; it never changes rendered text
    /// or performs actions on behalf of terminal output.
    pub fn set_default_colors(&mut self, foreground: Rgb, background: Rgb, cursor: Rgb) {
        *self
            .replies
            .default_colors
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some([foreground, background, cursor]);
    }

    /// Publish an incomplete synchronized update only when its deadline expires.
    /// The owner must call this while polling, including when no new PTY bytes
    /// arrive. Protocol replies created by the flush remain in `drain_responses`.
    pub fn expire_sync(&mut self, now: Instant) -> bool {
        let expired = self
            .parser
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|deadline| now >= deadline);
        if !expired {
            return false;
        }
        self.parser.stop_sync(&mut self.term);
        self.reading_revision = self.reading_revision.wrapping_add(1);
        true
    }

    /// A synchronized frame is buffered and cannot yet justify screen actions.
    pub fn sync_pending(&self) -> bool {
        self.parser.sync_timeout().sync_timeout().is_some()
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(MIN_COLUMNS);
        let rows = rows.max(MIN_SCREEN_LINES);
        if (cols, rows) == self.size() {
            return;
        }
        self.term.resize(TerminalDimensions::new(cols, rows));
        self.cols = cols;
        self.rows = rows;
        self.reading_revision = self.reading_revision.wrapping_add(1);
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn visible_cells(&self) -> Vec<CellSnapshot> {
        self.term
            .grid()
            .display_iter()
            .map(|item| CellSnapshot::from(item.cell))
            .collect()
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        let grid = self.term.grid();
        let content = self.term.renderable_content();
        let cursor_row = content.cursor.point.line.0 + content.display_offset as i32;
        let cursor = (content.cursor.shape != CursorShape::Hidden
            && cursor_row >= 0
            && (cursor_row as usize) < self.rows)
            .then_some(CursorSnapshot {
                row: cursor_row as usize,
                column: content.cursor.point.column.0,
            });
        TerminalSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells: self.visible_cells(),
            cursor,
            display_offset: content.display_offset,
            history_size: grid.history_size(),
            alternate_screen: content.mode.contains(TermMode::ALT_SCREEN),
        }
    }

    /// Current application screen, independent of the user's scrollback view.
    /// Semantic actions must never be recognized from an old scrolled page.
    pub fn live_snapshot(&self) -> TerminalSnapshot {
        let grid = self.term.grid();
        let content = self.term.renderable_content();
        let cursor_row = grid.cursor.point.line.0;
        let cursor = (content.cursor.shape != CursorShape::Hidden
            && cursor_row >= 0
            && (cursor_row as usize) < self.rows)
            .then_some(CursorSnapshot {
                row: cursor_row as usize,
                column: grid.cursor.point.column.0,
            });
        let cells = (0..self.rows)
            .flat_map(|row| {
                let line = &grid[alacritty_terminal::index::Line(row as i32)];
                (0..self.cols).map(move |column| {
                    CellSnapshot::from(&line[alacritty_terminal::index::Column(column)])
                })
            })
            .collect();
        TerminalSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor,
            display_offset: 0,
            history_size: grid.history_size(),
            alternate_screen: content.mode.contains(TermMode::ALT_SCREEN),
        }
    }

    /// Visible viewport text with combining marks, without wide spacer cells.
    pub fn visible_text(&self) -> String {
        self.visible_cells()
            .chunks(self.cols)
            .map(|row| {
                let line: String = row.iter().map(|cell| cell.text.as_str()).collect();
                line.trim_end_matches(' ').to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Scroll by rows; positive values move towards older output.
    pub fn scroll_display(&mut self, delta: i32) {
        // Upstream adds the delta to its offset as i32; clamp before adding so
        // even i32::MAX/MIN requests cannot overflow.
        let history = self.term.grid().history_size() as i32;
        self.term
            .scroll_display(Scroll::Delta(delta.clamp(-history, history)));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn modes(&self) -> InputModes {
        let modes = self.term.mode();
        InputModes {
            application_cursor: modes.contains(TermMode::APP_CURSOR),
            application_keypad: modes.contains(TermMode::APP_KEYPAD),
            bracketed_paste: modes.contains(TermMode::BRACKETED_PASTE),
            win32_input: self.input_modes.win32(),
        }
    }

    /// Take protocol replies produced by the VT parser, preserving their order.
    pub fn drain_responses(&mut self) -> Vec<u8> {
        self.replies.drain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn synchronized_updates_publish_only_the_complete_frame_across_split_sequences() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"old\r");
        let original = model.live_snapshot();
        model.push_bytes(b"\x1b[?20");
        model.push_bytes(b"26h\x1b[2Knew");
        assert!(model.sync_pending());
        let deadline = model.parser.sync_timeout().sync_timeout().unwrap();
        assert_eq!(model.live_snapshot(), original);
        assert!(!model.expire_sync(deadline - Duration::from_nanos(1)));
        assert_eq!(model.live_snapshot(), original);
        model.push_bytes(b" frame\x1b[?202");
        assert_eq!(model.live_snapshot(), original);
        model.push_bytes(b"6l");
        assert!(!model.sync_pending());
        assert!(model.visible_text().starts_with("new frame"));
        let revision = model.reading_revision();
        assert!(!model.expire_sync(deadline + Duration::from_secs(1)));
        assert_eq!(model.reading_revision(), revision);
    }

    #[test]
    fn synchronized_update_timeout_publishes_without_new_bytes_and_drains_replies_once() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"old\r\x1b[?2026h\x1b[2Knew\x1b[6n");
        let deadline = model.parser.sync_timeout().sync_timeout().unwrap();
        let revision = model.reading_revision();
        assert!(model.visible_text().starts_with("old"));
        assert!(model.drain_responses().is_empty());
        assert!(model.expire_sync(deadline));
        assert!(!model.sync_pending());
        assert!(model.visible_text().starts_with("new"));
        assert_eq!(model.reading_document().text, "new");
        assert_eq!(model.reading_revision(), revision + 1);
        assert_eq!(model.drain_responses(), b"\x1b[1;4R");
        assert!(model.drain_responses().is_empty());
        assert!(!model.expire_sync(deadline + Duration::from_secs(1)));
        assert_eq!(model.reading_revision(), revision + 1);
        model.push_bytes(b" suffix");
        assert!(model.visible_text().starts_with("new suffix"));
    }

    #[test]
    fn repeated_sync_begin_extends_the_deadline_without_publishing_partial_text() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"\x1b[?2026hfirst");
        let first = model.parser.sync_timeout().sync_timeout().unwrap();
        model.push_bytes(b"\x1b[?2026hsecond");
        let extended = model.parser.sync_timeout().sync_timeout().unwrap();
        assert!(extended >= first);
        assert!(!model.expire_sync(extended - Duration::from_nanos(1)));
        assert!(model.visible_text().trim().is_empty());
        assert!(model.expire_sync(extended));
        assert!(model.visible_text().starts_with("firstsecond"));
    }

    #[test]
    fn live_screen_does_not_follow_the_readers_scrollback_view() {
        let mut terminal = TerminalModel::new(20, 3);
        terminal.push_bytes(b"old1\r\nold2\r\nold3\r\nnew4\r\nnew5");
        let live = terminal.live_snapshot();
        terminal.scroll_display(2);
        assert!(terminal.snapshot().display_offset > 0);
        assert_eq!(terminal.live_snapshot(), live);
        assert_ne!(terminal.snapshot().cells, live.cells);
        terminal.push_bytes(b"\x1b[?1049h\x1b[HALT");
        assert!(terminal.live_snapshot().alternate_screen);
        assert_eq!(terminal.live_snapshot().display_offset, 0);
    }

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
        assert_eq!(model.snapshot().cells[5].text, " ");
        model.push_bytes(&[0xA9]);
        for byte in "😀".as_bytes() {
            model.push_bytes(&[*byte]);
        }
        assert!(model.visible_text().starts_with("Helloé😀"));
        assert_eq!(model.snapshot().cells[6].text, "😀");
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
    fn concealed_snapshots_never_export_base_or_combining_characters() {
        let mut model = TerminalModel::new(40, 3);
        let secret = "Secret界🙂e\u{301}";
        model.push_bytes(format!("[\x1b[8m{secret}\x1b[28m]\x1b[8mX\x1b[0m!").as_bytes());
        for snapshot in [model.snapshot(), model.live_snapshot()] {
            let text: String = snapshot
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect();
            let legacy: String = snapshot.cells.iter().map(|cell| cell.c).collect();
            for character in secret.chars().chain(['X']) {
                assert!(!text.contains(character));
                assert!(!legacy.contains(character));
            }
            assert!(text.contains('[') && text.contains("] ") && text.contains('!'));
            let hidden: Vec<_> = snapshot
                .cells
                .iter()
                .filter(|cell| cell.flags.contains(Flags::HIDDEN))
                .collect();
            assert!(hidden
                .iter()
                .any(|cell| cell.flags.contains(Flags::WIDE_CHAR)));
            assert!(hidden
                .iter()
                .all(|cell| cell.c == ' ' && cell.text.chars().all(|c| c == ' ')));
        }
        let text = model.visible_text();
        assert!(!text.contains(secret));
        assert!(text.starts_with('[') && text.ends_with("\n\n"));
    }

    #[test]
    fn split_sgr_conceal_and_unicode_stay_masked_after_scrolling_and_reset() {
        let mut model = TerminalModel::new(20, 2);
        for byte in "\x1b[8mHidden界🙂e\u{301}\x1b[28mvisible\r\nnext\r\nlast".as_bytes() {
            model.push_bytes(&[*byte]);
        }
        model.scroll_display(1);
        let snapshot = model.snapshot();
        assert!(snapshot.display_offset > 0);
        let text = model.visible_text();
        for character in "Hidden界🙂e\u{301}".chars() {
            assert!(!snapshot
                .cells
                .iter()
                .any(|cell| cell.flags.contains(Flags::HIDDEN)
                    && (cell.c == character || cell.text.contains(character))));
        }
        assert!(text.contains("visible"));
        assert!(!text.contains("Hidden"));
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
        let main_cursor = model.snapshot().cursor;
        model.push_bytes(b"\x1b[?1049h\x1b[H");
        model.push_bytes(b"alt");
        assert!(model.snapshot().alternate_screen);
        assert_eq!(model.snapshot().history_size, 0);
        assert!(model.visible_text().contains("alt"));
        assert!(!model.visible_text().contains("main"));
        model.push_bytes(b"\x1b[?1049l");
        assert!(!model.snapshot().alternate_screen);
        assert_eq!(model.snapshot().cursor, main_cursor);
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

    #[test]
    fn wide_and_combining_cells_preserve_display_columns() {
        let mut model = TerminalModel::new(8, 3);
        model.push_bytes("Ae\u{301}界\u{301}Z".as_bytes());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.cells.len(), 8 * 3);
        assert_eq!(snapshot.cells[1].c, 'e');
        assert_eq!(snapshot.cells[1].text, "e\u{301}");
        assert_eq!(snapshot.cells[2].text, "界\u{301}");
        assert!(snapshot.cells[2].flags.contains(Flags::WIDE_CHAR));
        assert!(snapshot.cells[3].flags.contains(Flags::WIDE_CHAR_SPACER));
        assert!(snapshot.cells[3].text.is_empty());
        assert_eq!(snapshot.cells[4].text, "Z");
        assert_eq!(model.visible_text(), "Ae\u{301}界\u{301}Z\n\n");
        assert_eq!(snapshot.cursor, Some(CursorSnapshot { row: 0, column: 5 }));
    }

    #[test]
    fn wide_character_wrap_has_no_textual_placeholder() {
        let mut model = TerminalModel::new(4, 3);
        model.push_bytes("abc界x".as_bytes());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.cells.len(), 12);
        assert!(snapshot.cells[3]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER | Flags::WRAPLINE));
        assert!(snapshot.cells[3].text.is_empty());
        assert_eq!(snapshot.cells[4].text, "界");
        assert!(snapshot.cells[5].text.is_empty());
        assert_eq!(model.visible_text(), "abc\n界x\n");

        model.push_bytes(b"\x1b[2;2H");
        assert_eq!(
            model.snapshot().cursor,
            Some(CursorSnapshot { row: 1, column: 0 })
        );
    }

    #[test]
    fn cursor_visibility_moves_and_line_erase() {
        let mut model = TerminalModel::new(10, 4);
        model.push_bytes(b"abcd\r\nsecond\r\nthird\x1b[2;3H\x1b[K");
        assert_eq!(model.visible_text(), "abcd\nse\nthird\n");
        assert_eq!(
            model.snapshot().cursor,
            Some(CursorSnapshot { row: 1, column: 2 })
        );

        model.push_bytes(b"\x1b[1A\x1b[2DX");
        assert_eq!(model.visible_text(), "Xbcd\nse\nthird\n");
        assert_eq!(
            model.snapshot().cursor,
            Some(CursorSnapshot { row: 0, column: 1 })
        );
        model.push_bytes(b"\x1b[?25l");
        assert_eq!(model.snapshot().cursor, None);
        model.push_bytes(b"\x1b[?25h\x1b[2J");
        assert_eq!(model.visible_text(), "\n\n\n");
        assert_eq!(
            model.snapshot().cursor,
            Some(CursorSnapshot { row: 0, column: 1 })
        );
    }

    #[test]
    fn scrollback_view_stays_still_while_output_arrives() {
        let mut model = TerminalModel::new(8, 3);
        model.push_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        assert_eq!(model.visible_text(), "three\nfour\nfive");
        assert_eq!(model.snapshot().history_size, 2);
        assert_eq!(model.snapshot().display_offset, 0);

        model.scroll_display(1);
        assert_eq!(model.snapshot().display_offset, 1);
        assert_eq!(model.snapshot().cells.len(), 24);
        assert_eq!(model.visible_text(), "two\nthree\nfour");
        assert_eq!(model.snapshot().cursor, None);

        model.scroll_display(i32::MAX);
        assert_eq!(model.snapshot().display_offset, 2);
        assert_eq!(model.visible_text(), "one\ntwo\nthree");
        model.push_bytes(b"\r\nsix");
        assert_eq!(model.snapshot().display_offset, 3);
        assert_eq!(model.visible_text(), "one\ntwo\nthree");

        model.scroll_to_bottom();
        assert_eq!(model.snapshot().display_offset, 0);
        assert_eq!(model.visible_text(), "four\nfive\nsix");
        assert_eq!(
            model.snapshot().cursor,
            Some(CursorSnapshot { row: 2, column: 3 })
        );
        model.scroll_display(1);
        model.scroll_display(i32::MIN);
        assert_eq!(model.snapshot().display_offset, 0);
    }

    #[test]
    fn history_is_bounded_and_alternate_screen_has_no_scrollback() {
        let mut model = TerminalModel::new(2, 1);
        model.push_bytes("x\r\n".repeat(SCROLLBACK_LINES + 7).as_bytes());
        assert_eq!(model.snapshot().history_size, SCROLLBACK_LINES);
        model.scroll_display(i32::MAX);
        assert_eq!(model.snapshot().display_offset, SCROLLBACK_LINES);

        model.push_bytes(b"\x1b[?1049h");
        model.push_bytes(b"a\r\nb\r\nc");
        model.scroll_display(1);
        assert!(model.snapshot().alternate_screen);
        assert_eq!(model.snapshot().history_size, 0);
        assert_eq!(model.snapshot().display_offset, 0);
        model.push_bytes(b"\x1b[?1049l");
        assert_eq!(model.snapshot().history_size, SCROLLBACK_LINES);
    }

    #[test]
    fn resize_preserves_unicode_and_minimum_dimensions() {
        let mut model = TerminalModel::new(8, 3);
        model.push_bytes("ae\u{301}界b".as_bytes());
        model.resize(4, 3);
        assert_eq!(model.snapshot().cells.len(), 12);
        // Reflow keeps the cursor on the screen and moves the first wrapped
        // row into history; it must remain available in the scrolled viewport.
        assert_eq!(model.snapshot().history_size, 1);
        model.scroll_display(1);
        assert_eq!(model.visible_text().replace('\n', ""), "ae\u{301}界b");
        model.resize(12, 5);
        let snapshot = model.snapshot();
        assert_eq!((snapshot.cols, snapshot.rows), (12, 5));
        assert_eq!(snapshot.cells.len(), 60);
        assert!(model.visible_text().starts_with("ae\u{301}界b"));

        let mut minimum = TerminalModel::new(0, 0);
        assert_eq!(minimum.size(), (2, 1));
        minimum.push_bytes("界".as_bytes());
        minimum.resize(1, 0);
        assert_eq!(minimum.snapshot().cells.len(), 2);
        assert_eq!(minimum.visible_text(), "界");
    }

    #[test]
    fn terminal_input_modes_follow_enable_and_disable_sequences() {
        let mut model = TerminalModel::new(20, 3);
        assert_eq!(model.modes(), InputModes::default());
        model.push_bytes(b"\x1b[?1h\x1b=\x1b[?2004h");
        assert_eq!(
            model.modes(),
            InputModes {
                application_cursor: true,
                application_keypad: true,
                bracketed_paste: true,
                win32_input: false,
            }
        );
        model.push_bytes(b"\x1b[?1l\x1b>\x1b[?2004l");
        assert_eq!(model.modes(), InputModes::default());
    }

    #[test]
    fn dsr_responses_are_ordered_and_drained_once() {
        let mut model = TerminalModel::new(20, 5);
        model.push_bytes(b"abc\x1b[5n\x1b[6");
        assert_eq!(model.drain_responses(), b"\x1b[0n");
        model.push_bytes(b"n\x1b[3;7H\x1b[6n");
        assert_eq!(model.drain_responses(), b"\x1b[1;4R\x1b[3;7R");
        assert!(model.drain_responses().is_empty());
    }

    #[test]
    fn osc_clipboard_requests_have_no_effect_or_reply() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"\x1b]52;c;dGV4dA==\x07\x1b]52;c;?\x07");
        assert_eq!(model.visible_text(), "\n\n");
        assert!(model.drain_responses().is_empty());
    }

    #[test]
    fn default_color_queries_follow_the_host_palette_and_preserve_protocol_order() {
        let mut model = TerminalModel::new(20, 3);
        model.push_bytes(b"\x1b]11;?\x07");
        assert!(model.drain_responses().is_empty(), "no guessed host color");
        model.set_default_colors(
            Rgb {
                r: 209,
                g: 213,
                b: 219,
            },
            Rgb {
                r: 21,
                g: 24,
                b: 27,
            },
            Rgb {
                r: 153,
                g: 196,
                b: 255,
            },
        );
        model.push_bytes(b"abc\x1b]10;?\x07\x1b]11;?");
        assert_eq!(model.drain_responses(), b"\x1b]10;rgb:d1d1/d5d5/dbdb\x07");
        model.push_bytes(b"\x1b\\\x1b[6n\x1b]12;?\x07");
        assert_eq!(
            model.drain_responses(),
            b"\x1b]11;rgb:1515/1818/1b1b\x1b\\\x1b[1;4R\x1b]12;rgb:9999/c4c4/ffff\x07"
        );
        assert!(model.drain_responses().is_empty());
        model.set_default_colors(
            Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            Rgb { r: 0, g: 0, b: 0 },
            Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        );
        model.push_bytes(b"\x1b]11;?\x07\x1b]4;1;?\x07\x1b]52;c;?\x07");
        assert_eq!(model.drain_responses(), b"\x1b]11;rgb:0000/0000/0000\x07");
        assert_eq!(model.visible_text(), "abc\n\n");
    }

    #[test]
    fn coalesced_and_split_sync_frames_expose_every_completed_page_once() {
        let first = b"\x1b[?2026h\x1b[Hfirst\x1b[?2026l";
        let second = b"\x1b[?2026h\x1b[Hother\x1b[?2026l";
        let all = [first.as_slice(), second.as_slice(), b"!"].concat();
        let mut whole = TerminalModel::new(20, 3);
        let mut consumed = 0;
        let mut frames = Vec::new();
        while consumed < all.len() {
            let (used, complete) = whole.push_bytes_until_frame(&all[consumed..]);
            assert!(used > 0);
            consumed += used;
            if complete {
                frames.push(whole.visible_text());
            }
        }
        assert_eq!(frames, ["first\n\n", "other\n\n"]);
        let mut split = TerminalModel::new(20, 3);
        let mut individual = Vec::new();
        for byte in &all {
            let (used, complete) = split.push_bytes_until_frame(std::slice::from_ref(byte));
            assert_eq!(used, 1);
            if complete {
                individual.push(split.visible_text());
            }
        }
        assert_eq!(individual, frames);
        assert_eq!(whole.live_snapshot(), split.live_snapshot());
        let mut normal = TerminalModel::new(20, 3);
        normal.push_bytes(&all);
        assert_eq!(whole.live_snapshot(), normal.live_snapshot());
        assert_eq!(
            whole.push_bytes_until_frame(b"\x1b]0;literal h and l\x07"),
            (20, false)
        );
    }
}
