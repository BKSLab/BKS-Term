//! Echo only text whose insertion is confirmed by a live VT frame.
//! The terminal remains the sole text authority; no speculative editing occurs.

use std::time::{Duration, Instant};

use bks_terminal::{Flags, TerminalSnapshot};

const MAX_AGE: Duration = Duration::from_millis(750);
const MAX_PENDING_CHARS: usize = 64;
const MAX_ROWS: usize = 256;
const MAX_COLS: usize = 512;
const MAX_SCREEN_CHARS: usize = 65_536;

#[derive(Debug, PartialEq, Eq)]
pub struct ConfirmedEcho {
    pub text: String,
    pub row: usize,
    /// An insertion point; `frame.cols` denotes the end of a full physical row.
    pub column: usize,
}

#[derive(Default)]
pub struct InputEcho {
    pending: Option<Pending>,
    confirmed: Option<Anchor>,
}

struct Pending {
    before: Screen,
    expected: Vec<char>,
    origin: Option<Point>,
    started: Instant,
}

struct Anchor {
    geometry: Geometry,
    point: Point,
    row: Row,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Point {
    row: usize,
    column: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Geometry {
    cols: usize,
    rows: usize,
    alternate: bool,
    history: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Glyph {
    ch: char,
    start: Point,
    end: Point,
    visible: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Row {
    glyphs: Vec<Glyph>,
    wrapped: bool,
}

struct Line {
    row: usize,
    glyphs: Vec<Glyph>,
}

struct Screen {
    geometry: Geometry,
    cursor: Option<Point>,
    concealed: bool,
    rows: Vec<Row>,
    lines: Vec<Line>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Insertion {
    count: usize,
    end: Point,
}

impl InputEcho {
    /// Call before forwarding committed printable Unicode to the PTY. Several
    /// calls before its acknowledgement retain one baseline and ordered queue.
    pub fn typed(&mut self, frame: &TerminalSnapshot, text: &str, now: Instant) {
        if text.is_empty() {
            return;
        }
        // Check before extending an existing request: the concealed frame may
        // be observed by a key event and disappear again before the next poll.
        if frame.cells.len() > MAX_ROWS * MAX_COLS
            || frame
                .cells
                .iter()
                .any(|cell| cell.flags.contains(Flags::HIDDEN))
        {
            self.cancel();
            return;
        }
        let characters: Vec<_> = text.chars().take(MAX_PENDING_CHARS + 1).collect();
        if characters.len() > MAX_PENDING_CHARS || characters.iter().any(|ch| ch.is_control()) {
            self.cancel_pending();
            return;
        }
        if self.pending.as_ref().is_some_and(|pending| {
            now.saturating_duration_since(pending.started) >= MAX_AGE
                || !pending.before.geometry.matches(frame)
        }) {
            self.cancel_pending();
        }
        if let Some(pending) = &mut self.pending {
            if pending.expected.len() + characters.len() > MAX_PENDING_CHARS {
                self.cancel_pending();
            } else {
                pending.expected.extend(characters);
            }
            return;
        }
        let Some(before) = Screen::new(frame) else {
            self.cancel();
            return;
        };
        let origin = before.cursor.or_else(|| {
            self.confirmed
                .as_ref()
                .filter(|anchor| anchor.matches(&before))
                .map(|anchor| anchor.point)
        });
        self.pending = Some(Pending {
            before,
            expected: characters,
            origin,
            started: now,
        });
    }

    /// Acknowledged prefixes are returned once, without waiting for later keys.
    /// Concealed/masked input, unmatched redraws and expired requests stay quiet.
    pub fn observe(&mut self, frame: &TerminalSnapshot, now: Instant) -> Option<ConfirmedEcho> {
        let mut pending = self.pending.take()?;
        if now.saturating_duration_since(pending.started) >= MAX_AGE
            || !pending.before.geometry.matches(frame)
        {
            return None;
        }
        let current = Screen::new(frame)?;
        // Public VT projections already redact concealed text to spaces.
        // Its absence cannot leave a raw typing request armed for a later
        // redraw which reveals the same input. Prefer silence for this frame.
        if current.concealed {
            self.cancel();
            return None;
        }
        let insertion = match find_insertion(&pending, &current) {
            Ok(Some(insertion)) => insertion,
            Ok(None) => {
                self.pending = Some(pending);
                return None;
            }
            // An expected insertion in concealed cells must not be spoken if
            // a later redraw merely reveals those same cells.
            Err(()) => return None,
        };
        let text = pending.expected.drain(..insertion.count).collect();
        self.confirmed = Some(Anchor {
            geometry: current.geometry,
            point: insertion.end,
            row: current.rows[insertion.end.row].clone(),
        });
        pending.before = current;
        pending.origin = Some(insertion.end);
        if !pending.expected.is_empty() {
            self.pending = Some(pending);
        }
        Some(ConfirmedEcho {
            text,
            row: insertion.end.row,
            column: insertion.end.column,
        })
    }

    /// A confirmed hidden-cursor anchor is usable only while that exact physical
    /// row and the live-screen identity remain unchanged. This does not infer
    /// cursor movement: cancel on navigation without a trustworthy VT cursor.
    pub fn last_confirmed(&self, frame: &TerminalSnapshot) -> Option<(usize, usize)> {
        let anchor = self.confirmed.as_ref()?;
        if !anchor.geometry.matches(frame) {
            return None;
        }
        let mut budget = MAX_SCREEN_CHARS;
        let row = capture_row(frame, anchor.point.row, &mut budget)?;
        (row == anchor.row).then_some((anchor.point.row, anchor.point.column))
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    /// Call on focus, session or reading-mode changes.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.confirmed = None;
    }
}

impl Geometry {
    fn matches(self, frame: &TerminalSnapshot) -> bool {
        frame.display_offset == 0
            && self.cols == frame.cols
            && self.rows == frame.rows
            && self.alternate == frame.alternate_screen
            && self.history == frame.history_size
    }
}

impl Anchor {
    fn matches(&self, screen: &Screen) -> bool {
        self.geometry == screen.geometry && screen.rows.get(self.point.row) == Some(&self.row)
    }
}

impl Screen {
    fn new(frame: &TerminalSnapshot) -> Option<Self> {
        if frame.display_offset != 0
            || frame.cols == 0
            || frame.cols > MAX_COLS
            || frame.rows == 0
            || frame.rows > MAX_ROWS
            || frame.cells.len() != frame.cols.checked_mul(frame.rows)?
        {
            return None;
        }
        let mut budget = MAX_SCREEN_CHARS;
        let rows: Vec<_> = (0..frame.rows)
            .map(|row| capture_row(frame, row, &mut budget))
            .collect::<Option<_>>()?;
        let mut lines = Vec::new();
        let mut glyphs = Vec::new();
        let mut first = 0;
        for (index, row) in rows.iter().enumerate() {
            glyphs.extend_from_slice(&row.glyphs);
            if !row.wrapped || index + 1 == rows.len() {
                lines.push(Line { row: first, glyphs });
                glyphs = Vec::new();
                first = index + 1;
            }
        }
        Some(Self {
            concealed: frame
                .cells
                .iter()
                .any(|cell| cell.flags.contains(Flags::HIDDEN)),
            geometry: Geometry {
                cols: frame.cols,
                rows: frame.rows,
                alternate: frame.alternate_screen,
                history: frame.history_size,
            },
            // Some TUI editors keep the hardware cursor hidden and render one
            // inverse cell instead. Use the same unambiguous cursor as reading.
            cursor: crate::direct_accessibility::reading_cursor(frame, None).and_then(|cursor| {
                (cursor.row < frame.rows && cursor.column < frame.cols).then_some(Point {
                    row: cursor.row,
                    column: cursor.column,
                })
            }),
            rows,
            lines,
        })
    }
}

fn capture_row(frame: &TerminalSnapshot, row: usize, budget: &mut usize) -> Option<Row> {
    let start = row.checked_mul(frame.cols)?;
    let cells = frame.cells.get(start..start.checked_add(frame.cols)?)?;
    let wrapped = cells.last()?.flags.contains(Flags::WRAPLINE);
    let mut glyphs = Vec::new();
    for (column, cell) in cells.iter().enumerate() {
        let end = Point {
            row,
            column: column
                + if cell.flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                },
        };
        for ch in cell.text.chars() {
            *budget = budget.checked_sub(1)?;
            let ch = if ch == '\t' { ' ' } else { ch };
            if ch.is_control() || end.column > frame.cols {
                return None;
            }
            glyphs.push(Glyph {
                ch,
                start: Point { row, column },
                end,
                visible: !cell.flags.contains(Flags::HIDDEN) && cell.fg != cell.bg,
            });
        }
    }
    if !wrapped {
        let caret = frame
            .cursor
            .filter(|cursor| cursor.row == row)
            .map_or(0, |cursor| cursor.column);
        while glyphs
            .last()
            .is_some_and(|glyph| glyph.ch == ' ' && glyph.end.column > caret)
        {
            glyphs.pop();
        }
    }
    Some(Row { glyphs, wrapped })
}

fn same_text(before: &[Glyph], after: &[Glyph]) -> bool {
    before.len() == after.len() && before.iter().zip(after).all(|(a, b)| a.ch == b.ch)
}

fn matches_caret(point: Point, caret: Point, cols: usize) -> bool {
    point == caret || (point.row == caret.row && point.column == cols && caret.column + 1 == cols)
}

fn find_insertion(pending: &Pending, current: &Screen) -> Result<Option<Insertion>, ()> {
    if let Some(insertion) = find_cursor_insertion(pending, current)? {
        return Ok(Some(insertion));
    }
    let mut found = None;
    let mut transformed = false;
    for after in &current.lines {
        let Some(before) = pending
            .before
            .lines
            .iter()
            .find(|line| line.row == after.row)
        else {
            continue;
        };
        let Some(growth) = after
            .glyphs
            .len()
            .checked_sub(before.glyphs.len())
            .filter(|n| *n > 0)
        else {
            continue;
        };
        let mut candidates = Vec::new();
        if growth <= pending.expected.len() {
            let prefix = before
                .glyphs
                .iter()
                .zip(&after.glyphs)
                .take_while(|(a, b)| a.ch == b.ch)
                .count();
            let suffix = before
                .glyphs
                .iter()
                .rev()
                .zip(after.glyphs.iter().rev())
                .take_while(|(a, b)| a.ch == b.ch)
                .count();
            for start in before.glyphs.len() - suffix..=prefix {
                let origin = before
                    .glyphs
                    .get(start)
                    .map(|glyph| glyph.start)
                    .or_else(|| before.glyphs.last().map(|glyph| glyph.end))
                    .unwrap_or(Point {
                        row: before.row,
                        column: 0,
                    });
                candidates.push((start, growth, origin));
            }
        }
        // A previously blank gap can become internal text when a hidden cursor
        // types after a prompt's trailing padding. Do not echo that old padding.
        if same_text(&before.glyphs, &after.glyphs[..before.glyphs.len()]) {
            for gap in 1..growth.min(current.geometry.cols + 1) {
                let start = before.glyphs.len() + gap;
                if after.glyphs[start - 1].ch != ' ' {
                    break;
                }
                let count = growth - gap;
                if count <= pending.expected.len() {
                    candidates.push((start, count, after.glyphs[start].start));
                }
            }
        }
        for (start, count, origin) in candidates {
            let inserted = &after.glyphs[start..start + count];
            if pending
                .origin
                .is_some_and(|caret| !matches_caret(origin, caret, current.geometry.cols))
            {
                continue;
            }
            let end = inserted.last().ok_or(())?.end;
            if current
                .cursor
                .is_some_and(|caret| !matches_caret(end, caret, current.geometry.cols))
            {
                continue;
            }
            if !inserted
                .iter()
                .map(|glyph| glyph.ch)
                .eq(pending.expected[..count].iter().copied())
            {
                // Prefix/suffix and any live cursor agree on an insertion,
                // but its text was transformed. Do not keep raw input armed
                // for a later reveal, even when the cursor is hidden.
                transformed = true;
                continue;
            }
            if inserted.iter().any(|glyph| !glyph.visible) {
                return Err(());
            }
            let insertion = Insertion { count, end };
            if found.is_some_and(|previous| previous != insertion) {
                return Ok(None);
            }
            found = Some(insertion);
        }
    }
    // Trimming old padding can create alternative candidate boundaries. An
    // exact acknowledgement wins over those alternatives, including partial
    // acknowledgements and a literal space followed by the next character.
    if found.is_none() && transformed {
        Err(())
    } else {
        Ok(found)
    }
}

/// A bordered editor fills the remainder of its row with spaces. Inserting
/// text consumes that padding, so the complete row need not grow at all (wide
/// glyphs can even reduce its character count). Confirm at both live cursors,
/// retaining the old text on either side and allowing only blank fill to go.
/// Comparing logical-line text also allows the composer to move during redraw.
fn find_cursor_insertion(pending: &Pending, current: &Screen) -> Result<Option<Insertion>, ()> {
    let (Some(origin), Some(end)) = (pending.origin, current.cursor) else {
        return Ok(None);
    };
    let Some(before) = line_at(&pending.before, origin) else {
        return Ok(None);
    };
    let Some(after) = line_at(current, end) else {
        return Ok(None);
    };
    let start = caret_index(before, origin);
    let stop = caret_index(after, end);
    let Some(count) = stop.checked_sub(start).filter(|count| *count > 0) else {
        return Ok(None);
    };
    if !same_text(&before.glyphs[..start], &after.glyphs[..start])
        || !same_text_without_blank_fill(&before.glyphs[start..], &after.glyphs[stop..])
    {
        return Ok(None);
    }
    let inserted = &after.glyphs[start..stop];
    if count > pending.expected.len()
        || !inserted
            .iter()
            .map(|glyph| glyph.ch)
            .eq(pending.expected[..count].iter().copied())
    {
        // A masked/transformed insertion disarms the raw request before any
        // later reveal. Do not depend on recognizing a particular mask glyph.
        return Err(());
    }
    if inserted.iter().any(|glyph| !glyph.visible) {
        return Err(());
    }
    Ok(Some(Insertion { count, end }))
}

fn line_at(screen: &Screen, point: Point) -> Option<&Line> {
    screen.lines.iter().rev().find(|line| line.row <= point.row)
}

fn caret_index(line: &Line, point: Point) -> usize {
    line.glyphs.partition_point(|glyph| {
        glyph.end.row < point.row
            || (glyph.end.row == point.row && glyph.end.column <= point.column)
    })
}

fn same_text_without_blank_fill(before: &[Glyph], after: &[Glyph]) -> bool {
    let Some(removed) = before.len().checked_sub(after.len()) else {
        return false;
    };
    let prefix = before
        .iter()
        .zip(after)
        .take_while(|(a, b)| a.ch == b.ch)
        .count();
    before[prefix..prefix + removed]
        .iter()
        .all(|glyph| glyph.ch == ' ')
        && same_text(&before[prefix + removed..], &after[prefix..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{Color, CursorSnapshot, TerminalModel};

    fn terminal(cols: usize, text: &str) -> TerminalModel {
        let mut terminal = TerminalModel::new(cols, 5);
        terminal.push_bytes(text.as_bytes());
        terminal
    }

    // Kimi's editor uses fixed-width, space-padded rows inside side borders,
    // a single inverse software cursor, and a hidden hardware cursor.
    fn composer(row: usize, text: &str, padding: usize) -> TerminalSnapshot {
        terminal(
            24,
            &format!(
                "\x1b[{};1H│ > {text}\x1b[7m \x1b[0m{}│\x1b[?25l",
                row + 1,
                " ".repeat(padding)
            ),
        )
        .live_snapshot()
    }

    fn assert_echo(
        echo: &mut InputEcho,
        frame: &TerminalSnapshot,
        now: Instant,
        text: &str,
        row: usize,
        column: usize,
    ) {
        assert_eq!(
            echo.observe(frame, now),
            Some(ConfirmedEcho {
                text: text.to_owned(),
                row,
                column,
            })
        );
        assert_eq!(echo.observe(frame, now), None);
        assert_eq!(echo.last_confirmed(frame), Some((row, column)));
    }

    #[test]
    fn visible_insertion_is_confirmed_once_without_local_speculation() {
        let mut model = terminal(20, "> ");
        let before = model.live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "hello", now);
        assert_eq!(echo.observe(&before, now), None);
        assert_eq!(echo.last_confirmed(&before), None);
        model.push_bytes(b"hello");
        assert_echo(&mut echo, &model.live_snapshot(), now, "hello", 0, 7);
    }

    #[test]
    fn rapid_input_retains_the_queue_across_partial_pty_acknowledgements() {
        let mut model = terminal(20, "");
        let before = model.live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "a", now);
        echo.typed(&before, "b", now);
        model.push_bytes(b"a");
        let first = model.live_snapshot();
        assert_eq!(
            echo.observe(&first, now).map(|confirmed| confirmed.text),
            Some("a".to_owned())
        );
        echo.typed(&first, "c", now);
        model.push_bytes(b"bc");
        assert_echo(&mut echo, &model.live_snapshot(), now, "bc", 0, 3);
    }

    #[test]
    fn another_key_before_observe_does_not_discard_an_already_drawn_prefix() {
        let mut model = terminal(20, "");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "a", now);
        model.push_bytes(b"a");
        echo.typed(&model.live_snapshot(), "b", now);
        assert_eq!(
            echo.observe(&model.live_snapshot(), now)
                .map(|confirmed| confirmed.text),
            Some("a".to_owned())
        );
        model.push_bytes(b"b");
        assert_echo(&mut echo, &model.live_snapshot(), now, "b", 0, 2);
    }

    #[test]
    fn hidden_cursor_uses_unique_exact_insertion_after_existing_blank_padding() {
        let mut model = terminal(20, "> \x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "abc", now);
        model.push_bytes(b"abc");
        assert_echo(&mut echo, &model.live_snapshot(), now, "abc", 0, 5);
    }

    #[test]
    fn hidden_cursor_can_confirm_an_initially_empty_row_with_indent() {
        let mut model = terminal(20, "\x1b[3;5H\x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "x", now);
        model.push_bytes(b"x");
        assert_echo(&mut echo, &model.live_snapshot(), now, "x", 2, 5);
    }

    #[test]
    fn bordered_software_cursor_confirms_unicode_when_the_row_does_not_grow() {
        let now = Instant::now();
        let mut echo = InputEcho::default();
        let before = composer(2, "", 18);
        let text = "Ж界e\u{301}🙂";
        let after = composer(2, text, 12);
        assert!(before.cursor.is_none() && after.cursor.is_none());
        assert_eq!(before.cells[2 * 24 + 23].text, "│");
        assert_eq!(after.cells[2 * 24 + 23].text, "│");
        echo.typed(&before, text, now);
        assert_eq!(echo.observe(&before, now), None);
        assert_echo(&mut echo, &after, now, text, 2, 10);

        // An immediately repeated character remains unambiguous at the live
        // software cursor, even though the old confirmed row is redrawn.
        echo.typed(&after, "🙂", now);
        assert_echo(
            &mut echo,
            &composer(2, &format!("{text}🙂"), 10),
            now,
            "🙂",
            2,
            12,
        );
    }

    #[test]
    fn composer_redraw_can_move_rows_before_the_unicode_input_is_acknowledged() {
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&composer(3, "hi", 16), "Ж界", now);
        assert_eq!(echo.observe(&composer(1, "hi", 16), now), None);
        assert_echo(&mut echo, &composer(1, "hiЖ界", 13), now, "Ж界", 1, 9);
    }

    #[test]
    fn bordered_editor_confirms_characters_and_spaces_across_partial_acknowledgements() {
        let now = Instant::now();
        let mut echo = InputEcho::default();
        let before = composer(2, "", 18);
        echo.typed(&before, "a b", now);
        assert_echo(&mut echo, &composer(2, "a", 17), now, "a", 2, 5);
        assert_echo(&mut echo, &composer(2, "a ", 16), now, " ", 2, 6);
        assert_echo(&mut echo, &composer(2, "a b", 15), now, "b", 2, 7);
    }

    #[test]
    fn bordered_editor_confirms_middle_insertion_without_losing_the_suffix() {
        let now = Instant::now();
        let mut echo = InputEcho::default();
        let before = terminal(
            24,
            &format!("│ > ab\x1b[7mc\x1b[0md{}│\x1b[?25l", " ".repeat(15)),
        )
        .live_snapshot();
        echo.typed(&before, "Ж界", now);
        let changed_suffix = terminal(
            24,
            &format!("│ > abЖ界\x1b[7mc\x1b[0mx{}│\x1b[?25l", " ".repeat(12)),
        )
        .live_snapshot();
        assert_eq!(echo.observe(&changed_suffix, now), None);
        let after = terminal(
            24,
            &format!("│ > abЖ界\x1b[7mc\x1b[0md{}│\x1b[?25l", " ".repeat(12)),
        )
        .live_snapshot();
        assert_echo(&mut echo, &after, now, "Ж界", 0, 9);
    }

    #[test]
    fn bordered_masks_and_concealment_disarm_echo_before_a_later_reveal() {
        for displayed in [
            "******",
            "••••••",
            "●●●●●●",
            "xxxxxx",
            "\x1b[8msecret\x1b[0m",
            "\x1b[30;40msecret\x1b[0m",
        ] {
            let now = Instant::now();
            let mut echo = InputEcho::default();
            echo.typed(&composer(2, "", 18), "secret", now);
            assert_eq!(echo.observe(&composer(2, displayed, 12), now), None);
            assert!(echo.pending.is_none());
            assert_eq!(echo.observe(&composer(2, "secret", 12), now), None);
            assert_eq!(echo.last_confirmed(&composer(2, "secret", 12)), None);
        }
    }

    #[test]
    fn repeated_labels_are_ambiguous_without_a_cursor_or_confirmed_anchor() {
        let before = terminal(20, "aa\x1b[?25l").live_snapshot();
        let after = terminal(20, "aaa\x1b[?25l").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "a", now);
        assert_eq!(echo.observe(&after, now), None);
        assert_eq!(echo.last_confirmed(&after), None);

        let before = terminal(20, "> \r\n> \x1b[?25l").live_snapshot();
        let after = terminal(20, "> a\r\n> a\x1b[?25l").live_snapshot();
        echo.typed(&before, "a", now + MAX_AGE);
        assert_eq!(echo.observe(&after, now + MAX_AGE), None);
    }

    #[test]
    fn a_valid_hidden_cursor_anchor_disambiguates_repeated_characters() {
        let mut model = terminal(20, "> \x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "a", now);
        model.push_bytes(b"a");
        assert_echo(&mut echo, &model.live_snapshot(), now, "a", 0, 3);
        echo.typed(&model.live_snapshot(), "a", now);
        model.push_bytes(b"a");
        assert_echo(&mut echo, &model.live_snapshot(), now, "a", 0, 4);
    }

    #[test]
    fn unicode_wide_combining_and_surrogates_return_the_real_cell_boundary() {
        let mut model = terminal(30, "> \x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        let input = "Ж界e\u{301}🙂";
        echo.typed(&model.live_snapshot(), input, now);
        model.push_bytes(input.as_bytes());
        let mut frame = model.live_snapshot();
        assert_echo(&mut echo, &frame, now, input, 0, 8);
        frame.cursor = Some(CursorSnapshot { row: 0, column: 8 });
        let document = model.reading_document();
        assert_eq!(
            crate::direct_accessibility::cursor_offset(&document, &frame),
            document.text.encode_utf16().count() as u32
        );
    }

    #[test]
    fn a_combining_mark_changes_its_base_cell_without_advancing_columns() {
        let mut model = terminal(10, "e");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "\u{301}", now);
        model.push_bytes("\u{301}".as_bytes());
        assert_echo(&mut echo, &model.live_snapshot(), now, "\u{301}", 0, 1);
    }

    #[test]
    fn a_wide_character_wraps_without_reading_a_leading_spacer_as_input() {
        let mut model = terminal(4, "abc\x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "界", now);
        model.push_bytes("界".as_bytes());
        let mut frame = model.live_snapshot();
        assert_echo(&mut echo, &frame, now, "界", 1, 2);
        frame.cursor = Some(CursorSnapshot { row: 1, column: 2 });
        assert_eq!(
            crate::direct_accessibility::cursor_offset(&model.reading_document(), &frame),
            4
        );
    }

    #[test]
    fn a_full_row_and_its_delayed_wrap_keep_the_insertion_point() {
        let mut model = terminal(4, "abc");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "d", now);
        model.push_bytes(b"d");
        assert_echo(&mut echo, &model.live_snapshot(), now, "d", 0, 4);
        echo.typed(&model.live_snapshot(), "e", now);
        model.push_bytes(b"e");
        assert_echo(&mut echo, &model.live_snapshot(), now, "e", 1, 1);
    }

    #[test]
    fn visible_space_and_hidden_space_followed_by_a_letter_are_confirmed() {
        let mut model = terminal(20, "a");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), " ", now);
        model.push_bytes(b" ");
        assert_echo(&mut echo, &model.live_snapshot(), now, " ", 0, 2);

        let mut model = terminal(20, "a\x1b[?25l");
        echo.cancel();
        echo.typed(&model.live_snapshot(), " ", now);
        model.push_bytes(b" ");
        assert_eq!(echo.observe(&model.live_snapshot(), now), None);
        echo.typed(&model.live_snapshot(), "b", now);
        model.push_bytes(b"b");
        assert_echo(&mut echo, &model.live_snapshot(), now, " b", 0, 3);
    }

    #[test]
    fn unchanged_hidden_passwords_and_masks_never_echo_raw_characters() {
        let before = terminal(20, "Password: \x1b[?25l").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "secret", now);
        assert_eq!(echo.observe(&before, now), None);
        let masked = terminal(20, "Password: ******\x1b[?25l").live_snapshot();
        assert_eq!(echo.observe(&masked, now), None);
        assert_eq!(echo.last_confirmed(&masked), None);
        assert_eq!(echo.observe(&masked, now + MAX_AGE), None);
        assert!(echo.pending.is_none());
    }

    #[test]
    fn hidden_cursor_transformed_insertions_cancel_before_a_later_reveal() {
        for suffix in ["", " end"] {
            for displayed in ["******", "••••••", "xxxxxx", "界🙂Ж"] {
                let now = Instant::now();
                let before = terminal(20, &format!("Password: {suffix}\x1b[?25l")).live_snapshot();
                let mut echo = InputEcho::default();
                echo.typed(&before, "secret", now);
                assert_eq!(echo.observe(&before, now), None);

                let transformed = terminal(20, &format!("Password: {displayed}{suffix}\x1b[?25l"))
                    .live_snapshot();
                assert_eq!(echo.observe(&transformed, now), None);
                assert!(echo.pending.is_none());
                let revealed =
                    terminal(20, &format!("Password: secret{suffix}\x1b[?25l")).live_snapshot();
                assert_eq!(
                    echo.observe(&revealed, now + Duration::from_millis(100)),
                    None
                );
                assert_eq!(echo.last_confirmed(&revealed), None);
            }
        }
    }

    #[test]
    fn concealed_or_equal_foreground_background_cells_cancel_the_echo() {
        for conceal in ["\x1b[8m", "\x1b[30;40m"] {
            let mut model = terminal(20, "> \x1b[?25l");
            let now = Instant::now();
            let mut echo = InputEcho::default();
            echo.typed(&model.live_snapshot(), "secret", now);
            model.push_bytes(format!("{conceal}secret\x1b[0m").as_bytes());
            assert_eq!(echo.observe(&model.live_snapshot(), now), None);
            assert!(echo.pending.is_none());
            model.push_bytes(b"\x1b[1;3H\x1b[0msecret");
            assert_eq!(echo.observe(&model.live_snapshot(), now), None);
        }
    }

    #[test]
    fn typing_in_a_concealed_frame_cancels_before_a_later_visible_poll() {
        for armed in [false, true] {
            let mut model = terminal(20, "> \x1b[?25l");
            let now = Instant::now();
            let mut echo = InputEcho::default();
            if armed {
                echo.typed(&model.live_snapshot(), "s", now);
            }
            model.push_bytes(b"\x1b[8ms");
            echo.typed(&model.live_snapshot(), "e", now);
            assert!(echo.pending.is_none());
            model.push_bytes(b"\x1b[1;3H\x1b[0mse");
            assert_eq!(echo.observe(&model.live_snapshot(), now), None);
            assert_eq!(echo.last_confirmed(&model.live_snapshot()), None);
        }
    }

    #[test]
    fn redraw_color_spinner_and_cursor_updates_cannot_echo_a_pending_character() {
        let before = terminal(30, "> \r\nWorking |\x1b[?25l").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "x", now);
        let mut color = before.clone();
        color.cells[0].fg = Color::Indexed(4);
        assert_eq!(echo.observe(&color, now), None);
        let spinner = terminal(30, "> \r\nWorking /\x1b[?25l").live_snapshot();
        assert_eq!(echo.observe(&spinner, now), None);
        let mut cursor = before.clone();
        cursor.cursor = Some(CursorSnapshot { row: 1, column: 3 });
        assert_eq!(echo.observe(&cursor, now), None);
        let actual = terminal(30, "> x\r\nWorking -\x1b[?25l").live_snapshot();
        assert_echo(&mut echo, &actual, now, "x", 0, 3);
    }

    #[test]
    fn a_visible_cursor_rejects_a_matching_insert_on_an_unrelated_status_row() {
        let before = terminal(30, "> \r\nStatus\x1b[1;3H").live_snapshot();
        let after = terminal(30, "> \r\nStatusx\x1b[1;3H").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "x", now);
        assert_eq!(echo.observe(&after, now), None);
    }

    #[test]
    fn anchor_survives_only_unchanged_rows_and_cancel_pending_keeps_it() {
        let mut model = terminal(20, "> \x1b[?25l");
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&model.live_snapshot(), "x", now);
        model.push_bytes(b"x");
        let confirmed = model.live_snapshot();
        assert_echo(&mut echo, &confirmed, now, "x", 0, 3);
        echo.typed(&confirmed, "y", now);
        echo.cancel_pending();
        assert_eq!(echo.last_confirmed(&confirmed), Some((0, 3)));
        let mut recolored = confirmed.clone();
        recolored.cells[2].fg = Color::Indexed(3);
        assert_eq!(echo.last_confirmed(&recolored), Some((0, 3)));
        model.push_bytes(b"\x1b[3;1Hother output");
        assert_eq!(echo.last_confirmed(&model.live_snapshot()), Some((0, 3)));
        model.push_bytes(b"\x1b[1;3Hz");
        assert_eq!(echo.last_confirmed(&model.live_snapshot()), None);
        echo.cancel();
        assert_eq!(echo.last_confirmed(&confirmed), None);
    }

    #[test]
    fn requests_expire_from_the_first_character_and_bounds_cancel_safely() {
        let before = terminal(20, "").live_snapshot();
        let after = terminal(20, "ab").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.typed(&before, "a", now);
        echo.typed(&before, "b", now + Duration::from_millis(600));
        assert_eq!(echo.observe(&after, now + MAX_AGE), None);
        echo.typed(&before, &"a".repeat(MAX_PENDING_CHARS + 1), now);
        assert!(echo.pending.is_none());
        echo.typed(&before, "\r", now);
        assert!(echo.pending.is_none());
        let mut scrolled = before.clone();
        scrolled.display_offset = 1;
        echo.typed(&scrolled, "a", now);
        assert!(echo.pending.is_none());
        let mut oversized = before.clone();
        oversized.rows = MAX_ROWS + 1;
        echo.typed(&oversized, "a", now);
        assert!(echo.pending.is_none());
    }

    #[test]
    fn geometry_alternate_screen_and_history_changes_discard_pending_echo() {
        let before = terminal(20, "").live_snapshot();
        let after = terminal(20, "a").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        let mut invalid = [after.clone(), after.clone(), after.clone(), after.clone()];
        invalid[0].cols += 1;
        invalid[1].alternate_screen = true;
        invalid[2].history_size += 1;
        invalid[3].display_offset = 1;
        for frame in invalid {
            echo.typed(&before, "a", now);
            assert_eq!(echo.observe(&frame, now), None);
            assert!(echo.pending.is_none());
        }
        assert_eq!(echo.observe(&after, now), None);
    }
}
