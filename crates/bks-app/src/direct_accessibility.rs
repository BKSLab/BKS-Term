//! Pure caret mapping and short feedback for explicitly requested VT navigation.
//! This module produces text only; it never recognizes or executes actions.

use std::time::{Duration, Instant};

use bks_terminal::{CellSnapshot, Color, Flags, NamedColor, ReadingDocument, TerminalSnapshot};

const STABLE_FOR: Duration = Duration::from_millis(90);
const MAX_AGE: Duration = Duration::from_millis(800);
const MAX_SCREEN_ROWS: usize = 256;
const MAX_ROW_CHARS: usize = 1_024;
const MAX_EVENT_CHARS: usize = 300;
const MAX_FALLBACK_ROW_CHARS: usize = 140;

/// Text reading may also follow a single inverse cell used as a software
/// cursor, or a recently confirmed visible insertion. Neither hint is an
/// actionable UI element, and ambiguous highlights provide no cursor hint.
pub fn reading_cursor(
    frame: &TerminalSnapshot,
    confirmed: Option<(usize, usize)>,
) -> Option<bks_terminal::CursorSnapshot> {
    if frame.cursor.is_some() {
        return frame.cursor;
    }
    let mut inverse = frame.cells.iter().enumerate().filter(|(_, cell)| {
        cell.flags.contains(Flags::INVERSE)
            && !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    });
    let first = inverse.next();
    if let Some((index, _)) = first.filter(|_| inverse.next().is_none()) {
        if frame.cols > 0 && index / frame.cols < frame.rows {
            return Some(bks_terminal::CursorSnapshot {
                row: index / frame.cols,
                column: index % frame.cols,
            });
        }
    }
    confirmed
        .filter(|(row, column)| *row < frame.rows && *column <= frame.cols)
        .map(|(row, column)| bks_terminal::CursorSnapshot { row, column })
}

/// Read an entire logical row, including a soft-wrapped input, using native
/// UTF-16 offsets. Explicit reading never forwards terminal input.
pub fn current_line(document: &ReadingDocument, offset: u32) -> Option<String> {
    let index = document
        .lines
        .iter()
        .position(|line| line.start <= offset && offset <= line.end)?;
    let mut characters = document.text.split("\r\n").nth(index)?.trim().chars();
    let mut text: String = characters.by_ref().take(4_096).collect();
    if characters.next().is_some() {
        text.push_str("… Полный текст — в документе чтения.");
    }
    Some(text)
}

pub fn adjacent_line(document: &ReadingDocument, offset: u32, previous: bool) -> u32 {
    let index = document
        .lines
        .partition_point(|line| line.start <= offset)
        .saturating_sub(1);
    let index = if previous {
        index.saturating_sub(1)
    } else {
        index.saturating_add(1)
    };
    document
        .lines
        .get(index)
        .or_else(|| document.lines.last())
        .map_or(0, |line| line.start)
}

/// Keep punctuation audible through the notification speech channel.
pub fn typing_message(text: &str) -> String {
    if text.chars().count() != 1 {
        return if text.chars().all(|character| character == ' ') {
            "пробел".to_string()
        } else {
            text.to_string()
        };
    }
    let label = match text {
        " " => "пробел",
        "." => "точка",
        "," => "запятая",
        "/" => "слэш",
        "\\" => "обратный слэш",
        "?" => "вопросительный знак",
        "!" => "восклицательный знак",
        ":" => "двоеточие",
        ";" => "точка с запятой",
        "-" => "дефис",
        "_" => "подчёркивание",
        "+" => "плюс",
        "=" => "равно",
        "(" => "открывающая круглая скобка",
        ")" => "закрывающая круглая скобка",
        "[" => "открывающая квадратная скобка",
        "]" => "закрывающая квадратная скобка",
        "{" => "открывающая фигурная скобка",
        "}" => "закрывающая фигурная скобка",
        "<" => "меньше",
        ">" => "больше",
        "|" => "вертикальная черта",
        "#" => "решётка",
        "@" => "собака",
        "$" => "доллар",
        "%" => "процент",
        "^" => "крышка",
        "&" => "амперсанд",
        "*" => "звёздочка",
        "\"" => "двойная кавычка",
        "'" => "апостроф",
        "`" => "обратная кавычка",
        "~" => "тильда",
        _ => text,
    };
    label.to_string()
}

/// Map a VT cursor to a UTF-16 insertion point in the reading document.
/// Wide spacers map to their glyph's start, combining marks stay with their
/// base, and trimmed right padding maps to the physical row's text end.
/// A hidden cursor has no position in a snapshot and falls back to document EOF.
pub fn cursor_offset(document: &ReadingDocument, frame: &TerminalSnapshot) -> u32 {
    let end = document.lines.last().map_or(0, |line| line.end);
    let Some(cursor) = frame.cursor else {
        return end;
    };
    if document.alternate_screen != frame.alternate_screen {
        return end;
    }
    let Ok(row) = i32::try_from(cursor.row) else {
        return end;
    };
    let Ok(display_offset) = i32::try_from(frame.display_offset) else {
        return end;
    };
    let row = row - display_offset;
    let first = document.cells.partition_point(|cell| cell.line < row);
    let mut previous_end = None;
    for cell in document.cells[first..]
        .iter()
        .take_while(|cell| cell.line == row)
    {
        if cursor.column < cell.column.saturating_add(cell.width) {
            return cell.start;
        }
        previous_end = Some(cell.end);
    }
    if let Some(offset) = previous_end {
        return offset;
    }

    // Empty hard lines retain a grid anchor even though they have no cells.
    // The final empty physical row of a soft wrap belongs to its logical end.
    let logical = document.lines.partition_point(|line| line.line <= row);
    logical
        .checked_sub(1)
        .and_then(|index| document.lines.get(index))
        .map_or(0, |line| line.end)
}

/// One pending navigation response. Repeated keys replace the previous request
/// instead of building a speech backlog. Call `cancel` on tab/mode/focus changes.
#[derive(Default)]
pub struct InputFeedback {
    pending: Option<Navigation>,
}

struct Navigation {
    before: Screen,
    started: Instant,
    candidate: Option<Candidate>,
}

struct Candidate {
    reply: Reply,
    stable_since: Instant,
}

#[derive(PartialEq, Eq)]
struct Reply {
    text: String,
    row: Option<usize>,
}

enum MarkerTransfer {
    None,
    Selected(Reply),
    Ambiguous,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    foreground: Color,
    background: Color,
    inverse: bool,
}

struct Screen {
    rows: Vec<String>,
    styles: Vec<Vec<CellStyle>>,
    cursor: Option<usize>,
    cols: usize,
    height: usize,
    alternate: bool,
}

impl InputFeedback {
    /// Arm feedback immediately before forwarding an explicit navigation key.
    /// Both this frame and subsequent observations must be live VT snapshots.
    pub fn begin(&mut self, frame: &TerminalSnapshot, now: Instant) {
        self.cancel();
        if frame.display_offset != 0 {
            return;
        }
        self.pending = Some(Navigation {
            before: Screen::new(frame),
            started: now,
            candidate: None,
        });
    }

    /// Return at most one reply after its target and text remain stable for 90 ms.
    /// The entire request expires after 800 ms, including incomplete redraws.
    /// Reciprocal highlight transfers and visible cursor row moves also qualify;
    /// incidental colors, borders, spinners and cursor visibility alone do not.
    pub fn observe(&mut self, frame: &TerminalSnapshot, now: Instant) -> Option<String> {
        let pending = self.pending.as_mut()?;
        if now.saturating_duration_since(pending.started) >= MAX_AGE
            || frame.display_offset != 0
            || frame.alternate_screen != pending.before.alternate
            || frame.cols != pending.before.cols
            || frame.rows != pending.before.height
        {
            self.cancel();
            return None;
        }

        let current = Screen::new(frame);
        let Some(reply) = feedback(&pending.before, &current) else {
            pending.candidate = None;
            return None;
        };
        if let Some(candidate) = pending
            .candidate
            .as_ref()
            .filter(|candidate| candidate.reply == reply)
        {
            if now.saturating_duration_since(candidate.stable_since) >= STABLE_FOR {
                self.cancel();
                return Some(clipped(&reply.text, MAX_EVENT_CHARS));
            }
            return None;
        }
        pending.candidate = Some(Candidate {
            reply,
            stable_since: now,
        });
        None
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

impl Screen {
    fn new(frame: &TerminalSnapshot) -> Self {
        let (rows, styles): (Vec<_>, Vec<_>) = frame
            .cells
            .chunks(frame.cols.max(1))
            .take(frame.rows.min(MAX_SCREEN_ROWS))
            .map(screen_row)
            .unzip();
        let cursor = frame
            .cursor
            .filter(|cursor| cursor.row < rows.len() && cursor.column < frame.cols)
            .map(|cursor| cursor.row);
        Self {
            rows,
            styles,
            cursor,
            cols: frame.cols,
            height: frame.rows,
            alternate: frame.alternate_screen,
        }
    }
}

fn screen_row(cells: &[CellSnapshot]) -> (String, Vec<CellStyle>) {
    let mut text = String::new();
    let mut styles = Vec::new();
    let mut count = 0;
    for cell in cells {
        if count == MAX_ROW_CHARS {
            break;
        }
        let mut meaningful = false;
        let concealed = cell.flags.contains(Flags::HIDDEN);
        for character in cell
            .text
            .chars()
            .map(|character| {
                if concealed || character == '\t' {
                    ' '
                } else {
                    character
                }
            })
            .filter(|character| !character.is_control())
            .take(MAX_ROW_CHARS - count)
        {
            text.push(character);
            count += 1;
            meaningful |= character.is_alphanumeric();
        }
        // Padding, borders, wide spacers and spinner glyphs cannot establish a
        // selection. Each retained style belongs to a substantive text cell.
        if meaningful {
            styles.push(CellStyle {
                foreground: cell.fg,
                background: cell.bg,
                inverse: cell.flags.contains(Flags::INVERSE),
            });
        }
    }
    let text = text
        .trim_matches(|character: char| character.is_whitespace() || is_border(character))
        .to_owned();
    (text, styles)
}

fn feedback(before: &Screen, after: &Screen) -> Option<Reply> {
    match marker_transfer(&before.rows, &after.rows) {
        MarkerTransfer::Selected(reply) => return Some(reply),
        MarkerTransfer::Ambiguous => return None,
        MarkerTransfer::None => {}
    }
    if let Some(reply) = feedback_text(&before.rows, &after.rows, after.cursor) {
        return Some(reply);
    }
    if before.rows.len() != after.rows.len()
        || !before
            .rows
            .iter()
            .zip(&after.rows)
            .all(|(before, after)| same_row_text(before, after))
    {
        return None;
    }
    if let Some(row) = selection_row(before, after) {
        return Some(Reply {
            text: after.rows[row].clone(),
            row: Some(row),
        });
    }
    // A hidden cursor becoming visible is not evidence of movement. Cursor
    // feedback also requires unchanged text/style, so an ambiguous repaint
    // cannot acquire meaning merely because its cursor was repositioned.
    if before.styles != after.styles {
        return None;
    }
    let (Some(previous), Some(row)) = (before.cursor, after.cursor) else {
        return None;
    };
    (previous != row && !after.styles[row].is_empty()).then(|| Reply {
        text: after.rows[row].clone(),
        row: Some(row),
    })
}

/// The same prefix leaving one stable label and appearing on another is
/// selection evidence even when the labels contain several different styles.
/// Unknown glyphs qualify only through this reciprocal transfer, never merely
/// because a line starts with punctuation.
fn marker_transfer(before: &[String], after: &[String]) -> MarkerTransfer {
    let mut selected = None;
    for (row, text) in after.iter().enumerate() {
        let Some((marker, label)) = selector_prefix(text) else {
            continue;
        };
        if !before
            .get(row)
            .is_some_and(|previous| same_row_text(previous, label))
        {
            continue;
        }
        let lost = before
            .iter()
            .enumerate()
            .filter(|(previous_row, text)| {
                *previous_row != row
                    && selector_prefix(text).is_some_and(|(previous_marker, previous_label)| {
                        previous_marker == marker
                            && after
                                .get(*previous_row)
                                .is_some_and(|current| same_row_text(current, previous_label))
                    })
            })
            .count();
        if lost == 0 {
            continue;
        }
        // Only actual losses/gains compete; an unchanged prompt or bullet
        // using the same glyph elsewhere is not another transfer.
        if lost != 1 || selected.is_some() {
            return MarkerTransfer::Ambiguous;
        }
        selected = Some(Reply {
            text: label.to_owned(),
            row: Some(row),
        });
    }
    selected.map_or(MarkerTransfer::None, MarkerTransfer::Selected)
}

fn selector_prefix(text: &str) -> Option<(&str, &str)> {
    let first = text.chars().next()?;
    if first.is_alphanumeric()
        || first.is_whitespace()
        || first.is_control()
        || is_border(first)
        || is_spinner(first)
    {
        return None;
    }
    // A presentation selector belongs to the glyph, rather than to its label.
    let rest = text[first.len_utf8()..]
        .strip_prefix('\u{fe0f}')
        .unwrap_or(&text[first.len_utf8()..]);
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = rest.trim_start();
    label
        .chars()
        .any(char::is_alphanumeric)
        .then_some((&text[..text.len() - rest.len()], label))
}

fn selection_row(before: &Screen, after: &Screen) -> Option<usize> {
    let changed: Vec<_> = before
        .styles
        .iter()
        .zip(&after.styles)
        .enumerate()
        .filter(|(_, (before, after))| before != after)
        .map(|(row, _)| row)
        .take(3)
        .collect();
    let [first, second] = changed.as_slice() else {
        return None;
    };
    // There must be both a loss and a gain on unchanged substantive rows.
    // Several changing rows are a theme/redraw, not an inferred selection.
    let choices = [
        reciprocal_row(before, after, *first, *second, |style| style.inverse, false),
        reciprocal_row(
            before,
            after,
            *first,
            *second,
            |style| style.background,
            Color::Named(NamedColor::Background),
        ),
        reciprocal_row(
            before,
            after,
            *first,
            *second,
            |style| style.foreground,
            Color::Named(NamedColor::Foreground),
        ),
    ];
    let row = choices.iter().copied().flatten().next()?;
    choices
        .iter()
        .flatten()
        .all(|candidate| *candidate == row)
        .then_some(row)
}

fn changed_style<T: Copy + Eq>(
    before: &[CellStyle],
    after: &[CellStyle],
    attribute: fn(&CellStyle) -> T,
) -> Option<(T, T)> {
    if before.len() != after.len() {
        return None;
    }
    let mut changes = before
        .iter()
        .zip(after)
        .map(|(before, after)| (attribute(before), attribute(after)))
        .filter(|(before, after)| before != after);
    let pair = changes.next()?;
    // One changing cell may be a software caret or incidental decoration.
    // All changing substantive cells must exchange the same two values, but
    // an unchanged provider, description or other styled suffix may remain.
    (changes.next() == Some(pair) && changes.all(|change| change == pair)).then_some(pair)
}

fn supports_normal_style<T: Copy + Eq>(
    styles: &[CellStyle],
    attribute: fn(&CellStyle) -> T,
    normal: T,
    selected: T,
) -> bool {
    styles.iter().all(|style| attribute(style) != selected)
        && styles
            .iter()
            .filter(|style| attribute(style) == normal)
            .take(2)
            .count()
            == 2
}

fn reciprocal_row<T: Copy + Eq>(
    before: &Screen,
    after: &Screen,
    first: usize,
    second: usize,
    attribute: fn(&CellStyle) -> T,
    default: T,
) -> Option<usize> {
    let (old, new) = changed_style(&before.styles[first], &after.styles[first], attribute)?;
    if changed_style(&before.styles[second], &after.styles[second], attribute)? != (new, old) {
        return None;
    }
    // Unchanged neighboring rows establish the normal style on custom themes.
    // Without that evidence only a default/background or inverse distinction
    // can disambiguate which side of a reciprocal exchange gained a highlight.
    let mut old_neighbors = 0;
    let mut new_neighbors = 0;
    for (row, styles) in before.styles.iter().enumerate() {
        if row != first && row != second {
            old_neighbors += usize::from(supports_normal_style(styles, attribute, old, new));
            new_neighbors += usize::from(supports_normal_style(styles, attribute, new, old));
        }
    }
    let normal = if old == default || new == default {
        default
    } else {
        match old_neighbors.cmp(&new_neighbors) {
            std::cmp::Ordering::Greater => old,
            std::cmp::Ordering::Less => new,
            std::cmp::Ordering::Equal => return None,
        }
    };
    let row = if old == normal { first } else { second };
    let selected = if old == normal { new } else { old };
    // Multiple highlighted rows are ambiguous even if just one of them moves.
    if [&before.styles, &after.styles].iter().any(|rows| {
        rows.iter()
            .filter(|styles| styles.iter().any(|style| attribute(style) == selected))
            .count()
            != 1
    }) {
        return None;
    }
    Some(row)
}

fn feedback_text(before: &[String], after: &[String], cursor: Option<usize>) -> Option<Reply> {
    let changed: Vec<_> = after
        .iter()
        .enumerate()
        .filter(|(index, text)| {
            before
                .get(*index)
                .is_none_or(|before| !same_row_text(before, text))
                && !before.iter().any(|before| same_row_text(before, text))
                && text.chars().any(char::is_alphanumeric)
        })
        .collect();

    // A newly marked option is more useful than the old option losing its
    // marker, or unrelated status/help text redrawn alongside the selection.
    // Without a proven transfer the prefix remains part of the spoken text.
    let mut selected = changed.iter().filter_map(|(row, text)| {
        let (marker, _) = selector_prefix(text)?;
        marker
            .chars()
            .next()
            .is_some_and(is_selection_marker)
            .then_some((*row, *text))
    });
    if let Some((row, text)) = selected.next() {
        if selected.next().is_some() {
            return None;
        }
        return Some(Reply {
            text: text.to_owned(),
            row: Some(row),
        });
    }
    if let Some((row, text)) = changed.iter().find(|(row, _)| Some(*row) == cursor) {
        return Some(Reply {
            text: text.to_string(),
            row: Some(*row),
        });
    }

    let rows: Vec<_> = changed
        .iter()
        .take(2)
        .map(|(_, text)| clipped(text, MAX_FALLBACK_ROW_CHARS))
        .collect();
    (!rows.is_empty()).then(|| Reply {
        text: rows.join("\n"),
        row: None,
    })
}

fn same_row_text(before: &str, after: &str) -> bool {
    spinner_text(before) == spinner_text(after)
}

fn spinner_text(text: &str) -> &str {
    let mut text = text.trim();
    if let Some(first) = text
        .chars()
        .next()
        .filter(|character| is_spinner(*character))
    {
        let rest = &text[first.len_utf8()..];
        if rest.starts_with(char::is_whitespace) {
            text = rest.trim_start();
        }
    }
    if let Some(last) = text
        .chars()
        .next_back()
        .filter(|character| is_spinner(*character))
    {
        let rest = &text[..text.len() - last.len_utf8()];
        if rest.ends_with(char::is_whitespace) {
            text = rest.trim_end();
        }
    }
    text
}

fn is_spinner(character: char) -> bool {
    matches!(character, '|' | '/' | '-' | '\\' | '◐' | '◓' | '◑' | '◒')
        || ('\u{2800}'..='\u{28ff}').contains(&character)
}

fn is_selection_marker(character: char) -> bool {
    matches!(
        character,
        '>' | '❯' | '›' | '▸' | '▶' | '►' | '➤' | '➜' | '→' | '❱' | '»'
    )
}

fn is_border(character: char) -> bool {
    character == '|' || ('\u{2500}'..='\u{257f}').contains(&character)
}

fn clipped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{CursorSnapshot, TerminalModel};

    fn offset(terminal: &TerminalModel) -> u32 {
        cursor_offset(&terminal.reading_document(), &terminal.live_snapshot())
    }

    fn frame(text: &str) -> TerminalSnapshot {
        let mut terminal = TerminalModel::new(400, 6);
        terminal.push_bytes(text.as_bytes());
        terminal.live_snapshot()
    }

    #[test]
    fn hidden_cursor_follows_unique_software_cursor_without_treating_menu_as_input() {
        let mut terminal = TerminalModel::new(20, 4);
        terminal.push_bytes("\x1b[?25l> привет\x1b[7m \x1b[0m".as_bytes());
        let frame = terminal.live_snapshot();
        let cursor = reading_cursor(&frame, None).unwrap();
        assert_eq!((cursor.row, cursor.column), (0, 8));
        terminal.push_bytes(b"\r\n\x1b[7mSelected menu item\x1b[0m");
        assert_eq!(reading_cursor(&terminal.live_snapshot(), None), None);
    }

    #[test]
    fn explicit_reading_covers_a_unicode_input_across_soft_wraps() {
        let mut terminal = TerminalModel::new(8, 6);
        terminal.push_bytes("first\r\n> привет😀e\u{301}".as_bytes());
        let document = terminal.reading_document();
        let cursor = cursor_offset(&document, &terminal.live_snapshot());
        assert_eq!(
            current_line(&document, cursor).as_deref(),
            Some("> привет😀e\u{301}")
        );
        assert_eq!(current_line(&document, u32::MAX), None);
    }

    #[test]
    fn ctrl_arrow_reading_moves_between_logical_lines_without_splitting_unicode() {
        let mut terminal = TerminalModel::new(8, 6);
        terminal.push_bytes("above\r\n> привет😀e\u{301}\r\nbelow".as_bytes());
        let document = terminal.reading_document();
        let end = document.lines.last().unwrap().end;
        let input = adjacent_line(&document, end, true);
        assert_eq!(
            current_line(&document, input).as_deref(),
            Some("> привет😀e\u{301}")
        );
        let first = adjacent_line(&document, input, true);
        assert_eq!(current_line(&document, first).as_deref(), Some("above"));
        assert_eq!(adjacent_line(&document, first, true), first);
        assert_eq!(adjacent_line(&document, first, false), input);
        let bottom = adjacent_line(&document, input, false);
        assert_eq!(current_line(&document, bottom).as_deref(), Some("below"));
        assert_eq!(adjacent_line(&document, bottom, false), bottom);
    }

    #[test]
    fn hidden_navigation_forgets_the_old_input_point_and_can_confirm_a_new_one() {
        let mut terminal = TerminalModel::new(20, 4);
        terminal.push_bytes(b"> \x1b[?25l");
        let mut echo = crate::tui_input_echo::InputEcho::default();
        let now = Instant::now();
        echo.typed(&terminal.live_snapshot(), "x", now);
        terminal.push_bytes(b"x");
        assert!(echo.observe(&terminal.live_snapshot(), now).is_some());
        assert_eq!(echo.last_confirmed(&terminal.live_snapshot()), Some((0, 3)));
        // The integration cancels all evidence before explicit navigation.
        echo.cancel();
        terminal.push_bytes(b"\x1b[2;3H");
        let moved = terminal.live_snapshot();
        assert_eq!(reading_cursor(&moved, echo.last_confirmed(&moved)), None);
        echo.typed(&moved, "y", now);
        terminal.push_bytes(b"y");
        let reply = echo.observe(&terminal.live_snapshot(), now).unwrap();
        assert_eq!((reply.text.as_str(), reply.row, reply.column), ("y", 1, 3));
    }

    fn row_background(frame: &mut TerminalSnapshot, row: usize, background: Color) {
        for cell in &mut frame.cells[row * frame.cols..(row + 1) * frame.cols] {
            cell.bg = background;
        }
    }

    fn row_foreground(frame: &mut TerminalSnapshot, row: usize, foreground: Color) {
        for cell in &mut frame.cells[row * frame.cols..(row + 1) * frame.cols] {
            cell.fg = foreground;
        }
    }

    fn row_inverse(frame: &mut TerminalSnapshot, row: usize, inverse: bool) {
        for cell in &mut frame.cells[row * frame.cols..(row + 1) * frame.cols] {
            cell.flags.set(Flags::INVERSE, inverse);
        }
    }

    fn navigation_reply(before: &TerminalSnapshot, after: &TerminalSnapshot) -> Option<String> {
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(before, now);
        assert_eq!(feedback.observe(after, now), None);
        let reply = feedback.observe(after, now + STABLE_FOR);
        assert_eq!(
            feedback.observe(after, now + Duration::from_millis(300)),
            None
        );
        reply
    }

    #[test]
    fn caret_tracks_history_crlf_and_empty_rows() {
        let mut terminal = TerminalModel::new(12, 3);
        terminal.push_bytes(b"old\r\none\r\ntwo\r\nthree");
        assert_eq!(offset(&terminal), 20);
        terminal.push_bytes(b"\x1b[1;2H");
        assert_eq!(offset(&terminal), 6);

        let mut blank = TerminalModel::new(12, 4);
        blank.push_bytes(b"one\r\n\r\nthree\x1b[2;9H");
        assert_eq!(offset(&blank), 5);
        blank.push_bytes(b"\x1b[1;10H");
        assert_eq!(offset(&blank), 3);
    }

    #[test]
    fn caret_tracks_soft_wrap_wide_combining_and_utf16() {
        let mut terminal = TerminalModel::new(4, 4);
        terminal.push_bytes("ae\u{301}界\u{301}b🙂".as_bytes());
        assert_eq!(offset(&terminal), 8);
        terminal.push_bytes(b"\x1b[1;2H");
        assert_eq!(offset(&terminal), 1);
        terminal.push_bytes(b"\x1b[1;4H");
        assert_eq!(offset(&terminal), 3);
        terminal.push_bytes(b"\x1b[2;2H");
        assert_eq!(offset(&terminal), 6);
        terminal.push_bytes(b"\x1b[2;3H");
        assert_eq!(offset(&terminal), 6);
        terminal.push_bytes(b"\x1b[2;4H");
        assert_eq!(offset(&terminal), 8);
    }

    #[test]
    fn caret_maps_leading_wide_spacer_and_alternate_screen() {
        let mut terminal = TerminalModel::new(4, 4);
        terminal.push_bytes("abc界x".as_bytes());
        terminal.push_bytes(b"\x1b[1;4H");
        assert_eq!(offset(&terminal), 3);
        terminal.push_bytes(b"\x1b[?1049h\x1b[2;1Halt");
        assert_eq!(offset(&terminal), 5);
        terminal.push_bytes(b"\x1b[1;1H");
        assert_eq!(offset(&terminal), 0);
    }

    #[test]
    fn caret_uses_grid_coordinates_in_scrolled_view_and_hidden_fallback() {
        let mut terminal = TerminalModel::new(8, 3);
        terminal.push_bytes(b"one\r\ntwo\r\nthree\r\nfour");
        terminal.push_bytes(b"\x1b[1;2H");
        let expected = offset(&terminal);
        terminal.scroll_display(1);
        assert_eq!(
            cursor_offset(&terminal.reading_document(), &terminal.snapshot()),
            expected
        );
        terminal.push_bytes(b"\x1b[?25l");
        let document = terminal.reading_document();
        assert_eq!(
            cursor_offset(&document, &terminal.live_snapshot()),
            document.text.encode_utf16().count() as u32
        );
    }

    #[test]
    fn new_marked_option_wins_and_emits_once_after_stability() {
        let before = frame("❯ First\r\n  Second\r\nhelp");
        let after = frame("  First\r\n❯ Second\r\nnew help");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        assert_eq!(feedback.observe(&after, now), None);
        assert_eq!(
            feedback.observe(&after, now + STABLE_FOR - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            feedback.observe(&after, now + STABLE_FOR),
            Some("Second".to_owned())
        );
        assert_eq!(
            feedback.observe(&after, now + Duration::from_millis(300)),
            None
        );
    }

    #[test]
    fn kimi_slash_list_pointer_is_omitted_from_the_selected_label() {
        // Source-derived SelectList/WrappingSelectList layout: the selected
        // label and description share primary, while other descriptions are muted.
        let before = frame(concat!(
            "\x1b[36m→ /help   Show help\x1b[0m\r\n",
            "  /model  \x1b[90mSwitch model\x1b[0m\r\n",
            "  /quit   \x1b[90mExit\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "  /help   \x1b[90mShow help\x1b[0m\r\n",
            "\x1b[36m→ /model  Switch model\x1b[0m\r\n",
            "  /quit   \x1b[90mExit\x1b[0m\x1b[?25l"
        ));
        assert_eq!(
            navigation_reply(&before, &after),
            Some("/model  Switch model".to_owned())
        );
    }

    #[test]
    fn kimi_model_pointer_transfer_handles_mixed_label_and_provider_colors() {
        // Source-derived ModelSelectorComponent layout, not a recorded session:
        // the name alone gains primary/bold; provider and current mark stay styled.
        let before = frame(concat!(
            "\x1b[36m  ❯ \x1b[1mAlpha model\x1b[0m  \x1b[90mKimi Code\x1b[32m ✓\x1b[0m\r\n",
            "\x1b[90m    \x1b[0mBravo model  \x1b[90mKimi Code\x1b[0m\r\n",
            "\x1b[90m    \x1b[0mCharlie      \x1b[90mOther provider\x1b[0m\r\n",
            " Thinking: on\x1b[?25l"
        ));
        let after = frame(concat!(
            "\x1b[90m    \x1b[0mAlpha model  \x1b[90mKimi Code\x1b[32m ✓\x1b[0m\r\n",
            "\x1b[36m  ❯ \x1b[1mBravo model\x1b[0m  \x1b[90mKimi Code\x1b[0m\r\n",
            "\x1b[90m    \x1b[0mCharlie      \x1b[90mOther provider\x1b[0m\r\n",
            " Thinking: off\x1b[?25l"
        ));
        assert_eq!(
            navigation_reply(&before, &after),
            Some("Bravo model  Kimi Code".to_owned())
        );
    }

    #[test]
    fn unfamiliar_selectors_need_a_unique_transfer_between_stable_labels() {
        for marker in ["◆", "●", "☛", "👉\u{fe0f}"] {
            let before = frame(&format!("{marker} Alpha\r\n  Bravo > 1 → target\x1b[?25l"));
            let after = frame(&format!("  Alpha\r\n{marker} Bravo > 1 → target\x1b[?25l"));
            assert_eq!(
                navigation_reply(&before, &after),
                Some("Bravo > 1 → target".to_owned()),
                "marker {marker}"
            );
        }
    }

    #[test]
    fn static_prompts_and_markers_do_not_compete_with_a_unique_transfer() {
        for (before, after) in [
            (
                "> previous chat\r\n> Alpha\r\n  Bravo\x1b[?25l",
                "> previous chat\r\n  Alpha\r\n> Bravo\x1b[?25l",
            ),
            (
                "◆ Alpha\r\n  Bravo\r\n◆ Charlie\x1b[?25l",
                "  Alpha\r\n◆ Bravo\r\n◆ Charlie\x1b[?25l",
            ),
        ] {
            assert_eq!(
                navigation_reply(&frame(before), &frame(after)),
                Some("Bravo".to_owned())
            );
        }
    }

    #[test]
    fn duplicate_or_conflicting_selector_transfers_remain_ambiguous() {
        for (before, after) in [
            (
                "◆ Alpha\r\n  Bravo\r\n◆ Charlie\x1b[?25l",
                "  Alpha\r\n◆ Bravo\r\n  Charlie\x1b[?25l",
            ),
            (
                "◆ Alpha\r\n  Bravo\r\n◆ Charlie\r\n  Delta\x1b[?25l",
                "  Alpha\r\n◆ Bravo\r\n  Charlie\r\n◆ Delta\x1b[?25l",
            ),
            (
                "◆ Alpha\r\n  Bravo\r\n● Charlie\r\n  Delta\x1b[?25l",
                "  Alpha\r\n◆ Bravo\r\n  Charlie\r\n● Delta\x1b[?25l",
            ),
            ("old\x1b[?25l", "❯ Alpha\r\n→ Bravo\x1b[?25l"),
        ] {
            assert_eq!(navigation_reply(&frame(before), &frame(after)), None);
        }
    }

    #[test]
    fn selector_transfer_ignores_surrounding_borders_and_their_repaint() {
        let before = frame(
            "┌───────────────┐\r\n│ ◆ Alpha      │\r\n│   Bravo → x  │\r\n└───────────────┘\x1b[?25l",
        );
        let after = frame(
            "╔═══════════════╗\r\n║   Alpha      ║\r\n║ ◆ Bravo → x  ║\r\n╚═══════════════╝\x1b[?25l",
        );
        assert_eq!(
            navigation_reply(&before, &after),
            Some("Bravo → x".to_owned())
        );
    }

    #[test]
    fn spinner_and_border_prefixes_cannot_establish_a_selector_transfer() {
        for marker in ["⠋", "◐", "|", "─"] {
            let before = frame(&format!("{marker} Alpha\r\n  Bravo\x1b[?25l"));
            let after = frame(&format!("  Alpha\r\n{marker} Bravo\x1b[?25l"));
            assert_eq!(navigation_reply(&before, &after), None);
        }
    }

    #[test]
    fn ordinary_content_keeps_punctuation_and_mathematical_signs() {
        for text in [
            "+ count > 0 → result",
            "≥ 5 remains true",
            "2 > 1 → result",
            "→value > 1",
            ">= 1 remains true",
            "> 1",
            "> limit",
            "→ target",
        ] {
            assert_eq!(
                navigation_reply(&frame("previous text"), &frame(text)),
                Some(text.to_owned())
            );
        }
        assert_eq!(
            navigation_reply(&frame("→ old label"), &frame("→ a > b → c")),
            Some("→ a > b → c".to_owned())
        );
        let mut terminal = TerminalModel::new(40, 3);
        terminal.push_bytes("→ a > b → c".as_bytes());
        assert_eq!(
            current_line(&terminal.reading_document(), offset(&terminal)).as_deref(),
            Some("→ a > b → c")
        );
    }

    #[test]
    fn a_new_marker_hint_keeps_full_content_among_unchanged_prompts() {
        let before = frame("> previous chat\r\nprevious text\x1b[?25l");
        let after = frame("> previous chat\r\n> limit\x1b[?25l");
        assert_eq!(
            navigation_reply(&before, &after),
            Some("> limit".to_owned())
        );
    }

    #[test]
    fn concealed_labels_and_markers_never_supply_speech_or_selection_evidence() {
        let before = frame("\x1b[8m❯ Alpha\x1b[0m\r\n\x1b[8m  Bravo\x1b[0m\x1b[?25l");
        let after = frame("\x1b[8m  Alpha\x1b[0m\r\n\x1b[8m❯ Bravo\x1b[0m\x1b[?25l");
        assert_eq!(navigation_reply(&before, &after), None);

        // Remain safe even if a caller constructs a snapshot with unredacted
        // text instead of using TerminalModel's already redacted projection.
        let mut before = frame("❯ Alpha\r\n  Bravo\x1b[?25l");
        let mut after = frame("  Alpha\r\n❯ Bravo\x1b[?25l");
        for snapshot in [&mut before, &mut after] {
            for cell in &mut snapshot.cells {
                cell.flags.insert(Flags::HIDDEN);
            }
            assert!(Screen::new(snapshot).rows.iter().all(String::is_empty));
        }
        assert_eq!(navigation_reply(&before, &after), None);
    }

    #[test]
    fn cursor_visibility_incidental_color_and_identical_redraws_remain_silent() {
        let before = frame("❯ First\r\n  Second");
        let mut after = before.clone();
        after.cursor = None;
        after.cells[0].fg = Color::Spec(bks_terminal::Rgb { r: 3, g: 4, b: 5 });
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        assert_eq!(feedback.observe(&after, now), None);
        assert_eq!(feedback.observe(&after, now + STABLE_FOR), None);
        // Returning on the same row is just visibility; an actual visible
        // row-to-row move now has its own affirmative regression below.
        after.cursor = Some(CursorSnapshot { row: 1, column: 4 });
        assert_eq!(
            feedback.observe(&after, now + Duration::from_millis(300)),
            None
        );
    }

    #[test]
    fn hidden_cursor_background_only_selection_announces_the_gained_row_once() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_background(&mut before, 0, Color::Indexed(4));
        let mut after = before.clone();
        row_background(&mut after, 0, Color::Named(NamedColor::Background));
        row_background(&mut after, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn inverse_only_selection_announces_without_changing_text_or_colors() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_inverse(&mut before, 0, true);
        let mut after = before.clone();
        row_inverse(&mut after, 0, false);
        row_inverse(&mut after, 1, true);
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn foreground_only_selection_requires_a_reciprocal_transfer() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_foreground(&mut before, 0, Color::Indexed(3));
        let mut after = before.clone();
        row_foreground(&mut after, 0, Color::Named(NamedColor::Foreground));
        row_foreground(&mut after, 1, Color::Indexed(3));
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn mixed_foreground_selection_preserves_provider_and_current_mark_styles() {
        let before = frame(concat!(
            "\x1b[36mAlpha model\x1b[0m  \x1b[90mProvider\x1b[32m ✓\x1b[0m\r\n",
            "Bravo model  \x1b[90mProvider\x1b[0m\r\n",
            "Charlie      \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "Alpha model  \x1b[90mProvider\x1b[32m ✓\x1b[0m\r\n",
            "\x1b[36mBravo model\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "Charlie      \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(Screen::new(&before).rows, Screen::new(&after).rows);
        assert_eq!(
            navigation_reply(&before, &after),
            Some("Bravo model  Provider".to_owned())
        );
        assert_eq!(
            navigation_reply(&after, &before),
            Some("Alpha model  Provider ✓".to_owned())
        );
    }

    #[test]
    fn mixed_custom_foregrounds_use_unchanged_neighbor_names_as_normal_evidence() {
        let before = frame(concat!(
            "\x1b[36mAlpha\x1b[90m  Provider\x1b[0m\r\n",
            "\x1b[37mBravo\x1b[90m  Provider\x1b[0m\r\n",
            "\x1b[37mCharlie\x1b[90m  Provider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "\x1b[37mAlpha\x1b[90m  Provider\x1b[0m\r\n",
            "\x1b[36mBravo\x1b[90m  Provider\x1b[0m\r\n",
            "\x1b[37mCharlie\x1b[90m  Provider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(
            navigation_reply(&before, &after),
            Some("Bravo  Provider".to_owned())
        );
    }

    #[test]
    fn mixed_one_cell_or_nonreciprocal_color_changes_do_not_establish_selection() {
        let before = frame(concat!(
            "\x1b[36mA\x1b[0mlpha  \x1b[90mProvider\x1b[0m\r\n",
            "Bravo  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "Alpha  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mB\x1b[0mravo  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(navigation_reply(&before, &after), None);

        let before = frame(concat!(
            "\x1b[36mAlpha\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "Bravo  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "Alpha  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[31mBravo\x1b[0m  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(navigation_reply(&before, &after), None);
    }

    #[test]
    fn multiple_mixed_highlights_and_whole_theme_changes_stay_ambiguous() {
        let before = frame(concat!(
            "\x1b[36mAlpha\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "Bravo  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mCharlie\x1b[0m  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "Alpha  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mBravo\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mCharlie\x1b[0m  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(navigation_reply(&before, &after), None);

        let before = frame(concat!(
            "Alpha  \x1b[90mProvider\x1b[0m\r\n",
            "Bravo  \x1b[90mProvider\x1b[0m\r\n",
            "Charlie  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        let after = frame(concat!(
            "\x1b[36mAlpha\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mBravo\x1b[0m  \x1b[90mProvider\x1b[0m\r\n",
            "\x1b[36mCharlie\x1b[0m  \x1b[90mProvider\x1b[0m\x1b[?25l"
        ));
        assert_eq!(navigation_reply(&before, &after), None);
    }

    #[test]
    fn background_selection_can_preserve_different_foreground_colors() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_foreground(&mut before, 0, Color::Indexed(1));
        row_foreground(&mut before, 1, Color::Indexed(2));
        row_background(&mut before, 0, Color::Indexed(4));
        let mut after = before.clone();
        row_background(&mut after, 0, Color::Named(NamedColor::Background));
        row_background(&mut after, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn unchanged_neighbors_disambiguate_custom_background_colors() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        for row in 0..3 {
            row_background(&mut before, row, Color::Indexed(0));
        }
        row_background(&mut before, 0, Color::Indexed(4));
        let mut after = before.clone();
        row_background(&mut after, 0, Color::Indexed(0));
        row_background(&mut after, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn a_single_incidental_color_change_is_not_a_selection() {
        let before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        let mut foreground = before.clone();
        row_foreground(&mut foreground, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &foreground), None);
        let mut same_foreground = before.clone();
        row_background(&mut same_foreground, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &same_foreground), None);
        let mut inverse = before.clone();
        row_inverse(&mut inverse, 1, true);
        assert_eq!(navigation_reply(&before, &inverse), None);
    }

    #[test]
    fn all_row_theme_color_and_inverse_changes_remain_quiet() {
        let before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        let mut theme = before.clone();
        for row in 0..3 {
            row_foreground(&mut theme, row, Color::Indexed(7));
            row_background(&mut theme, row, Color::Indexed(0));
        }
        assert_eq!(navigation_reply(&before, &theme), None);
        let mut inverse = before.clone();
        for row in 0..3 {
            row_inverse(&mut inverse, row, true);
        }
        assert_eq!(navigation_reply(&before, &inverse), None);
    }

    #[test]
    fn multiple_highlights_remain_ambiguous_when_only_one_moves() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_background(&mut before, 0, Color::Indexed(4));
        row_background(&mut before, 2, Color::Indexed(4));
        let mut after = before.clone();
        row_background(&mut after, 0, Color::Named(NamedColor::Background));
        row_background(&mut after, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &after), None);

        let mut before = frame("Alpha\r\nBravo\r\nCharlie\x1b[?25l");
        row_inverse(&mut before, 0, true);
        row_inverse(&mut before, 2, true);
        let mut after = before.clone();
        row_inverse(&mut after, 0, false);
        row_inverse(&mut after, 1, true);
        assert_eq!(navigation_reply(&before, &after), None);
    }

    #[test]
    fn custom_colors_without_an_unambiguous_normal_style_remain_quiet() {
        let mut before = frame("Alpha\r\nBravo\x1b[?25l");
        row_background(&mut before, 0, Color::Indexed(4));
        row_background(&mut before, 1, Color::Indexed(0));
        let mut after = before.clone();
        row_background(&mut after, 0, Color::Indexed(0));
        row_background(&mut after, 1, Color::Indexed(4));
        assert_eq!(navigation_reply(&before, &after), None);
    }

    #[test]
    fn visible_cursor_row_movement_announces_unchanged_content_once() {
        let mut before = frame("Alpha\r\nBravo\r\nCharlie");
        before.cursor = Some(CursorSnapshot { row: 0, column: 2 });
        let mut after = before.clone();
        after.cursor = Some(CursorSnapshot { row: 1, column: 3 });
        assert_eq!(navigation_reply(&before, &after), Some("Bravo".to_owned()));
    }

    #[test]
    fn cursor_columns_visibility_and_moves_to_empty_or_border_rows_are_quiet() {
        let mut before = frame("Alpha\r\nBravo\r\n\r\n────────");
        before.cursor = Some(CursorSnapshot { row: 0, column: 2 });
        for cursor in [
            None,
            Some(CursorSnapshot { row: 0, column: 4 }),
            Some(CursorSnapshot { row: 2, column: 0 }),
            Some(CursorSnapshot { row: 3, column: 0 }),
        ] {
            let mut after = before.clone();
            after.cursor = cursor;
            assert_eq!(navigation_reply(&before, &after), None);
        }
        let mut hidden = before.clone();
        hidden.cursor = None;
        let mut visible = hidden.clone();
        visible.cursor = Some(CursorSnapshot { row: 1, column: 0 });
        assert_eq!(navigation_reply(&hidden, &visible), None);
    }

    #[test]
    fn spinner_only_ticks_with_unchanged_words_are_quiet() {
        for (before, after) in [
            ("⠋ Working\r\nAlpha", "⠙ Working\r\nAlpha"),
            ("Working /\r\nAlpha", "Working -\r\nAlpha"),
        ] {
            assert_eq!(navigation_reply(&frame(before), &frame(after)), None);
        }
    }

    #[test]
    fn moving_highlight_between_equal_labels_restarts_stability() {
        let mut before = frame("Alpha\r\nDuplicate\r\nDuplicate\x1b[?25l");
        row_background(&mut before, 0, Color::Indexed(4));
        let mut second = before.clone();
        row_background(&mut second, 0, Color::Named(NamedColor::Background));
        row_background(&mut second, 1, Color::Indexed(4));
        let mut third = second.clone();
        row_background(&mut third, 1, Color::Named(NamedColor::Background));
        row_background(&mut third, 2, Color::Indexed(4));
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        assert_eq!(feedback.observe(&second, now), None);
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(80)),
            None
        );
        assert_eq!(feedback.observe(&third, now + STABLE_FOR), None);
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(170)),
            Some("Duplicate".to_owned())
        );
    }

    #[test]
    fn border_repaints_do_not_create_speech() {
        let before = frame("┌──────────┐\r\n│ ❯ First │\r\n└──────────┘");
        let after = frame("╔══════════╗\r\n║ ❯ First ║\r\n╚══════════╝");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        assert_eq!(feedback.observe(&after, now), None);
        assert_eq!(feedback.observe(&after, now + STABLE_FOR), None);
    }

    #[test]
    fn changed_cursor_row_wins_over_other_new_rows() {
        let before = frame("old status\r\nold input\r\nold help");
        let mut after = frame("new status\r\nnew input\r\nnew help");
        after.cursor = Some(CursorSnapshot { row: 1, column: 5 });
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&after, now);
        assert_eq!(
            feedback.observe(&after, now + STABLE_FOR),
            Some("new input".to_owned())
        );
    }

    #[test]
    fn fallback_contains_at_most_two_short_new_rows() {
        let before = frame("old one\r\nold two\r\nold three");
        let mut after = frame("new one\r\nnew two\r\nnew three");
        after.cursor = None;
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&after, now);
        assert_eq!(
            feedback.observe(&after, now + STABLE_FOR),
            Some("new one\nnew two".to_owned())
        );
    }

    #[test]
    fn changing_or_reverting_candidate_resets_stability() {
        let before = frame("> One\r\n  Two\r\n  Three");
        let second = frame("  One\r\n> Two\r\n  Three");
        let third = frame("  One\r\n  Two\r\n> Three");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&second, now);
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(80)),
            None
        );
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(100)),
            None
        );
        assert_eq!(
            feedback.observe(&before, now + Duration::from_millis(150)),
            None
        );
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(200)),
            None
        );
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(290)),
            Some("Three".to_owned())
        );
    }

    #[test]
    fn requests_expire_and_explicit_cancel_discards_candidate() {
        let before = frame("> One");
        let after = frame("> Two");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&after, now + Duration::from_millis(750));
        assert_eq!(
            feedback.observe(&after, now + Duration::from_millis(840)),
            None
        );
        assert!(feedback.pending.is_none());

        feedback.begin(&before, now);
        feedback.observe(&after, now);
        feedback.cancel();
        assert_eq!(feedback.observe(&after, now + STABLE_FOR), None);
    }

    #[test]
    fn another_key_replaces_the_request_and_geometry_or_screen_change_cancels() {
        let before = frame("> One");
        let second = frame("> Two");
        let mut third = frame("> Three");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&second, now);
        feedback.begin(&second, now + Duration::from_millis(30));
        feedback.observe(&third, now + Duration::from_millis(30));
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(90)),
            None
        );
        assert_eq!(
            feedback.observe(&third, now + Duration::from_millis(120)),
            Some("> Three".to_owned())
        );

        feedback.begin(&second, now);
        feedback.observe(&third, now);
        third.alternate_screen = true;
        assert_eq!(feedback.observe(&third, now + STABLE_FOR), None);
        assert!(feedback.pending.is_none());
        third.alternate_screen = false;
        feedback.begin(&second, now);
        feedback.observe(&third, now);
        third.cols += 1;
        assert_eq!(feedback.observe(&third, now + STABLE_FOR), None);
        assert!(feedback.pending.is_none());
    }

    #[test]
    fn reply_is_bounded_by_unicode_scalars_without_splitting_utf8() {
        let before = frame("❯ old");
        let after = frame(&format!("❯ {}", "я".repeat(350)));
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&after, now);
        let reply = feedback.observe(&after, now + STABLE_FOR).unwrap();
        assert_eq!(reply.chars().count(), MAX_EVENT_CHARS);
        assert!(reply.starts_with("❯ "));
        assert!(reply.ends_with('я'));
    }

    #[test]
    fn unspoken_tail_changes_still_restart_stability() {
        let before = frame("❯ old");
        let prefix = format!("❯ {}", "x".repeat(320));
        let first = frame(&format!("{prefix}one"));
        let second = frame(&format!("{prefix}two"));
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        feedback.begin(&before, now);
        feedback.observe(&first, now);
        feedback.observe(&second, now + Duration::from_millis(80));
        assert_eq!(feedback.observe(&second, now + STABLE_FOR), None);
        let reply = feedback
            .observe(&second, now + Duration::from_millis(170))
            .unwrap();
        assert_eq!(reply.chars().count(), MAX_EVENT_CHARS);
    }

    #[test]
    fn no_request_or_scrolled_history_never_speaks() {
        let before = frame("old");
        let mut after = frame("new");
        let now = Instant::now();
        let mut feedback = InputFeedback::default();
        assert_eq!(feedback.observe(&after, now), None);
        after.display_offset = 1;
        feedback.begin(&after, now);
        assert!(feedback.pending.is_none());
        feedback.begin(&before, now);
        assert_eq!(feedback.observe(&after, now + STABLE_FOR), None);
        assert!(feedback.pending.is_none());
    }
}
