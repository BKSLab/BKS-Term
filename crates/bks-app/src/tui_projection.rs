//! A conservative text projection of one active, visible terminal composer.
//! Detection never supplies actions, keys, commands, or application identities.

use bks_terminal::{
    Color, CursorSnapshot, Flags, ReadingDocument, ReadingLine, StyleSpan, TerminalSnapshot,
    TextStyle,
};

const MAX_COLUMNS: usize = 1_024;
const MAX_ROWS: usize = 256;
const MAX_FIELD_ROWS: usize = 32;
const MAX_FIELD_UNITS: usize = 32_768;
const MAX_BOX_CANDIDATES: usize = 128;
const MIN_RULE_COLUMNS: usize = 8;

#[cfg(test)]
#[path = "codex_projection_tests.rs"]
mod codex_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Bare,
    Box,
    Rules,
    Panel,
    Padded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composer {
    pub text: String,
    pub caret: Option<u32>,
    /// Inclusive physical screen rows, including the box and prompt chrome.
    pub first_row: usize,
    pub last_row: usize,
    pub first_column: usize,
    pub last_column: usize,
    pub protected: bool,
    shell: bool,
    boundary: Boundary,
    source: Source,
    anchor: CursorSnapshot,
    panel: Option<PanelProof>,
    adapter: crate::tui_adapters::AdapterKind,
}

impl Composer {
    pub fn adapter(&self) -> crate::tui_adapters::AdapterKind {
        self.adapter
    }

    pub fn accepts_requests(&self) -> bool {
        (self.adapter != crate::tui_adapters::AdapterKind::OpenCode || !self.shell)
            && self
                .panel
                .as_ref()
                .is_none_or(|proof| !proof.partial || proof.intent.is_some())
    }

    /// Explicit user-input intent used only to bind a later submitted request.
    /// A clipped editor still publishes only `text`, the actual source window.
    pub fn request_text(&self) -> &str {
        self.panel
            .as_ref()
            .and_then(|proof| proof.intent.as_ref())
            .map_or(&self.text, |intent| intent.text.as_str())
    }

    pub fn request_caret(&self) -> Option<u32> {
        self.panel
            .as_ref()
            .and_then(|proof| proof.intent.as_ref())
            .map_or(self.caret, |intent| Some(intent.caret))
    }

    pub fn is_partial(&self) -> bool {
        self.panel.as_ref().is_some_and(|proof| proof.partial)
    }

    pub(crate) fn uses_alternate_screen(&self) -> bool {
        self.source.alternate
    }

    pub fn matches_source(&self, frame: &TerminalSnapshot) -> bool {
        self.source == Source::of(frame)
    }

    pub(crate) fn matches_buffer(&self, frame: &TerminalSnapshot) -> bool {
        self.source.alternate == frame.alternate_screen
            && (self.adapter == crate::tui_adapters::AdapterKind::Codex
                || self.source.history == frame.history_size)
    }

    pub(crate) fn permits_codex_repaint(&self, frame: &TerminalSnapshot) -> bool {
        self.adapter == crate::tui_adapters::AdapterKind::Codex
            && self.boundary == Boundary::Padded
            && !self.protected
            && self.panel.is_some()
            && self.source.columns == frame.cols
            && self.source.rows == frame.rows
            && self.source.alternate == frame.alternate_screen
            && self.source.history <= frame.history_size
    }
}

/// Explicit text accepted by the PTY writer, including normalized line breaks
/// from a complete bracketed paste. This is an origin and
/// expected prefix, not displayed text: only a newer matching source frame can
/// confirm it. Callers bound its length/age and reset it on control/navigation.
#[derive(Clone, Debug)]
pub struct AcceptedSourceEvidence {
    pub text: String,
    pub row: usize,
    pub column: usize,
    pub before_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcceptedEditKind {
    Backspace,
    Delete,
}

#[derive(Clone, Debug)]
pub struct AcceptedEditEvidence {
    kind: AcceptedEditKind,
    before_revision: u64,
    adapter: crate::tui_adapters::AdapterKind,
    source: Source,
    bounds: (usize, usize, usize, usize),
    text: String,
    caret: u32,
    palette: Vec<TextStyle>,
}

impl AcceptedEditEvidence {
    pub(crate) fn record(
        kind: AcceptedEditKind,
        frame: &TerminalSnapshot,
        revision: u64,
        baseline: &Composer,
    ) -> Option<Self> {
        let caret = baseline.caret?;
        if !valid_frame(frame)
            || !baseline.matches_source(frame)
            || frame.cursor != Some(baseline.anchor)
            || baseline.protected
            || baseline.boundary == Boundary::Bare
            || baseline
                .panel
                .as_ref()
                .is_some_and(|proof| !proof.acknowledged)
            || (kind == AcceptedEditKind::Backspace && caret == 0)
            || (kind == AcceptedEditKind::Delete
                && caret as usize >= baseline.text.encode_utf16().count())
        {
            return None;
        }
        let content = composer_content(frame, baseline)?;
        let observed = compose(frame, baseline.anchor, composer_bounds(baseline), content)?;
        let canonical = baseline.adapter == crate::tui_adapters::AdapterKind::OpenCode
            && baseline
                .panel
                .as_ref()
                .is_some_and(|proof| proof.rendered == panel_rendered(frame, proof))
            && observed.caret.and_then(|caret| {
                crate::opencode::editor_caret(&observed.text, &baseline.text, caret)
            }) == baseline.caret;
        if observed.protected
            || (!canonical && (observed.text != baseline.text || observed.caret != baseline.caret))
        {
            return None;
        }
        Some(Self {
            kind,
            before_revision: revision,
            adapter: baseline.adapter,
            source: baseline.source,
            bounds: (
                baseline.first_row,
                baseline.last_row,
                baseline.first_column,
                baseline.last_column,
            ),
            text: baseline.text.clone(),
            caret,
            palette: content_palette(frame, &content)?,
        })
    }

    pub(crate) fn kind(&self) -> AcceptedEditKind {
        self.kind
    }

    pub(crate) fn matches_result(&self, input: &Composer) -> bool {
        let bounds = (
            input.first_row,
            input.last_row,
            input.first_column,
            input.last_column,
        );
        let same_bounds = if self.adapter == crate::tui_adapters::AdapterKind::OpenCode
            && input.boundary == Boundary::Panel
        {
            (bounds.2, bounds.3) == (self.bounds.2, self.bounds.3)
        } else {
            bounds == self.bounds
        };
        if input.protected
            || input.adapter != self.adapter
            || input.source != self.source
            || !same_bounds
        {
            return false;
        }
        let Some(caret) = input.caret else {
            return false;
        };
        let old: Vec<_> = self.text.encode_utf16().collect();
        let current: Vec<_> = input.text.encode_utf16().collect();
        let (first, last) = match self.kind {
            AcceptedEditKind::Backspace if caret < self.caret => {
                (caret as usize, self.caret as usize)
            }
            AcceptedEditKind::Delete if caret == self.caret && current.len() < old.len() => {
                (caret as usize, caret as usize + old.len() - current.len())
            }
            _ => return false,
        };
        // The source reports how much its editor removed. Do not predict a
        // scalar/grapheme/CRLF count from one key or from UTF-16 key repeats.
        first < last
            && last <= old.len()
            && String::from_utf16(&old[..first]).is_ok()
            && String::from_utf16(&old[last..]).is_ok()
            && current
                .iter()
                .copied()
                .eq(old[..first].iter().chain(&old[last..]).copied())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PanelProof {
    first: usize,
    last: usize,
    start: usize,
    end: usize,
    background: Color,
    edge: PanelEdge,
    acknowledged: bool,
    rendered: Vec<(String, Color, Flags)>,
    palette: Vec<TextStyle>,
    revision: u64,
    insertion: Option<PanelInsertion>,
    partial: bool,
    intent: Option<LiteralIntent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiteralIntent {
    text: String,
    caret: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PanelEdge {
    Gutter(char, Color),
    Prompt(char, Color),
    Rule(char, Color),
    Box(char, Color),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PanelInsertion {
    text: String,
    caret: u32,
    origin: CursorSnapshot,
    before_revision: u64,
}

#[derive(Clone, Debug)]
pub struct Projection {
    pub input: Option<Composer>,
    pub output: ReadingDocument,
}

/// Presentation for the native TUI viewport in both input and reading modes.
/// Canonical search/selection offsets are retained separately from layout
/// padding; a physical TUI source keeps the same rows across a focus change.
pub struct VisualDocument {
    pub document: ReadingDocument,
    offsets: Vec<VisualOffset>,
    line_starts: Vec<(u32, u32)>,
    source_length: u32,
}

struct VisualOffset {
    source_start: u32,
    source_end: u32,
    visual_start: u32,
    leading_break: bool,
}

impl VisualDocument {
    /// Inserted layout whitespace belongs to the preceding canonical offset.
    pub fn source_offset(&self, offset: u32) -> u32 {
        let index = self.offsets.partition_point(|span| {
            span.visual_start + span.source_end - span.source_start <= offset
        });
        if let Some(span) = self.offsets.get(index) {
            if offset >= span.visual_start {
                return span.source_start + offset - span.visual_start;
            }
        }
        index
            .checked_sub(1)
            .map_or(0, |index| self.offsets[index].source_end)
            .min(self.source_length)
    }

    pub fn visual_offset(&self, offset: u32) -> u32 {
        let offset = offset.min(self.source_length);
        if let Ok(index) = self
            .line_starts
            .binary_search_by_key(&offset, |(source, _)| *source)
        {
            return self.line_starts[index].1;
        }
        let index = self
            .offsets
            .partition_point(|span| span.source_end <= offset);
        if let Some(span) = self.offsets.get(index) {
            if offset >= span.source_start {
                if offset == span.source_start && span.leading_break {
                    // A selection ending before canonical CRLF must stop at
                    // its last glyph, before the added colored row padding.
                    if let Some(previous) = index
                        .checked_sub(1)
                        .and_then(|index| self.offsets.get(index))
                    {
                        return previous.visual_start + previous.source_end - previous.source_start;
                    }
                }
                return span.visual_start + offset - span.source_start;
            }
        }
        index.checked_sub(1).map_or(0, |index| {
            let span = &self.offsets[index];
            span.visual_start + span.source_end - span.source_start
        })
    }

    /// Follow the live page, rather than scrolling its header off-screen to fit
    /// a source viewport that may also include the separate shared input area.
    pub fn live_start(&self) -> u32 {
        self.document
            .lines
            .iter()
            .find(|line| line.line >= 0)
            .map_or_else(
                || self.document.lines.last().map_or(0, |line| line.end),
                |line| line.start,
            )
    }

    fn unchanged(document: &ReadingDocument) -> Self {
        let source_length = document.text.encode_utf16().count() as u32;
        Self {
            document: document.clone(),
            offsets: vec![VisualOffset {
                source_start: 0,
                source_end: source_length,
                visual_start: 0,
                leading_break: false,
            }],
            line_starts: Vec::new(),
            source_length,
        }
    }
}

/// Keep physical rows in the live presentation: RichEdit word wrapping drops
/// long runs of layout spaces in soft-wrapped VT paragraphs. Rows removed by
/// composer/menu projection stay removed. Retained history stays logical.
/// `excluded_rows` supplies those known removals when restoring an otherwise
/// unanchored, completely empty last row of a soft-wrapped paragraph.
pub fn visual_document(
    frame: &TerminalSnapshot,
    source: &ReadingDocument,
    excluded_rows: &[(usize, usize)],
) -> VisualDocument {
    visual_document_inner(frame, source, excluded_rows)
        .unwrap_or_else(|| VisualDocument::unchanged(source))
}

fn visual_document_inner(
    frame: &TerminalSnapshot,
    source: &ReadingDocument,
    excluded_rows: &[(usize, usize)],
) -> Option<VisualDocument> {
    if !valid_frame(frame) || frame.alternate_screen != source.alternate_screen {
        return None;
    }
    let source_units: Vec<_> = source.text.encode_utf16().collect();
    let mut rows: Vec<_> = source
        .lines
        .iter()
        .map(|line| line.line)
        .chain(source.cells.iter().map(|cell| cell.line))
        .filter(|row| *row >= 0 && (*row as usize) < frame.rows)
        .collect();
    // A paragraph ending in an empty physical row has no glyph/line anchor
    // for that final row. The preceding WRAPLINE proves it is part of the same
    // paragraph. Only restore blank padding, never text removed by projection.
    for (index, line) in source.lines.iter().enumerate() {
        let end = source.cells.partition_point(|cell| cell.end <= line.end);
        let Some(last) = end
            .checked_sub(1)
            .and_then(|index| source.cells.get(index))
            .filter(|cell| cell.start >= line.start && cell.line >= 0)
        else {
            continue;
        };
        let row = last.line as usize + 1;
        if row >= frame.rows
            || source
                .lines
                .get(index + 1)
                .is_some_and(|next| next.line <= row as i32)
            || excluded_rows
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&row))
            || !frame.cells[row * frame.cols - 1]
                .flags
                .contains(Flags::WRAPLINE)
        {
            continue;
        }
        if frame.cells[row * frame.cols..(row + 1) * frame.cols]
            .iter()
            .all(|cell| {
                cell.text
                    .chars()
                    .all(|character| matches!(character, ' ' | '\t'))
            })
        {
            rows.push(row as i32);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    let first_row = *rows.first()?;
    let prefix_end = source
        .lines
        .iter()
        .filter(|line| line.line >= 0)
        .map(|line| line.start)
        .chain(
            source
                .cells
                .iter()
                .filter(|cell| cell.line >= 0)
                .map(|cell| cell.start),
        )
        .min()?;
    let mut output = VisualBuilder {
        units: Vec::new(),
        view: VisualDocument {
            document: ReadingDocument {
                revision: source.revision,
                alternate_screen: source.alternate_screen,
                text: String::new(),
                cells: Vec::new(),
                lines: Vec::new(),
                styles: Vec::new(),
            },
            offsets: Vec::new(),
            line_starts: Vec::new(),
            source_length: source_units.len().try_into().ok()?,
        },
    };
    output.copy_source(source, &source_units, 0, prefix_end)?;
    output.view.document.cells.extend(
        source
            .cells
            .iter()
            .filter(|cell| cell.line < 0 && cell.end <= prefix_end)
            .copied(),
    );
    output.view.document.lines.extend(
        source
            .lines
            .iter()
            .filter(|line| line.line < 0 && line.start < prefix_end)
            .map(|line| ReadingLine {
                end: line.end.min(prefix_end),
                ..*line
            }),
    );
    for row in rows {
        if row != first_row || (prefix_end != 0 && !output.units.ends_with(&[13, 10])) {
            let original_break = source
                .lines
                .iter()
                .find(|line| line.line == row)
                .map(|line| line.start)
                .filter(|start| {
                    *start >= 2
                        && source_units.get(*start as usize - 2..*start as usize) == Some(&[13, 10])
                });
            if let Some(end) = original_break {
                output.copy_source(source, &source_units, end - 2, end)?;
            } else {
                output.literal(&[13, 10], TextStyle::default());
            }
        }
        let line_start = output.units.len() as u32;
        if let Some(original) = source.lines.iter().find(|line| line.line == row) {
            output.view.line_starts.push((original.start, line_start));
        }
        let begin = source.cells.partition_point(|cell| cell.line < row);
        let end = source.cells.partition_point(|cell| cell.line <= row);
        let source_cells = &source.cells[begin..end];
        let frame_cells = &frame.cells[row as usize * frame.cols..(row as usize + 1) * frame.cols];
        let painted_end = frame_cells.iter().rposition(|cell| {
            !cell.flags.contains(Flags::HIDDEN)
                && (cell.bg != Color::Named(bks_terminal::NamedColor::Background)
                    || cell
                        .flags
                        .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT))
        });
        let row_end = source_cells
            .last()
            .map_or(0, |cell| cell.column + cell.width)
            .max(painted_end.map_or(0, |column| column + 1))
            .min(frame.cols);
        let mut source_index = 0;
        for (column, cell) in frame_cells.iter().enumerate().take(row_end) {
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let start = output.units.len() as u32;
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            if let Some(original) = source_cells
                .get(source_index)
                .filter(|original| original.column == column)
            {
                let units = source_units.get(original.start as usize..original.end as usize)?;
                if original.width != width
                    || !units
                        .iter()
                        .copied()
                        .eq(cell
                            .text
                            .encode_utf16()
                            .map(|unit| if unit == 9 { 32 } else { unit }))
                {
                    return None;
                }
                output.copy_source(source, &source_units, original.start, original.end)?;
                source_index += 1;
            } else {
                if cell.text.chars().any(|character| character != ' ') {
                    return None;
                }
                let style = if cell.flags.contains(Flags::HIDDEN) {
                    TextStyle::default()
                } else {
                    TextStyle::new(cell.fg, cell.bg, cell.flags)
                };
                output.literal(&[32], style);
            }
            output.view.document.cells.push(bks_terminal::ReadingCell {
                start,
                end: output.units.len() as u32,
                line: row,
                column,
                width,
            });
        }
        if source_index != source_cells.len() {
            return None;
        }
        output.view.document.lines.push(ReadingLine {
            start: line_start,
            end: output.units.len() as u32,
            line: row,
        });
    }
    output.view.document.text = String::from_utf16(&output.units).ok()?;
    Some(output.view)
}

struct VisualBuilder {
    units: Vec<u16>,
    view: VisualDocument,
}

impl VisualBuilder {
    fn literal(&mut self, units: &[u16], style: TextStyle) {
        let start = self.units.len() as u32;
        self.units.extend_from_slice(units);
        StyleSpan::append(
            &mut self.view.document.styles,
            start,
            self.units.len() as u32,
            style,
        );
    }

    fn copy_source(
        &mut self,
        source: &ReadingDocument,
        units: &[u16],
        start: u32,
        end: u32,
    ) -> Option<()> {
        let text = units.get(start as usize..end as usize)?;
        if start == end {
            return Some(());
        }
        let visual_start = self.units.len() as u32;
        self.units.extend_from_slice(text);
        let mut at = source.styles.partition_point(|span| span.end <= start);
        let mut cursor = start;
        while cursor < end {
            let span = source.styles.get(at)?;
            if span.start > cursor {
                return None;
            }
            let next = end.min(span.end);
            StyleSpan::append(
                &mut self.view.document.styles,
                visual_start + cursor - start,
                visual_start + next - start,
                span.style,
            );
            cursor = next;
            at += 1;
        }
        if let Some(previous) = self.view.offsets.last_mut().filter(|span| {
            span.source_end == start
                && span.visual_start + span.source_end - span.source_start == visual_start
        }) {
            previous.source_end = end;
        } else {
            self.view.offsets.push(VisualOffset {
                source_start: start,
                source_end: end,
                visual_start,
                leading_break: text.starts_with(&[13, 10]),
            });
        }
        Some(())
    }
}

/// Evidence for fields already returned by `project`; row contents are never
/// searched. Absolute anchors follow proven rows into retained scrollback.
#[derive(Default)]
pub struct ComposerHistory {
    source: Option<Source>,
    fields: Vec<ProvenField>,
}

struct ProvenField {
    rows: Vec<ProvenRow>,
}

struct ProvenRow {
    absolute: usize,
    fingerprint: RowFingerprint,
}

#[derive(PartialEq, Eq)]
struct RowFingerprint {
    first: u64,
    second: u64,
    glyphs: usize,
    units: usize,
}

// TerminalModel retains at most 10,000 history rows. At saturation history
// size alone cannot prove row displacement, so ambiguous history is retained.
const MAX_HISTORY_ROWS: usize = 10_000;

impl ComposerHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn sync_source(&mut self, frame: &TerminalSnapshot) -> bool {
        if !valid_frame(frame) || frame.history_size >= MAX_HISTORY_ROWS {
            self.reset();
            return false;
        }
        let next = Source::of(frame);
        if self.source.is_some_and(|old| {
            old.columns != next.columns
                || old.rows != next.rows
                || old.alternate != next.alternate
                || old.history > next.history
        }) {
            self.fields.clear();
        }
        self.source = Some(next);
        true
    }

    /// Call before projecting the current field, with the untouched raw source.
    /// Matching proven negative rows leave the output independently. Changed
    /// rows remain visible; matching positive proofs survive partial scrolls.
    pub fn filter(
        &mut self,
        frame: &TerminalSnapshot,
        document: &ReadingDocument,
    ) -> ReadingDocument {
        if !self.sync_source(frame) || frame.alternate_screen != document.alternate_screen {
            return document.clone();
        }
        let units: Vec<_> = document.text.encode_utf16().collect();
        let mut removed = Vec::new();
        self.fields.retain_mut(|field| {
            field.rows.retain(|proof| {
                let row = proof.absolute as i64 - frame.history_size as i64;
                if row < -(frame.history_size as i64)
                    || row >= frame.rows as i64
                    || row_fingerprint(document, &units, row as i32).as_ref()
                        != Some(&proof.fingerprint)
                {
                    return false;
                }
                if row < 0 {
                    removed.push((row as i32, row as i32));
                }
                true
            });
            !field.rows.is_empty()
        });
        if removed.is_empty() {
            return document.clone();
        }
        removed.sort_unstable();
        let mut intervals: Vec<(i32, i32)> = Vec::new();
        for (first, last) in removed {
            if let Some(previous) = intervals
                .last_mut()
                .filter(|previous| first <= previous.1 + 1)
            {
                previous.1 = previous.1.max(last);
            } else {
                intervals.push((first, last));
            }
        }
        without_matching_rows(document, |row| {
            intervals
                .partition_point(|(first, _)| *first <= row)
                .checked_sub(1)
                .is_some_and(|index| row <= intervals[index].1)
        })
        .unwrap_or_else(|| document.clone())
    }

    /// Call after current projection, using its composer and the raw document.
    pub fn remember(
        &mut self,
        frame: &TerminalSnapshot,
        document: &ReadingDocument,
        input: &Composer,
    ) {
        if input.source != Source::of(frame) {
            return;
        }
        self.remember_rows(frame, document, input.first_row, input.last_row);
    }

    /// The caller has already proved that these complete inclusive rows belong
    /// to one projected region, e.g. a detected menu. This does no detection.
    pub fn remember_rows(
        &mut self,
        frame: &TerminalSnapshot,
        document: &ReadingDocument,
        first_row: usize,
        last_row: usize,
    ) {
        if !self.sync_source(frame)
            || first_row > last_row
            || last_row >= frame.rows
            || !matches_source(frame, document, first_row, last_row)
        {
            return;
        }
        let Some(first) = frame.history_size.checked_add(first_row) else {
            return;
        };
        let units: Vec<_> = document.text.encode_utf16().collect();
        let Some(rows) = (first_row..=last_row)
            .map(|row| {
                row_fingerprint(document, &units, row as i32).map(|fingerprint| ProvenRow {
                    absolute: first + row - first_row,
                    fingerprint,
                })
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let end = first + rows.len();
        self.fields.retain_mut(|old| {
            old.rows
                .retain(|row| row.absolute < first || row.absolute >= end);
            !old.rows.is_empty()
        });
        self.fields.push(ProvenField { rows });
        while self
            .fields
            .iter()
            .map(|field| field.rows.len())
            .sum::<usize>()
            > MAX_HISTORY_ROWS
        {
            self.fields.remove(0);
        }
    }
}

fn row_fingerprint(document: &ReadingDocument, units: &[u16], row: i32) -> Option<RowFingerprint> {
    use std::hash::{Hash, Hasher};
    let first = document.cells.partition_point(|cell| cell.line < row);
    let last = document.cells.partition_point(|cell| cell.line <= row);
    let cells = &document.cells[first..last];
    if cells.is_empty()
        && document
            .lines
            .binary_search_by_key(&row, |line| line.line)
            .is_err()
    {
        return None;
    }
    let mut one = std::collections::hash_map::DefaultHasher::new();
    let mut two = std::collections::hash_map::DefaultHasher::new();
    0x6a09_e667_f3bc_c909u64.hash(&mut one);
    0xbb67_ae85_84ca_a73bu64.hash(&mut two);
    let mut count = 0;
    for cell in cells {
        let text = units.get(cell.start as usize..cell.end as usize)?;
        (cell.column, cell.width, text).hash(&mut one);
        (cell.column, cell.width, text).hash(&mut two);
        count += text.len();
    }
    Some(RowFingerprint {
        first: one.finish(),
        second: two.finish(),
        glyphs: cells.len(),
        units: count,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    columns: usize,
    rows: usize,
    history: usize,
    alternate: bool,
}

impl Source {
    fn of(frame: &TerminalSnapshot) -> Self {
        Self {
            columns: frame.cols,
            rows: frame.rows,
            history: frame.history_size,
            alternate: frame.alternate_screen,
        }
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
}

#[derive(Clone, Copy)]
struct Content {
    first: usize,
    last: usize,
    start: usize,
    first_start: usize,
    end: usize,
    boundary: Boundary,
}

#[derive(Clone, Copy)]
struct Border {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

const BORDERS: [Border; 5] = [
    Border {
        top_left: '┌',
        top_right: '┐',
        bottom_left: '└',
        bottom_right: '┘',
        horizontal: '─',
        vertical: '│',
    },
    Border {
        top_left: '╭',
        top_right: '╮',
        bottom_left: '╰',
        bottom_right: '╯',
        horizontal: '─',
        vertical: '│',
    },
    Border {
        top_left: '╔',
        top_right: '╗',
        bottom_left: '╚',
        bottom_right: '╝',
        horizontal: '═',
        vertical: '║',
    },
    Border {
        top_left: '┏',
        top_right: '┓',
        bottom_left: '┗',
        bottom_right: '┛',
        horizontal: '━',
        vertical: '┃',
    },
    Border {
        top_left: '+',
        top_right: '+',
        bottom_left: '+',
        bottom_right: '+',
        horizontal: '-',
        vertical: '|',
    },
];

/// Projection without an accepted-input acknowledgement, retained for callers
/// that only inspect established fields and for deterministic source replays.
#[cfg_attr(not(test), allow(dead_code))]
pub fn project(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    previous: Option<&Composer>,
) -> Projection {
    project_confirmed(frame, document, previous, None)
}

/// The selected CLI supplies a preferred, verified shape. Unsupported input
/// stays in the source document; selecting a CLI never proves entered text.
pub fn project_for_adapter(
    adapter: crate::tui_adapters::AdapterKind,
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    previous: Option<&Composer>,
    accepted: Option<&AcceptedSourceEvidence>,
) -> Projection {
    use crate::tui_adapters::ComposerStrategy;
    let strategy = adapter.policy().composer;
    if matches!(strategy, ComposerStrategy::Universal) {
        return tag_adapter(
            project_confirmed(frame, document, previous, accepted),
            adapter,
        );
    }
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    if !valid_frame(frame) || frame.alternate_screen != document.alternate_screen {
        return unchanged();
    }
    let Some(cursor) = frame.cursor.or_else(|| previous.map(|input| input.anchor)) else {
        return unchanged();
    };
    if cursor.row >= frame.rows || cursor.column >= frame.cols {
        return unchanged();
    }
    if matches!(strategy, ComposerStrategy::Bounded) {
        if let Some(projection) = bounded_common_fallback(frame, document, previous, accepted) {
            return tag_adapter(projection, adapter);
        }
    }
    let shape = match strategy {
        ComposerStrategy::Padded => match padded_shape(frame, cursor) {
            Ok(Some(shape)) => Some(shape),
            Ok(None) => footerless_padded_shape(frame, cursor),
            Err(()) => None,
        },
        ComposerStrategy::Ruled => ruled_panel_shape(frame, cursor),
        ComposerStrategy::Gutter => panel_shape(frame, cursor),
        ComposerStrategy::Bounded => {
            box_panel_shape(frame, cursor).or_else(|| ruled_panel_shape(frame, cursor))
        }
        ComposerStrategy::Universal => unreachable!(),
    };
    let Some(shape) = shape else {
        return unchanged();
    };
    let input = if frame.cursor.is_some() {
        compose_panel(
            frame,
            cursor,
            shape,
            previous,
            accepted,
            document.revision,
            adapter,
        )
    } else {
        previous
            .filter(|old| {
                same_panel(frame, &shape, old)
                    && shape.bounds.top == old.first_row
                    && shape.bounds.bottom == old.last_row
                    && old
                        .panel
                        .as_ref()
                        .is_some_and(|proof| panel_rendered(frame, &shape.proof) == proof.rendered)
            })
            .cloned()
            .map(|mut input| {
                input.caret = None;
                input
            })
    };
    let Some(input) = input else {
        return unchanged();
    };
    if !matches_source(frame, document, input.first_row, input.last_row) {
        return unchanged();
    }
    let Some(output) = without_rows(document, input.first_row, input.last_row) else {
        return unchanged();
    };
    tag_adapter(
        Projection {
            input: Some(input),
            output,
        },
        adapter,
    )
}

fn tag_adapter(
    mut projection: Projection,
    adapter: crate::tui_adapters::AdapterKind,
) -> Projection {
    if let Some(input) = projection.input.as_mut() {
        input.adapter = adapter;
    }
    projection
}

/// Recover only a source-observed deletion after an explicitly accepted key.
/// A hidden partial redraw never publishes this private baseline as new text.
pub fn project_edited_for_adapter(
    adapter: crate::tui_adapters::AdapterKind,
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
    evidence: &AcceptedEditEvidence,
) -> Projection {
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    if frame.cursor.is_none()
        || !valid_frame(frame)
        || !baseline.matches_source(frame)
        || document.revision <= evidence.before_revision
        || evidence.adapter != adapter
        || baseline.adapter != adapter
        || baseline.source != evidence.source
        || baseline.text != evidence.text
        || baseline.caret != Some(evidence.caret)
    {
        return unchanged();
    }
    let mut projection = project_for_adapter(adapter, frame, document, Some(baseline), None);
    let Some(input) = projection.input.as_mut() else {
        return unchanged();
    };
    if adapter == crate::tui_adapters::AdapterKind::OpenCode {
        let changed = input.anchor != baseline.anchor
            || match (input.panel.as_ref(), baseline.panel.as_ref()) {
                (Some(current), Some(old)) => current.rendered != old.rendered,
                _ => false,
            };
        if !changed {
            return unchanged();
        }
        let Some((text, caret)) = input.caret.and_then(|caret| {
            crate::opencode::deleted_input(
                &evidence.text,
                evidence.caret,
                &input.text,
                caret,
                evidence.kind == AcceptedEditKind::Backspace,
                input.last_row - input.first_row + 1,
            )
        }) else {
            return unchanged();
        };
        input.text = text;
        input.caret = Some(caret);
    }
    if input
        .panel
        .as_ref()
        .is_some_and(|proof| !proof.acknowledged)
        || !evidence.matches_result(input)
    {
        return unchanged();
    }
    let Some(content) = composer_content(frame, input) else {
        return unchanged();
    };
    let Some(palette) = content_palette(frame, &content) else {
        return unchanged();
    };
    if !palette.iter().all(|style| evidence.palette.contains(style)) {
        return unchanged();
    }
    projection
}

fn bounded_common_fallback(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    previous: Option<&Composer>,
    accepted: Option<&AcceptedSourceEvidence>,
) -> Option<Projection> {
    // Preserve the common parser's original contract for an already entered,
    // unambiguous bounded field. An unentered adapter hint has a panel proof
    // and must still wait for its exact source acknowledgement.
    let projection = project_confirmed(frame, document, previous, accepted);
    let input = projection.input.as_ref()?;
    if !matches!(input.boundary, Boundary::Box | Boundary::Rules) || input.protected {
        return None;
    }
    let previous =
        previous.filter(|old| old.source == input.source && old.boundary == input.boundary);
    if previous.is_some_and(|input| input.panel.is_some()) {
        return None;
    }
    let units = input.text.encode_utf16().count() as u32;
    let entered = match previous {
        None => {
            accepted.is_none() && frame.cursor.is_some() && units > 0 && input.caret == Some(units)
        }
        Some(old) => {
            input.text == old.text
                || input.caret.is_some_and(|caret| caret > 0)
                || (input.caret == Some(0) && old.text.ends_with(&input.text))
        }
    };
    entered.then_some(projection)
}

pub fn project_recovered_for_adapter(
    adapter: crate::tui_adapters::AdapterKind,
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
    accepted: &AcceptedSourceEvidence,
) -> Projection {
    if frame.cursor.is_some() {
        let mut repainted;
        let baseline = if adapter == crate::tui_adapters::AdapterKind::Codex
            && baseline.permits_codex_repaint(frame)
        {
            // Startup tips can move a live editor and grow scrollback between
            // the accepted paste and its acknowledgement. Keep the original
            // key origin; only a full exact insertion can justify this rebase.
            repainted = baseline.clone();
            repainted.source = Source::of(frame);
            &repainted
        } else {
            baseline
        };
        let projection =
            project_for_adapter(adapter, frame, document, Some(baseline), Some(accepted));
        if projection.input.as_ref().is_some_and(|current| {
            confirmed_panel_insertion(current, baseline, accepted, document.revision).is_some()
        }) {
            return projection;
        }
    }
    Projection {
        input: None,
        output: document.clone(),
    }
}

pub fn project_continued_for_adapter(
    adapter: crate::tui_adapters::AdapterKind,
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
) -> Projection {
    if frame.cursor.is_some() && baseline.caret.is_some() {
        let projection = project_for_adapter(adapter, frame, document, Some(baseline), None);
        if projection
            .input
            .as_ref()
            .is_some_and(|current| continues_composer(current, baseline))
        {
            return projection;
        }
    }
    Projection {
        input: None,
        output: document.clone(),
    }
}

/// Only an explicit host resize supplies this private baseline. Reflow may
/// move every edge and insert physical word wraps, but it must reproduce all
/// previously acknowledged text in the same buffer, mode and input palette.
pub(crate) fn project_resized_for_adapter(
    adapter: crate::tui_adapters::AdapterKind,
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
) -> Projection {
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    let restore = || -> Option<Projection> {
        let codex = adapter == crate::tui_adapters::AdapterKind::Codex;
        if !matches!(
            adapter,
            crate::tui_adapters::AdapterKind::Codex | crate::tui_adapters::AdapterKind::OpenCode
        ) || baseline.adapter != adapter
            || baseline.protected
            || baseline.boundary
                != if codex {
                    Boundary::Padded
                } else {
                    Boundary::Panel
                }
            || if codex {
                baseline.source.alternate != frame.alternate_screen
            } else {
                !baseline.matches_buffer(frame)
            }
            || !valid_frame(frame)
            || document.alternate_screen != frame.alternate_screen
        {
            return None;
        }
        let old = baseline.panel.as_ref().filter(|proof| {
            proof.acknowledged && (codex || matches!(proof.edge, PanelEdge::Gutter(..)))
        })?;
        let cursor = frame.cursor?;
        let mut shape = if codex {
            padded_shape(frame, cursor)
                .ok()?
                .or_else(|| footerless_padded_shape(frame, cursor))?
        } else {
            panel_shape(frame, cursor)?
        };
        if shape.proof.edge != old.edge || shape.proof.background != old.background {
            return None;
        }
        let mut input = compose(
            frame,
            cursor,
            shape.bounds,
            Content {
                first: shape.proof.first,
                last: shape.proof.last,
                start: shape.proof.start,
                first_start: shape.proof.start,
                end: shape.proof.end,
                boundary: shape.boundary,
            },
        )?;
        input.shell = panel_is_shell(frame, &shape);
        if input.protected || input.shell != baseline.shell {
            return None;
        }
        let canonical = if codex && old.partial {
            let intent = old.intent.as_ref()?;
            if let Some(caret) = crate::codex::editor_caret(&input.text, &intent.text, input.caret?)
            {
                Some((intent.text.clone(), caret))
            } else {
                let caret = crate::codex::editor_suffix(&input.text, &intent.text, input.caret?)
                    .or_else(|| {
                        crate::codex::editor_window(&input.text, &intent.text, input.caret?)
                    })?;
                shape.proof.partial = true;
                shape.proof.intent = Some(LiteralIntent {
                    text: intent.text.clone(),
                    caret,
                });
                None
            }
        } else {
            let caret = if codex {
                crate::codex::editor_caret(&input.text, &baseline.text, input.caret?)?
            } else {
                crate::opencode::editor_caret(&input.text, &baseline.text, input.caret?)?
            };
            Some((baseline.text.clone(), caret))
        };
        let palette = panel_palette(frame, &shape.proof)?;
        if !palette.iter().all(|style| old.palette.contains(style))
            || !matches_source(frame, document, input.first_row, input.last_row)
        {
            return None;
        }
        let output = without_rows(document, input.first_row, input.last_row)?;
        shape.proof.acknowledged = true;
        shape.proof.palette = old.palette.clone();
        shape.proof.rendered = panel_rendered(frame, &shape.proof);
        shape.proof.revision = document.revision;
        // A pending insertion still refers to the old physical coordinates.
        shape.proof.insertion = None;
        if let Some((text, caret)) = canonical {
            input.text = text;
            input.caret = Some(caret);
        }
        input.panel = Some(shape.proof);
        Some(tag_adapter(
            Projection {
                input: Some(input),
                output,
            },
            adapter,
        ))
    };
    restore().unwrap_or_else(unchanged)
}

/// Recover an input field from a private, pending-input baseline after a
/// hidden-cursor redraw. Unlike ordinary field continuation, matching the
/// previous palette is insufficient: source text and caret must prove an exact
/// prefix of the accepted insertion. The caller owns the baseline's lifetime.
#[cfg_attr(not(test), allow(dead_code))]
pub fn project_recovered(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
    accepted: &AcceptedSourceEvidence,
) -> Projection {
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    if frame.cursor.is_none() {
        return unchanged();
    }
    let projection = project_confirmed(frame, document, Some(baseline), Some(accepted));
    let confirmed = projection.input.as_ref().is_some_and(|current| {
        let Some(proof) = &current.panel else {
            return false;
        };
        confirmed_panel_insertion(current, baseline, accepted, document.revision)
            .is_some_and(|insertion| proof.insertion.as_ref() == Some(&insertion.base))
    });
    if confirmed {
        projection
    } else {
        unchanged()
    }
}

/// Restore an unchanged, previously confirmed panel after a hidden redraw.
/// The caller bounds the private baseline's lifetime. This cannot acknowledge
/// newly rendered text or use palette similarity to continue an old field.
/// A menu may translate the whole field vertically; its dimensions, content
/// padding and exact rendered text/style must remain the same.
#[cfg_attr(not(test), allow(dead_code))]
pub fn project_continued(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    baseline: &Composer,
) -> Projection {
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    if frame.cursor.is_none() || baseline.caret.is_none() {
        return unchanged();
    }
    let projection = project(frame, document, Some(baseline));
    let confirmed = projection
        .input
        .as_ref()
        .is_some_and(|current| continues_composer(current, baseline));
    if confirmed {
        projection
    } else {
        unchanged()
    }
}

fn continues_composer(current: &Composer, baseline: &Composer) -> bool {
    current.caret.is_some()
        && current.text == baseline.text
        && current.protected == baseline.protected
        && current.boundary == baseline.boundary
        && current.source == baseline.source
        && current.last_row - current.first_row == baseline.last_row - baseline.first_row
        && current.first_column == baseline.first_column
        && current.last_column == baseline.last_column
        && match (current.panel.as_ref(), baseline.panel.as_ref()) {
            (Some(now), Some(old)) => {
                now.first.checked_sub(current.first_row)
                    == old.first.checked_sub(baseline.first_row)
                    && now.last.checked_sub(current.first_row)
                        == old.last.checked_sub(baseline.first_row)
                    && now.start == old.start
                    && now.end == old.end
                    && now.background == old.background
                    && now.edge == old.edge
                    && now.rendered == old.rendered
                    && now.palette == old.palette
            }
            (None, None) => matches!(current.boundary, Boundary::Box | Boundary::Rules),
            _ => false,
        }
}

pub fn project_confirmed(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    previous: Option<&Composer>,
    accepted: Option<&AcceptedSourceEvidence>,
) -> Projection {
    let unchanged = || Projection {
        input: None,
        output: document.clone(),
    };
    if !valid_frame(frame) || frame.alternate_screen != document.alternate_screen {
        return unchanged();
    }
    let input = if let Some(cursor) = frame.cursor {
        if cursor.row >= frame.rows || cursor.column >= frame.cols {
            return unchanged();
        }
        boxed(frame, cursor).or_else(|| {
            match padded_shape(frame, cursor) {
                Ok(Some(shape)) => {
                    // A bounded field with an unconfirmed display must not fall
                    // through to the bare prompt and expose its placeholder.
                    return compose_panel(
                        frame,
                        cursor,
                        shape,
                        previous,
                        accepted,
                        document.revision,
                        crate::tui_adapters::AdapterKind::Auto,
                    );
                }
                Err(()) => return None,
                Ok(None) => {}
            }
            match ruled(frame, cursor) {
                Ok(input) => input
                    .or_else(|| bare(frame, cursor))
                    .or_else(|| panel(frame, cursor, previous, accepted, document.revision)),
                Err(()) => None,
            }
        })
    } else {
        previous.and_then(|previous| {
            if previous.boundary == Boundary::Bare || previous.source != Source::of(frame) {
                return None;
            }
            let bounds = Bounds {
                top: previous.first_row,
                bottom: previous.last_row,
                left: previous.first_column,
                right: previous.last_column,
            };
            let mut current = match previous.boundary {
                Boundary::Box => compose_box(frame, previous.anchor, bounds),
                Boundary::Rules => ruled(frame, previous.anchor)
                    .ok()
                    .flatten()
                    .filter(|current| {
                        current.first_row == bounds.top
                            && current.last_row == bounds.bottom
                            && current.first_column == bounds.left
                            && current.last_column == bounds.right
                    }),
                Boundary::Bare => None,
                Boundary::Panel | Boundary::Padded => {
                    retain_panel(frame, previous, document.revision)
                }
            }?;
            if current.text != previous.text || current.protected != previous.protected {
                return None;
            }
            current.caret = None;
            Some(current)
        })
    };
    let Some(input) = input else {
        return unchanged();
    };
    if input.boundary == Boundary::Bare {
        let index = document
            .lines
            .partition_point(|line| line.line <= input.first_row as i32);
        if index
            .checked_sub(1)
            .and_then(|index| document.lines.get(index))
            .is_some_and(|line| line.line < 0)
        {
            return unchanged();
        }
    }
    if !matches_source(frame, document, input.first_row, input.last_row) {
        return unchanged();
    }
    let Some(output) = without_rows(document, input.first_row, input.last_row) else {
        return unchanged();
    };
    Projection {
        input: Some(input),
        output,
    }
}

/// Remove inclusive physical screen rows while retaining original grid anchors.
/// It can also be applied to an already projected document (e.g. a menu).
pub fn exclude_rows(
    document: &ReadingDocument,
    first_row: usize,
    last_row: usize,
) -> ReadingDocument {
    without_rows(document, first_row, last_row).unwrap_or_else(|| document.clone())
}

fn valid_frame(frame: &TerminalSnapshot) -> bool {
    (2..=MAX_COLUMNS).contains(&frame.cols)
        && (1..=MAX_ROWS).contains(&frame.rows)
        && frame.display_offset == 0
        && frame.cols.checked_mul(frame.rows) == Some(frame.cells.len())
        && frame.cells.iter().all(|cell| cell.text.len() <= 256)
}

fn cell(
    frame: &TerminalSnapshot,
    row: usize,
    column: usize,
) -> Option<&bks_terminal::CellSnapshot> {
    (row < frame.rows && column < frame.cols).then(|| &frame.cells[row * frame.cols + column])
}

fn glyph(frame: &TerminalSnapshot, row: usize, column: usize) -> Option<char> {
    let cell = cell(frame, row, column)?;
    if cell.flags.contains(Flags::HIDDEN) {
        return None;
    }
    let mut chars = cell.text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn blank(frame: &TerminalSnapshot, row: usize, column: usize) -> bool {
    cell(frame, row, column).is_some_and(|cell| {
        !cell.flags.contains(Flags::HIDDEN) && cell.text.chars().all(|ch| ch == ' ')
    })
}

fn wrapped(frame: &TerminalSnapshot, row: usize) -> bool {
    cell(frame, row, frame.cols - 1).is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
}

fn box_border(frame: &TerminalSnapshot, bounds: Bounds) -> Option<Border> {
    if bounds.bottom >= frame.rows
        || bounds.right >= frame.cols
        || bounds.bottom <= bounds.top + 1
        || bounds.right <= bounds.left + 2
        || bounds.bottom - bounds.top > MAX_FIELD_ROWS + 1
    {
        return None;
    }
    let border = BORDERS.into_iter().find(|border| {
        glyph(frame, bounds.top, bounds.left) == Some(border.top_left)
            && glyph(frame, bounds.top, bounds.right) == Some(border.top_right)
            && glyph(frame, bounds.bottom, bounds.left) == Some(border.bottom_left)
            && glyph(frame, bounds.bottom, bounds.right) == Some(border.bottom_right)
    })?;
    if !(bounds.left + 1..bounds.right).all(|column| {
        glyph(frame, bounds.top, column) == Some(border.horizontal)
            && glyph(frame, bounds.bottom, column) == Some(border.horizontal)
    }) || !(bounds.top + 1..bounds.bottom).all(|row| {
        glyph(frame, row, bounds.left) == Some(border.vertical)
            && glyph(frame, row, bounds.right) == Some(border.vertical)
    }) {
        return None;
    }
    // Removing whole physical rows must never remove a side-by-side body pane.
    if !(bounds.top..=bounds.bottom).all(|row| {
        (0..bounds.left)
            .chain(bounds.right + 1..frame.cols)
            .all(|column| blank(frame, row, column))
    }) {
        return None;
    }
    Some(border)
}

fn boxed(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<Composer> {
    let mut candidates = Vec::new();
    let mut examined = 0;
    for top in cursor.row.saturating_sub(MAX_FIELD_ROWS)..cursor.row {
        for left in 0..cursor.column {
            if !BORDERS
                .iter()
                .any(|border| glyph(frame, top, left) == Some(border.top_left))
            {
                continue;
            }
            for right in cursor.column + 1..frame.cols {
                if !BORDERS
                    .iter()
                    .any(|border| glyph(frame, top, right) == Some(border.top_right))
                {
                    continue;
                }
                examined += 1;
                if examined > MAX_BOX_CANDIDATES {
                    return None;
                }
                for bottom in cursor.row + 1..frame.rows.min(top + MAX_FIELD_ROWS + 2) {
                    let bounds = Bounds {
                        top,
                        bottom,
                        left,
                        right,
                    };
                    if let Some(composer) = compose_box(frame, cursor, bounds) {
                        candidates.push(composer);
                    }
                }
            }
        }
    }
    candidates.sort_by_key(|input| {
        (input.last_row - input.first_row) * (input.last_column - input.first_column)
    });
    let first = candidates.first()?;
    let area = (first.last_row - first.first_row) * (first.last_column - first.first_column);
    if candidates.get(1).is_some_and(|other| {
        (other.last_row - other.first_row) * (other.last_column - other.first_column) == area
    }) {
        return None;
    }
    candidates.into_iter().next()
}

fn prompt_end(frame: &TerminalSnapshot, row: usize, start: usize, end: usize) -> Option<usize> {
    let first = (start..end).find(|&column| !blank(frame, row, column))?;
    let symbol = glyph(frame, row, first)?;
    if matches!(symbol, '>' | '›' | '❯' | '$' | '#' | '»' | 'λ')
        && first + 1 < end
        && prompt_separator(frame, row, first + 1)
    {
        return Some(first + 2);
    }
    // A bounded path-shaped prefix ending in "> " is a generic shell prompt.
    let mut path = false;
    for column in first..end.min(first + 128) {
        let ch = glyph(frame, row, column)?;
        if ch == '>' {
            return (path && column + 1 < end && prompt_separator(frame, row, column + 1))
                .then_some(column + 2);
        }
        if matches!(ch, '/' | '\\' | '~') {
            path = true
        }
        if !(ch.is_alphanumeric()
            || matches!(ch, ' ' | '/' | '\\' | '~' | ':' | '.' | '_' | '-' | '@'))
        {
            return None;
        }
    }
    None
}

fn prompt_separator(frame: &TerminalSnapshot, row: usize, column: usize) -> bool {
    matches!(
        glyph(frame, row, column),
        Some(' ' | '\u{00a0}' | '\u{202f}')
    )
}

fn compose_box(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
    bounds: Bounds,
) -> Option<Composer> {
    box_border(frame, bounds)?;
    if cursor.row <= bounds.top
        || cursor.row >= bounds.bottom
        || cursor.column <= bounds.left
        || cursor.column >= bounds.right
    {
        return None;
    }
    let first = bounds.top + 1;
    let padding = usize::from((first..bounds.bottom).all(|row| blank(frame, row, bounds.left + 1)));
    let start = bounds.left + 1 + padding;
    // Several equally shaped prompt rows are a menu, not one editor.
    if (first..bounds.bottom)
        .filter(|&row| prompt_end(frame, row, start, bounds.right).is_some())
        .count()
        > 1
    {
        return None;
    }
    let first_start = prompt_end(frame, first, start, bounds.right).unwrap_or(start);
    compose(
        frame,
        cursor,
        bounds,
        Content {
            first,
            last: bounds.bottom - 1,
            start,
            first_start,
            end: bounds.right,
            boundary: Boundary::Box,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Rule {
    left: usize,
    right: usize,
    glyph: char,
}

fn rule_prefix(frame: &TerminalSnapshot, row: usize) -> Option<Rule> {
    let left = (0..frame.cols).find(|&column| !blank(frame, row, column))?;
    let horizontal = glyph(frame, row, left)?;
    if !matches!(horizontal, '─' | '━' | '═' | '-') {
        return None;
    }
    let end = (left..frame.cols)
        .find(|&column| glyph(frame, row, column) != Some(horizontal))
        .unwrap_or(frame.cols);
    (end - left >= MIN_RULE_COLUMNS).then_some(Rule {
        left,
        right: end - 1,
        glyph: horizontal,
    })
}

fn full_rule(frame: &TerminalSnapshot, row: usize) -> Option<Rule> {
    let rule = rule_prefix(frame, row)?;
    (rule.right + 1..frame.cols)
        .all(|column| blank(frame, row, column))
        .then_some(rule)
}

/// Matching horizontal rules and one source cursor/prompt prove a field
/// without requiring vertical edges. A prompt may follow the first rule on
/// the same physical row; only soft-wrapped continuations belong to that form.
/// A recognized but ambiguous boundary must not fall back to a bare prompt.
fn ruled(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Result<Option<Composer>, ()> {
    let mut candidates = Vec::new();
    let mut bounded = false;
    for top in cursor.row.saturating_sub(MAX_FIELD_ROWS)..=cursor.row {
        let Some(rule) = rule_prefix(frame, top) else {
            continue;
        };
        let separate = full_rule(frame, top).is_some();
        let first = top + usize::from(separate);
        if first > cursor.row {
            continue;
        }
        if !separate && !(first..cursor.row).all(|row| wrapped(frame, row)) {
            continue;
        }
        let Some((bottom, lower)) = (cursor.row + 1..frame.rows.min(top + MAX_FIELD_ROWS + 2))
            .find_map(|row| full_rule(frame, row).map(|lower| (row, lower)))
        else {
            continue;
        };
        bounded = true;
        if (separate && lower != rule)
            || (!separate
                && (lower.left != rule.left
                    || lower.glyph != rule.glyph
                    || lower.right < rule.right))
        {
            continue;
        }
        let end = if separate { rule.right + 1 } else { frame.cols };
        let prompt_start = if separate { rule.left } else { rule.right + 1 };
        let Some(first_start) = prompt_end(frame, first, prompt_start, end) else {
            continue;
        };
        if cursor.column < rule.left
            || cursor.column >= end
            || (top..=bottom).any(|row| {
                (0..rule.left)
                    .chain(end..frame.cols)
                    .any(|column| !blank(frame, row, column))
            })
            || (first + 1..bottom).any(|row| prompt_end(frame, row, rule.left, end).is_some())
            || (!separate && (first..bottom - 1).any(|row| !wrapped(frame, row)))
        {
            continue;
        }
        // A separate one-symbol prompt occupies its own two-column prefix
        // when every continuation leaves that prefix empty. Preserve spaces
        // after it, including user indentation and a native software cursor.
        let start = if separate
            && first_start == rule.left + 2
            && (first + 1..bottom)
                .all(|row| (rule.left..first_start).all(|column| blank(frame, row, column)))
        {
            first_start
        } else {
            rule.left
        };
        if let Some(input) = compose(
            frame,
            cursor,
            Bounds {
                top,
                bottom,
                left: rule.left,
                right: end - 1,
            },
            Content {
                first,
                last: bottom - 1,
                start,
                first_start,
                end,
                boundary: Boundary::Rules,
            },
        ) {
            candidates.push(input);
        }
    }
    if candidates.len() == 1 {
        Ok(candidates.pop())
    } else if bounded {
        Err(())
    } else {
        Ok(None)
    }
}

fn bare(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<Composer> {
    let mut first = cursor.row;
    while first > 0 && wrapped(frame, first - 1) {
        first -= 1;
        if cursor.row - first >= MAX_FIELD_ROWS {
            return None;
        }
    }
    let mut last = cursor.row;
    while last + 1 < frame.rows && wrapped(frame, last) {
        last += 1;
        if last - first >= MAX_FIELD_ROWS {
            return None;
        }
    }
    let start = prompt_end(frame, first, 0, frame.cols)?;
    if (first.saturating_sub(1)..=(last + 1).min(frame.rows - 1))
        .any(|row| (row < first || row > last) && prompt_end(frame, row, 0, frame.cols).is_some())
    {
        return None;
    }
    compose(
        frame,
        cursor,
        Bounds {
            top: first,
            bottom: last,
            left: 0,
            right: frame.cols - 1,
        },
        Content {
            first,
            last,
            start: 0,
            first_start: start,
            end: frame.cols,
            boundary: Boundary::Bare,
        },
    )
}

struct PanelShape {
    bounds: Bounds,
    boundary: Boundary,
    proof: PanelProof,
}

fn ruled_panel_shape(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<PanelShape> {
    let input = ruled(frame, cursor).ok().flatten()?;
    let rule = full_rule(frame, input.first_row)?;
    let first = input.first_row + 1;
    let end = input.last_column + 1;
    let start = prompt_end(frame, first, input.first_column, end)?;
    if start != input.first_column + 2
        || (first + 1..input.last_row)
            .any(|row| !(input.first_column..start).all(|column| blank(frame, row, column)))
    {
        return None;
    }
    Some(PanelShape {
        bounds: Bounds {
            top: input.first_row,
            bottom: input.last_row,
            left: input.first_column,
            right: input.last_column,
        },
        boundary: Boundary::Rules,
        proof: PanelProof {
            first,
            last: input.last_row - 1,
            start,
            end,
            background: cell(frame, cursor.row, cursor.column)?.bg,
            edge: PanelEdge::Rule(rule.glyph, cell(frame, input.first_row, rule.left)?.fg),
            acknowledged: false,
            rendered: Vec::new(),
            palette: Vec::new(),
            revision: 0,
            insertion: None,
            partial: false,
            intent: None,
        },
    })
}

fn box_panel_shape(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<PanelShape> {
    let input = boxed(frame, cursor)?;
    let bounds = Bounds {
        top: input.first_row,
        bottom: input.last_row,
        left: input.first_column,
        right: input.last_column,
    };
    let first = bounds.top + 1;
    let padding = usize::from((first..bounds.bottom).all(|row| blank(frame, row, bounds.left + 1)));
    let prompt = bounds.left + 1 + padding;
    let start = prompt_end(frame, first, prompt, bounds.right)?;
    if start != prompt + 2
        || (first + 1..bounds.bottom)
            .any(|row| !(prompt..start).all(|column| blank(frame, row, column)))
    {
        return None;
    }
    Some(PanelShape {
        bounds,
        boundary: Boundary::Box,
        proof: PanelProof {
            first,
            last: bounds.bottom - 1,
            start,
            end: bounds.right,
            background: cell(frame, cursor.row, cursor.column)?.bg,
            edge: PanelEdge::Box(
                glyph(frame, bounds.top, bounds.left)?,
                cell(frame, bounds.top, bounds.left)?.fg,
            ),
            acknowledged: false,
            rendered: Vec::new(),
            palette: Vec::new(),
            revision: 0,
            insertion: None,
            partial: false,
            intent: None,
        },
    })
}

// The selected padded renderer reserves exactly one blank row above/below
// its textarea even while its optional status footer is absent. Universal
// mode deliberately requires the stronger footer or color-rectangle proof.
fn footerless_padded_shape(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<PanelShape> {
    let empty = |row| (0..frame.cols).all(|column| blank(frame, row, column));
    let first = (cursor.row.saturating_sub(MAX_FIELD_ROWS).max(1)..=cursor.row)
        .rev()
        .find(|&row| empty(row - 1) && prompt_end(frame, row, 0, frame.cols) == Some(2))?;
    let bottom = (cursor.row + 1..frame.rows).find(|&row| empty(row))?;
    if bottom - first >= MAX_FIELD_ROWS || !(bottom..frame.rows).all(empty) {
        return None;
    }
    padded_candidate(
        frame,
        cursor,
        Bounds {
            top: first - 1,
            bottom,
            left: 0,
            right: frame.cols - 1,
        },
    )
}

/// A uniform background rectangle with empty top/bottom padding and one
/// prompt bounds editors that draw no box edges. Continuations use the same
/// content columns, independently of application labels or foreground colors.
fn padded_shape(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
) -> Result<Option<PanelShape>, ()> {
    if let Some(bounds) = colored_padded_bounds(frame, cursor) {
        // Once a rectangle bounds the row, an invalid shape cannot become
        // a bare field that would swallow unrelated text beside it.
        padded_candidate(frame, cursor, bounds).map(Some).ok_or(())
    } else {
        Ok(plain_padded_shape(frame, cursor))
    }
}

fn colored_padded_bounds(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<Bounds> {
    let background = cell(frame, cursor.row, cursor.column)?.bg;
    let left = (0..=cursor.column)
        .rev()
        .take_while(|column| cell(frame, cursor.row, *column).is_some_and(|c| c.bg == background))
        .last()?;
    let right = (cursor.column..frame.cols)
        .take_while(|column| cell(frame, cursor.row, *column).is_some_and(|c| c.bg == background))
        .last()?;
    if right - left + 1 < MIN_RULE_COLUMNS {
        return None;
    }
    let matches_row = |row| {
        (left..=right).all(|column| cell(frame, row, column).is_some_and(|c| c.bg == background))
            && (0..left)
                .chain(right + 1..frame.cols)
                .all(|column| blank(frame, row, column))
    };
    let empty_row = |row| (left..=right).all(|column| blank(frame, row, column));
    let mut top = cursor.row;
    while top > 0 && cursor.row - top < MAX_FIELD_ROWS && matches_row(top - 1) {
        top -= 1;
    }
    let mut bottom = cursor.row;
    while bottom + 1 < frame.rows && bottom - top < MAX_FIELD_ROWS && matches_row(bottom + 1) {
        bottom += 1;
    }
    if bottom - top < 2
        || bottom - top >= MAX_FIELD_ROWS
        || cursor.row <= top
        || cursor.row >= bottom
        || !empty_row(top)
        || !empty_row(bottom)
        || (top == 0 && bottom + 1 == frame.rows)
    {
        return None;
    }
    Some(Bounds {
        top,
        bottom,
        left,
        right,
    })
}

/// Before a palette query is answered (and with color disabled), empty
/// padding plus an aligned footer can still bound the same editor. The footer
/// follows the padding directly or after one additional blank separator; it
/// can have a left segment, a right segment, or both.
/// A bare prompt or an unfinished screen has no such proof and stays bare.
fn plain_padded_shape(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<PanelShape> {
    let empty_row = |row| (0..frame.cols).all(|column| blank(frame, row, column));
    let mut candidates = Vec::new();
    for first in cursor.row.saturating_sub(MAX_FIELD_ROWS).max(1)..=cursor.row {
        if !empty_row(first - 1) || prompt_end(frame, first, 0, frame.cols) != Some(2) {
            continue;
        }
        for bottom in cursor.row + 1..frame.rows.saturating_sub(1).min(first + MAX_FIELD_ROWS) {
            if !empty_row(bottom) || (bottom > cursor.row + 1 && empty_row(bottom - 1)) {
                continue;
            }
            let Some(footer) =
                (bottom + 1..frame.rows.min(bottom + 3)).find(|row| !empty_row(*row))
            else {
                continue;
            };
            let footer_end = (footer..frame.rows)
                .find(|row| empty_row(*row))
                .unwrap_or(frame.rows);
            if footer_end - footer > 3
                || !(footer_end..frame.rows).all(empty_row)
                || (footer..footer_end).any(|row| {
                    let first = (0..frame.cols).find(|column| !blank(frame, row, *column));
                    let last = (0..frame.cols).rfind(|column| !blank(frame, row, *column));
                    let aligned = first == Some(2)
                        || (first.is_some_and(|column| column > 2)
                            && last == frame.cols.checked_sub(2));
                    !aligned || prompt_end(frame, row, 0, frame.cols).is_some()
                })
            {
                continue;
            }
            let bounds = Bounds {
                top: first - 1,
                bottom,
                left: 0,
                right: frame.cols - 1,
            };
            if let Some(shape) = padded_candidate(frame, cursor, bounds) {
                candidates.push(shape);
            }
        }
    }
    (candidates.len() == 1).then(|| candidates.pop()).flatten()
}

fn padded_candidate(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
    bounds: Bounds,
) -> Option<PanelShape> {
    let Bounds {
        top,
        bottom,
        left,
        right,
    } = bounds;
    let background = cell(frame, cursor.row, cursor.column)?.bg;
    let first = top + 1;
    let start = prompt_end(frame, first, left, right)?;
    // A single prompt glyph plus a separator gives unambiguous left padding.
    if start != left + 2
        || cursor.column < start
        || cursor.column >= right
        || (top..=bottom).any(|row| {
            (left..=right).any(|column| cell(frame, row, column).is_none_or(|c| c.bg != background))
                || (0..left)
                    .chain(right + 1..frame.cols)
                    .any(|column| !blank(frame, row, column))
        })
        || !(first..bottom).all(|row| blank(frame, row, right))
        || (first + 1..bottom).any(|row| {
            !(left..start).all(|column| blank(frame, row, column))
                || prompt_end(frame, row, start, right).is_some()
        })
    {
        return None;
    }
    let prompt = cell(frame, first, left)?;
    Some(PanelShape {
        bounds,
        boundary: Boundary::Padded,
        proof: PanelProof {
            first,
            last: bottom - 1,
            start,
            end: right,
            background,
            edge: PanelEdge::Prompt(glyph(frame, first, left)?, prompt.fg),
            acknowledged: false,
            rendered: Vec::new(),
            palette: Vec::new(),
            revision: 0,
            insertion: None,
            partial: false,
            intent: None,
        },
    })
}

/// A background rectangle, continuous left gutter, blank top padding and a
/// matching lower rail bound the panel. A blank separator and aligned footer
/// bound the textarea independently of labels, foreground colors or content.
fn panel_shape(frame: &TerminalSnapshot, cursor: CursorSnapshot) -> Option<PanelShape> {
    let background = cell(frame, cursor.row, cursor.column)?.bg;
    let left = (0..=cursor.column)
        .rev()
        .take_while(|column| cell(frame, cursor.row, *column).is_some_and(|c| c.bg == background))
        .last()?;
    let right = (cursor.column..frame.cols)
        .take_while(|column| cell(frame, cursor.row, *column).is_some_and(|c| c.bg == background))
        .last()?;
    let gutter_column = left.checked_sub(1)?;
    let gutter = glyph(frame, cursor.row, gutter_column)?;
    if !matches!(gutter, '│' | '┃' | '║') || right - left + 1 < MIN_RULE_COLUMNS {
        return None;
    }
    let gutter_cell = cell(frame, cursor.row, gutter_column)?;
    let gutter_color = gutter_cell.fg;
    let outside = gutter_cell.bg;
    if background == outside || gutter_color == background {
        return None;
    }
    let matches_row = |row| {
        glyph(frame, row, gutter_column) == Some(gutter)
            && cell(frame, row, gutter_column)
                .is_some_and(|c| c.fg == gutter_color && c.bg == outside)
            && (left..=right)
                .all(|column| cell(frame, row, column).is_some_and(|c| c.bg == background))
            && (0..gutter_column)
                .chain(right + 1..frame.cols)
                .all(|column| blank(frame, row, column))
    };
    let mut top = cursor.row;
    while top > 0 && cursor.row - top < MAX_FIELD_ROWS && matches_row(top - 1) {
        top -= 1;
    }
    if top == cursor.row || !(left..=right).all(|column| blank(frame, top, column)) {
        return None;
    }
    let mut bottom = cursor.row + 1;
    while bottom < frame.rows && bottom - top <= MAX_FIELD_ROWS && matches_row(bottom) {
        bottom += 1;
    }
    if bottom >= frame.rows || bottom - top > MAX_FIELD_ROWS {
        return None;
    }
    let corner = glyph(frame, bottom, gutter_column)?;
    let horizontal = glyph(frame, bottom, left)?;
    let matching_rail = matches!(
        (gutter, corner, horizontal),
        ('│', '└' | '╰', '─') | ('┃', '┗', '━') | ('║', '╚', '═')
    );
    let block_rail =
        matches!((gutter, corner), ('│', '╵') | ('┃', '╹')) && matches!(horizontal, '▀' | '▄');
    if !(matching_rail || block_rail)
        || cell(frame, bottom, gutter_column)
            .is_none_or(|c| c.fg != gutter_color || c.bg != outside)
        || !(left..=right).all(|column| {
            glyph(frame, bottom, column) == Some(horizontal)
                && cell(frame, bottom, column).is_some_and(|c| {
                    c.bg == outside && c.fg == if block_rail { background } else { gutter_color }
                })
        })
        || !(0..gutter_column)
            .chain(right + 1..frame.cols)
            .all(|column| blank(frame, bottom, column))
    {
        return None;
    }
    let empty_row = |row| (left..=right).all(|column| blank(frame, row, column));
    let first = (top + 1..=cursor.row).find(|row| *row == cursor.row || !empty_row(*row))?;
    let separator = (cursor.row + 1..bottom).rev().find(|row| {
        empty_row(*row)
            && *row + 1 < bottom
            && bottom - row <= 4
            && (row + 1..bottom).all(|footer| !empty_row(footer))
    })?;
    let last = separator - 1;
    let start = (left..=right).find(|column| !blank(frame, separator + 1, *column))?;
    if !(1..=4).contains(&(start - left))
        || cursor.column < start
        || (first..=last).any(|row| !(left..start).all(|column| blank(frame, row, column)))
        || (separator + 1..bottom)
            .any(|row| (left..=right).find(|column| !blank(frame, row, *column)) != Some(start))
    {
        return None;
    }
    Some(PanelShape {
        bounds: Bounds {
            top,
            bottom,
            left: gutter_column,
            right,
        },
        boundary: Boundary::Panel,
        proof: PanelProof {
            first,
            last,
            start,
            end: right + 1,
            background,
            edge: PanelEdge::Gutter(gutter, gutter_color),
            acknowledged: false,
            rendered: Vec::new(),
            palette: Vec::new(),
            revision: 0,
            insertion: None,
            partial: false,
            intent: None,
        },
    })
}

fn panel_rendered(frame: &TerminalSnapshot, proof: &PanelProof) -> Vec<(String, Color, Flags)> {
    (proof.first..=proof.last)
        .flat_map(|row| (proof.start..proof.end).map(move |column| (row, column)))
        .filter_map(|(row, column)| cell(frame, row, column).map(|cell| (row, column, cell)))
        .map(|(row, column, c)| {
            (
                c.text.clone(),
                c.fg,
                input_flags(frame, row, column, c.flags),
            )
        })
        .collect()
}

fn input_flags(frame: &TerminalSnapshot, row: usize, column: usize, mut flags: Flags) -> Flags {
    if frame.cursor.is_some_and(|cursor| {
        cursor.row == row
            && (cursor.column == column
                || (cursor.column.checked_add(1) == Some(column)
                    && cell(frame, row, cursor.column)
                        .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR))))
    }) {
        flags.remove(Flags::INVERSE);
    }
    flags
}

fn same_panel(frame: &TerminalSnapshot, shape: &PanelShape, previous: &Composer) -> bool {
    let Some(proof) = &previous.panel else {
        return false;
    };
    previous.boundary == shape.boundary
        && previous.source == Source::of(frame)
        && previous.first_column == shape.bounds.left
        && previous.last_column == shape.bounds.right
        && proof.start == shape.proof.start
        && proof.end == shape.proof.end
        && proof.background == shape.proof.background
        && proof.edge == shape.proof.edge
}

fn panel_palette(frame: &TerminalSnapshot, proof: &PanelProof) -> Option<Vec<TextStyle>> {
    content_palette(
        frame,
        &Content {
            first: proof.first,
            last: proof.last,
            start: proof.start,
            first_start: proof.start,
            end: proof.end,
            boundary: Boundary::Panel,
        },
    )
}

fn composer_bounds(input: &Composer) -> Bounds {
    Bounds {
        top: input.first_row,
        bottom: input.last_row,
        left: input.first_column,
        right: input.last_column,
    }
}

fn composer_content(frame: &TerminalSnapshot, input: &Composer) -> Option<Content> {
    if let Some(proof) = input.panel.as_ref() {
        return Some(Content {
            first: proof.first,
            last: proof.last,
            start: proof.start,
            first_start: proof.start,
            end: proof.end,
            boundary: input.boundary,
        });
    }
    let bounds = composer_bounds(input);
    match input.boundary {
        Boundary::Box => {
            box_border(frame, bounds)?;
            let first = bounds.top + 1;
            let padding =
                usize::from((first..bounds.bottom).all(|row| blank(frame, row, bounds.left + 1)));
            let start = bounds.left + 1 + padding;
            Some(Content {
                first,
                last: bounds.bottom - 1,
                start,
                first_start: prompt_end(frame, first, start, bounds.right).unwrap_or(start),
                end: bounds.right,
                boundary: Boundary::Box,
            })
        }
        Boundary::Rules => {
            let rule = rule_prefix(frame, bounds.top)?;
            let separate = full_rule(frame, bounds.top).is_some();
            let first = bounds.top + usize::from(separate);
            let end = bounds.right + 1;
            let first_start = prompt_end(
                frame,
                first,
                if separate {
                    bounds.left
                } else {
                    rule.right + 1
                },
                end,
            )?;
            let start = if separate
                && first_start == bounds.left + 2
                && (first + 1..bounds.bottom)
                    .all(|row| (bounds.left..first_start).all(|column| blank(frame, row, column)))
            {
                first_start
            } else {
                bounds.left
            };
            Some(Content {
                first,
                last: bounds.bottom - 1,
                start,
                first_start,
                end,
                boundary: Boundary::Rules,
            })
        }
        _ => None,
    }
}

fn content_palette(frame: &TerminalSnapshot, content: &Content) -> Option<Vec<TextStyle>> {
    let mut palette = Vec::new();
    for row in content.first..=content.last {
        for column in (if row == content.first {
            content.first_start
        } else {
            content.start
        })..content.end
        {
            let c = cell(frame, row, column)?;
            if c.text.chars().all(char::is_whitespace) {
                continue;
            }
            // A source-proven software caret inverts its current glyph. Its
            // movement is not a new input palette or a different placeholder.
            let flags = input_flags(frame, row, column, c.flags);
            let style = TextStyle::new(c.fg, c.bg, flags);
            if !palette.contains(&style) {
                palette.push(style);
            }
            if palette.len() > 32 {
                return None;
            }
        }
    }
    Some(palette)
}

struct ConfirmedInsertion {
    base: PanelInsertion,
    canonical: Option<(String, u32)>,
    intent: Option<LiteralIntent>,
}

/// Startup can expose the textarea before its footer is complete. No previous
/// composer then exists. Acquire it only when every source glyph is an exact
/// prefix of input explicitly accepted at this field's beginning.
fn confirmed_initial_panel_insertion(
    current: &Composer,
    shape: &PanelShape,
    accepted: &AcceptedSourceEvidence,
    revision: u64,
    adapter: crate::tui_adapters::AdapterKind,
) -> Option<ConfirmedInsertion> {
    if current.protected
        || current.text.is_empty()
        || revision <= accepted.before_revision
        || accepted.row != shape.proof.first
        || accepted.column != shape.proof.start
        || accepted.text.is_empty()
        || accepted.text.chars().take(65).count() > 64
        || accepted
            .text
            .replace("\r\n", "")
            .chars()
            .any(char::is_control)
    {
        return None;
    }
    let (text, caret) = accepted
        .text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(accepted.text.len()))
        .rev()
        .filter(|end| *end > 0)
        .find_map(|end| {
            let expected = &accepted.text[..end];
            let caret = match adapter {
                crate::tui_adapters::AdapterKind::Codex => {
                    crate::codex::editor_caret(&current.text, expected, current.caret?)?
                }
                crate::tui_adapters::AdapterKind::OpenCode => {
                    crate::opencode::editor_caret(&current.text, expected, current.caret?)?
                }
                _ => return None,
            };
            (caret as usize == expected.encode_utf16().count()).then_some((expected, caret))
        })?;
    Some(ConfirmedInsertion {
        base: PanelInsertion {
            text: String::new(),
            caret: 0,
            origin: CursorSnapshot {
                row: accepted.row,
                column: accepted.column,
            },
            before_revision: accepted.before_revision,
        },
        canonical: Some((text.to_owned(), caret)),
        intent: (adapter == crate::tui_adapters::AdapterKind::Codex && text != accepted.text).then(
            || LiteralIntent {
                text: accepted.text.clone(),
                caret: accepted.text.encode_utf16().count() as u32,
            },
        ),
    })
}

fn confirmed_panel_insertion(
    current: &Composer,
    previous: &Composer,
    accepted: &AcceptedSourceEvidence,
    revision: u64,
) -> Option<ConfirmedInsertion> {
    if revision <= accepted.before_revision
        || accepted.text.is_empty()
        || accepted.text.chars().take(MAX_FIELD_UNITS + 1).count()
            > if matches!(
                previous.adapter,
                crate::tui_adapters::AdapterKind::Codex
                    | crate::tui_adapters::AdapterKind::OpenCode
            ) {
                MAX_FIELD_UNITS
            } else {
                64
            }
        || accepted
            .text
            .replace("\r\n", "")
            .chars()
            .any(char::is_control)
        || current.protected
        || previous.protected
    {
        return None;
    }
    let old_proof = previous.panel.as_ref()?;
    let origin = CursorSnapshot {
        row: accepted.row,
        column: accepted.column,
    };
    let base = old_proof
        .insertion
        .as_ref()
        .filter(|base| base.origin == origin && base.before_revision == accepted.before_revision)
        .cloned()
        .or_else(|| {
            if origin == previous.anchor && old_proof.revision <= accepted.before_revision {
                Some(PanelInsertion {
                    text: previous.request_text().to_owned(),
                    caret: previous.request_caret()?,
                    origin,
                    before_revision: accepted.before_revision,
                })
            } else {
                None
            }
        })?;
    let base_caret = base.caret;
    let units: Vec<_> = base.text.encode_utf16().collect();
    let split = base.caret as usize;
    let prefix = String::from_utf16(units.get(..split)?).ok()?;
    let suffix = String::from_utf16(units.get(split..)?).ok()?;
    let expected = format!("{prefix}{}{suffix}", accepted.text);
    if expected.encode_utf16().count() > MAX_FIELD_UNITS {
        return None;
    }
    if let Some(insertion) = current
        .text
        .strip_prefix(&prefix)
        .and_then(|text| text.strip_suffix(&suffix))
    {
        if !insertion.is_empty()
            && accepted.text.starts_with(insertion)
            && current.caret == Some(base.caret + insertion.encode_utf16().count() as u32)
        {
            return Some(ConfirmedInsertion {
                base,
                canonical: None,
                intent: (previous.adapter == crate::tui_adapters::AdapterKind::Codex
                    && insertion != accepted.text)
                    .then(|| LiteralIntent {
                        text: expected.clone(),
                        caret: base_caret + accepted.text.encode_utf16().count() as u32,
                    }),
            });
        }
    }
    let caret = match previous.adapter {
        crate::tui_adapters::AdapterKind::Codex => {
            crate::codex::editor_caret(&current.text, &expected, current.caret?)
        }
        crate::tui_adapters::AdapterKind::OpenCode => {
            crate::opencode::editor_caret(&current.text, &expected, current.caret?)
        }
        _ => return None,
    };
    let expected_caret = base_caret + accepted.text.encode_utf16().count() as u32;
    if let Some(caret) = caret.filter(|caret| *caret == expected_caret) {
        return Some(ConfirmedInsertion {
            base,
            canonical: Some((expected, caret)),
            intent: None,
        });
    }
    if previous.adapter == crate::tui_adapters::AdapterKind::Codex && suffix.is_empty() {
        if let Some((consumed, caret)) =
            crate::codex::editor_prefix(&current.text, &expected, current.caret?)
                .filter(|(consumed, caret)| *consumed > base_caret && consumed == caret)
        {
            let units: Vec<_> = expected.encode_utf16().take(consumed as usize).collect();
            return Some(ConfirmedInsertion {
                base,
                canonical: Some((String::from_utf16(&units).ok()?, caret)),
                intent: Some(LiteralIntent {
                    text: expected,
                    caret: expected_caret,
                }),
            });
        }
    }
    if previous.adapter == crate::tui_adapters::AdapterKind::Codex
        && crate::codex::editor_suffix(&current.text, &expected, current.caret?)
            == Some(expected_caret)
    {
        return Some(ConfirmedInsertion {
            base,
            canonical: None,
            intent: Some(LiteralIntent {
                text: expected,
                caret: expected_caret,
            }),
        });
    }
    if previous.adapter == crate::tui_adapters::AdapterKind::Codex
        && suffix.is_empty()
        && crate::codex::editor_window(&current.text, &expected, current.caret?)
            .is_some_and(|caret| caret > base_caret && caret < expected_caret)
    {
        return Some(ConfirmedInsertion {
            base,
            canonical: None,
            intent: Some(LiteralIntent {
                text: expected,
                caret: expected_caret,
            }),
        });
    }
    None
}

fn panel(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
    previous: Option<&Composer>,
    accepted: Option<&AcceptedSourceEvidence>,
    revision: u64,
) -> Option<Composer> {
    let shape = panel_shape(frame, cursor)?;
    compose_panel(
        frame,
        cursor,
        shape,
        previous,
        accepted,
        revision,
        crate::tui_adapters::AdapterKind::Auto,
    )
}

fn panel_is_shell(frame: &TerminalSnapshot, shape: &PanelShape) -> bool {
    matches!(shape.proof.edge, PanelEdge::Gutter(..))
        && shape.proof.last + 3 == shape.bounds.bottom
        && (shape.proof.start..shape.proof.end)
            .filter_map(|column| cell(frame, shape.proof.last + 2, column))
            .map(|cell| cell.text.as_str())
            .collect::<String>()
            .trim()
            == "Shell"
}

fn compose_panel(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
    mut shape: PanelShape,
    previous: Option<&Composer>,
    accepted: Option<&AcceptedSourceEvidence>,
    revision: u64,
    adapter: crate::tui_adapters::AdapterKind,
) -> Option<Composer> {
    let mut current = compose(
        frame,
        cursor,
        shape.bounds,
        Content {
            first: shape.proof.first,
            last: shape.proof.last,
            start: shape.proof.start,
            first_start: shape.proof.start,
            end: shape.proof.end,
            boundary: shape.boundary,
        },
    )?;
    // OpenCode's shell mode is indicated in the proven toolbar slot, never
    // inferred from the user's text (which may also contain the word Shell).
    current.shell = panel_is_shell(frame, &shape);
    let old = previous.filter(|previous| {
        same_panel(frame, &shape, previous)
            || (previous.adapter == crate::tui_adapters::AdapterKind::OpenCode
                && previous.shell != current.shell
                && previous.text == current.text
                && previous.caret == current.caret
                && previous.source == Source::of(frame)
                && previous.first_column == shape.bounds.left
                && previous.last_column == shape.bounds.right
                && previous.panel.as_ref().is_some_and(|proof| {
                    proof.start == shape.proof.start
                        && proof.end == shape.proof.end
                        && proof.background == shape.proof.background
                        && matches!((&proof.edge, &shape.proof.edge), (PanelEdge::Gutter(old, _), PanelEdge::Gutter(new, _)) if old == new)
                }))
    });
    let at_start = cursor.row == shape.proof.first && cursor.column == shape.proof.start;
    let confirmation = old
        .and_then(|old| {
            accepted
                .and_then(|accepted| confirmed_panel_insertion(&current, old, accepted, revision))
        })
        .or_else(|| {
            (matches!(
                adapter,
                crate::tui_adapters::AdapterKind::Codex
                    | crate::tui_adapters::AdapterKind::OpenCode
            ) && old.is_none())
            .then(|| {
                accepted.and_then(|accepted| {
                    confirmed_initial_panel_insertion(&current, &shape, accepted, revision, adapter)
                })
            })
            .flatten()
        });
    if let Some((text, caret)) = confirmation
        .as_ref()
        .and_then(|confirmation| confirmation.canonical.as_ref())
    {
        current.text = text.clone();
        current.caret = Some(*caret);
    } else if confirmation.is_none() {
        if let Some(old) = old.filter(|old| {
            matches!(
                old.adapter,
                crate::tui_adapters::AdapterKind::Codex
                    | crate::tui_adapters::AdapterKind::OpenCode
            ) && old.panel.as_ref().is_some_and(|proof| {
                proof.acknowledged && proof.rendered == panel_rendered(frame, &shape.proof)
            })
        }) {
            if let Some(caret) = current.caret.and_then(|caret| match old.adapter {
                crate::tui_adapters::AdapterKind::Codex => {
                    crate::codex::editor_caret(&current.text, &old.text, caret)
                }
                _ => crate::opencode::editor_caret(&current.text, &old.text, caret),
            }) {
                current.text = old.text.clone();
                current.caret = Some(caret);
            }
        }
    }
    let intent = confirmation
        .as_ref()
        .and_then(|confirmation| confirmation.intent.clone());
    let insertion = confirmation.map(|confirmation| confirmation.base);
    let palette = panel_palette(frame, &shape.proof)?;
    if current.protected {
        shape.proof.acknowledged = false;
    } else if insertion.is_some() {
        shape.proof.acknowledged = true;
        shape.proof.palette = palette;
        shape.proof.insertion = insertion;
        shape.proof.partial = intent.is_some();
        shape.proof.intent = intent;
    } else if let Some(old) = old.filter(|old| {
        // Backspace after the first glyph, Delete at Home, or deleting a
        // native selection beginning at Home leaves an owned suffix at caret
        // zero. It is still entered text when its source ink is unchanged.
        let removed_prefix =
            at_start && current.text.len() < old.text.len() && old.text.ends_with(&current.text);
        old.panel.as_ref().is_some_and(|proof| proof.acknowledged)
            && (current.text == old.text
                || ((!at_start || removed_prefix)
                    && palette
                        .iter()
                        .all(|style| old.panel.as_ref().unwrap().palette.contains(style))))
    }) {
        let proof = old.panel.as_ref()?;
        shape.proof.acknowledged = true;
        shape.proof.palette = proof.palette.clone();
        shape.proof.insertion = proof.insertion.clone();
        if proof.partial {
            shape.proof.partial = !current.text.is_empty();
            shape.proof.intent = proof.intent.as_ref().and_then(|intent| {
                if current.text == old.text {
                    let caret =
                        crate::codex::editor_suffix(&current.text, &intent.text, current.caret?)
                            .or_else(|| (current.caret == old.caret).then_some(intent.caret))?;
                    Some(LiteralIntent {
                        text: intent.text.clone(),
                        caret,
                    })
                } else {
                    None
                }
            });
        }
    } else if at_start {
        // Geometry proves a textarea, not that its right-hand display text is
        // entered content. Preserve its exact rendering as a future baseline;
        // a gray foreground alone never identifies a placeholder.
        current.text.clear();
        current.caret = Some(0);
    } else {
        return None;
    }
    shape.proof.revision = revision;
    shape.proof.rendered = panel_rendered(frame, &shape.proof);
    current.panel = Some(shape.proof);
    Some(current)
}

fn retain_panel(frame: &TerminalSnapshot, previous: &Composer, revision: u64) -> Option<Composer> {
    let shape = match previous.boundary {
        Boundary::Panel => panel_shape(frame, previous.anchor)?,
        Boundary::Padded => padded_shape(frame, previous.anchor).ok().flatten()?,
        _ => return None,
    };
    if !same_panel(frame, &shape, previous)
        || shape.bounds.top != previous.first_row
        || shape.bounds.bottom != previous.last_row
        || panel_rendered(frame, &shape.proof) != previous.panel.as_ref()?.rendered
    {
        return None;
    }
    let mut current = previous.clone();
    current.caret = None;
    current.panel.as_mut()?.revision = revision;
    Some(current)
}

fn compose(
    frame: &TerminalSnapshot,
    cursor: CursorSnapshot,
    bounds: Bounds,
    content: Content,
) -> Option<Composer> {
    let Content {
        first,
        last,
        start,
        first_start,
        end,
        boundary,
    } = content;
    let mut text = String::new();
    let mut caret = None;
    let mut protected = false;
    let mut last_used = 0;
    let mut units = 0;
    for row in first..=last {
        // OpenTUI may fill the whole terminal row while painting a shorter
        // gutter field, leaving WRAPLINE on its background. A blank field edge
        // cannot be a content wrap and must preserve an actual new input row.
        let continuation = row > first
            && boundary != Boundary::Padded
            && wrapped(frame, row - 1)
            && !(boundary == Boundary::Panel && blank(frame, row - 1, end - 1));
        if row > first && !continuation {
            text.push_str("\r\n");
            units += 2;
        }
        let row_start = if row == first { first_start } else { start };
        if cursor.row == row && cursor.column < row_start {
            return None;
        }
        let mut cells: Vec<_> = (row_start..end)
            .filter_map(|column| {
                let cell = cell(frame, row, column)?;
                (!cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
                .then_some((column, cell))
            })
            .collect();
        while cells.last().is_some_and(|(column, cell)| {
            !cell.flags.contains(Flags::HIDDEN)
                && cell.text.chars().all(|ch| ch == ' ')
                && (row != cursor.row || *column >= cursor.column)
        }) {
            cells.pop();
        }
        for (column, cell) in cells {
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            if row == cursor.row && caret.is_none() && cursor.column < column + width {
                caret = Some(units as u32)
            }
            if cell.flags.contains(Flags::HIDDEN) {
                protected = true;
                let redacted = cell.text.encode_utf16().count().max(1);
                text.extend(std::iter::repeat_n(' ', redacted));
                units += redacted;
            } else {
                if cell.text.chars().any(char::is_control) {
                    return None;
                }
                text.push_str(&cell.text);
                units += cell.text.encode_utf16().count();
            }
            if units > MAX_FIELD_UNITS {
                return None;
            }
        }
        if row == cursor.row && caret.is_none() {
            caret = Some(units as u32)
        }
        if row <= cursor.row || !text[last_used..].trim_matches(['\r', '\n']).is_empty() {
            last_used = text.len()
        }
    }
    text.truncate(last_used);
    Some(Composer {
        text,
        caret: if protected { None } else { caret },
        first_row: bounds.top,
        last_row: bounds.bottom,
        first_column: bounds.left,
        last_column: bounds.right,
        protected,
        shell: false,
        boundary,
        source: Source::of(frame),
        anchor: cursor,
        panel: None,
        adapter: crate::tui_adapters::AdapterKind::Universal,
    })
}

fn matches_source(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    first: usize,
    last: usize,
) -> bool {
    let units: Vec<_> = document.text.encode_utf16().collect();
    for row in first..=last {
        let begin = document
            .cells
            .partition_point(|cell| cell.line < row as i32);
        let row_cells = &document.cells[begin
            ..document
                .cells
                .partition_point(|cell| cell.line <= row as i32)];
        for column in 0..frame.cols {
            let source = &frame.cells[row * frame.cols + column];
            if source
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                || blank(frame, row, column)
            {
                continue;
            }
            let Some(cell) = row_cells.iter().find(|cell| cell.column == column) else {
                return false;
            };
            let Some(text) = units.get(cell.start as usize..cell.end as usize) else {
                return false;
            };
            if source.flags.contains(Flags::HIDDEN) {
                if text.iter().any(|unit| *unit != b' ' as u16) {
                    return false;
                }
            } else if !text.iter().copied().eq(source.text.encode_utf16()) {
                return false;
            }
        }
    }
    true
}

fn without_rows(document: &ReadingDocument, first: usize, last: usize) -> Option<ReadingDocument> {
    if first > last || last > i32::MAX as usize {
        return None;
    }
    without_matching_rows(document, |row| row >= first as i32 && row <= last as i32)
}

fn without_matching_rows(
    document: &ReadingDocument,
    removed: impl Fn(i32) -> bool,
) -> Option<ReadingDocument> {
    let source: Vec<_> = document.text.encode_utf16().collect();
    let mut units = Vec::new();
    let mut output = ReadingDocument {
        revision: document.revision,
        alternate_screen: document.alternate_screen,
        text: String::new(),
        cells: Vec::new(),
        lines: Vec::new(),
        styles: Vec::new(),
    };
    let mut cell_index = 0;
    let mut style_index = 0;
    for line in &document.lines {
        if line.start > line.end || line.end as usize > source.len() {
            return None;
        }
        let first_cell = cell_index;
        while document
            .cells
            .get(cell_index)
            .is_some_and(|cell| cell.start < line.end)
        {
            cell_index += 1
        }
        let cells = &document.cells[first_cell..cell_index];
        // Gaps between anchors can already be removed composer/menu rows.
        // They are not proof of empty rows inside this line: a second removal
        // must not recreate the field that the first projection excluded.
        let Some(anchor) = std::iter::once(line.line)
            .chain(cells.iter().map(|cell| cell.line))
            .find(|row| !removed(*row))
        else {
            continue;
        };
        if !output.lines.is_empty() {
            let start = units.len() as u32;
            units.extend_from_slice(&[13, 10]);
            StyleSpan::append(&mut output.styles, start, start + 2, TextStyle::default());
        }
        let line_start = units.len() as u32;
        for cell in cells.iter().filter(|cell| !removed(cell.line)) {
            if cell.start < line.start || cell.end > line.end || cell.start > cell.end {
                return None;
            }
            let text = source.get(cell.start as usize..cell.end as usize)?;
            let start = units.len() as u32;
            units.extend_from_slice(text);
            let end = units.len() as u32;
            output.cells.push(bks_terminal::ReadingCell {
                start,
                end,
                ..*cell
            });
            while document
                .styles
                .get(style_index)
                .is_some_and(|span| span.end <= cell.start)
            {
                style_index += 1
            }
            let mut at = style_index;
            while let Some(span) = document.styles.get(at).filter(|span| span.start < cell.end) {
                let old_start = span.start.max(cell.start);
                let old_end = span.end.min(cell.end);
                StyleSpan::append(
                    &mut output.styles,
                    start + old_start - cell.start,
                    start + old_end - cell.start,
                    span.style,
                );
                at += 1;
            }
        }
        output.lines.push(ReadingLine {
            start: line_start,
            end: units.len() as u32,
            line: anchor,
        });
    }
    output.text = String::from_utf16(&units).ok()?;
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{Color, NamedColor, TerminalModel};

    fn fixture(
        cols: usize,
        rows: usize,
        text: &str,
        cursor: (usize, usize),
    ) -> (TerminalSnapshot, ReadingDocument) {
        let mut model = TerminalModel::new(cols, rows);
        model.push_bytes(text.as_bytes());
        model.push_bytes(format!("\x1b[{};{}H", cursor.0 + 1, cursor.1 + 1).as_bytes());
        (model.live_snapshot(), model.reading_document())
    }

    fn check_document(document: &ReadingDocument) {
        let units: Vec<_> = document.text.encode_utf16().collect();
        for cell in &document.cells {
            assert!(cell.start < cell.end && cell.end as usize <= units.len());
            assert!(String::from_utf16(&units[cell.start as usize..cell.end as usize]).is_ok());
        }
        for line in &document.lines {
            assert!(line.start <= line.end && line.end as usize <= units.len())
        }
        assert_eq!(
            document.styles.first().map(|span| span.start),
            (!units.is_empty()).then_some(0)
        );
        assert_eq!(
            document.styles.last().map(|span| span.end),
            (!units.is_empty()).then_some(units.len() as u32)
        );
        for pair in document.styles.windows(2) {
            assert_eq!(pair[0].end, pair[1].start)
        }
    }

    fn push_screen_into_history(model: &mut TerminalModel, rows: usize, next: &str) {
        model.push_bytes(
            format!("\x1b[{rows};1H{}\x1b[2J\x1b[H{next}", "\r\n".repeat(rows)).as_bytes(),
        );
    }

    fn panel_model(body: &[&str], cursor: (usize, usize), placeholder: bool) -> TerminalModel {
        let mut model = TerminalModel::new(48, 18);
        let top = 2;
        let left = 4;
        let start = left + 3;
        let end = 37;
        let bottom = top + body.len() + 3;
        let mut source = String::from("\x1b[38;2;44;190;90mreply\x1b[0m");
        for row in top..bottom {
            source.push_str(&format!(
                "\x1b[{};{}H\x1b[48;2;10;10;10m\x1b[38;2;92;156;245m┃\x1b[48;2;30;30;30m{}",
                row + 1,
                left + 1,
                " ".repeat(end - left)
            ));
        }
        for (line, text) in body.iter().enumerate() {
            let shade = if placeholder { 128 } else { 238 };
            source.push_str(&format!(
                "\x1b[{};{}H\x1b[38;2;{shade};{shade};{shade}m{text}",
                top + 2 + line,
                start + 1
            ));
        }
        source.push_str(&format!(
            "\x1b[{};{}H\x1b[38;2;92;156;245mrole\x1b[38;2;128;128;128m · model",
            bottom,
            start + 1
        ));
        source.push_str(&format!(
            "\x1b[{};{}H\x1b[48;2;10;10;10m\x1b[38;2;92;156;245m╹\x1b[38;2;30;30;30m{}\x1b[0m\x1b[{};1Hfooter\x1b[{};{}H",
            bottom + 1, left + 1, "▀".repeat(end - left), bottom + 2,
            top + 2 + cursor.0, start + cursor.1 + 1
        ));
        model.push_bytes(source.as_bytes());
        model
    }

    fn panel_fixture(
        body: &[&str],
        cursor: (usize, usize),
        placeholder: bool,
        revision: u64,
    ) -> (TerminalSnapshot, ReadingDocument) {
        let model = panel_model(body, cursor, placeholder);
        let frame = model.live_snapshot();
        let mut document = model.reading_document();
        document.revision = revision;
        (frame, document)
    }

    fn accepted(previous: &Composer, text: &str, revision: u64) -> AcceptedSourceEvidence {
        AcceptedSourceEvidence {
            text: text.into(),
            row: previous.anchor.row,
            column: previous.anchor.column,
            before_revision: revision,
        }
    }

    #[test]
    fn initial_opencode_input_requires_exact_accepted_prefix_origin_and_visible_caret() {
        use crate::tui_adapters::AdapterKind;
        let (frame, document) = panel_fixture(&["пр"], (0, 2), false, 2);
        let evidence = AcceptedSourceEvidence {
            text: "привет".into(),
            row: 3,
            column: 7,
            before_revision: 1,
        };
        let confirmed = project_for_adapter(
            AdapterKind::OpenCode,
            &frame,
            &document,
            None,
            Some(&evidence),
        );
        let input = confirmed.input.unwrap();
        assert_eq!(input.text, "пр");
        assert_eq!(input.caret, Some(2));
        assert!(!confirmed.output.text.contains("пр"));
        check_document(&confirmed.output);
        for case in [
            "missing evidence",
            "unaccepted glyph",
            "wrong row",
            "wrong column",
            "old revision",
            "wrong caret",
            "hidden cursor",
            "protected",
            "too much input",
            "generic adapter",
        ] {
            let mut changed = frame.clone();
            let mut accepted = evidence.clone();
            let mut adapter = AdapterKind::OpenCode;
            match case {
                "missing evidence" => {}
                "unaccepted glyph" => accepted.text = "different".into(),
                "wrong row" => accepted.row -= 1,
                "wrong column" => accepted.column += 1,
                "old revision" => accepted.before_revision = document.revision,
                "wrong caret" => changed.cursor.as_mut().unwrap().column -= 1,
                "hidden cursor" => changed.cursor = None,
                "protected" => changed.cells[3 * changed.cols + 7]
                    .flags
                    .insert(Flags::HIDDEN),
                "too much input" => accepted.text = "пр".repeat(33),
                "generic adapter" => adapter = AdapterKind::Auto,
                _ => unreachable!(),
            }
            let projection = project_for_adapter(
                adapter,
                &changed,
                &document,
                None,
                (case != "missing evidence").then_some(&accepted),
            );
            assert!(
                projection.input.is_none_or(|input| input.text.is_empty()),
                "{case} must not transfer source text"
            );
        }
        let (prefilled, document) = panel_fixture(&["draftпр"], (0, 7), false, 2);
        assert!(project_for_adapter(
            AdapterKind::OpenCode,
            &prefilled,
            &document,
            None,
            Some(&evidence)
        )
        .input
        .is_none());
    }

    #[test]
    fn resized_opencode_field_requires_all_prior_text_palette_and_buffer_proof() {
        use crate::tui_adapters::AdapterKind;
        let adapter = AdapterKind::OpenCode;
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let hint = project_for_adapter(adapter, &frame, &document, None, None)
            .input
            .unwrap();
        let (frame, document) = panel_fixture(&["old text"], (0, 8), false, 2);
        let baseline = project_for_adapter(
            adapter,
            &frame,
            &document,
            Some(&hint),
            Some(&accepted(&hint, "old text", 1)),
        )
        .input
        .unwrap();
        let mut resized = panel_model(&["old", "text"], (1, 4), false);
        resized.resize(60, 22);
        let frame = resized.live_snapshot();
        let document = resized.reading_document();
        let restored = project_resized_for_adapter(adapter, &frame, &document, &baseline);
        let input = restored.input.unwrap();
        assert_eq!(input.text, "old text");
        assert_eq!(input.caret, Some(8));
        assert!(input.matches_source(&frame));
        assert!(!restored.output.text.contains("old"));
        check_document(&restored.output);

        for case in [
            "hidden cursor",
            "history",
            "alternate",
            "palette",
            "flags",
            "protected",
            "changed glyph",
            "document",
            "shell mode",
            "unacknowledged hint",
            "other adapter",
        ] {
            let mut changed = frame.clone();
            let mut changed_document = document.clone();
            let mut old = baseline.clone();
            let mut selected = adapter;
            let glyph = 3 * changed.cols + 7;
            match case {
                "hidden cursor" => changed.cursor = None,
                "history" => changed.history_size += 1,
                "alternate" => changed.alternate_screen = !changed.alternate_screen,
                "palette" => changed.cells[glyph].fg = Color::Named(NamedColor::Red),
                "flags" => changed.cells[glyph].flags.insert(Flags::BOLD),
                "protected" => changed.cells[glyph].flags.insert(Flags::HIDDEN),
                "changed glyph" => changed.cells[glyph].text = "X".into(),
                "document" => changed_document.text = "unrelated output".into(),
                "shell mode" => old.shell = true,
                "unacknowledged hint" => old = hint.clone(),
                "other adapter" => selected = AdapterKind::Kimi,
                _ => unreachable!(),
            }
            let rejected = project_resized_for_adapter(selected, &changed, &changed_document, &old);
            assert!(rejected.input.is_none(), "{case}");
            assert_eq!(rejected.output, changed_document, "{case}");
        }
    }

    #[test]
    fn recovered_panel_rejects_unaccepted_text_even_with_an_acknowledged_palette() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let empty = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["x"], (0, 1), false, 2);
        let baseline = project_confirmed(
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "x", 1)),
        )
        .input
        .unwrap();
        assert!(baseline.panel.as_ref().unwrap().acknowledged);
        let evidence = accepted(&baseline, "/", 2);
        let (frame, document) = panel_fixture(&["unexpected"], (0, 10), false, 3);
        // Ordinary continuation deliberately accepts edits in a known field;
        // a private recovery baseline must not inherit that permission.
        assert_eq!(
            project_confirmed(&frame, &document, Some(&baseline), Some(&evidence))
                .input
                .unwrap()
                .text,
            "unexpected"
        );
        let rejected = project_recovered(&frame, &document, &baseline, &evidence);
        assert!(rejected.input.is_none());
        assert_eq!(rejected.output, document);

        let (frame, document) = panel_fixture(&["x/"], (0, 2), false, 3);
        let recovered = project_recovered(&frame, &document, &baseline, &evidence);
        assert_eq!(recovered.input.unwrap().text, "x/");
    }

    #[test]
    fn recovered_panel_requires_a_visible_cursor_at_the_exact_accepted_prefix() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let baseline = project(&frame, &document, None).input.unwrap();
        let evidence = accepted(&baseline, "/he", 1);
        for text in ["/", "/he"] {
            let (mut frame, document) = panel_fixture(&[text], (0, text.len()), false, 2);
            let recovered = project_recovered(&frame, &document, &baseline, &evidence);
            let input = recovered.input.unwrap();
            assert_eq!(input.text, text);
            assert_eq!(input.caret, Some(text.len() as u32));
            assert!(!recovered.output.text.contains(text));
            check_document(&recovered.output);
            frame.cursor = None;
            let hidden = project_recovered(&frame, &document, &baseline, &evidence);
            assert!(hidden.input.is_none());
            assert_eq!(hidden.output, document);
        }
        let (frame, document) = panel_fixture(&["/he"], (0, 2), false, 2);
        let wrong_caret = project_recovered(&frame, &document, &baseline, &evidence);
        assert!(wrong_caret.input.is_none());
        assert_eq!(wrong_caret.output, document);
    }

    #[test]
    fn continued_panel_restores_only_identical_visible_panel_or_padded_source() {
        type Fixture =
            fn(&[&str], (usize, usize), bool, u64) -> (TerminalSnapshot, ReadingDocument);
        for fixture in [panel_fixture as Fixture, padded_fixture as Fixture] {
            let (frame, document) = fixture(&["hint"], (0, 0), true, 1);
            let empty = project(&frame, &document, None).input.unwrap();
            let (mut frame, document) = fixture(&["/"], (0, 1), false, 2);
            let baseline = project_confirmed(
                &frame,
                &document,
                Some(&empty),
                Some(&accepted(&empty, "/", 1)),
            )
            .input
            .unwrap();
            let continued = project_continued(&frame, &document, &baseline);
            assert_eq!(continued.input.as_ref().unwrap().text, "/");
            assert_eq!(continued.input.as_ref().unwrap().caret, Some(1));
            assert!(!continued.output.text.contains('/'));
            check_document(&continued.output);
            let (home, home_document) = fixture(&["/"], (0, 0), false, 3);
            let moved_caret = project_continued(&home, &home_document, &baseline)
                .input
                .unwrap();
            assert_eq!(moved_caret.text, "/");
            assert_eq!(moved_caret.caret, Some(0));
            frame.cursor = None;
            let hidden = project_continued(&frame, &document, &baseline);
            assert!(hidden.input.is_none());
            assert_eq!(hidden.output, document);
            let (frame, document) = fixture(&["/x"], (0, 2), false, 3);
            assert_eq!(
                project(&frame, &document, Some(&baseline))
                    .input
                    .unwrap()
                    .text,
                "/x"
            );
            let changed = project_continued(&frame, &document, &baseline);
            assert!(changed.input.is_none());
            assert_eq!(changed.output, document);
        }
    }

    #[test]
    fn continued_panel_allows_vertical_translation_but_rejects_changed_shape_or_source() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let empty = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["/"], (0, 1), false, 2);
        let baseline = project_confirmed(
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "/", 1)),
        )
        .input
        .unwrap();
        let mut moved = panel_model(&["/"], (0, 1), false);
        moved.push_bytes(
            format!(
                "\x1b[2;1H\x1b[L\x1b[{};{}H",
                baseline.anchor.row + 2,
                baseline.anchor.column + 1
            )
            .as_bytes(),
        );
        let shifted =
            project_continued(&moved.live_snapshot(), &moved.reading_document(), &baseline);
        let shifted = shifted.input.unwrap();
        assert_eq!(shifted.text, "/");
        assert_eq!(shifted.caret, Some(1));
        assert_eq!(shifted.first_row, baseline.first_row + 1);
        assert_eq!(shifted.last_row, baseline.last_row + 1);
        let taller = panel_model(&["/", ""], (0, 1), false);
        let rejected = project_continued(
            &taller.live_snapshot(),
            &taller.reading_document(),
            &baseline,
        );
        assert!(rejected.input.is_none());
        assert_eq!(rejected.output, taller.reading_document());
        for case in ["cursor", "history", "alternate", "color", "flags"] {
            let mut changed = frame.clone();
            match case {
                "cursor" => changed.cursor = Some(CursorSnapshot { row: 0, column: 0 }),
                "history" => changed.history_size += 1,
                "alternate" => changed.alternate_screen = !changed.alternate_screen,
                "color" => {
                    changed.cells[baseline.anchor.row * changed.cols + baseline.first_column + 3]
                        .fg = Color::Named(NamedColor::Red)
                }
                "flags" => changed.cells
                    [baseline.anchor.row * changed.cols + baseline.first_column + 3]
                    .flags
                    .insert(Flags::BOLD),
                _ => unreachable!(),
            }
            let rejected = project_continued(&changed, &document, &baseline);
            assert!(rejected.input.is_none(), "{case}");
            assert_eq!(rejected.output, document, "{case}");
        }
    }

    // Source-equivalent geometry from Codex 0.164.0 composer_layout.rs and
    // chat_composer.rs: uniform background, one padding row, a two-column
    // prompt, one right margin, then a separate footer. No app label is used.
    fn padded_fixture(
        body: &[&str],
        cursor: (usize, usize),
        placeholder: bool,
        revision: u64,
    ) -> (TerminalSnapshot, ReadingDocument) {
        padded_fixture_with_color(body, cursor, placeholder, revision, true)
    }

    fn padded_fixture_with_color(
        body: &[&str],
        cursor: (usize, usize),
        placeholder: bool,
        revision: u64,
        colored: bool,
    ) -> (TerminalSnapshot, ReadingDocument) {
        let mut model = TerminalModel::new(40, 12);
        model.push_bytes(b"\x1b[31mreply\x1b[0m");
        let top = 2;
        let bottom = top + body.len() + 1;
        for row in top..=bottom {
            model.push_bytes(
                format!(
                    "\x1b[{};1H{}{}",
                    row + 1,
                    if colored {
                        "\x1b[48;2;40;40;40m"
                    } else {
                        "\x1b[49m"
                    },
                    " ".repeat(40)
                )
                .as_bytes(),
            );
        }
        model.push_bytes(format!("\x1b[{};1H\x1b[1m›\x1b[22m", top + 2).as_bytes());
        for (row, text) in body.iter().enumerate() {
            model.push_bytes(
                format!(
                    "\x1b[{};3H{}{text}\x1b[22m",
                    top + row + 2,
                    if placeholder { "\x1b[2m" } else { "" }
                )
                .as_bytes(),
            );
        }
        model.push_bytes(
            format!(
                "\x1b[0m\x1b[{};3Hfooter\x1b[{};{}H",
                bottom + 3,
                top + cursor.0 + 2,
                cursor.1 + 3
            )
            .as_bytes(),
        );
        let mut document = model.tui_reading_document();
        document.revision = revision;
        (model.live_snapshot(), document)
    }

    #[test]
    fn padded_prompt_confirms_source_input_before_mirroring_and_never_duplicates_upper_text() {
        use crate::source_input::SourceInput;
        let now = std::time::Instant::now();
        let (frame, document) = padded_fixture(&["arbitrary dim hint"], (0, 0), true, 1);
        let original = document.clone();
        let initial = project(&frame, &document, None).input.unwrap();
        assert_eq!(initial.boundary, Boundary::Padded);
        assert!(initial.text.is_empty());
        assert_eq!(initial.caret, Some(0));
        let mut pending = SourceInput::default();
        pending.accepted(&frame, 1, "\x1b[200~Ж界🙂\x1b[201~".as_bytes(), None, now);
        assert!(pending.evidence(&frame, 1, now).is_none());
        assert_eq!(document, original);
        // Pending writer bytes do not replace the source placeholder.
        assert!(project(&frame, &document, Some(&initial))
            .input
            .unwrap()
            .text
            .is_empty());
        let (frame, document) = padded_fixture(&["Ж界🙂"], (0, 5), false, 2);
        assert!(project(&frame, &document, Some(&initial)).input.is_none());
        let raw = document.clone();
        let projection = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            pending.evidence(&frame, 2, now),
        );
        let input = projection.input.unwrap();
        assert_eq!(input.text, "Ж界🙂");
        assert_eq!(input.caret, Some(4));
        assert_eq!((input.first_row, input.last_row), (2, 4));
        assert!(!projection.output.text.contains('›'));
        assert!(!projection.output.text.contains("Ж界🙂"));
        assert!(
            projection.output.text.contains("reply") && projection.output.text.contains("footer")
        );
        assert_eq!(
            projection.output.styles[0].style.foreground,
            Color::Named(NamedColor::Red)
        );
        assert_eq!(document, raw);
        check_document(&projection.output);
        let visual = visual_document(&frame, &projection.output, &[(2, 4)]);
        assert!(!visual.document.text.contains("Ж界🙂"));
    }

    #[test]
    fn padded_prompt_keeps_multiline_unicode_and_native_source_cursor_navigation() {
        let (frame, document) = padded_fixture(&["hint"], (0, 0), true, 1);
        let empty = project(&frame, &document, None).input.unwrap();
        let (frame, document) = padded_fixture(&["first"], (0, 5), false, 2);
        let initial = project_confirmed(
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "first", 1)),
        )
        .input
        .unwrap();
        let body = ["first", "", "Ж界🙂e\u{301} tail"];
        let mut previous = initial;
        for (row, column, expected) in [(2, 11, 20), (0, 0, 0), (2, 5, 13), (0, 3, 3)] {
            let (frame, document) = padded_fixture(&body, (row, column), false, 3);
            let projection = project(&frame, &document, Some(&previous));
            let input = projection.input.unwrap();
            assert_eq!(input.text, "first\r\n\r\nЖ界🙂e\u{301} tail");
            assert_eq!(input.caret, Some(expected));
            assert_eq!((input.first_row, input.last_row), (2, 6));
            assert!(!projection.output.text.contains("first"));
            check_document(&projection.output);
            previous = input;
        }
        let (mut frame, document) = padded_fixture(&body, (0, 3), false, 4);
        frame.cursor = None;
        let retained = project(&frame, &document, Some(&previous)).input.unwrap();
        assert_eq!(retained.text, previous.text);
        assert_eq!(retained.caret, None);
        let (mut frame, document) = padded_fixture(&["replacement"], (0, 0), true, 5);
        frame.cursor = None;
        assert!(project(&frame, &document, Some(&retained)).input.is_none());
    }

    #[test]
    fn padded_prompt_rejects_ambiguous_menu_rows_and_resets_to_unentered_hint() {
        let (frame, document) = padded_fixture(&["hint"], (0, 0), true, 1);
        let empty = project(&frame, &document, None).input.unwrap();
        let (frame, document) = padded_fixture(&["draft"], (0, 5), false, 2);
        let draft = project_confirmed(
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "draft", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = padded_fixture(&["new hint"], (0, 0), true, 3);
        let reset = project(&frame, &document, Some(&draft)).input.unwrap();
        assert_eq!(reset.text, "");
        for rows in [["one", "› two"], ["one", "> two"]] {
            let (frame, document) = padded_fixture(&rows, (1, 2), false, 3);
            let output = project(&frame, &document, Some(&draft));
            assert!(output.input.is_none());
            assert_eq!(output.output, document);
        }
        let (mut frame, document) = padded_fixture(&["changed"], (0, 0), true, 4);
        frame.cursor = None;
        assert!(project(&frame, &document, Some(&draft)).input.is_none());
    }

    #[test]
    fn padded_prompt_initial_multiline_paste_requires_exact_source_with_or_without_color() {
        use crate::source_input::SourceInput;
        let now = std::time::Instant::now();
        for colored in [false, true] {
            let (frame, document) =
                padded_fixture_with_color(&["unentered hint"], (0, 0), true, 1, colored);
            let initial = project(&frame, &document, None).input.unwrap();
            assert_eq!(initial.boundary, Boundary::Padded);
            assert_eq!(initial.text, "");
            let mut pending = SourceInput::default();
            pending.accepted(
                &frame,
                1,
                "\x1b[200~Ж\n界🙂\r\n  e\u{301}\x1b[201~".as_bytes(),
                Some(&initial),
                now,
            );
            assert!(pending.evidence(&frame, 1, now).is_none());
            let expected = "Ж\r\n界🙂\r\n  e\u{301}";
            let body = ["Ж", "界🙂", "  e\u{301}"];
            let (frame, document) = padded_fixture_with_color(&body, (2, 3), false, 2, colored);
            assert!(project(&frame, &document, Some(&initial)).input.is_none());
            let projection = project_confirmed(
                &frame,
                &document,
                Some(&initial),
                pending.evidence(&frame, 2, now),
            );
            let input = projection.input.unwrap();
            assert_eq!(input.text, expected);
            assert_eq!(input.caret, Some(expected.encode_utf16().count() as u32));
            assert!(!projection.output.text.contains('Ж'));
            // Source arrows/Home move the native caret without altering the
            // pasted line breaks or the user's two indentation spaces.
            for cursor in [(0, 0), (1, 3), (2, 3)] {
                let (frame, document) = padded_fixture_with_color(&body, cursor, false, 3, colored);
                assert_eq!(
                    project(&frame, &document, Some(&input)).input.unwrap().text,
                    expected
                );
            }
        }
    }

    #[test]
    fn ruled_native_pointer_nbsp_multiline_has_no_layout_prefix_or_upper_duplicate() {
        // Claude 2.1.240's installed renderer uses horizontal borders, a
        // two-column pointer + NBSP (or $ + NBSP for screen readers), and a
        // separately positioned multiline textarea with an inverse cursor.
        for prompt in ['❯', '>', '$'] {
            for software in [false, true] {
                let vt = format!(
                    "\x1b[32mreply\x1b[0m\r\n{}\r\n{prompt}\u{00a0}Ж界🙂\r\n  next\r\n    indented\r\n{}\r\nfooter",
                    "─".repeat(38), "─".repeat(38),
                );
                for (row, column, expected) in [(2, 7, 4), (3, 4, 8), (4, 12, 22), (2, 2, 0)] {
                    let (mut frame, document) = fixture(40, 10, &vt, (row, column));
                    if software {
                        frame.cursor = None;
                        frame.cells[row * frame.cols + column]
                            .flags
                            .insert(Flags::INVERSE);
                        frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                    }
                    let source = document.clone();
                    let projection = project(&frame, &document, None);
                    let input = projection.input.unwrap();
                    assert_eq!(input.text, "Ж界🙂\r\nnext\r\n  indented");
                    assert_eq!(
                        input.caret,
                        Some(expected),
                        "{prompt} software={software} row={row}"
                    );
                    assert_eq!((input.first_row, input.last_row), (1, 5));
                    assert_eq!(projection.output.text, "reply\r\nfooter");
                    assert_eq!(
                        projection.output.styles[0].style.foreground,
                        Color::Named(NamedColor::Green)
                    );
                    assert_eq!(document, source);
                    check_document(&projection.output);
                }
            }
        }
    }

    #[test]
    fn actual_isolated_auth_and_offline_startup_screens_stay_readable_without_input() {
        for bytes in [
            include_bytes!(
                "../tests/fixtures/tui-codex-claude-composers/codex-0.164.0-auth-startup.vt"
            )
            .as_slice(),
            include_bytes!(
                "../tests/fixtures/tui-codex-claude-composers/claude-2.1.240-offline-startup.vt"
            )
            .as_slice(),
        ] {
            let mut model = TerminalModel::new(110, 36);
            model.push_bytes(bytes);
            let mut frame = model.live_snapshot();
            frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
            let document = model.tui_reading_document();
            let projection = project(&frame, &document, None);
            assert!(projection.input.is_none());
            assert_eq!(projection.output, document);
        }
    }

    #[test]
    fn actual_offline_composer_does_not_publish_its_unentered_hint() {
        let bytes = include_bytes!(
            "../tests/fixtures/tui-codex-claude-composers/codex-0.164.0-offline-composer.vt"
        );
        let mut model = TerminalModel::new(100, 29);
        model.push_bytes(bytes);
        let frame = model.live_snapshot();
        let document = model.tui_reading_document();
        let original = document.clone();
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert_eq!(input.boundary, Boundary::Padded);
        assert!(input.text.is_empty(), "{}", input.text);
        assert_eq!(input.caret, Some(0));
        assert!(!projection.output.text.contains("Ask Codex to do anything"));
        assert!(projection.output.text.contains("bks-offline"));
        assert_eq!(document, original);
        check_document(&projection.output);
    }

    #[test]
    fn selected_padded_adapter_handles_optional_footer_without_publishing_hint() {
        use crate::tui_adapters::AdapterKind;
        let (frame, mut document) = fixture(40, 10, "reply\r\n\r\n› arbitrary hint\r\n", (2, 2));
        document.revision = 1;
        assert_eq!(
            project(&frame, &document, None).input.unwrap().text,
            "arbitrary hint"
        );
        let original = document.clone();
        let input = project_for_adapter(AdapterKind::Codex, &frame, &document, None, None)
            .input
            .unwrap();
        assert!(input.text.is_empty());
        assert_eq!(input.caret, Some(0));
        let (frame, mut document) = fixture(40, 10, "reply\r\n\r\n› Ж界🙂\r\n", (2, 7));
        document.revision = 2;
        assert!(
            project_for_adapter(AdapterKind::Codex, &frame, &document, Some(&input), None)
                .input
                .is_none()
        );
        let projection = project_for_adapter(
            AdapterKind::Codex,
            &frame,
            &document,
            Some(&input),
            Some(&accepted(&input, "Ж界🙂", 1)),
        );
        assert_eq!(projection.input.unwrap().text, "Ж界🙂");
        assert_eq!(original.text, "reply\r\n\r\n› arbitrary hint");
        assert!(!projection.output.text.contains("Ж界🙂"));
    }

    #[test]
    fn selected_ruled_adapter_keeps_hint_unentered_until_exact_source_ack() {
        use crate::tui_adapters::AdapterKind;
        let rule = "─".repeat(38);
        let source = |text: &str| format!("reply\r\n{rule}\r\n❯\u{00a0}{text}\r\n{rule}\r\nfooter");
        let (frame, mut document) = fixture(40, 10, &source("arbitrary hint"), (2, 2));
        document.revision = 1;
        let initial = project_for_adapter(AdapterKind::Claude, &frame, &document, None, None)
            .input
            .unwrap();
        assert!(initial.text.is_empty());
        assert_eq!(
            project(&frame, &document, None).input.unwrap().text,
            "arbitrary hint"
        );
        let (mut frame, mut document) = fixture(40, 10, &source("Ж界🙂"), (2, 7));
        document.revision = 2;
        let input = project_for_adapter(
            AdapterKind::Claude,
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "Ж界🙂", 1)),
        )
        .input
        .unwrap();
        assert_eq!(input.text, "Ж界🙂");
        frame.cursor = None;
        let retained =
            project_for_adapter(AdapterKind::Claude, &frame, &document, Some(&input), None)
                .input
                .unwrap();
        assert_eq!(retained.text, input.text);
        assert_eq!(retained.caret, None);
    }

    #[test]
    fn selected_adapters_preserve_unknown_layout_and_real_trust_screen() {
        use crate::tui_adapters::AdapterKind;
        let (frame, document) = fixture(40, 8, "unknown header\r\n> arbitrary display", (1, 2));
        for adapter in [
            AdapterKind::Codex,
            AdapterKind::Claude,
            AdapterKind::OpenCode,
            AdapterKind::Kimi,
            AdapterKind::Gemini,
        ] {
            let projection = project_for_adapter(adapter, &frame, &document, None, None);
            assert!(projection.input.is_none(), "{adapter:?}");
            assert_eq!(projection.output, document, "{adapter:?}");
        }
        let mut model = TerminalModel::new(100, 29);
        model.push_bytes(include_bytes!(
            "../tests/fixtures/tui-kimi-2.4.1-offline-trust.vt"
        ));
        let mut frame = model.live_snapshot();
        frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
        let document = model.tui_reading_document();
        let projection = project_for_adapter(AdapterKind::Kimi, &frame, &document, None, None);
        assert!(projection.input.is_none());
        assert_eq!(projection.output, document);
        assert!(document.text.contains("Trust this folder?"));
    }

    #[test]
    fn selected_strategies_replay_real_unicode_and_prefix_deletion() {
        use crate::tui_adapters::AdapterKind;
        for (adapter, bytes, ends) in [
            (
                AdapterKind::Codex,
                include_bytes!(
                    "../tests/fixtures/tui-codex-claude-composers/codex-0.164.0-offline-edit.vt"
                )
                .as_slice(),
                [4132, 4188, 4222, 4242, 4287],
            ),
            (
                AdapterKind::Claude,
                include_bytes!(
                    "../tests/fixtures/tui-codex-claude-composers/claude-2.1.240-offline-edit.vt"
                )
                .as_slice(),
                [2676, 2750, 2780, 2801, 2832],
            ),
        ] {
            let mut model = TerminalModel::new(100, 29);
            let mut previous = None;
            let mut begin = 0;
            for (phase, end) in ends.into_iter().enumerate() {
                let evidence = (phase == 1).then(|| {
                    accepted(
                        previous.as_ref().unwrap(),
                        "Ж界🙂",
                        model.reading_revision(),
                    )
                });
                model.push_bytes(&bytes[begin..end]);
                begin = end;
                let mut frame = model.live_snapshot();
                frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                let document = model.tui_reading_document();
                if matches!(phase, 2 | 3) {
                    let restored = project_continued_for_adapter(
                        adapter,
                        &frame,
                        &document,
                        previous.as_ref().unwrap(),
                    )
                    .input
                    .unwrap_or_else(|| panic!("continuity {adapter:?} phase {phase}"));
                    assert_eq!(restored.text, "Ж界🙂");
                    assert_eq!(restored.caret, Some(if phase == 2 { 0 } else { 1 }));
                }
                let input = project_for_adapter(
                    adapter,
                    &frame,
                    &document,
                    previous.as_ref(),
                    evidence.as_ref(),
                )
                .input
                .unwrap_or_else(|| panic!("{adapter:?} phase {phase}"));
                let (text, caret) = [
                    ("", 0),
                    ("Ж界🙂", 4),
                    ("Ж界🙂", 0),
                    ("Ж界🙂", 1),
                    ("界🙂", 0),
                ][phase];
                assert_eq!(input.text, text, "{adapter:?} phase {phase}");
                assert_eq!(input.caret, Some(caret), "{adapter:?} phase {phase}");
                previous = Some(input);
            }
        }
    }

    #[test]
    fn unverified_cli_policies_offer_only_bounded_source_confirmed_fallback() {
        use crate::tui_adapters::AdapterKind;
        let source = |text: &str, padding: usize| {
            format!(
                "+--------------------+\r\n| > {text}{}|\r\n+--------------------+",
                " ".repeat(padding)
            )
        };
        for adapter in [AdapterKind::Kimi, AdapterKind::Gemini] {
            let (frame, mut document) = fixture(30, 8, &source("hint", 13), (1, 4));
            document.revision = 1;
            let initial = project_for_adapter(adapter, &frame, &document, None, None)
                .input
                .unwrap();
            assert!(initial.text.is_empty());
            let (frame, mut document) = fixture(30, 8, &source("Ж界🙂", 12), (1, 9));
            document.revision = 2;
            let rejected = project_for_adapter(adapter, &frame, &document, Some(&initial), None);
            assert!(rejected.input.is_none());
            assert_eq!(rejected.output, document);
            let confirmed = project_for_adapter(
                adapter,
                &frame,
                &document,
                Some(&initial),
                Some(&accepted(&initial, "Ж界🙂", 1)),
            )
            .input
            .unwrap();
            assert_eq!(confirmed.text, "Ж界🙂");
            assert_eq!(confirmed.caret, Some(4));
        }
    }

    #[test]
    fn bounded_adapters_keep_real_conpty_prefill_at_initial_synthetic_handoff() {
        use crate::tui_adapters::AdapterKind;
        let mut model = TerminalModel::new(100, 30);
        model.push_bytes(include_bytes!(
            "../tests/fixtures/tui-bounded-prefill-handoff.vt"
        ));
        let frame = model.live_snapshot();
        let document = model.tui_reading_document();
        let seed = "BKS_SINGLE_INPUT Ж界e\u{301}😀";
        let generic = project(&frame, &document, None).input.unwrap();
        assert_eq!(generic.boundary, Boundary::Box);
        assert_eq!(generic.text, seed);
        for adapter in [AdapterKind::Kimi, AdapterKind::Gemini] {
            let projection = project_for_adapter(adapter, &frame, &document, None, None);
            let input = projection
                .input
                .unwrap_or_else(|| panic!("{adapter:?} lost bounded source prefill"));
            assert_eq!(input.text, seed);
            assert_eq!(input.caret, Some(seed.encode_utf16().count() as u32));
            assert!(projection.output.text.contains("BKS_SINGLE_OUTPUT_UNIQUE"));
            assert!(!projection.output.text.contains(seed));
            assert_eq!(document, model.tui_reading_document());
        }
    }

    #[test]
    fn bounded_shared_prefill_keeps_navigation_and_does_not_promote_new_hints() {
        use crate::tui_adapters::AdapterKind;
        let source = |text: &str, padding: usize| {
            format!(
                "+--------------------+\r\n| > {text}{}|\r\n+--------------------+",
                " ".repeat(padding)
            )
        };
        for adapter in [AdapterKind::Kimi, AdapterKind::Gemini] {
            let (frame, document) = fixture(40, 10, &source("Ж界🙂", 12), (1, 9));
            let prefill = project_for_adapter(adapter, &frame, &document, None, None)
                .input
                .unwrap();
            assert_eq!(prefill.text, "Ж界🙂");
            let (frame, document) = fixture(40, 10, &source("Ж界🙂", 12), (1, 4));
            let home = project_for_adapter(adapter, &frame, &document, Some(&prefill), None)
                .input
                .unwrap();
            assert_eq!(home.text, "Ж界🙂");
            assert_eq!(home.caret, Some(0));
            let continued = project_continued_for_adapter(adapter, &frame, &document, &prefill)
                .input
                .unwrap();
            assert_eq!(continued.text, "Ж界🙂");
            assert_eq!(continued.caret, Some(0));
            let (frame, document) = fixture(40, 10, &source("界🙂", 13), (1, 4));
            let remaining = project_for_adapter(adapter, &frame, &document, Some(&home), None)
                .input
                .unwrap();
            assert_eq!(remaining.text, "界🙂");
            assert_eq!(remaining.caret, Some(0));
            let (frame, document) = fixture(40, 10, &source("unentered hint", 3), (1, 4));
            let hint = project_for_adapter(adapter, &frame, &document, Some(&remaining), None)
                .input
                .unwrap();
            assert!(hint.text.is_empty());
            let (frame, document) = fixture(40, 10, &source("unexpected", 7), (1, 14));
            let unconfirmed = project_for_adapter(adapter, &frame, &document, Some(&hint), None);
            assert!(unconfirmed.input.is_none());
            assert_eq!(unconfirmed.output, document);
            // Shared inline-rule layout has a different content prefix from
            // the specialized ruled adapter. Its geometry and end caret still
            // prove a current bounded prefill after a layout transition.
            let rule = "─".repeat(12);
            let (frame, document) = fixture(
                40,
                10,
                &format!("before\r\n{rule}❯ Ж界🙂\r\n{rule}\r\nfooter"),
                (1, 19),
            );
            let ruled = project_for_adapter(adapter, &frame, &document, Some(&remaining), None)
                .input
                .unwrap();
            assert_eq!(ruled.text, "Ж界🙂");
            assert_eq!(ruled.caret, Some(4));
        }
    }

    #[test]
    fn captured_padded_wide_tail_deletion_recovers_only_after_visible_cursor_tail() {
        use crate::{source_input::SourceInput, tui_adapters::AdapterKind};
        let bytes = include_bytes!("../tests/fixtures/tui-padded-hidden-delete.vt");
        let ends = [204, 384, 567, 754, 941, 1128, 1313, 1498, 1679];
        let keys: [&[u8]; 9] = [
            b"",
            "Ж".as_bytes(),
            "界".as_bytes(),
            "🙂".as_bytes(),
            b"\x1b[H",
            b"\x1b[C",
            b"\x7f",
            b"\x1b[F",
            b"\x7f",
        ];
        let expected = [
            ("", 0),
            ("Ж", 1),
            ("Ж界", 2),
            ("Ж界🙂", 4),
            ("Ж界🙂", 0),
            ("Ж界🙂", 1),
            ("界🙂", 0),
            ("界🙂", 3),
            ("界", 1),
        ];
        let now = std::time::Instant::now();
        let mut model = TerminalModel::new(100, 30);
        let mut pending = SourceInput::default();
        let mut previous = None;
        let mut begin = 0;
        for (phase, end) in ends.into_iter().enumerate() {
            if phase > 0 {
                pending.accepted(
                    &model.live_snapshot(),
                    model.reading_revision(),
                    keys[phase],
                    previous.as_ref(),
                    now,
                );
            }
            if phase == 8 {
                // Original GUI stream offsets 31333/31339, relative 1673/1679.
                model.push_bytes(&bytes[begin..1673]);
                let frame = model.live_snapshot();
                let document = model.tui_reading_document();
                assert!(frame.cursor.is_none());
                assert!(project_for_adapter(
                    AdapterKind::Shell,
                    &frame,
                    &document,
                    previous.as_ref(),
                    None
                )
                .input
                .is_none());
                let (evidence, baseline) = pending
                    .edit_confirmation(&frame, model.reading_revision(), now)
                    .unwrap();
                assert!(project_edited_for_adapter(
                    AdapterKind::Shell,
                    &frame,
                    &document,
                    baseline,
                    evidence
                )
                .input
                .is_none());
                previous = None;
                model.push_bytes(&bytes[1673..end]);
            } else {
                model.push_bytes(&bytes[begin..end]);
            }
            begin = end;
            let frame = model.live_snapshot();
            let document = model.tui_reading_document();
            let accepted = pending.confirmation(&frame, model.reading_revision(), now);
            let mut projection = project_for_adapter(
                AdapterKind::Shell,
                &frame,
                &document,
                previous.as_ref(),
                accepted.map(|(evidence, _)| evidence),
            );
            if phase == 8 {
                assert!(projection.input.is_none());
                let cursor = frame.cursor.unwrap();
                assert_eq!((cursor.row, cursor.column), (2, 4));
                assert_eq!(frame.cells[2 * frame.cols + 2].text, "界");
                assert!(frame.cells[2 * frame.cols + 3]
                    .flags
                    .contains(Flags::WIDE_CHAR_SPACER));
                let (evidence, baseline) = pending
                    .edit_confirmation(&frame, model.reading_revision(), now)
                    .unwrap();
                projection = project_edited_for_adapter(
                    AdapterKind::Shell,
                    &frame,
                    &document,
                    baseline,
                    evidence,
                );
            }
            let input = projection.input.unwrap_or_else(|| panic!("phase {phase}"));
            assert_eq!(input.text, expected[phase].0, "phase {phase}");
            assert_eq!(input.caret, Some(expected[phase].1), "phase {phase}");
            pending.acknowledge(&input);
            previous = Some(input);
        }
        assert!(pending
            .edit_confirmation(&model.live_snapshot(), model.reading_revision() + 1, now)
            .is_none());
    }

    #[test]
    fn deletion_recovery_rejects_wrong_text_caret_style_source_and_adapter() {
        use crate::tui_adapters::AdapterKind;
        let (frame, document) = padded_fixture(&["hint"], (0, 0), true, 1);
        let empty = project_for_adapter(AdapterKind::Shell, &frame, &document, None, None)
            .input
            .unwrap();
        let (frame, document) = padded_fixture(&["界🙂"], (0, 4), false, 2);
        let entered = project_for_adapter(
            AdapterKind::Shell,
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "界🙂", 1)),
        )
        .input
        .unwrap();
        let evidence =
            AcceptedEditEvidence::record(AcceptedEditKind::Backspace, &frame, 2, &entered).unwrap();
        for case in [
            "text",
            "caret",
            "style",
            "source",
            "geometry",
            "revision",
            "adapter",
            "hidden",
            "hint",
            "protected",
        ] {
            let (mut current, mut document) = match case {
                "text" => padded_fixture(&["other"], (0, 5), false, 3),
                "caret" => padded_fixture(&["界"], (0, 0), false, 3),
                "style" => padded_fixture(&["\x1b[31m界\x1b[39m"], (0, 2), false, 3),
                "geometry" => padded_fixture(&["界", ""], (0, 2), false, 3),
                "hint" => padded_fixture(&["new hint"], (0, 0), true, 3),
                "protected" => padded_fixture(&["\x1b[8m界\x1b[28m"], (0, 2), false, 3),
                _ => padded_fixture(&["界"], (0, 2), false, 3),
            };
            if case == "source" {
                current.history_size += 1;
            }
            if case == "revision" {
                document.revision = 2;
            }
            if case == "hidden" {
                current.cursor = None;
            }
            let adapter = if case == "adapter" {
                AdapterKind::Codex
            } else {
                AdapterKind::Shell
            };
            let result =
                project_edited_for_adapter(adapter, &current, &document, &entered, &evidence);
            assert!(result.input.is_none(), "{case}");
            assert_eq!(result.output, document, "{case}");
        }
    }

    #[test]
    fn accepted_delete_uses_observed_delta_without_predicting_unicode_width() {
        use crate::tui_adapters::AdapterKind;
        let (frame, document) = padded_fixture(&["hint"], (0, 0), true, 1);
        let empty = project_for_adapter(AdapterKind::Shell, &frame, &document, None, None)
            .input
            .unwrap();
        let (frame, document) = padded_fixture(&["界🙂"], (0, 4), false, 2);
        let entered = project_for_adapter(
            AdapterKind::Shell,
            &frame,
            &document,
            Some(&empty),
            Some(&accepted(&empty, "界🙂", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = padded_fixture(&["界🙂"], (0, 0), false, 3);
        let home = project_for_adapter(AdapterKind::Shell, &frame, &document, Some(&entered), None)
            .input
            .unwrap();
        let evidence =
            AcceptedEditEvidence::record(AcceptedEditKind::Delete, &frame, 3, &home).unwrap();
        let (current, document) = padded_fixture(&["🙂"], (0, 0), false, 4);
        let result =
            project_edited_for_adapter(AdapterKind::Shell, &current, &document, &home, &evidence)
                .input
                .unwrap();
        assert_eq!(result.text, "🙂");
        assert_eq!(result.caret, Some(0));
        let (current, document) = padded_fixture(&["界"], (0, 1), false, 4);
        assert!(project_edited_for_adapter(
            AdapterKind::Shell,
            &current,
            &document,
            &home,
            &evidence
        )
        .input
        .is_none());
    }

    #[test]
    fn actual_offline_editors_confirm_unicode_navigation_deletion_clear_and_menu() {
        use crate::source_input::SourceInput;
        let now = std::time::Instant::now();
        for (label, bytes, ends, clear) in [
            (
                "padded",
                include_bytes!(
                    "../tests/fixtures/tui-codex-claude-composers/codex-0.164.0-offline-edit.vt"
                )
                .as_slice(),
                [4132, 4188, 4222, 4242, 4287, 4307, 4375, 5267, 5540, 6101],
                b"\x7f\x7f".as_slice(),
            ),
            (
                "ruled",
                include_bytes!(
                    "../tests/fixtures/tui-codex-claude-composers/claude-2.1.240-offline-edit.vt"
                )
                .as_slice(),
                [2676, 2750, 2780, 2801, 2832, 2872, 3017, 3469, 3690, 3791],
                b"\x15".as_slice(),
            ),
        ] {
            let inputs = [
                b"".as_slice(),
                "\x1b[200~Ж界🙂\x1b[201~".as_bytes(),
                b"\x1b[H",
                b"\x1b[C",
                b"\x7f",
                b"\x1b[F",
                clear,
                b"/",
                b"\x1b[B",
                b"\x1b",
            ];
            let expected = [
                ("", 0),
                ("Ж界🙂", 4),
                ("Ж界🙂", 0),
                ("Ж界🙂", 1),
                ("界🙂", 0),
                ("界🙂", 3),
                ("", 0),
                ("/", 1),
                ("/", 1),
                ("/", 1),
            ];
            let mut model = TerminalModel::new(100, 29);
            let mut previous = None;
            let mut pending = SourceInput::default();
            let mut begin = 0;
            for (phase, end) in ends.into_iter().enumerate() {
                if phase > 0 {
                    let mut before = model.live_snapshot();
                    before.cursor = crate::direct_accessibility::reading_cursor(&before, None);
                    pending.accepted(
                        &before,
                        model.reading_revision(),
                        inputs[phase],
                        previous.as_ref(),
                        now,
                    );
                }
                model.push_bytes(&bytes[begin..end]);
                begin = end;
                let mut frame = model.live_snapshot();
                frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                let raw = model.tui_reading_document();
                let projection = project_confirmed(
                    &frame,
                    &raw,
                    previous.as_ref(),
                    pending.evidence(&frame, model.reading_revision(), now),
                );
                let input = projection
                    .input
                    .unwrap_or_else(|| panic!("{label} phase {phase}: {}", raw.text));
                assert_eq!(
                    input.text,
                    expected[phase].0,
                    "{label} phase {phase}: old={:?}; new={:?}",
                    previous
                        .as_ref()
                        .and_then(|input: &Composer| input.panel.as_ref())
                        .map(|proof| &proof.palette),
                    input
                        .panel
                        .as_ref()
                        .map(|proof| &proof.rendered[..proof.rendered.len().min(8)])
                );
                assert_eq!(
                    input.caret,
                    Some(expected[phase].1),
                    "{label} phase {phase}"
                );
                assert!(
                    !projection.output.text.contains("Ж界🙂"),
                    "{label} phase {phase}"
                );
                assert!(!projection.output.text.contains("Ask Codex to do anything"));
                assert_eq!(raw, model.tui_reading_document());
                check_document(&projection.output);
                pending.acknowledge(&input);
                previous = Some(input);
            }
        }
    }

    #[test]
    fn plain_padded_hint_and_source_edits_allow_a_right_aligned_footer() {
        use crate::source_input::SourceInput;
        let now = std::time::Instant::now();
        for gap in [0, 1] {
            for footer_column in [2, 28] {
                let body = |text: &str| {
                    format!(
                        "reply\r\n\r\n› {text}\r\n\r\n\x1b[{};{}Hstatus info\x1b[3;3H",
                        5 + gap,
                        footer_column + 1,
                    )
                };
                let (frame, mut document) =
                    fixture(40, 10, &body("\x1b[2marbitrary hint\x1b[22m"), (2, 2));
                document.revision = 1;
                let projection = project(&frame, &document, None);
                let initial = projection.input.unwrap();
                assert_eq!(initial.boundary, Boundary::Padded);
                assert!(initial.text.is_empty());
                let mut pending = SourceInput::default();
                pending.accepted(&frame, 1, "Ж界🙂".as_bytes(), Some(&initial), now);
                let (frame, mut document) = fixture(40, 10, &body("Ж界🙂"), (2, 7));
                document.revision = 2;
                let input = project_confirmed(
                    &frame,
                    &document,
                    Some(&initial),
                    pending.evidence(&frame, 2, now),
                )
                .input
                .unwrap();
                assert_eq!(input.text, "Ж界🙂");
                assert_eq!(input.caret, Some(4));
                let (frame, mut document) = fixture(40, 10, &body("Ж界🙂"), (2, 2));
                document.revision = 3;
                let home = project(&frame, &document, Some(&input)).input.unwrap();
                assert_eq!(home.text, "Ж界🙂");
                assert_eq!(home.caret, Some(0));
                assert!(!projection.output.text.contains("arbitrary hint"));
                assert!(projection.output.text.contains("status info"));
                check_document(&projection.output);
            }
        }
    }

    #[test]
    fn padded_background_with_adjacent_body_cannot_fall_back_to_a_bare_field() {
        for side_column in [0, 35] {
            let mut vt = String::from("reply");
            for row in 2..=4 {
                vt.push_str(&format!(
                    "\x1b[{};6H\x1b[48;2;40;40;40m{}",
                    row + 1,
                    " ".repeat(28)
                ));
            }
            vt.push_str(&format!(
                "\x1b[4;6H› hint\x1b[0m\x1b[4;{}HSIDE\x1b[7;3Hfooter",
                side_column + 1
            ));
            let (frame, document) = fixture(44, 10, &vt, (3, 7));
            let projection = project(&frame, &document, None);
            assert!(projection.input.is_none());
            assert_eq!(projection.output, document);
            assert!(projection.output.text.contains("SIDE"));
        }
    }

    fn check_visual_mapping(source: &ReadingDocument, visual: &VisualDocument) {
        check_document(&visual.document);
        let source_units: Vec<_> = source.text.encode_utf16().collect();
        let visual_units: Vec<_> = visual.document.text.encode_utf16().collect();
        for original in &source.cells {
            let start = visual.visual_offset(original.start);
            let cell = visual.document.cell_at(start).unwrap();
            assert_eq!(
                (cell.line, cell.column, cell.width),
                (original.line, original.column, original.width)
            );
            assert_eq!(visual.source_offset(start), original.start);
            assert_eq!(visual.source_offset(cell.end), original.end);
            assert_eq!(
                &visual_units[cell.start as usize..cell.end as usize],
                &source_units[original.start as usize..original.end as usize]
            );
        }
    }

    #[test]
    fn visual_rows_keep_full_backgrounds_unicode_and_empty_line_offsets() {
        let (frame, source) = fixture(
            24,
            8,
            "\x1b[48;2;21;32;43m\x1b[2J\x1b[2;4H界🙂e\u{301}\x1b[4;7Htail\x1b[6;1H\x1b[8msecret🙂\x1b[28m\tX",
            (7, 0),
        );
        let original = source.clone();
        let visual =
            visual_document_inner(&frame, &source, &[]).expect("matching native-grid source");
        assert_eq!(source, original);
        assert!(!visual.document.text.contains("secret"));
        assert_eq!(
            visual
                .document
                .lines
                .iter()
                .filter(|line| line.line >= 0)
                .count(),
            8
        );
        assert_eq!(
            visual.document.text.split("\r\n").count(),
            visual.document.lines.len()
        );
        for (row, line) in visual
            .document
            .lines
            .iter()
            .filter(|line| line.line >= 0)
            .enumerate()
        {
            assert_eq!(line.line, row as i32);
            let cells: Vec<_> = visual
                .document
                .cells
                .iter()
                .filter(|cell| cell.line == line.line)
                .collect();
            assert_eq!(
                cells.iter().map(|cell| cell.width).sum::<usize>(),
                frame.cols
            );
            let padding = cells.last().unwrap();
            let style = visual
                .document
                .styles
                .iter()
                .find(|span| span.start <= padding.start && span.end > padding.start)
                .unwrap();
            assert_eq!(
                style.style.background,
                Color::Spec(bks_terminal::Rgb {
                    r: 21,
                    g: 32,
                    b: 43
                })
            );
        }
        for original_line in &source.lines {
            let mapped = visual.visual_offset(original_line.start);
            assert_eq!(visual.source_offset(mapped), original_line.start);
            assert_eq!(
                mapped,
                visual
                    .document
                    .lines
                    .iter()
                    .find(|line| line.line == original_line.line)
                    .unwrap()
                    .start
            );
        }
        check_visual_mapping(&source, &visual);
    }

    #[test]
    fn visual_captured_fullscreen_retains_rows_and_source_menu_selection() {
        let bytes = include_bytes!("../tests/fixtures/gutter-panel-menu.vt");
        let mut model = TerminalModel::new(110, 36);
        model.push_bytes(&bytes[..13_222]);
        let initial = project(&model.live_snapshot(), &model.reading_document(), None);
        let input = initial.input.unwrap();
        let previous_revision = model.reading_revision();
        model.push_bytes(&bytes[13_222..14_947]);
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let source = project_confirmed(
            &frame,
            &raw,
            Some(&input),
            Some(&accepted(&input, "/", previous_revision)),
        )
        .output;
        let original = source.clone();
        let visual =
            visual_document_inner(&frame, &source, &[(18, 22)]).expect("matching captured source");
        assert_eq!(source, original);
        assert!(
            source.lines.len() < 5,
            "captured space-filled rows are logically soft-wrapped"
        );
        assert!(visual.document.lines.len() >= 30);
        assert!(visual.document.text.contains("/agents"));
        assert!(!visual
            .document
            .lines
            .iter()
            .any(|line| (18..=22).contains(&line.line)));
        let logo = visual
            .document
            .cells
            .iter()
            .find(|cell| cell.line == 12 && cell.column == 69)
            .unwrap();
        assert_eq!(
            visual
                .document
                .lines
                .iter()
                .position(|line| line.line == logo.line),
            Some(12)
        );
        check_visual_mapping(&source, &visual);
        let start = source.text[..source.text.find("/agents").unwrap()]
            .encode_utf16()
            .count() as u32;
        let end = start + "/agents".len() as u32;
        let mapped = (visual.visual_offset(start), visual.visual_offset(end));
        assert_eq!(
            (
                visual.source_offset(mapped.0),
                visual.source_offset(mapped.1)
            ),
            (start, end)
        );
        let units: Vec<_> = visual.document.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[mapped.0 as usize..mapped.1 as usize]).unwrap(),
            "/agents"
        );
        // A separately exposed native source menu remains excluded from the
        // visual document, just as it already is from the canonical reader.
        let menu_row = source.cell_at(start).unwrap().line as usize;
        let filtered = exclude_rows(&source, menu_row, menu_row);
        let filtered_view = visual_document(&frame, &filtered, &[(18, 22), (menu_row, menu_row)]);
        assert!(!filtered_view.document.text.contains("/agents"));
        assert!(!filtered_view
            .document
            .lines
            .iter()
            .any(|line| line.line == menu_row as i32));
        check_visual_mapping(&filtered, &filtered_view);
    }

    #[test]
    fn physical_reader_keeps_captured_menu_composer_and_source_offsets() {
        let bytes = include_bytes!("../tests/fixtures/gutter-panel-menu.vt");
        let mut model = TerminalModel::new(110, 36);
        let mut previous = None;
        let mut menu_observer = crate::tui_suggestions::MenuObserver::default();
        let mut begin = 0;
        for (phase, end) in [13_222, 14_947, 15_233, 15_519, 16_567]
            .into_iter()
            .enumerate()
        {
            let before_revision = model.reading_revision();
            if phase > 0 {
                let input = match phase {
                    1 => b"\x1b[200~/\x1b[201~".as_slice(),
                    2 => b"\x1b[B",
                    3 => b"\x1b[A",
                    _ => b"\x1b",
                };
                menu_observer.accepted(input, before_revision);
            }
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let frame = model.live_snapshot();
            let source = model.tui_reading_document();
            assert_eq!(source.lines.len(), frame.rows);
            let evidence =
                (phase == 1).then(|| accepted(previous.as_ref().unwrap(), "/", before_revision));
            let projection =
                project_confirmed(&frame, &source, previous.as_ref(), evidence.as_ref());
            let input = projection
                .input
                .expect("physical source keeps the composer proof");
            assert_eq!(input.text, if phase == 0 { "" } else { "/" });
            let mut excluded = vec![(input.first_row, input.last_row)];
            let mut output = projection.output;
            let menu = menu_observer.observe(
                &frame,
                source.revision,
                Some((input.first_row, input.last_row)),
            );
            if (1..=3).contains(&phase) {
                let menu = menu.expect("source menu survives physical source export");
                assert_eq!(menu.items.len(), 10);
                assert_eq!(menu.selected, usize::from(phase == 2));
                excluded.push((menu.first_row, menu.last_row));
                output = exclude_rows(&output, menu.first_row, menu.last_row);
            } else {
                assert!(menu.is_none());
            }
            assert!(!output.text.contains("Ask anything"));
            assert!(!output.text.contains("/agents"));
            for row in 0..frame.rows {
                if excluded
                    .iter()
                    .any(|(first, last)| (*first..=*last).contains(&row))
                {
                    continue;
                }
                assert!(
                    matches_source(&frame, &output, row, row),
                    "phase {phase}, row {row}"
                );
            }
            let visual = visual_document_inner(&frame, &output, &excluded)
                .unwrap_or_else(|| panic!("visual mapping of physical source, phase {phase}"));
            assert_eq!(visual.document.lines.len(), output.lines.len());
            assert_eq!(
                visual
                    .document
                    .lines
                    .iter()
                    .map(|line| line.line)
                    .collect::<Vec<_>>(),
                output
                    .lines
                    .iter()
                    .map(|line| line.line)
                    .collect::<Vec<_>>()
            );
            check_document(&output);
            check_visual_mapping(&output, &visual);
            previous = Some(input);
        }
    }

    #[test]
    fn physical_two_answer_reader_preserves_search_selection_and_painted_rows() {
        let bytes = include_bytes!("../tests/fixtures/gutter-panel-menu.vt");
        let mut model = TerminalModel::new(110, 36);
        model.push_bytes(&bytes[..13_222]);
        let cursor = model.live_snapshot().cursor.unwrap();
        let mut redraw = String::from("\x1b[H\x1b[0m\x1b[48;2;10;10;10m");
        for row in 0..18 {
            let text = match row {
                0 => "Conversation",
                2 => "Question one",
                4 => "Answer one: alpha",
                5 => "continued answer one",
                8 => "Question two",
                10 => "Answer two: beta",
                11 => "continued answer two",
                _ => "",
            };
            redraw.push_str(&format!("{text:<110}"));
        }
        redraw.push_str(&format!(
            "\x1b[5;18H Ж🙂e\u{301}\x1b[{};{}H",
            cursor.row + 1,
            cursor.column + 1,
        ));
        model.push_bytes(redraw.as_bytes());
        assert!(
            model.reading_document().lines.len() < 5,
            "continuous renderer padding must wrap"
        );
        let frame = model.live_snapshot();
        let raw = model.tui_reading_document();
        let projection = project(&frame, &raw, None);
        let input = projection.input.unwrap();
        let excluded = [(input.first_row, input.last_row)];
        let source = projection.output;
        let source_before = source.clone();
        assert_eq!(source.lines.len(), 31);
        assert!(source
            .text
            .contains("Answer one: alpha Ж🙂e\u{301}\r\ncontinued answer one"));
        assert!(source
            .text
            .contains("Answer two: beta\r\ncontinued answer two"));
        let input_view = visual_document_inner(&frame, &source, &excluded).unwrap();
        let reader_view = visual_document_inner(&frame, &source, &excluded).unwrap();
        assert_eq!(
            input_view.document, reader_view.document,
            "focus must not replace native text"
        );
        check_visual_mapping(&source, &reader_view);
        assert_eq!(reader_view.document.lines.len(), source.lines.len());
        let mut search = crate::search::SearchState::default();
        search.set_query("Answer".into(), &source);
        assert_eq!(search.matches().len(), 4);
        for found in search.matches() {
            let range = (
                reader_view.visual_offset(found.start),
                reader_view.visual_offset(found.end),
            );
            let units: Vec<_> = reader_view.document.text.encode_utf16().collect();
            assert_eq!(
                String::from_utf16(&units[range.0 as usize..range.1 as usize])
                    .unwrap()
                    .to_lowercase(),
                "answer"
            );
        }
        let selected = "alpha Ж🙂e\u{301}";
        let start = source.text[..source.text.find(selected).unwrap()]
            .encode_utf16()
            .count() as u32;
        let finish = start + selected.encode_utf16().count() as u32;
        let viewport = source
            .lines
            .iter()
            .find(|line| line.line == 2)
            .unwrap()
            .start;
        let mut reading = crate::reading_state::ReadingState::default();
        let native = (
            reader_view.visual_offset(start),
            reader_view.visual_offset(finish),
        );
        reading.capture(
            &source,
            (
                reader_view.source_offset(native.0),
                reader_view.source_offset(native.1),
            ),
            reader_view.source_offset(reader_view.visual_offset(viewport)),
            false,
        );
        let restored = reading.restore(&source);
        assert_eq!(restored.selection, (start, finish));
        assert_eq!(restored.viewport_offset, viewport);
        assert!(!restored.follow);
        assert_eq!(
            (
                input_view.visual_offset(restored.selection.0),
                input_view.visual_offset(restored.selection.1)
            ),
            native
        );
        for row in 0..18 {
            let last = reader_view
                .document
                .cells
                .iter()
                .rfind(|cell| cell.line == row)
                .unwrap();
            assert_eq!(
                last.column + last.width,
                110,
                "full painted background of row {row}"
            );
        }
        assert_eq!(source, source_before);
    }

    #[test]
    fn actual_two_turn_tui_capture_keeps_native_reader_paragraphs_and_glyph_ranges() {
        // OpenCode 2.0.32, isolated ConPTY 110x36, a loopback provider returning
        // two fixed answers; no real model request. Endpoints are the completed
        // answer frames of the byte-exact capture, before its explicit /exit.
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-two-answers.vt");
        let mut model = TerminalModel::new(110, 36);
        let mut begin = 0;
        for (end, selected) in [(78_185, "Ж🙂"), (121_923, "DeepSeek V4.1 Flash Free.")] {
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let frame = model.live_snapshot();
            let logical = model.reading_document();
            assert!(
                logical.lines.len() < 5,
                "actual full-screen renderer wraps its padding"
            );
            let raw = model.tui_reading_document();
            assert_eq!(raw.lines.len(), 36);
            let projection = project(&frame, &raw, None);
            let input = projection.input.expect("actual completed-answer composer");
            assert!(input.text.is_empty());
            let excluded = [(input.first_row, input.last_row)];
            let source = projection.output;
            let visual = visual_document_inner(&frame, &source, &excluded)
                .expect("same physical source for both native focus modes");
            assert_eq!(source.lines.len(), 31);
            assert_eq!(visual.document.lines.len(), source.lines.len());
            assert!(source.text.contains(selected));
            assert!(source
                .text
                .contains("Длинная строка для проверки переноса:"));
            let source_units: Vec<_> = source.text.encode_utf16().collect();
            for line in &source.lines {
                let actual =
                    String::from_utf16(&source_units[line.start as usize..line.end as usize])
                        .unwrap();
                let expected: String = frame.cells
                    [line.line as usize * frame.cols..(line.line as usize + 1) * frame.cols]
                    .iter()
                    .flat_map(|cell| cell.text.chars())
                    .map(|character| if character == '\t' { ' ' } else { character })
                    .collect();
                assert_eq!(
                    actual,
                    expected.trim_end_matches(' '),
                    "physical row {}",
                    line.line
                );
                assert!(line.end - line.start <= 111, "no huge native paragraph");
            }
            check_document(&source);
            check_visual_mapping(&source, &visual);
            let mut search = crate::search::SearchState::default();
            search.set_query(selected.into(), &source);
            let found = search.current().unwrap();
            let native = (
                visual.visual_offset(found.start),
                visual.visual_offset(found.end),
            );
            let mut state = crate::reading_state::ReadingState::default();
            state.capture(
                &source,
                (
                    visual.source_offset(native.0),
                    visual.source_offset(native.1),
                ),
                source.lines[2].start,
                false,
            );
            let restored = state.restore(&source);
            assert_eq!(restored.selection, (found.start, found.end));
            let focused = visual_document(&frame, &source, &excluded);
            assert_eq!(
                focused.document, visual.document,
                "focus cannot rebuild paragraphs"
            );
            assert_eq!(
                (
                    focused.visual_offset(restored.selection.0),
                    focused.visual_offset(restored.selection.1)
                ),
                native
            );
            assert_eq!(
                model.reading_document(),
                logical,
                "canonical shell export is unchanged"
            );
        }
    }

    #[test]
    fn visual_history_stays_logical_and_live_page_mapping_survives_resize() {
        let mut model = TerminalModel::new(12, 4);
        model.push_bytes("retained paragraph that wraps across the history boundary\r\n\x1b[44m界🙂 content\r\nlast".as_bytes());
        for columns in [12, 9, 17] {
            model.resize(columns, 4);
            let frame = model.live_snapshot();
            let source = model.reading_document();
            let visual = visual_document(&frame, &source, &[]);
            check_visual_mapping(&source, &visual);
            assert!(source.cells.iter().any(|cell| cell.line < 0));
            let source_live = source
                .cells
                .iter()
                .find(|cell| cell.line >= 0)
                .unwrap()
                .start;
            assert_eq!(visual.source_offset(visual.live_start()), source_live);
            let units: Vec<_> = visual.document.text.encode_utf16().collect();
            let original_units: Vec<_> = source.text.encode_utf16().collect();
            assert_eq!(
                &units[..source_live as usize],
                &original_units[..source_live as usize]
            );
            assert!(visual.document.lines.iter().any(|line| line.line < 0));
            let history_only = exclude_rows(&source, 0, frame.rows - 1);
            let history_view = visual_document(&frame, &history_only, &[(0, frame.rows - 1)]);
            assert_eq!(history_view.document, history_only);
            assert_eq!(
                history_view.live_start(),
                history_only.text.encode_utf16().count() as u32
            );
        }
    }

    #[test]
    fn captured_gutter_panel_replaces_placeholder_and_keeps_source_menu_navigation() {
        // Captured public startup/slash/Down/Up/Escape UI from an isolated CLI,
        // with fresh HOME/XDG and no credentials or model request. Only the VT
        // parser replays this fixture; no executable or command is launched.
        let bytes = include_bytes!("../tests/fixtures/gutter-panel-menu.vt");
        let ends = [13_222, 14_947, 15_233, 15_519, 16_567];
        let mut model = TerminalModel::new(110, 36);
        let mut begin = 0;
        let mut previous = None;
        let mut before_revision = 0;
        for (phase, end) in ends.into_iter().enumerate() {
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let frame = model.live_snapshot();
            let document = model.reading_document();
            let original = frame.clone();
            let evidence =
                (phase == 1).then(|| accepted(previous.as_ref().unwrap(), "/", before_revision));
            let projection =
                project_confirmed(&frame, &document, previous.as_ref(), evidence.as_ref());
            let input = projection.input.unwrap();
            assert_eq!(input.text, if phase == 0 { "" } else { "/" });
            assert_eq!(input.caret, Some(u32::from(phase != 0)));
            assert_eq!((input.first_row, input.last_row), (18, 22));
            assert_eq!((input.first_column, input.last_column), (18, 92));
            assert!(!projection.output.text.contains("Ask anything"));
            assert!(!projection.output.text.contains("Big Pickle"));
            assert_eq!(
                projection.output.text.contains("/agents"),
                (1..=3).contains(&phase)
            );
            assert_eq!(frame, original);
            check_document(&projection.output);
            previous = Some(input);
            before_revision = document.revision;
        }
    }

    #[test]
    fn panel_never_publishes_placeholder_from_color_or_a_late_unmatched_prefix() {
        let (frame, document) = panel_fixture(&["A muted hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        assert_eq!(initial.text, "");
        assert_eq!(initial.caret, Some(0));
        let (frame, document) = panel_fixture(&["typed"], (0, 5), false, 2);
        assert!(project(&frame, &document, Some(&initial)).input.is_none());
        for mut evidence in [
            accepted(&initial, "other", 1),
            accepted(&initial, "typed", 2),
        ] {
            assert!(
                project_confirmed(&frame, &document, Some(&initial), Some(&evidence))
                    .input
                    .is_none()
            );
            evidence.row += 1;
            assert!(
                project_confirmed(&frame, &document, Some(&initial), Some(&evidence))
                    .input
                    .is_none()
            );
        }
        // An unknown palette is still display text when its cursor starts at
        // the input origin; no particular gray or literal hint is special.
        let (frame, document) = panel_fixture(&["any color"], (0, 0), false, 3);
        assert_eq!(project(&frame, &document, None).input.unwrap().text, "");
    }

    #[test]
    fn panel_acknowledges_only_the_visible_prefix_then_keeps_the_complete_tail_at_home() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let evidence = accepted(&initial, "abcdef", 1);
        let (frame, document) = panel_fixture(&["abc"], (0, 3), false, 2);
        let partial = project_confirmed(&frame, &document, Some(&initial), Some(&evidence))
            .input
            .unwrap();
        assert_eq!(partial.text, "abc");
        let (frame, document) = panel_fixture(&["abcdef"], (0, 6), false, 3);
        let complete = project_confirmed(&frame, &document, Some(&partial), Some(&evidence))
            .input
            .unwrap();
        assert_eq!(complete.text, "abcdef");
        let mut previous = complete;
        for (column, revision) in [(0, 4), (1, 5), (0, 6), (6, 7)] {
            let (frame, document) = panel_fixture(&["abcdef"], (0, column), false, revision);
            let input = project(&frame, &document, Some(&previous)).input.unwrap();
            assert_eq!(input.text, "abcdef");
            assert_eq!(input.caret, Some(column as u32));
            previous = input;
        }
    }

    #[test]
    fn panel_unicode_combining_insertions_and_real_trailing_spaces_keep_utf16_carets() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["Ж界🙂"], (0, 5), false, 2);
        let input = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "Ж界🙂", 1)),
        )
        .input
        .unwrap();
        assert_eq!(input.text, "Ж界🙂");
        assert_eq!(input.caret, Some(4));
        let (frame, document) = panel_fixture(&["e"], (0, 1), false, 2);
        let e = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "e", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = panel_fixture(&["e\u{301} "], (0, 2), false, 3);
        let combined = project_confirmed(
            &frame,
            &document,
            Some(&e),
            Some(&accepted(&e, "\u{301} ", 2)),
        )
        .input
        .unwrap();
        assert_eq!(combined.text, "e\u{301} ");
        assert_eq!(combined.caret, Some(3));
    }

    #[test]
    fn owned_prefix_deletion_at_home_keeps_suffix_without_accepting_a_new_hint() {
        type Fixture =
            fn(&[&str], (usize, usize), bool, u64) -> (TerminalSnapshot, ReadingDocument);
        for fixture in [panel_fixture as Fixture, padded_fixture as Fixture] {
            let (frame, document) = fixture(&["arbitrary hint"], (0, 0), true, 1);
            let empty = project(&frame, &document, None).input.unwrap();
            let (frame, document) = fixture(&["Ж界🙂"], (0, 5), false, 2);
            let entered = project_confirmed(
                &frame,
                &document,
                Some(&empty),
                Some(&accepted(&empty, "Ж界🙂", 1)),
            )
            .input
            .unwrap();
            for suffix in ["界🙂", "🙂", ""] {
                let (frame, document) = fixture(&[suffix], (0, 0), false, 3);
                let current = project(&frame, &document, Some(&entered)).input.unwrap();
                assert_eq!(current.text, suffix);
                assert_eq!(current.caret, Some(0));
            }
            let (frame, document) = fixture(&["🙂"], (0, 0), true, 3);
            assert!(project(&frame, &document, Some(&entered))
                .input
                .unwrap()
                .text
                .is_empty());
            let (frame, document) = fixture(&["unrelated text"], (0, 0), false, 3);
            assert!(project(&frame, &document, Some(&entered))
                .input
                .unwrap()
                .text
                .is_empty());
        }
    }

    #[test]
    fn panel_multiline_body_and_internal_blank_rows_exclude_only_the_footer() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["first"], (0, 5), false, 2);
        let initial = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "first", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = panel_fixture(&["first", "", "Ж界🙂 tail"], (2, 10), false, 3);
        let input = project(&frame, &document, Some(&initial)).input.unwrap();
        assert_eq!(input.text, "first\r\n\r\nЖ界🙂 tail");
        assert_eq!(input.caret, Some(input.text.encode_utf16().count() as u32));
        let (frame, document) = panel_fixture(&["first", "", "Ж界🙂 tail"], (0, 0), false, 4);
        let projection = project(&frame, &document, Some(&input));
        assert_eq!(projection.input.unwrap().text, input.text);
        assert!(projection.output.text.contains("reply"));
        assert!(projection.output.text.contains("footer"));
        assert!(!projection.output.text.contains("role"));
        check_document(&projection.output);
    }

    #[test]
    fn panel_hidden_cursor_retains_only_unchanged_source_geometry_and_rendering() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let (mut frame, document) = panel_fixture(&["hello"], (0, 5), false, 2);
        let initial = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "hello", 1)),
        )
        .input
        .unwrap();
        frame.cursor = None;
        let input = project(&frame, &document, Some(&initial)).input.unwrap();
        assert_eq!(input.text, "hello");
        assert_eq!(input.caret, None);
        let (mut changed, document) = panel_fixture(&["other"], (0, 5), false, 3);
        changed.cursor = None;
        assert!(project(&changed, &document, Some(&input)).input.is_none());
        changed = frame.clone();
        changed.history_size += 1;
        assert!(project(&changed, &document, Some(&input)).input.is_none());
        changed = frame;
        changed.alternate_screen = !changed.alternate_screen;
        assert!(project(&changed, &document, Some(&input)).input.is_none());
    }

    #[test]
    fn proven_panel_rows_leave_history_without_removing_unrelated_body_or_footer() {
        let mut model = panel_model(&["unentered hint"], (0, 0), true);
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &input);
        push_screen_into_history(&mut model, 18, "new reply");
        let raw = model.reading_document();
        assert!(raw.text.contains("unentered hint"));
        let filtered = history.filter(&model.live_snapshot(), &raw);
        assert!(filtered.text.contains("reply"));
        assert!(filtered.text.contains("footer"));
        assert!(filtered.text.contains("new reply"));
        assert!(!filtered.text.contains("unentered hint"));
        assert!(!filtered.text.contains("role"));
        check_document(&filtered);
    }

    #[test]
    fn panel_concealed_cells_and_new_unknown_styles_do_not_reveal_pending_or_old_text() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["secret"], (0, 6), false, 2);
        let input = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "secret", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = panel_fixture(&["other"], (0, 5), true, 3);
        assert!(project(&frame, &document, Some(&input)).input.is_none());
        let mut model = panel_model(&[""], (0, 0), false);
        model.push_bytes(b"\x1b[4;8H\x1b[8msecret\x1b[0m\x1b[4;14H");
        let frame = model.live_snapshot();
        let mut document = model.reading_document();
        document.revision = 3;
        let projection = project_confirmed(
            &frame,
            &document,
            Some(&input),
            Some(&accepted(&input, "raw-password", 2)),
        );
        if let Some(current) = projection.input {
            assert!(current.protected);
            assert!(!current.text.contains("secret"));
            assert!(!current.text.contains("raw-password"));
            assert_eq!(current.caret, None);
        }
    }

    #[test]
    fn panels_require_a_matching_background_rail_padding_and_bounded_footer() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        for variant in 0..5 {
            let mut broken = frame.clone();
            match variant {
                0 => broken.cells[6 * 48 + 4].text = "│".into(),
                1 => broken.cells[4 * 48 + 7].bg = Color::Named(NamedColor::Background),
                2 => broken.cells[2 * 48 + 8].text = "x".into(),
                3 => broken.cells[3 * 48 + 2].text = "body".into(),
                _ => {
                    for column in 5..=37 {
                        broken.cells[5 * 48 + column].text = " ".into();
                    }
                }
            }
            let result = project(&broken, &document, None);
            assert!(result.input.is_none(), "variant={variant}");
            assert_eq!(result.output, document);
        }
        let (frame, document) = fixture(
            32,
            8,
            "body\r\n┃ quoted text\r\n┃ another paragraph\r\n┗━━━━━━━━━━━━━━━━",
            (1, 2),
        );
        assert!(project(&frame, &document, None).input.is_none());
        let (frame, document) = fixture(
            32,
            8,
            "body\r\n\x1b[48;2;30;30;30m   output panel   \r\n   output only    \x1b[0m",
            (1, 3),
        );
        assert!(project(&frame, &document, None).input.is_none());
    }

    #[test]
    fn panel_resets_on_placeholder_menu_cursor_or_changed_source_identity() {
        let (frame, document) = panel_fixture(&["hint"], (0, 0), true, 1);
        let initial = project(&frame, &document, None).input.unwrap();
        let (frame, document) = panel_fixture(&["hello"], (0, 5), false, 2);
        let acknowledged = project_confirmed(
            &frame,
            &document,
            Some(&initial),
            Some(&accepted(&initial, "hello", 1)),
        )
        .input
        .unwrap();
        let (frame, document) = panel_fixture(&["new unproved display"], (0, 0), false, 3);
        let reset = project(&frame, &document, Some(&acknowledged))
            .input
            .unwrap();
        assert_eq!(reset.text, "");
        assert!(!reset.panel.as_ref().unwrap().acknowledged);
        let (mut frame, document) = panel_fixture(&["hello"], (0, 5), false, 3);
        frame.cursor = Some(CursorSnapshot { row: 0, column: 0 });
        assert!(project(&frame, &document, Some(&acknowledged))
            .input
            .is_none());
        frame.cursor = Some(acknowledged.anchor);
        frame.history_size += 1;
        assert!(project(&frame, &document, Some(&acknowledged))
            .input
            .is_none());
        let (frame, stale) = panel_fixture(&["hello"], (0, 5), false, 2);
        let (changed, _) = panel_fixture(&["other"], (0, 5), false, 3);
        assert!(project(&changed, &stale, Some(&acknowledged))
            .input
            .is_none());
        assert_eq!(frame.cells.len(), changed.cells.len());
    }

    #[test]
    fn history_filter_removes_only_proven_field_at_its_shifted_absolute_rows() {
        let mut model = TerminalModel::new(20, 6);
        model.push_bytes(
            b"old body\r\n+--------+\r\n| > hi   |\r\n+--------+\r\nold footer\x1b[3;7H",
        );
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &input);
        push_screen_into_history(
            &mut model,
            6,
            "new body\r\n+--------+\r\n| > next |\r\n+--------+\r\nnew footer\x1b[3;9H",
        );
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        assert_eq!(frame.history_size, 6);
        let filtered = history.filter(&frame, &raw);
        let projection = project(&frame, &filtered, Some(&input));
        assert_eq!(projection.input.unwrap().text, "next");
        assert!(!projection.output.text.contains('+'));
        assert!(!projection.output.text.contains("hi"));
        for body in ["old body", "old footer", "new body", "new footer"] {
            assert!(projection.output.text.contains(body), "{body}");
        }
        assert!(raw.text.contains("| > hi"));
        check_document(&filtered);
        check_document(&projection.output);
    }

    #[test]
    fn historical_lookalike_boxes_and_changed_proofs_are_retained() {
        let mut model = TerminalModel::new(20, 10);
        model.push_bytes(b"body\r\n+--------+\r\n| > hi   |\r\n+--------+\r\nquote\r\n+--------+\r\n| > hi   |\r\n+--------+\r\nfooter\x1b[3;7H");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &input);
        push_screen_into_history(&mut model, 10, "new body");
        let raw = model.reading_document();
        let filtered = history.filter(&model.live_snapshot(), &raw);
        assert_eq!(raw.text.matches("| > hi").count(), 2);
        assert_eq!(filtered.text.matches("| > hi").count(), 1);
        assert!(filtered.text.contains("quote"));
        check_document(&filtered);

        let mut model = TerminalModel::new(20, 6);
        model.push_bytes(b"body\r\n+--------+\r\n| > hi   |\r\n+--------+\x1b[3;7H");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &input);
        model.push_bytes(b"\x1b[3;5Hx");
        push_screen_into_history(&mut model, 6, "next");
        let raw = model.reading_document();
        let filtered = history.filter(&model.live_snapshot(), &raw);
        assert!(filtered.text.contains("| > xi"));
        assert!(!filtered.text.contains("+--------+"));
        assert_eq!(history.fields[0].rows.len(), 2);
        check_document(&filtered);
    }

    #[test]
    fn partial_history_and_overlapping_new_field_preserve_independent_row_proofs() {
        let mut model = TerminalModel::new(20, 8);
        model.push_bytes(b"old body\r\n+--------+\r\n| > hi   |\r\n+--------+\r\nfooter\x1b[3;7H");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &input);
        model.push_bytes(b"\x1b[8;1H\r\n\r\n");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        assert_eq!(frame.history_size, 2);
        let filtered = history.filter(&frame, &raw);
        assert_eq!(raw.text.matches("+--------+").count(), 2);
        assert_eq!(filtered.text.matches("+--------+").count(), 1);
        assert_eq!(history.fields[0].rows.len(), 3);
        assert!(filtered.text.contains("| > hi"));
        model.push_bytes(b"\x1b[H+--------+\r\n| > new  |\r\n+--------+\x1b[2;8H");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        history.remember(&frame, &raw, &input);
        assert_eq!(history.fields.len(), 2);
        assert_eq!(
            history
                .fields
                .iter()
                .map(|field| field.rows.len())
                .sum::<usize>(),
            4
        );
        let filtered = history.filter(&frame, &raw);
        let projection = project(&frame, &filtered, Some(&input));
        assert!(!projection.output.text.contains("+--------+"));
        assert!(!projection.output.text.contains("| > hi"));
        assert!(projection.output.text.contains("old body"));
        check_document(&filtered);
        check_document(&projection.output);
    }

    #[test]
    fn proven_menu_and_field_survive_sequential_filtering_with_canonical_offsets() {
        let mut model = TerminalModel::new(20, 9);
        model.push_bytes(b"choice one\r\n\r\nchoice two\r\nbody\r\n+--------+\r\n| > hi   |\r\n+--------+\r\nfooter\x1b[6;7H");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let input = project(&frame, &raw, None).input.unwrap();
        let mut history = ComposerHistory::new();
        history.remember_rows(&frame, &raw, 0, 2);
        history.remember(&frame, &raw, &input);
        history.remember(&frame, &raw, &input);
        history.remember_rows(&frame, &raw, 8, 8);
        assert_eq!(history.fields.len(), 2);
        push_screen_into_history(&mut model, 9, "new body");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let filtered = history.filter(&frame, &raw);
        assert!(!filtered.text.contains("choice"));
        assert!(!filtered.text.contains('+'));
        assert!(filtered.text.contains("body") && filtered.text.contains("footer"));
        assert_eq!(history.filter(&frame, &raw), filtered);
        check_document(&filtered);
        let mut changed = frame.clone();
        changed.history_size = MAX_HISTORY_ROWS;
        assert_eq!(history.filter(&changed, &raw), raw);
        assert!(history.fields.is_empty());
        history.reset();
        assert!(history.source.is_none());
    }

    #[test]
    fn boxed_unicode_multiline_moves_only_the_field_and_keeps_output_style_and_source() {
        let (frame, document) = fixture(24, 9,
            "\x1b[31mAnswer Ж🙂\x1b[0m\r\n╭────────────────────╮\r\n│ > hi界🙂e\u{301}          │\r\n│   next             │\r\n╰────────────────────╯\r\nfooter", (2, 10));
        let source = document.clone();
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert_eq!(input.text, "hi界🙂e\u{301}\r\n  next");
        assert_eq!((input.first_row, input.last_row), (1, 4));
        assert_eq!(input.caret, Some(5));
        assert_eq!(projection.output.text, "Answer Ж🙂\r\nfooter");
        assert_eq!(
            projection.output.styles[0].style.foreground,
            Color::Named(NamedColor::Red)
        );
        assert_eq!(document, source);
        check_document(&projection.output);
    }

    #[test]
    fn bare_prompt_soft_wrap_has_one_input_and_removes_just_its_logical_row() {
        let (frame, document) = fixture(10, 5, "body\r\n> abcdefghijkl", (2, 4));
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert_eq!(input.text, "abcdefghijkl");
        assert_eq!(input.caret, Some(12));
        assert_eq!((input.first_row, input.last_row), (1, 2));
        assert_eq!(projection.output.text, "body");
        check_document(&projection.output);
    }

    #[test]
    fn prompt_nbsp_separators_work_with_visible_and_source_software_cursors() {
        let body = "/a\u{00a0} b\u{202f} c  ";
        for separator in ['\u{00a0}', '\u{202f}'] {
            for software in [false, true] {
                for prompt in ["❯", "$"] {
                    for ruled in [false, true] {
                        let (row, source) = if ruled {
                            (
                                2,
                                format!(
                                    "reply\r\n{}\r\n{prompt}{separator}{body}\r\n{}\r\nfooter",
                                    "─".repeat(32),
                                    "─".repeat(32)
                                ),
                            )
                        } else {
                            (1, format!("reply\r\n{prompt}{separator}{body}\r\nfooter"))
                        };
                        let column = 2 + body.chars().count();
                        let (mut frame, document) = fixture(40, 8, &source, (row, column));
                        if software {
                            frame.cursor = None;
                            frame.cells[row * frame.cols + column]
                                .flags
                                .insert(Flags::INVERSE);
                            frame.cursor =
                                crate::direct_accessibility::reading_cursor(&frame, None);
                        }
                        let original = frame.clone();
                        assert!(!blank(&frame, row, 1));
                        assert!(prompt_separator(&frame, row, 1));
                        let projection = project(&frame, &document, None);
                        let input = projection.input.unwrap_or_else(|| {
                            panic!("{separator:?} software={software} ruled={ruled}")
                        });
                        assert_eq!(input.text, body);
                        assert_eq!(input.caret, Some(body.encode_utf16().count() as u32));
                        assert!(projection.output.text.contains("reply"));
                        assert!(projection.output.text.contains("footer"));
                        assert_eq!(frame, original);
                        check_document(&projection.output);
                    }
                }
            }
        }
    }

    #[test]
    fn prompt_separator_support_is_local_to_the_prompt_and_preserves_path_input_spaces() {
        for separator in [' ', '\u{00a0}', '\u{202f}'] {
            let body = " a\u{00a0}b\u{202f}c  ";
            let prompt = "C:\\work>";
            let source = format!("{prompt}{separator}{body}");
            let (frame, document) =
                fixture(40, 4, &source, (0, prompt.len() + 1 + body.chars().count()));
            let input = project(&frame, &document, None).input.unwrap();
            assert_eq!(input.text, body);
        }
        let (mut frame, document) = fixture(20, 4, "❯ /", (0, 3));
        frame.cells[1].text = "\u{2003}".into();
        assert!(!prompt_separator(&frame, 0, 1));
        assert!(!blank(&frame, 0, 1));
        assert!(project(&frame, &document, None).input.is_none());
    }

    #[test]
    fn horizontal_rules_remove_only_the_proven_composer_and_keep_unicode_and_body_styles() {
        let (frame, document) = fixture(
            30,
            8,
            "\x1b[31mAnswer Ж🙂\x1b[0m\r\n────────────────────\r\n❯ hi界🙂e\u{301}\r\n────────────────────\r\nfooter",
            (2, 9),
        );
        let original = document.clone();
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert_eq!(input.text, "hi界🙂e\u{301}");
        assert_eq!(input.caret, Some(7));
        assert_eq!((input.first_row, input.last_row), (1, 3));
        assert_eq!(projection.output.text, "Answer Ж🙂\r\nfooter");
        assert_eq!(
            projection.output.styles[0].style.foreground,
            Color::Named(NamedColor::Red)
        );
        assert_eq!(document, original);
        check_document(&projection.output);
    }

    #[test]
    fn inline_rule_prompt_and_soft_wrapped_input_are_generic_and_source_owned() {
        for (columns, text, cursor, expected) in [
            (
                30,
                "body\r\n────────────❯ hi\r\n────────────\r\nfooter",
                (1, 16),
                "hi",
            ),
            (
                20,
                "body\r\n────────────❯ abcdefgh\r\n────────────\r\nfooter",
                (2, 2),
                "abcdefgh",
            ),
        ] {
            let (frame, document) = fixture(columns, 8, text, cursor);
            let projection = project(&frame, &document, None);
            let input = projection.input.unwrap();
            assert_eq!(input.text, expected);
            assert_eq!(input.caret, Some(expected.len() as u32));
            assert_eq!(projection.output.text, "body\r\nfooter");
            check_document(&projection.output);
        }
        let (frame, document) = fixture(
            113,
            8,
            &format!(
                "body\r\n{}❯ /\r\n{}\r\nfooter",
                "─".repeat(104),
                "─".repeat(108)
            ),
            (1, 107),
        );
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert_eq!(input.text, "/");
        assert_eq!(input.caret, Some(1));
        assert_eq!(projection.output.text, "body\r\nfooter");
        check_document(&projection.output);
    }

    #[test]
    fn ambiguous_rules_and_side_by_side_content_remain_fully_readable() {
        for text in [
            "────────────\r\n> one\r\n\r\n> two\r\n────────────",
            "────────────\r\n> one\r\n───────────",
            "────────────\r\n> hi          BODY\r\n────────────",
        ] {
            let (frame, document) = fixture(30, 8, text, (1, 4));
            let projection = project(&frame, &document, None);
            assert!(projection.input.is_none(), "{text}");
            assert_eq!(projection.output, document);
        }
    }

    #[test]
    fn proven_rule_composer_survives_hidden_cursor_and_is_filtered_from_history() {
        let mut model = TerminalModel::new(30, 8);
        model.push_bytes("body\r\n────────────❯ hi\r\n────────────\r\nfooter\x1b[2;17H".as_bytes());
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let previous = project(&frame, &raw, None).input.unwrap();
        let mut hidden = frame.clone();
        hidden.cursor = None;
        let retained = project(&hidden, &raw, Some(&previous)).input.unwrap();
        assert_eq!(retained.text, "hi");
        assert_eq!(retained.caret, None);
        let mut history = ComposerHistory::new();
        history.remember(&frame, &raw, &previous);
        push_screen_into_history(&mut model, 8, "new body");
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let output = history.filter(&frame, &raw);
        assert!(!output.text.contains('─') && !output.text.contains("❯ hi"));
        assert!(output.text.contains("body") && output.text.contains("footer"));
        assert!(output.text.contains("new body"));
        check_document(&output);
    }

    #[test]
    fn ascii_box_and_path_prompt_are_generic() {
        for text in ["+--------+\r\n| > hi   |\r\n+--------+", "C:\\work> hi"] {
            let cursor = if text.starts_with('+') {
                (1, 6)
            } else {
                (0, 11)
            };
            let (frame, document) = fixture(24, 5, text, cursor);
            let projection = project(&frame, &document, None);
            assert_eq!(projection.input.unwrap().text, "hi");
            assert!(projection.output.text.is_empty());
        }
    }

    #[test]
    fn bare_prompt_keeps_earlier_prompt_and_reply_rows() {
        let (frame, document) = fixture(24, 6, "> old\r\nreply\r\n> new", (2, 5));
        let projection = project(&frame, &document, None);
        assert_eq!(projection.input.unwrap().text, "new");
        assert_eq!(projection.output.text, "> old\r\nreply");
    }

    #[test]
    fn visible_composer_is_redetected_after_move_resize_and_alternate_repaint() {
        let (frame, document) = fixture(20, 7, "+--------+\r\n| > hi   |\r\n+--------+", (1, 6));
        let previous = project(&frame, &document, None).input.unwrap();
        for (columns, prefix, alternate) in [
            (20, "body\r\n", false),
            (26, "body\r\n\r\n", false),
            (20, "body\r\n", true),
        ] {
            let top = prefix.matches('\n').count();
            let (mut frame, mut document) = fixture(
                columns,
                7,
                &format!("{prefix}+--------+\r\n| > hi   |\r\n+--------+\r\nfooter"),
                (top + 1, 6),
            );
            frame.alternate_screen = alternate;
            document.alternate_screen = alternate;
            let projection = project(&frame, &document, Some(&previous));
            let input = projection.input.unwrap();
            assert_eq!(input.text, "hi");
            assert_eq!((input.first_row, input.last_row), (top, top + 2));
            assert!(projection.output.text.starts_with("body"));
            assert!(projection.output.text.ends_with("footer"));
            assert!(!projection.output.text.contains('+'));
            check_document(&projection.output);
        }
    }

    #[test]
    fn stale_document_and_scrolled_frame_cannot_remove_an_old_body() {
        let (frame, document) = fixture(20, 7, "+--------+\r\n| > hi   |\r\n+--------+", (1, 6));
        let previous = project(&frame, &document, None).input.unwrap();
        let (mut moved, _) = fixture(
            20,
            7,
            "body\r\n+--------+\r\n| > hi   |\r\n+--------+",
            (2, 6),
        );
        let projection = project(&moved, &document, Some(&previous));
        assert!(projection.input.is_none());
        assert_eq!(projection.output, document);
        moved = frame;
        moved.display_offset = 1;
        assert!(project(&moved, &document, Some(&previous)).input.is_none());
        moved.display_offset = 0;
        moved.cells.pop();
        assert!(project(&moved, &document, Some(&previous)).input.is_none());
        let (frame, document) = fixture(10, 2, "0123456789> abcdefghijkl", (1, 4));
        assert!(frame.history_size > 0);
        let projection = project(&frame, &document, None);
        assert!(projection.input.is_none());
        assert_eq!(projection.output, document);
    }

    #[test]
    fn historical_body_and_wide_combining_glyphs_keep_their_grid_anchors() {
        let (frame, document) = fixture(
            12,
            5,
            "history1\r\nhistory2\r\nЖ🙂e\u{301}\r\n+--------+\r\n| > hi   |\r\n+--------+",
            (3, 6),
        );
        let projection = project(&frame, &document, None);
        assert_eq!(projection.input.unwrap().text, "hi");
        assert_eq!(
            projection.output.text,
            "history1\r\nhistory2\r\nЖ🙂e\u{301}"
        );
        assert!(projection.output.cells.iter().any(|cell| cell.line < 0));
        let wide = projection
            .output
            .cells
            .iter()
            .find(|cell| cell.width == 2)
            .unwrap();
        assert_eq!((wide.line, wide.column, wide.end - wide.start), (1, 1, 2));
        check_document(&projection.output);
    }

    #[test]
    fn row_exclusion_can_cut_a_physical_row_inside_a_soft_wrap() {
        let (frame, document) = fixture(6, 5, "abcdefghijklmno", (2, 3));
        assert!(wrapped(&frame, 0) && wrapped(&frame, 1));
        let output = exclude_rows(&document, 1, 1);
        assert_eq!(output.text, "abcdefmno");
        assert_eq!(output.lines.len(), 1);
        assert!(output
            .cells
            .iter()
            .all(|cell| cell.line == 0 || cell.line == 2));
        check_document(&output);
    }

    #[test]
    fn empty_multiline_capacity_and_typed_trailing_spaces_are_distinct() {
        let (frame, document) = fixture(
            20,
            7,
            "+--------+\r\n| > hi   |\r\n|        |\r\n|        |\r\n+--------+",
            (1, 7),
        );
        let input = project(&frame, &document, None).input.unwrap();
        assert_eq!(input.text, "hi ");
        assert_eq!(input.caret, Some(3));
        let (frame, document) = fixture(
            20,
            7,
            "+--------+\r\n| > hi   |\r\n|        |\r\n|        |\r\n+--------+",
            (2, 3),
        );
        let input = project(&frame, &document, None).input.unwrap();
        assert_eq!(input.text, "hi\r\n ");
        assert_eq!(input.caret, Some(5));
    }

    #[test]
    fn menus_broken_boxes_body_cursor_and_side_by_side_content_are_unchanged() {
        for (text, cursor) in [
            ("> one\r\n> two", (0, 3)),
            ("ordinary body text", (0, 8)),
            ("+--------+\r\n| > hi   |\r\n+--broken+", (1, 6)),
            ("+--------+ body\r\n| > hi   |\r\n+--------+", (1, 6)),
            (
                "+--------+\r\n| > one  |\r\n| > two  |\r\n+--------+",
                (1, 6),
            ),
        ] {
            let (frame, document) = fixture(24, 6, text, cursor);
            let projection = project(&frame, &document, None);
            assert!(projection.input.is_none(), "{text}");
            assert_eq!(projection.output, document);
        }
    }

    #[test]
    fn hidden_cursor_retains_only_an_unchanged_box_with_the_same_source() {
        let (mut frame, document) =
            fixture(24, 6, "+--------+\r\n| > hi   |\r\n+--------+", (1, 6));
        let previous = project(&frame, &document, None).input.unwrap();
        frame.cursor = None;
        let retained = project(&frame, &document, Some(&previous)).input.unwrap();
        assert_eq!(retained.text, "hi");
        assert_eq!(retained.caret, None);
        for source in 0..3 {
            let mut changed = frame.clone();
            match source {
                0 => changed.history_size += 1,
                1 => changed.alternate_screen = !changed.alternate_screen,
                _ => {
                    changed.rows -= 1;
                    changed.cells.truncate(changed.cols * changed.rows);
                }
            }
            assert!(project(&changed, &document, Some(&previous))
                .input
                .is_none());
        }
        frame.cells[frame.cols + 4].text = "x".into();
        assert!(project(&frame, &document, Some(&previous)).input.is_none());
    }

    #[test]
    fn hidden_text_is_redacted_and_never_recovered_from_previous_input() {
        let (frame, document) = fixture(
            24,
            6,
            "+------------+\r\n| > \x1b[8msecret\x1b[0m   |\r\n+------------+",
            (1, 7),
        );
        let projection = project(&frame, &document, None);
        let input = projection.input.unwrap();
        assert!(input.protected && input.caret.is_none());
        assert!(!input.text.contains("secret"));
        assert!(!projection.output.text.contains("secret"));
        check_document(&projection.output);
    }

    #[test]
    fn exclude_rows_retains_history_empty_lines_and_canonical_offsets_after_two_removals() {
        let mut model = TerminalModel::new(12, 5);
        model.push_bytes(b"history1\r\nhistory2\r\nbody\r\nmenu\r\ninput\r\nfooter");
        let document = model.reading_document();
        assert!(document.cells.iter().any(|cell| cell.line < 0));
        let first = exclude_rows(&document, 3, 3);
        let output = exclude_rows(&first, 2, 2);
        assert!(output.text.contains("history1\r\nhistory2"));
        assert!(!output.text.contains("menu") && !output.text.contains("input"));
        assert!(output.text.ends_with("footer"));
        check_document(&output);
    }
}
