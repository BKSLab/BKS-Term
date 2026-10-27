use alacritty_terminal::grid::{Dimensions, Row, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::TermMode;

use crate::{Flags, TerminalModel};

/// Text of the active screen and all its retained history, independent of the
/// visual viewport. Offsets are UTF-16 code units for native Windows controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingDocument {
    pub revision: u64,
    pub alternate_screen: bool,
    pub text: String,
    pub cells: Vec<ReadingCell>,
    pub lines: Vec<ReadingLine>,
}

/// One glyph, including combining characters, and its position in the VT grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingCell {
    pub start: u32,
    pub end: u32,
    /// Grid line: negative in history, zero at the top of the active screen.
    pub line: i32,
    pub column: usize,
    /// One or two terminal columns; wide spacers have no separate text entry.
    pub width: usize,
}

/// A logical line, which may span several soft-wrapped physical grid lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingLine {
    pub start: u32,
    /// Excludes the CRLF separating this line from the next logical line.
    pub end: u32,
    /// First physical grid line of this logical line, including empty lines.
    pub line: i32,
}

impl ReadingDocument {
    /// Return the glyph containing this UTF-16 offset, including the second
    /// half of a surrogate pair or a combining mark. CRLF and EOF have no glyph.
    pub fn cell_at(&self, offset: u32) -> Option<&ReadingCell> {
        let index = self.cells.partition_point(|cell| cell.end <= offset);
        self.cells.get(index).filter(|cell| cell.start <= offset)
    }
}

impl TerminalModel {
    /// Cheap cache key; visual scrolling does not change the reading document.
    pub fn reading_revision(&self) -> u64 {
        self.reading_revision
    }

    pub fn reading_document(&self) -> ReadingDocument {
        let grid = self.term.grid();
        let first_line = grid.topmost_line().0;
        // Preserve explicit blank lines up to the cursor, and all visible
        // content below it, while dropping unused bottom viewport padding.
        let last_content = (0..self.rows)
            .rev()
            .find(|&line| content_columns(&grid[Line(line as i32)], self.cols) > 0)
            .unwrap_or(0) as i32;
        let last_line = last_content.max(grid.cursor.point.line.0);
        let mut document = ReadingDocument {
            revision: self.reading_revision,
            alternate_screen: self.term.mode().contains(TermMode::ALT_SCREEN),
            text: String::new(),
            cells: Vec::new(),
            lines: Vec::new(),
        };
        let mut offset = 0u32;
        let mut logical_start = 0u32;
        let mut logical_line = first_line;
        for line in first_line..=last_line {
            let row = &grid[Line(line)];
            let wrapped = row[Column(self.cols - 1)].flags.contains(Flags::WRAPLINE);
            for column in 0..content_columns(row, self.cols) {
                let cell = &row[Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let start = offset;
                // A stored tab occupies blank screen cells. Export their
                // spaces so the native document does not expand the tab again.
                if cell.flags.contains(Flags::HIDDEN) {
                    // Keep the existing UTF-16 ranges and grid width while
                    // excluding concealed base and combining characters from
                    // every reading, search, copy, and speech consumer.
                    let units = cell.c.len_utf16()
                        + cell
                            .zerowidth()
                            .into_iter()
                            .flatten()
                            .map(|c| c.len_utf16())
                            .sum::<usize>();
                    document.text.extend(std::iter::repeat_n(' ', units));
                    offset += units as u32;
                } else {
                    let character = if cell.c == '\t' { ' ' } else { cell.c };
                    document.text.push(character);
                    offset += character.len_utf16() as u32;
                    for character in cell.zerowidth().into_iter().flatten() {
                        document.text.push(*character);
                        offset += character.len_utf16() as u32;
                    }
                }
                document.cells.push(ReadingCell {
                    start,
                    end: offset,
                    line,
                    column,
                    width: if cell.flags.contains(Flags::WIDE_CHAR) {
                        2
                    } else {
                        1
                    },
                });
            }
            if !wrapped || line == last_line {
                document.lines.push(ReadingLine {
                    start: logical_start,
                    end: offset,
                    line: logical_line,
                });
                if line != last_line {
                    document.text.push_str("\r\n");
                    offset += 2;
                    logical_start = offset;
                    logical_line = line + 1;
                }
            }
        }
        document
    }

    /// Reveal the glyph (or logical empty line) at a document offset without
    /// moving the PTY cursor. Valid EOF is accepted; out-of-range offsets fail.
    pub fn scroll_to_reading_offset(&mut self, offset: u32) -> bool {
        let document = self.reading_document();
        let Some(last) = document.lines.last() else {
            return false;
        };
        if offset > last.end {
            return false;
        }
        let line = document
            .cell_at(offset)
            .map(|cell| cell.line)
            .unwrap_or_else(|| {
                let index = document.lines.partition_point(|line| line.start <= offset);
                let logical_line = &document.lines[index.saturating_sub(1)];
                // CRLF/EOF after a wrapped logical line belongs to its final
                // physical row; an empty line has its own explicit grid anchor.
                let previous = document.cells.partition_point(|cell| cell.end <= offset);
                previous
                    .checked_sub(1)
                    .and_then(|index| document.cells.get(index))
                    .filter(|cell| cell.start >= logical_line.start)
                    .map_or(logical_line.line, |cell| cell.line)
            });
        let grid = self.term.grid();
        let current = grid.display_offset() as i32;
        let top = -current;
        let bottom = top + self.rows as i32 - 1;
        let target = if line < top {
            -line
        } else if line > bottom {
            self.rows as i32 - 1 - line
        } else {
            current
        };
        self.term.scroll_display(Scroll::Delta(target - current));
        true
    }
}

fn content_columns(row: &Row<Cell>, columns: usize) -> usize {
    if row[Column(columns - 1)].flags.contains(Flags::WRAPLINE) {
        return columns;
    }
    row[..]
        .iter()
        .rposition(|cell| {
            !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                && (!matches!(cell.c, ' ' | '\t')
                    || cell.zerowidth().is_some_and(|marks| !marks.is_empty()))
        })
        .map_or(0, |column| column + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concealed_reading_copy_and_search_source_preserve_utf16_and_grid_ranges() {
        let secret = "Secret界🙂e\u{301}";
        let mut model = TerminalModel::new(64, 3);
        model.push_bytes(format!("[\x1b[8m{secret}\x1b[28m]visible").as_bytes());
        let document = model.reading_document();
        let units = secret.encode_utf16().count();
        assert_eq!(document.text, format!("[{}]visible", " ".repeat(units)));
        for character in secret.chars() {
            assert!(!document.text.contains(character) || "]visible".contains(character));
        }
        assert!(!document.text.contains(secret));
        let after_secret = units as u32 + 1;
        assert_eq!(
            document.cell_at(after_secret).unwrap().column,
            1 + 6 + 2 + 2 + 1
        );
        let wide: Vec<_> = document
            .cells
            .iter()
            .filter(|cell| cell.width == 2)
            .collect();
        assert_eq!(wide.len(), 2);
        assert_eq!(wide[0].end - wide[0].start, 1);
        assert_eq!(wide[1].end - wide[1].start, 2);
        let copied: Vec<u16> = document.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&copied[1..after_secret as usize]).unwrap(),
            " ".repeat(units)
        );
        for cell in &document.cells {
            assert!(String::from_utf16(&copied[..cell.start as usize]).is_ok());
            assert!(String::from_utf16(&copied[cell.start as usize..cell.end as usize]).is_ok());
        }
    }

    #[test]
    fn concealed_history_and_alternate_screen_stay_masked_after_reflow() {
        let mut model = TerminalModel::new(8, 2);
        for byte in "\x1b[8mSecret界🙂e\u{301}\x1b[28m\r\nvisible\r\nlast".as_bytes() {
            model.push_bytes(&[*byte]);
        }
        model.resize(16, 2);
        let main = model.reading_document();
        assert!(!main.text.contains("Secret"));
        assert!(!main.text.contains(['界', '🙂', '\u{301}']));
        assert!(main.text.contains("visible"));
        model.push_bytes("\x1b[?1049h\x1b[8mPrivate界🙂\x1b[0mshown".as_bytes());
        let alternate = model.reading_document();
        assert!(alternate.alternate_screen);
        assert!(!alternate.text.contains("Private"));
        assert!(!alternate.text.contains(['界', '🙂']));
        assert!(alternate.text.contains("shown"));
        model.push_bytes(b"\x1b[?1049l");
        assert_eq!(model.reading_document().text, main.text);
    }

    #[test]
    fn exports_history_without_moving_or_depending_on_viewport() {
        let mut terminal = TerminalModel::new(8, 3);
        terminal.push_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        let bottom = terminal.reading_document();
        assert_eq!(bottom.text, "one\r\ntwo\r\nthree\r\nfour\r\nfive");
        assert_eq!(bottom.cells.first().unwrap().line, -2);
        assert_eq!(bottom.cells.last().unwrap().line, 2);
        assert_eq!(bottom.lines.len(), 5);
        terminal.scroll_display(1);
        let before = terminal.snapshot();
        assert_eq!(terminal.reading_document(), bottom);
        assert_eq!(terminal.snapshot(), before);
    }

    #[test]
    fn overwrite_and_clear_follow_real_grid_and_history() {
        let mut terminal = TerminalModel::new(16, 3);
        terminal.push_bytes(b"original\r\nsecond");
        terminal.push_bytes(b"\x1b[Hnew\x1b[K");
        assert_eq!(terminal.reading_document().text, "new\r\nsecond");
        terminal.push_bytes(b"\x1b[2J\x1b[H");
        assert!(terminal.visible_text().trim().is_empty());
        let cleared_screen = terminal.reading_document();
        assert!(cleared_screen.text.starts_with("new\r\nsecond"));
        assert!(cleared_screen.cells.iter().all(|cell| cell.line < 0));
        // Main-screen ED 2 saves the old viewport to history; ED 3 really
        // removes it. The reading document must expose exactly that behavior.
        terminal.push_bytes(b"\x1b[3J");
        let cleared_history = terminal.reading_document();
        assert_eq!(cleared_history.text, "");
        assert!(cleared_history.cells.is_empty());
        assert_eq!(
            cleared_history.lines,
            vec![ReadingLine {
                start: 0,
                end: 0,
                line: 0
            }]
        );
    }

    #[test]
    fn alternate_document_is_isolated_and_main_restores() {
        let mut terminal = TerminalModel::new(12, 2);
        terminal.push_bytes(b"history\r\nmain1\r\nmain2");
        terminal.scroll_display(1);
        let primary = terminal.reading_document();
        terminal.push_bytes(b"\x1b[?1049h\x1b[Halt");
        let alternate = terminal.reading_document();
        assert!(alternate.alternate_screen);
        assert_eq!(alternate.text, "alt");
        assert!(alternate.cells.iter().all(|cell| cell.line >= 0));
        terminal.push_bytes(b"\x1b[2J\x1b[H");
        assert_eq!(terminal.reading_document().text, "");
        terminal.push_bytes(b"\x1b[?1049l");
        let restored = terminal.reading_document();
        assert!(!restored.alternate_screen);
        assert_eq!(restored.text, primary.text);
        assert_eq!(restored.cells, primary.cells);
        assert_eq!(restored.lines, primary.lines);
        assert_eq!(terminal.snapshot().display_offset, 1);
    }

    #[test]
    fn unicode_glyph_ranges_include_combining_and_surrogates() {
        let mut terminal = TerminalModel::new(4, 3);
        terminal.push_bytes("ae\u{301}界\u{301}b🙂".as_bytes());
        let document = terminal.reading_document();
        assert_eq!(document.text, "ae\u{301}界\u{301}b🙂");
        assert_eq!(document.cells.len(), 5);
        assert_eq!(
            document.cells[1],
            ReadingCell {
                start: 1,
                end: 3,
                line: 0,
                column: 1,
                width: 1
            }
        );
        assert_eq!(
            document.cells[2],
            ReadingCell {
                start: 3,
                end: 5,
                line: 0,
                column: 2,
                width: 2
            }
        );
        assert_eq!(
            document.cells[4],
            ReadingCell {
                start: 6,
                end: 8,
                line: 1,
                column: 1,
                width: 2
            }
        );
        assert_eq!(document.cell_at(2), Some(&document.cells[1]));
        assert_eq!(document.cell_at(7), Some(&document.cells[4]));
        assert_eq!(document.cell_at(8), None);
        assert_eq!(
            document.lines,
            vec![ReadingLine {
                start: 0,
                end: 8,
                line: 0
            }]
        );

        let mut leading_spacer = TerminalModel::new(4, 3);
        leading_spacer.push_bytes("abc界\u{301}x".as_bytes());
        let document = leading_spacer.reading_document();
        assert_eq!(document.text, "abc界\u{301}x");
        assert_eq!(document.cells.len(), 5);
        assert_eq!(document.cells[3].line, 1);
        assert_eq!(document.cells[3].column, 0);
    }

    #[test]
    fn hard_breaks_trim_padding_keep_blank_lines_and_expand_screen_tabs() {
        let mut terminal = TerminalModel::new(16, 6);
        terminal.push_bytes(b"one   \r\n\r\n\tX  ");
        let document = terminal.reading_document();
        assert_eq!(document.text, "one\r\n\r\n        X");
        assert_eq!(
            document.lines,
            vec![
                ReadingLine {
                    start: 0,
                    end: 3,
                    line: 0
                },
                ReadingLine {
                    start: 5,
                    end: 5,
                    line: 1
                },
                ReadingLine {
                    start: 7,
                    end: 16,
                    line: 2
                },
            ]
        );
        assert_eq!(document.cell_at(3), None);
        assert_eq!(document.cell_at(4), None);
        assert_eq!(document.cell_at(5), None);
        assert_eq!(document.cells.last().unwrap().column, 8);
        assert!(terminal.scroll_to_reading_offset(5));
    }

    #[test]
    fn logical_text_survives_soft_wrap_reflow_and_resize() {
        let mut terminal = TerminalModel::new(4, 3);
        terminal.push_bytes(b"abcdefghij\r\nend");
        let logical = terminal.reading_document().text;
        assert_eq!(logical, "abcdefghij\r\nend");
        for (columns, rows) in [(8, 3), (3, 3), (12, 5)] {
            terminal.resize(columns, rows);
            assert_eq!(terminal.reading_document().text, logical);
            assert_eq!(terminal.reading_document().lines.len(), 2);
        }
    }

    #[test]
    fn reading_scroll_reveals_glyph_or_eof_without_changing_document_revision() {
        let mut terminal = TerminalModel::new(4, 2);
        terminal.push_bytes(b"abcdefghijklmnopqrst");
        let document = terminal.reading_document();
        assert_eq!(document.text, "abcdefghijklmnopqrst");
        assert!(terminal.scroll_to_reading_offset(0));
        assert_eq!(terminal.snapshot().display_offset, 3);
        assert_eq!(terminal.reading_document(), document);
        assert!(terminal.scroll_to_reading_offset(20));
        assert_eq!(terminal.snapshot().display_offset, 0);
        assert_eq!(terminal.reading_revision(), document.revision);
        assert!(!terminal.scroll_to_reading_offset(21));
        assert_eq!(terminal.snapshot().display_offset, 0);
    }

    #[test]
    fn revision_changes_only_for_input_and_actual_resize() {
        let mut terminal = TerminalModel::new(8, 3);
        assert_eq!(terminal.reading_revision(), 0);
        terminal.push_bytes(&[]);
        assert_eq!(terminal.reading_revision(), 0);
        terminal.push_bytes(b"text");
        let revision = terminal.reading_revision();
        assert_eq!(revision, 1);
        terminal.scroll_display(1);
        terminal.scroll_to_bottom();
        terminal.resize(8, 3);
        assert_eq!(terminal.reading_revision(), revision);
        terminal.resize(12, 3);
        assert_eq!(terminal.reading_revision(), revision + 1);
        assert_eq!(terminal.reading_document().revision, revision + 1);
    }

    #[test]
    fn history_export_obeys_engine_truncation() {
        let mut terminal = TerminalModel::new(16, 2);
        terminal.push_bytes(b"old-marker\r\n");
        for index in 0..crate::SCROLLBACK_LINES + 5 {
            terminal.push_bytes(format!("line{index:05}\r\n").as_bytes());
        }
        let document = terminal.reading_document();
        assert!(!document.text.contains("old-marker"));
        assert!(document.text.contains("line10004"));
        assert_eq!(terminal.snapshot().history_size, crate::SCROLLBACK_LINES);
        assert_eq!(document.lines.len(), crate::SCROLLBACK_LINES + 2);
        assert_eq!(
            document.lines.first().unwrap().line,
            -(crate::SCROLLBACK_LINES as i32)
        );
        assert_eq!(document.lines.last().unwrap().line, 1);
        assert!(document.text.ends_with("\r\n"));
        assert!(terminal.scroll_to_reading_offset(0));
        assert_eq!(terminal.snapshot().display_offset, crate::SCROLLBACK_LINES);
    }
}
