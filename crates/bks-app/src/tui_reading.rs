//! Tool-independent, read-only projection of a live terminal screen.
//! A marker or inverse attribute is a reading hint, never permission to act.

use bks_terminal::{Flags, TerminalSnapshot};

const MAX_ROWS: usize = 256;
const MAX_COLS: usize = 512;
const MAX_TEXT_CHARS: usize = 65_536;

pub struct ScreenReading {
    pub context: String,
    pub lines: Vec<String>,
    pub selected: usize,
}

pub fn inspect(frame: &TerminalSnapshot) -> ScreenReading {
    let mut context = String::new();
    let mut lines = Vec::new();
    let mut source_rows = Vec::new();
    let mut hints = Vec::new();
    let mut remaining = MAX_TEXT_CHARS;
    let mut clipped = frame.rows > MAX_ROWS || frame.cols > MAX_COLS;
    for (row, cells) in frame
        .cells
        .chunks(frame.cols.max(1))
        .take(frame.rows.min(MAX_ROWS))
        .enumerate()
    {
        let mut text = String::new();
        for character in cells
            .iter()
            .take(MAX_COLS)
            .flat_map(|cell| cell.text.chars())
        {
            if remaining == 0 {
                clipped = true;
                break;
            }
            if !character.is_control() && !is_direction_control(character) {
                text.push(character);
                remaining -= 1;
            }
        }
        let text = text.trim_end();
        if row > 0 {
            context.push_str("\r\n");
        }
        context.push_str(text);
        if !text.trim().is_empty() {
            let marker = text
                .trim_start_matches(|c: char| c.is_whitespace() || is_border(c))
                .starts_with(['>', '❯', '›', '▸', '▶', '►', '➤', '➜', '→', '❱', '»']);
            let meaningful: Vec<_> = cells
                .iter()
                .take(MAX_COLS)
                .filter(|cell| cell.text.chars().any(char::is_alphanumeric))
                .collect();
            let inverse = meaningful.len() >= 2
                && meaningful
                    .iter()
                    .all(|cell| cell.flags.contains(Flags::INVERSE));
            if marker || inverse {
                hints.push(lines.len());
            }
            source_rows.push(row);
            lines.push(format!("Строка {}: {}", row + 1, text.trim()));
        }
        if remaining == 0 {
            break;
        }
    }
    let selected = if hints.len() == 1 {
        let index = hints[0];
        lines[index] = format!("Возможное выделение. {}", lines[index]);
        index
    } else {
        frame
            .cursor
            .and_then(|cursor| source_rows.iter().position(|row| *row == cursor.row))
            .unwrap_or(0)
    };
    if lines.is_empty() {
        lines.push("Экран пуст".to_owned());
    }
    if context.trim().is_empty() {
        context = "Экран пуст".to_owned();
    }
    if clipped {
        context
            .push_str("\r\n[Обзор ограничен по размеру; полный текст доступен в журнале чтения.]");
    }
    ScreenReading {
        context,
        lines,
        selected,
    }
}

fn is_direction_control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn is_border(c: char) -> bool {
    c == '|' || ('\u{2500}'..='\u{257f}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    fn screen(text: &str) -> TerminalSnapshot {
        let mut model = TerminalModel::new(60, 8);
        model.push_bytes(text.as_bytes());
        model.live_snapshot()
    }

    #[test]
    fn unknown_tool_exposes_full_context_and_original_row_numbers() {
        let reading = inspect(&screen(
            "Question\r\n\r\n  One\r\n> Ж界e\u{301}😀\r\n  Three",
        ));
        assert!(reading.context.contains("Question\r\n\r\n  One"));
        assert_eq!(reading.selected, 2);
        assert!(reading.lines[2].contains("Строка 4: > Ж界e\u{301}😀"));
        assert!(reading.lines[2].starts_with("Возможное выделение."));
    }

    #[test]
    fn inverse_is_only_a_hint_and_multiple_markers_are_ambiguous() {
        let reading = inspect(&screen("Choose\r\nOne\r\n\x1b[7mTwo\x1b[0m\r\nThree"));
        assert_eq!(reading.selected, 2);
        assert!(reading.lines[2].starts_with("Возможное выделение."));
        let reading = inspect(&screen(
            "> instruction text\r\n> another instruction\x1b[1;1H",
        ));
        assert_eq!(reading.selected, 0);
        assert!(reading
            .lines
            .iter()
            .all(|line| !line.starts_with("Возможное")));
    }

    #[test]
    fn cursor_selects_reading_row_and_blank_screen_is_accessible() {
        let reading = inspect(&screen("first\r\nsecond\x1b[1;1H"));
        assert_eq!(reading.selected, 0);
        let reading = inspect(&screen(""));
        assert_eq!(reading.context, "Экран пуст");
        assert_eq!(reading.lines, ["Экран пуст"]);
        assert_eq!(reading.selected, 0);
    }

    #[test]
    fn projection_is_bounded_and_filters_controls_without_actions() {
        let mut frame = screen("ordinary text");
        frame.cells[0].text = "x\0\n\u{202e}y".to_owned();
        let reading = inspect(&frame);
        assert!(reading.context.starts_with("xy"));
        assert!(!reading.context.contains(['\0', '\u{202e}']));
        frame.rows = usize::MAX;
        frame.cols = 0;
        let reading = inspect(&frame);
        assert!(reading.lines.len() <= MAX_ROWS);
        assert!(reading.selected < reading.lines.len());
        assert!(reading.context.contains("ограничен"));
    }
}
