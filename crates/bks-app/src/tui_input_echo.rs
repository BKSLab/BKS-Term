//! Echo only text whose insertion is confirmed by a live VT frame.
//! The terminal remains the sole text authority; no speculative editing occurs.

use std::time::{Duration, Instant};

use crate::tui_projection::Composer;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// An explicit edit acknowledged by visible text and the live VT cursor.
/// Messages contain no deleted text, so even a displayed mask is not repeated.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfirmedEdit {
    pub message: String,
    pub row: usize,
    pub column: usize,
}

#[derive(Default)]
pub struct InputEcho {
    pending: Option<Pending>,
    editing: Option<Editing>,
    confirmed: Option<Anchor>,
}

struct Editing {
    before: Screen,
    origin: Point,
    action: EditAction,
    started: Instant,
}

struct Pending {
    before: Screen,
    expected: Vec<char>,
    origin: Option<Point>,
    started: Instant,
    empty_composer: Option<EmptyComposer>,
}

#[derive(Clone, Copy)]
struct EmptyComposer {
    adapter: crate::tui_adapters::AdapterKind,
    left: usize,
    right: usize,
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn typed(&mut self, frame: &TerminalSnapshot, text: &str, now: Instant) {
        self.typed_with_composer(frame, None, text, now);
    }

    /// The projector has already distinguished an empty editor from its hint.
    /// Use that fact only in the private baseline; speech still requires the
    /// child's actual visible glyphs and caret after this accepted key.
    pub fn typed_with_composer(
        &mut self,
        frame: &TerminalSnapshot,
        composer: Option<&Composer>,
        text: &str,
        now: Instant,
    ) {
        if text.is_empty() {
            return;
        }
        self.editing = None;
        // Check before extending an existing request: the concealed frame may
        // be observed by a key event and disappear again before the next poll.
        if frame.cols == 0
            || frame.cols > MAX_COLS
            || frame.rows == 0
            || frame.rows > MAX_ROWS
            || frame.cells.len() != frame.cols.saturating_mul(frame.rows)
            || composer.is_some_and(|input| input.protected)
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
                || !pending.accepts_geometry(frame)
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
        let empty_composer = composer.filter(|input| {
            !input.protected
                && input.text.is_empty()
                && input.caret == Some(0)
                && input.matches_source(frame)
        });
        let mut baseline = None;
        if let (Some(input), Some(cursor)) = (
            empty_composer,
            crate::direct_accessibility::reading_cursor(frame, None),
        ) {
            if (input.first_row..=input.last_row).contains(&cursor.row)
                && cursor.column >= input.first_column
                && cursor.column < input.last_column
                && input.last_column < frame.cols
            {
                let mut cleared = frame.clone();
                let right = &cleared.cells[cursor.row * frame.cols + input.last_column];
                let end = input.last_column
                    + usize::from(!matches!(right.text.as_str(), "│" | "┃" | "║" | "|"));
                for column in cursor.column..end {
                    let cell = &mut cleared.cells[cursor.row * frame.cols + column];
                    cell.c = ' ';
                    cell.text = " ".to_owned();
                    cell.flags.remove(
                        Flags::WIDE_CHAR
                            | Flags::WIDE_CHAR_SPACER
                            | Flags::LEADING_WIDE_CHAR_SPACER,
                    );
                }
                baseline = Some(cleared);
            }
        }
        let Some(before) = Screen::new(baseline.as_ref().unwrap_or(frame)) else {
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
            empty_composer: baseline.and(empty_composer).map(|input| EmptyComposer {
                adapter: input.adapter(),
                left: input.first_column,
                right: input.last_column,
            }),
        });
    }

    /// Acknowledged prefixes are returned once, without waiting for later keys.
    /// Concealed/masked input, unmatched redraws and expired requests stay quiet.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn observe(&mut self, frame: &TerminalSnapshot, now: Instant) -> Option<ConfirmedEcho> {
        self.observe_with_composer(frame, None, now)
    }

    pub fn observe_with_composer(
        &mut self,
        frame: &TerminalSnapshot,
        composer: Option<&Composer>,
        now: Instant,
    ) -> Option<ConfirmedEcho> {
        let mut pending = self.pending.take()?;
        if now.saturating_duration_since(pending.started) >= MAX_AGE
            || !pending.accepts_geometry(frame)
        {
            return None;
        }
        let current = Screen::new(frame)?;
        // Public VT projections already redact concealed text to spaces.
        // Its absence cannot leave a raw typing request armed for a later
        // redraw which reveals the same input. Prefer silence for this frame.
        if current.concealed || composer.is_some_and(|input| input.protected) {
            self.cancel();
            return None;
        }
        let source_insertion = composer
            .filter(|input| input.matches_source(frame))
            .map_or(Ok(None), |input| {
                confirmed_composer_prefix(&pending, &current, input)
            });
        let insertion = match source_insertion.and_then(|source| {
            source.map_or_else(
                || {
                    find_insertion(&pending, &current).map(|insertion| {
                        // History growth can move the whole editor. Only its fresh
                        // source projection may confirm this kind of acknowledgement.
                        pending
                            .before
                            .geometry
                            .matches(frame)
                            .then_some(insertion)
                            .flatten()
                    })
                },
                |insertion| Ok(Some(insertion)),
            )
        }) {
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
        pending.empty_composer = None;
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

    /// Arm only immediately before forwarding the user's unmodified edit key.
    /// A key alone never supplies a new cursor position or an edit result.
    pub fn edit(&mut self, frame: &TerminalSnapshot, action: EditAction, now: Instant) {
        self.cancel_pending();
        let before = Screen::new(frame);
        self.confirmed = None;
        let Some(before) = before.filter(|screen| !screen.concealed) else {
            return;
        };
        let Some(origin) = before.cursor else {
            return;
        };
        if line_at(&before, origin)
            .is_none_or(|line| line.glyphs.iter().any(|glyph| !glyph.visible))
        {
            return;
        }
        self.editing = Some(Editing {
            before,
            origin,
            action,
            started: now,
        });
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some() || self.editing.is_some()
    }

    /// Announce only a precise deletion at the old cursor, or a real cursor
    /// movement on unchanged logical text. Unrelated redraws remain silent.
    pub fn observe_edit(
        &mut self,
        frame: &TerminalSnapshot,
        now: Instant,
    ) -> Option<ConfirmedEdit> {
        let editing = self.editing.take()?;
        if now.saturating_duration_since(editing.started) >= MAX_AGE
            || !editing.before.geometry.matches(frame)
        {
            return None;
        }
        let current = Screen::new(frame)?;
        if current.concealed {
            self.cancel();
            return None;
        }
        let Some(end) = current.cursor else {
            if unchanged_edit_point(&editing, &current, editing.origin) {
                self.editing = Some(editing);
            }
            return None;
        };
        if line_at(&current, end).is_none_or(|line| line.glyphs.iter().any(|glyph| !glyph.visible))
        {
            self.cancel();
            return None;
        }
        let message = match confirmed_edit(&editing, &current, end) {
            Some(message) => message,
            None => {
                // Only an unchanged input point may still be waiting for the
                // key. A transformed/masked or incompatible redraw disarms the
                // request before a later reveal can look like an old edit.
                if unchanged_edit_point(&editing, &current, end) {
                    self.editing = Some(editing);
                }
                return None;
            }
        };
        self.confirmed = Some(Anchor {
            geometry: current.geometry,
            point: end,
            row: current.rows[end.row].clone(),
        });
        Some(ConfirmedEdit {
            message,
            row: end.row,
            column: end.column,
        })
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
        self.editing = None;
    }

    /// Call on focus, session or reading-mode changes.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.editing = None;
        self.confirmed = None;
    }
}

impl Pending {
    fn accepts_geometry(&self, frame: &TerminalSnapshot) -> bool {
        self.before.geometry.matches(frame)
            || (self
                .empty_composer
                .is_some_and(|input| input.adapter == crate::tui_adapters::AdapterKind::Codex)
                && self.before.geometry.same_viewport(frame)
                && self.before.geometry.history <= frame.history_size)
    }
}

impl Geometry {
    fn same_viewport(self, frame: &TerminalSnapshot) -> bool {
        frame.display_offset == 0
            && self.cols == frame.cols
            && self.rows == frame.rows
            && self.alternate == frame.alternate_screen
    }

    fn matches(self, frame: &TerminalSnapshot) -> bool {
        frame.display_offset == 0
            && self.cols == frame.cols
            && self.rows == frame.rows
            && self.alternate == frame.alternate_screen
            && self.history == frame.history_size
    }
}

fn confirmed_composer_prefix(
    pending: &Pending,
    current: &Screen,
    input: &Composer,
) -> Result<Option<Insertion>, ()> {
    let Some(empty) = pending.empty_composer else {
        return Ok(None);
    };
    let count = input.text.chars().count();
    if input.protected
        || input.adapter() != empty.adapter
        || input.first_column != empty.left
        || input.last_column != empty.right
        || count == 0
        || count > pending.expected.len()
        || !input
            .text
            .chars()
            .eq(pending.expected[..count].iter().copied())
        || input.caret != Some(input.text.encode_utf16().count() as u32)
    {
        return Ok(None);
    }
    let Some(end) = current
        .cursor
        .filter(|end| (input.first_row..=input.last_row).contains(&end.row))
    else {
        return Ok(None);
    };
    let Some(line) = line_at(current, end) else {
        return Ok(None);
    };
    let stop = caret_index(line, end);
    let Some(start) = stop.checked_sub(count) else {
        return Ok(None);
    };
    let inserted = &line.glyphs[start..stop];
    if !inserted
        .iter()
        .map(|glyph| glyph.ch)
        .eq(pending.expected[..count].iter().copied())
    {
        return Ok(None);
    }
    if inserted.iter().any(|glyph| !glyph.visible) {
        return Err(());
    }
    Ok(Some(Insertion { count, end }))
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

fn confirmed_edit(editing: &Editing, current: &Screen, end: Point) -> Option<String> {
    let before = line_at(&editing.before, editing.origin)?;
    let after = line_at(current, end)?;
    let start = caret_index(before, editing.origin);
    let stop = caret_index(after, end);
    // A concealed/indistinguishable part of the input must not become an edit
    // announcement when another redraw makes it visible later.
    if before
        .glyphs
        .iter()
        .chain(&after.glyphs)
        .any(|glyph| !glyph.visible)
    {
        return None;
    }
    let deleted = match editing.action {
        EditAction::Backspace if stop < start && start <= before.glyphs.len() => {
            let removed = &before.glyphs[stop..start];
            one_cell(removed)
                && same_text(&before.glyphs[..stop], &after.glyphs[..stop])
                && same_text_without_blank_fill(&after.glyphs[stop..], &before.glyphs[start..])
        }
        EditAction::Delete if stop == start => {
            let first = before.glyphs.get(start)?;
            let cell_end =
                before.glyphs[start..].partition_point(|glyph| glyph.start == first.start) + start;
            same_text(&before.glyphs[..start], &after.glyphs[..stop])
                && (start + 1..=cell_end).any(|removed_end| {
                    same_text_without_blank_fill(
                        &after.glyphs[stop..],
                        &before.glyphs[removed_end..],
                    )
                })
        }
        EditAction::Left | EditAction::Home if stop < start => {
            return movement_message(before, after, stop);
        }
        EditAction::Right | EditAction::End if stop > start => {
            return movement_message(before, after, stop);
        }
        _ => false,
    };
    deleted.then(|| "Удалено".to_owned())
}

fn one_cell(glyphs: &[Glyph]) -> bool {
    glyphs
        .first()
        .is_some_and(|first| glyphs.iter().all(|glyph| glyph.start == first.start))
}

fn movement_message(before: &Line, after: &Line, stop: usize) -> Option<String> {
    if !same_logical_text(before, after) {
        return None;
    }
    let mut previous = None;
    let position = after.glyphs[..stop]
        .iter()
        .filter(|glyph| {
            let new_cell = previous != Some(glyph.start);
            previous = Some(glyph.start);
            new_cell
        })
        .count()
        + 1;
    Some(format!("Позиция {position}"))
}

fn unchanged_edit_point(editing: &Editing, current: &Screen, end: Point) -> bool {
    let (Some(before), Some(after)) = (
        line_at(&editing.before, editing.origin),
        line_at(current, end),
    ) else {
        return false;
    };
    same_logical_text(before, after)
        && caret_index(before, editing.origin) == caret_index(after, end)
}

fn same_logical_text(before: &Line, after: &Line) -> bool {
    let trim = |glyphs: &[Glyph]| {
        glyphs
            .iter()
            .rposition(|glyph| glyph.ch != ' ')
            .map_or(0, |index| index + 1)
    };
    same_text(
        &before.glyphs[..trim(&before.glyphs)],
        &after.glyphs[..trim(&after.glyphs)],
    )
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

    fn empty_codex_with_hint() -> (TerminalModel, Composer) {
        let mut model = TerminalModel::new(40, 6);
        for row in 2..=4 {
            model.push_bytes(
                format!("\x1b[{row};1H\x1b[48;2;40;40;40m{}", " ".repeat(40)).as_bytes(),
            );
        }
        model.push_bytes("\x1b[3;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[3;3H".as_bytes());
        let frame = model.live_snapshot();
        let input = crate::tui_projection::project_for_adapter(
            crate::tui_adapters::AdapterKind::Codex,
            &frame,
            &model.tui_reading_document(),
            None,
            None,
        )
        .input
        .expect("a bounded empty source field");
        assert!(input.text.is_empty());
        assert_eq!(input.caret, Some(0));
        (model, input)
    }

    #[test]
    fn a_confirmed_empty_composer_replaces_its_hint_without_losing_or_repeating_echo() {
        let (mut model, input) = empty_codex_with_hint();
        let now = Instant::now();
        let before = model.live_snapshot();
        let mut echo = InputEcho::default();
        echo.typed_with_composer(&before, Some(&input), "Ж", now);
        assert_eq!(echo.observe_with_composer(&before, Some(&input), now), None);
        model.push_bytes("\x1b[3;3H\x1b[0m\x1b[48;2;40;40;40mЖ\x1b[K".as_bytes());
        assert_echo(&mut echo, &model.live_snapshot(), now, "Ж", 2, 3);
        echo.typed(&model.live_snapshot(), "a", now);
        model.push_bytes(b"a");
        assert_echo(&mut echo, &model.live_snapshot(), now, "a", 2, 4);
    }

    #[test]
    fn replacing_a_hint_cannot_echo_masked_or_concealed_text_after_a_later_reveal() {
        for concealment in ["mask", "hidden", "indistinguishable"] {
            let (mut model, input) = empty_codex_with_hint();
            let now = Instant::now();
            let before = model.live_snapshot();
            let mut echo = InputEcho::default();
            echo.typed_with_composer(&before, Some(&input), "a", now);
            model.push_bytes(b"\x1b[3;3H\x1b[0m\x1b[48;2;40;40;40m");
            model.push_bytes(match concealment {
                "mask" => b"*\x1b[K",
                "hidden" => b"\x1b[8ma\x1b[K",
                _ => b"\x1b[38;2;40;40;40ma\x1b[K",
            });
            assert_eq!(echo.observe(&model.live_snapshot(), now), None);
            assert!(!echo.has_pending(), "{concealment}");
            model.push_bytes(b"\x1b[3;3H\x1b[0m\x1b[48;2;40;40;40ma\x1b[K");
            assert_eq!(echo.observe(&model.live_snapshot(), now), None);
        }
    }

    #[test]
    fn an_unproven_hint_and_changed_source_do_not_supply_typing_speech() {
        let (mut model, input) = empty_codex_with_hint();
        let now = Instant::now();
        let before = model.live_snapshot();
        let mut unknown = InputEcho::default();
        unknown.typed_with_composer(&before, None, "a", now);
        let mut echo = InputEcho::default();
        echo.typed_with_composer(&before, Some(&input), "a", now);
        model.push_bytes(b"\x1b[3;3H\x1b[0m\x1b[48;2;40;40;40ma\x1b[K");
        let after = model.live_snapshot();
        assert_eq!(unknown.observe(&after, now), None);
        let mut alternate = after.clone();
        alternate.alternate_screen = !alternate.alternate_screen;
        assert_eq!(echo.observe(&alternate, now), None);
        assert!(!echo.has_pending());
        echo.typed_with_composer(&before, Some(&input), "a", now);
        assert_eq!(echo.observe(&after, now + MAX_AGE), None);
    }

    #[test]
    fn protected_composer_metadata_alone_cancels_typing_speech() {
        let (model, input) = empty_codex_with_hint();
        let frame = model.live_snapshot();
        let now = Instant::now();
        let mut protected = input.clone();
        protected.protected = true;
        let mut echo = InputEcho::default();
        echo.typed_with_composer(&frame, Some(&protected), "a", now);
        assert!(!echo.has_pending());
        echo.typed_with_composer(&frame, Some(&input), "a", now);
        assert!(echo.has_pending());
        assert_eq!(
            echo.observe_with_composer(&frame, Some(&protected), now),
            None
        );
        assert!(!echo.has_pending());
    }

    #[test]
    fn edits_require_the_visible_deletion_and_report_once_without_deleted_text() {
        let before = terminal(20, "> ab🙂e\u{301}").live_snapshot();
        let after = terminal(20, "> ab🙂").live_snapshot();
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.edit(&before, EditAction::Backspace, now);
        assert!(echo.has_pending());
        assert_eq!(echo.observe_edit(&before, now), None);
        let result = echo.observe_edit(&after, now).unwrap();
        assert_eq!(result.message, "Удалено");
        assert_eq!((result.row, result.column), (0, 6));
        assert_eq!(echo.last_confirmed(&after), Some((0, 6)));
        assert_eq!(echo.observe_edit(&after, now), None);
        assert!(!echo.has_pending());
    }

    #[test]
    fn delete_confirms_wide_and_combining_cells_without_moving_the_cursor() {
        let now = Instant::now();
        for (old, new) in [("🙂", ""), ("e\u{301}", "")] {
            let before = terminal(20, &format!("> a{old}b\x1b[4G")).live_snapshot();
            let after = terminal(20, &format!("> a{new}b\x1b[4G")).live_snapshot();
            let mut echo = InputEcho::default();
            echo.edit(&before, EditAction::Delete, now);
            let result = echo.observe_edit(&after, now).unwrap();
            assert_eq!(result.message, "Удалено");
            assert_eq!((result.row, result.column), (0, 3));
        }
    }

    #[test]
    fn removing_only_a_combining_mark_is_confirmed_at_the_same_screen_position() {
        let before = terminal(20, "> e\u{301}").live_snapshot();
        let after = terminal(20, "> e").live_snapshot();
        assert_eq!(before.cursor, after.cursor);
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.edit(&before, EditAction::Backspace, now);
        assert_eq!(echo.observe_edit(&after, now).unwrap().message, "Удалено");
    }

    #[test]
    fn padded_software_cursor_and_soft_wrapped_input_confirm_deletion() {
        let now = Instant::now();
        let mut echo = InputEcho::default();
        echo.edit(&composer(2, "ab", 16), EditAction::Backspace, now);
        let after = composer(2, "a", 17);
        assert_eq!(echo.observe_edit(&after, now).unwrap().message, "Удалено");
        assert_eq!(echo.last_confirmed(&after), Some((2, 5)));

        let mut terminal = terminal(6, "> ab🙂e\u{301}");
        let before = terminal.live_snapshot();
        echo.edit(&before, EditAction::Backspace, now);
        terminal.push_bytes(b"\x1b[2;1H\x1b[K");
        let result = echo.observe_edit(&terminal.live_snapshot(), now).unwrap();
        assert_eq!(result.message, "Удалено");
        assert_eq!((result.row, result.column), (1, 0));
    }

    #[test]
    fn cursor_motion_reports_only_the_observed_position_on_unchanged_text() {
        let now = Instant::now();
        for (action, first, last, expected) in [
            (EditAction::Left, 8, 7, "Позиция 6"),
            (EditAction::Right, 7, 8, "Позиция 7"),
            (EditAction::Home, 8, 3, "Позиция 3"),
            (EditAction::End, 3, 8, "Позиция 7"),
        ] {
            let before = terminal(24, &format!("> a🙂e\u{301} b\x1b[{first}G")).live_snapshot();
            let after = terminal(24, &format!("> a🙂e\u{301} b\x1b[{last}G")).live_snapshot();
            let mut echo = InputEcho::default();
            echo.edit(&before, action, now);
            assert_eq!(echo.observe_edit(&before, now), None);
            let result = echo.observe_edit(&after, now).unwrap();
            assert_eq!(result.message, expected);
            assert_eq!(echo.observe_edit(&after, now), None);
        }
    }

    #[test]
    fn redraws_wrong_direction_and_replaced_suffixes_do_not_confirm_edit_keys() {
        let now = Instant::now();
        let before = terminal(20, "> abcd\x1b[5G").live_snapshot();
        for (action, next) in [
            (EditAction::Backspace, "> aXcd\x1b[4G"),
            (EditAction::Delete, "> abXd\x1b[5G"),
            (EditAction::Left, "> abcd\x1b[6G"),
            (EditAction::Right, "> abcd\x1b[4G"),
            (EditAction::Home, "> axcd\x1b[3G"),
            (EditAction::End, "> abcx\x1b[7G"),
        ] {
            let mut echo = InputEcho::default();
            echo.edit(&before, action, now);
            assert_eq!(
                echo.observe_edit(&terminal(20, next).live_snapshot(), now),
                None
            );
        }
    }

    #[test]
    fn edit_requests_disarm_on_concealment_before_a_later_reveal() {
        let now = Instant::now();
        let plain = terminal(20, "> abc").live_snapshot();
        let after = terminal(20, "> ab").live_snapshot();
        for escape in ["\x1b[8m", "\x1b[30;40m"] {
            let hidden = terminal(20, &format!("> {escape}abc\x1b[0m")).live_snapshot();
            let mut echo = InputEcho::default();
            echo.edit(&hidden, EditAction::Backspace, now);
            assert!(!echo.has_pending());
            assert_eq!(echo.observe_edit(&after, now), None);
            echo.edit(&plain, EditAction::Backspace, now);
            assert_eq!(echo.observe_edit(&hidden, now), None);
            assert!(!echo.has_pending());
            assert_eq!(echo.observe_edit(&after, now), None);
        }
        // Displayed masks need no semantic guessing: their incompatible text
        // change disarms the old key, and no deleted value is ever announced.
        for mask in ["**", "••", "●●", "xx"] {
            let mut echo = InputEcho::default();
            echo.edit(&plain, EditAction::Backspace, now);
            let masked = terminal(20, &format!("> {mask}")).live_snapshot();
            assert_eq!(echo.observe_edit(&masked, now), None);
            assert!(!echo.has_pending());
            assert_eq!(echo.observe_edit(&after, now), None);
        }
    }

    #[test]
    fn edits_expire_and_focus_geometry_or_new_input_cancel_the_old_request() {
        let now = Instant::now();
        let before = terminal(20, "> abc").live_snapshot();
        let after = terminal(20, "> ab").live_snapshot();
        let mut echo = InputEcho::default();
        echo.edit(&before, EditAction::Backspace, now);
        assert_eq!(echo.observe_edit(&after, now + MAX_AGE), None);
        assert!(!echo.has_pending());
        echo.edit(&before, EditAction::Backspace, now);
        let resized = terminal(21, "> ab").live_snapshot();
        assert_eq!(echo.observe_edit(&resized, now), None);
        assert!(!echo.has_pending());
        echo.edit(&before, EditAction::Backspace, now);
        echo.cancel();
        assert_eq!(echo.observe_edit(&after, now), None);
        echo.edit(&before, EditAction::Backspace, now);
        echo.typed(&before, "x", now);
        assert_eq!(echo.observe_edit(&after, now), None);
    }

    #[test]
    fn unknown_cursors_and_multi_cell_deletion_do_not_guess_an_edit() {
        let now = Instant::now();
        let before = terminal(20, "> abc\x1b[?25l").live_snapshot();
        let mut echo = InputEcho::default();
        echo.edit(&before, EditAction::Backspace, now);
        assert!(!echo.has_pending());
        let before = terminal(20, "> abc").live_snapshot();
        echo.edit(&before, EditAction::Backspace, now);
        assert_eq!(
            echo.observe_edit(&terminal(20, "> a").live_snapshot(), now),
            None
        );
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
