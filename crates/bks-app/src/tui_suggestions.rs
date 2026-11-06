//! Source-confirmed inline suggestions. This module neither forwards input nor
//! recognizes actions. The caller gates detection on explicit input followed by
//! a VT update and owns layout, theme, focus and exactly-once key forwarding.

use std::marker::PhantomData;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bks_terminal::{CellSnapshot, Color, Flags, NamedColor, TerminalSnapshot};
use windows::core::{w, Error, Result};
use windows::Win32::Foundation::{E_INVALIDARG, E_OUTOFMEMORY, HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetParent, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    SendMessageW, ShowWindow, HMENU, LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY,
    LB_DELETESTRING, LB_GETCURSEL, LB_GETTOPINDEX, LB_INSERTSTRING, LB_RESETCONTENT, LB_SETCURSEL,
    LB_SETTOPINDEX, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETREDRAW,
    WS_BORDER, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VSCROLL,
};

const MAX_ITEMS: usize = 32;
const MAX_LABEL_CHARS: usize = 512;
const MAX_COLUMNS: usize = 1_024;
const MAX_GAP_ROWS: usize = 3;
const MAX_ROWS: usize = 256;
const MAX_COMPOSER_CHARS: usize = 32_768;
const NAVIGATION_LIFETIME: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<Suggestion>,
    pub selected: usize,
    /// Inclusive physical rows, including adjacent menu border/pager chrome.
    pub first_row: usize,
    pub last_row: usize,
}

/// A bounded, aligned list immediately above or below a known active composer. A unique
/// separated pointer or an unambiguous inverse/background distinction is
/// required. Plain lists, foreground colors, spinners and hidden text do not
/// establish a selection. A painted copy of a menu needs the caller's explicit
/// input/source-update evidence; a single snapshot cannot prove interactivity.
pub fn detect_menu(
    frame: &TerminalSnapshot,
    composer_rows: Option<(usize, usize)>,
) -> Option<Menu> {
    let (above, below) = nearby_candidates(frame, composer_rows)?;
    let above = above.and_then(|candidate| candidate.strict_menu());
    let below = below.and_then(|candidate| candidate.strict_menu());
    match (above, below) {
        (Some(_), Some(_)) => None,
        (above, below) => above.or(below),
    }
}

fn nearby_candidates(
    frame: &TerminalSnapshot,
    composer_rows: Option<(usize, usize)>,
) -> Option<(Option<MenuCandidate>, Option<MenuCandidate>)> {
    let (composer_first, composer_last) = composer_rows?;
    if frame.display_offset != 0
        || frame.cols == 0
        || frame.cols > MAX_COLUMNS
        || frame.cells.len() != frame.rows.checked_mul(frame.cols)?
        || composer_first > composer_last
        || composer_last >= frame.rows
        || frame
            .cursor
            .is_some_and(|cursor| !(composer_first..=composer_last).contains(&cursor.row))
    {
        return None;
    }
    let above = composer_first
        .checked_sub(1)
        .and_then(|row| adjacent_menu(frame, row, true));
    let below = composer_last
        .checked_add(1)
        .filter(|row| *row < frame.rows)
        .and_then(|row| adjacent_menu(frame, row, false));
    Some((above, below))
}

fn next_row(frame: &TerminalSnapshot, row: usize, backwards: bool) -> Option<usize> {
    if backwards {
        row.checked_sub(1)
    } else {
        row.checked_add(1).filter(|row| *row < frame.rows)
    }
}

fn adjacent_menu(
    frame: &TerminalSnapshot,
    anchor: usize,
    backwards: bool,
) -> Option<MenuCandidate> {
    let mut edge = anchor;
    let mut gap = 0;
    while is_gap(frame_row(frame, edge)) || is_pager(frame_row(frame, edge)) {
        gap += 1;
        if gap > MAX_GAP_ROWS {
            return None;
        }
        edge = next_row(frame, edge, backwards)?;
    }
    let mut row = edge;
    let mut items = vec![menu_row(frame_row(frame, row), row)?];
    let mut column = items[0].column;
    let mut continuation = None;
    let mut first_row = anchor.min(row);
    let mut last_row = anchor.max(row);
    while let Some(next) = next_row(frame, row, backwards) {
        let cells = frame_row(frame, next);
        if is_gap(cells) || is_pager(cells) {
            if continuation.is_some() {
                return None;
            }
            if is_border_row(cells) || is_pager(cells) {
                first_row = first_row.min(next);
                last_row = last_row.max(next);
                // The far border may be followed by the same small pager.
                if !backwards {
                    if let Some(pager) =
                        next_row(frame, next, false).filter(|row| is_pager(frame_row(frame, *row)))
                    {
                        last_row = pager;
                    }
                }
            }
            break;
        }
        let Some(mut item) = menu_row(cells, next) else {
            break;
        };
        if item.column > column {
            if backwards {
                if continuation.is_some() {
                    return None;
                }
                continuation = Some(item);
            } else {
                items.last_mut()?.append_continuation(item)?;
            }
        } else if item.column < column {
            // Walking up can encounter the final description line before its
            // first label. Only its exact two-column indent proves attachment.
            if backwards
                && items.len() == 1
                && continuation.is_none()
                && item.description_column == Some(column)
            {
                item.append_continuation(items.pop()?)?;
                column = item.column;
                items.push(item);
            } else if item.pointer || continuation.is_some() {
                return None;
            } else {
                break;
            }
        } else {
            if let Some(continued) = continuation.take() {
                item.append_continuation(continued)?;
            }
            if items.len() == MAX_ITEMS {
                return None;
            }
            items.push(item);
        }
        row = next;
        first_row = first_row.min(row);
        last_row = last_row.max(row);
    }
    if items.len() < 2 || continuation.is_some() {
        return None;
    }
    if backwards {
        items.reverse();
    }
    Some(MenuCandidate {
        items,
        first_row,
        last_row,
    })
}

#[derive(Clone)]
struct MenuCandidate {
    items: Vec<MenuRow>,
    first_row: usize,
    last_row: usize,
}

impl MenuCandidate {
    fn selection_evidence(&self) -> [Evidence; 3] {
        let pointers: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.pointer.then_some(index))
            .collect();
        let pointer = match pointers.as_slice() {
            [] => Evidence::None,
            [selected] => Evidence::Selected(*selected),
            _ => Evidence::Ambiguous,
        };
        [
            pointer,
            inverse_selection(&self.items),
            background_selection(&self.items),
        ]
    }

    fn strict_menu(&self) -> Option<Menu> {
        Some(self.menu(agree(self.selection_evidence())?))
    }

    fn temporal_eligible(&self) -> bool {
        self.selection_evidence()
            .into_iter()
            .all(|evidence| matches!(evidence, Evidence::None))
    }

    fn menu(&self, selected: usize) -> Menu {
        Menu {
            items: self.items.iter().map(|row| row.item.clone()).collect(),
            selected,
            first_row: self.first_row,
            last_row: self.last_row,
        }
    }

    fn same_labels(&self, other: &Self) -> bool {
        self.first_row == other.first_row
            && self.last_row == other.last_row
            && self.items.len() == other.items.len()
            && self.items.iter().zip(&other.items).all(|(old, new)| {
                old.item == new.item
                    && old.column == new.column
                    && old.description_column == new.description_column
                    && old.wrapped == new.wrapped
            })
    }

    fn same_appearance(&self, other: &Self) -> bool {
        self.same_labels(other)
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(old, new)| old.appearance == new.appearance)
    }
}

/// Foreground-only lists need a source-confirmed response to one explicit arrow.
/// This observer supplies labels/selection only; it never writes terminal input.
#[derive(Default)]
pub struct MenuObserver {
    before: Option<Observation>,
    pending: Option<PendingNavigation>,
    last_accepted_revision: Option<u64>,
    proof: Option<SelectionProof>,
}

impl MenuObserver {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Call after the bytes were successfully accepted by the owned PTY.
    pub fn accepted(&mut self, data: &[u8], before_revision: u64) {
        let direction = match data {
            b"\x1b[A" | b"\x1bOA" => Direction::Up,
            b"\x1b[B" | b"\x1bOB" => Direction::Down,
            _ => {
                self.clear();
                return;
            }
        };
        self.pending = None;
        let Some(before) = self.before.as_ref().filter(|before| {
            before.revision == before_revision
                && self.last_accepted_revision != Some(before_revision)
        }) else {
            self.proof = None;
            return;
        };
        self.last_accepted_revision = Some(before_revision);
        self.pending = Some(PendingNavigation {
            before: before.clone(),
            direction,
            started: Instant::now(),
        });
    }

    pub fn observe(
        &mut self,
        frame: &TerminalSnapshot,
        source_revision: u64,
        composer_rows: Option<(usize, usize)>,
    ) -> Option<Menu> {
        if let Some(menu) = detect_menu(frame, composer_rows) {
            self.clear();
            return Some(menu);
        }
        let Some(current) = Observation::new(frame, source_revision, composer_rows) else {
            self.clear();
            return None;
        };
        // A changed frame with an unchanged revision cannot acknowledge a key.
        if self.before.as_ref().is_some_and(|before| {
            before.revision == source_revision && !before.same_appearance(&current)
        }) {
            self.pending = None;
            self.proof = None;
        }
        if let Some(pending) = self.pending.take() {
            if pending.started.elapsed() < NAVIGATION_LIFETIME && pending.before.key == current.key
            {
                if source_revision != pending.before.revision {
                    let mut transfers = current.candidates.iter().filter_map(|candidate| {
                        let before = pending
                            .before
                            .candidates
                            .iter()
                            .find(|before| before.same_labels(candidate))?;
                        let (selected, palette) =
                            reciprocal_selection(before, candidate, pending.direction)?;
                        Some(SelectionProof {
                            key: current.key.clone(),
                            candidate: candidate.clone(),
                            selected,
                            palette,
                        })
                    });
                    let first = transfers.next();
                    if first.is_some() && transfers.next().is_none() {
                        self.proof = first;
                    } else if pending.before.same_appearance(&current) {
                        // Cursor-only or unrelated output can precede the list update.
                        self.pending = Some(pending);
                    } else {
                        self.proof = None;
                    }
                } else if pending.before.same_appearance(&current) {
                    self.pending = Some(pending);
                }
            }
        }
        let menu = self.proof.as_ref().and_then(|proof| {
            if proof.key != current.key {
                return None;
            }
            let candidate = current
                .candidates
                .iter()
                .find(|candidate| candidate.same_labels(&proof.candidate))?;
            (palette_selection(candidate, proof.palette) == Some(proof.selected))
                .then(|| candidate.menu(proof.selected))
        });
        if menu.is_none() {
            self.proof = None;
        }
        self.before = Some(current);
        menu
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SourceKey {
    cols: usize,
    rows: usize,
    alternate: bool,
    history: usize,
    composer_rows: (usize, usize),
    composer: String,
}

#[derive(Clone)]
struct Observation {
    revision: u64,
    key: SourceKey,
    candidates: Vec<MenuCandidate>,
}

impl Observation {
    fn new(
        frame: &TerminalSnapshot,
        revision: u64,
        composer_rows: Option<(usize, usize)>,
    ) -> Option<Self> {
        if frame.rows > MAX_ROWS {
            return None;
        }
        let (above, below) = nearby_candidates(frame, composer_rows)?;
        let candidates: Vec<_> = above.into_iter().chain(below).collect();
        if candidates.is_empty()
            || candidates
                .iter()
                .any(|candidate| !candidate.temporal_eligible())
        {
            return None;
        }
        let (first, last) = composer_rows?;
        let mut composer = String::new();
        let mut characters = 0;
        for row in first..=last {
            for character in frame_row(frame, row)
                .iter()
                .flat_map(|cell| visible_text(cell).chars())
            {
                characters += 1;
                if characters > MAX_COMPOSER_CHARS {
                    return None;
                }
                composer.push(character);
            }
            composer.push('\n');
        }
        Some(Self {
            revision,
            key: SourceKey {
                cols: frame.cols,
                rows: frame.rows,
                alternate: frame.alternate_screen,
                history: frame.history_size,
                composer_rows: (first, last),
                composer,
            },
            candidates,
        })
    }

    fn same_appearance(&self, other: &Self) -> bool {
        self.key == other.key
            && self.candidates.len() == other.candidates.len()
            && self
                .candidates
                .iter()
                .zip(&other.candidates)
                .all(|(old, new)| old.same_appearance(new))
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

impl Direction {
    fn matches(self, before: usize, after: usize, count: usize) -> bool {
        match self {
            Self::Up => after == (before + count - 1) % count,
            Self::Down => after == (before + 1) % count,
        }
    }
}

struct PendingNavigation {
    before: Observation,
    direction: Direction,
    started: Instant,
}

struct SelectionProof {
    key: SourceKey,
    candidate: MenuCandidate,
    selected: usize,
    palette: Palette,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Appearance {
    foreground: Color,
    dim: bool,
}

#[derive(Clone, Copy)]
struct Palette {
    normal: Appearance,
    selected: Appearance,
}

fn changed_appearance(
    before: &[Appearance],
    after: &[Appearance],
) -> Option<(Appearance, Appearance)> {
    if before.len() != after.len() {
        return None;
    }
    let mut changes = before
        .iter()
        .zip(after)
        .filter(|(old, new)| old != new)
        .map(|(old, new)| (*old, *new));
    let pair = changes.next()?;
    (changes.next() == Some(pair) && changes.all(|change| change == pair)).then_some(pair)
}

fn palette_selection(candidate: &MenuCandidate, palette: Palette) -> Option<usize> {
    let mut selected = None;
    for (index, row) in candidate.items.iter().enumerate() {
        let highlighted = row
            .appearance
            .iter()
            .filter(|style| **style == palette.selected)
            .count();
        if highlighted >= 2 && selected.is_none() {
            selected = Some(index);
        } else if highlighted != 0
            || row
                .appearance
                .iter()
                .filter(|style| **style == palette.normal)
                .take(2)
                .count()
                != 2
        {
            return None;
        }
    }
    selected
}

fn reciprocal_selection(
    before: &MenuCandidate,
    after: &MenuCandidate,
    direction: Direction,
) -> Option<(usize, Palette)> {
    if !before.same_labels(after) {
        return None;
    }
    let changed: Vec<_> = before
        .items
        .iter()
        .zip(&after.items)
        .enumerate()
        .filter_map(|(index, (old, new))| (old.appearance != new.appearance).then_some(index))
        .collect();
    let [first, second] = changed.as_slice() else {
        return None;
    };
    let (old, new) = changed_appearance(
        &before.items[*first].appearance,
        &after.items[*first].appearance,
    )?;
    if changed_appearance(
        &before.items[*second].appearance,
        &after.items[*second].appearance,
    )? != (new, old)
    {
        return None;
    }
    let normal = if old.dim != new.dim {
        if old.dim {
            old
        } else {
            new
        }
    } else {
        // Default foreground can highlight the selected row too. Unchanged
        // rows must establish the normal style; a two-item swap is ambiguous.
        let supports = |style: Appearance, selected: Appearance| {
            before
                .items
                .iter()
                .enumerate()
                .filter(|(index, row)| {
                    index != first
                        && index != second
                        && !row.appearance.contains(&selected)
                        && row
                            .appearance
                            .iter()
                            .filter(|value| **value == style)
                            .take(2)
                            .count()
                            == 2
                })
                .count()
        };
        match supports(old, new).cmp(&supports(new, old)) {
            std::cmp::Ordering::Greater => old,
            std::cmp::Ordering::Less => new,
            std::cmp::Ordering::Equal => return None,
        }
    };
    let palette = Palette {
        normal,
        selected: if old == normal { new } else { old },
    };
    let from = palette_selection(before, palette)?;
    let to = palette_selection(after, palette)?;
    direction
        .matches(from, to, after.items.len())
        .then_some((to, palette))
}

fn frame_row(frame: &TerminalSnapshot, row: usize) -> &[CellSnapshot] {
    &frame.cells[row * frame.cols..(row + 1) * frame.cols]
}

fn visible_text(cell: &CellSnapshot) -> &str {
    if cell.flags.contains(Flags::HIDDEN) {
        " "
    } else if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        ""
    } else {
        &cell.text
    }
}

fn is_blank(cell: &CellSnapshot) -> bool {
    visible_text(cell).chars().all(char::is_whitespace)
}

fn is_vertical(cell: &CellSnapshot) -> bool {
    matches!(visible_text(cell), "│" | "┃" | "║")
}

fn is_border_row(cells: &[CellSnapshot]) -> bool {
    let mut border = false;
    for character in cells.iter().flat_map(|cell| visible_text(cell).chars()) {
        if ('\u{2500}'..='\u{257f}').contains(&character) {
            border = true;
        } else if !character.is_whitespace() {
            return false;
        }
    }
    border
}

fn is_gap(cells: &[CellSnapshot]) -> bool {
    cells.iter().all(is_blank) || is_border_row(cells)
}

fn is_pager(cells: &[CellSnapshot]) -> bool {
    let Some(start) = cells.iter().position(|cell| !is_blank(cell)) else {
        return false;
    };
    let end = cells.iter().rposition(|cell| !is_blank(cell)).unwrap();
    let mut characters = cells[start..=end]
        .iter()
        .flat_map(|cell| visible_text(cell).chars());
    let text: String = characters.by_ref().take(64).collect();
    if characters.next().is_some() {
        return false;
    }
    let text = text
        .trim()
        .trim_matches(|c| matches!(c, '│' | '┃' | '║'))
        .trim();
    let text = text
        .strip_prefix('(')
        .and_then(|text| text.strip_suffix(')'))
        .unwrap_or(text);
    let Some((current, total)) = text.split_once('/') else {
        return false;
    };
    !current.is_empty()
        && !total.is_empty()
        && current.chars().all(|c| c.is_ascii_digit())
        && total.chars().all(|c| c.is_ascii_digit())
}

#[derive(Clone)]
struct MenuRow {
    item: Suggestion,
    column: usize,
    pointer: bool,
    description_column: Option<usize>,
    wrapped: bool,
    backgrounds: Vec<Color>,
    inverse: usize,
    appearance: Vec<Appearance>,
}

impl MenuRow {
    fn append_continuation(&mut self, continued: MenuRow) -> Option<()> {
        if self.wrapped || continued.pointer || self.description_column != Some(continued.column) {
            return None;
        }
        self.item.label.push(' ');
        self.item.label.push_str(&continued.item.label);
        if self.item.label.chars().count() > MAX_LABEL_CHARS {
            return None;
        }
        self.backgrounds.extend(continued.backgrounds);
        self.inverse += continued.inverse;
        self.appearance.extend(continued.appearance);
        self.wrapped = true;
        Some(())
    }
}

fn description_column(cells: &[CellSnapshot], start: usize, end: usize) -> Option<usize> {
    let mut gap = None;
    for (column, cell) in cells.iter().enumerate().take(end).skip(start) {
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        if is_blank(cell) {
            gap.get_or_insert(column);
        } else if gap.take().is_some_and(|gap| column - gap >= 2) {
            return Some(column);
        }
    }
    None
}

fn menu_row(cells: &[CellSnapshot], row: usize) -> Option<MenuRow> {
    let mut start = cells.iter().position(|cell| !is_blank(cell))?;
    let mut end = cells.iter().rposition(|cell| !is_blank(cell))? + 1;
    if is_vertical(&cells[start]) && is_vertical(&cells[end - 1]) {
        if end <= start + 1 {
            return None;
        }
        start += 1;
        end -= 1;
        start += cells[start..end].iter().position(|cell| !is_blank(cell))?;
        end = start + cells[start..end].iter().rposition(|cell| !is_blank(cell))? + 1;
    }
    let glyph = visible_text(&cells[start]);
    let first = glyph.chars().next()?;
    let separated = cells.get(start + 1).is_some_and(is_blank);
    let pointer = glyph.trim_end_matches('\u{fe0f}').chars().count() == 1
        && separated
        && matches!(
            first,
            '>' | '❯'
                | '›'
                | '▸'
                | '▶'
                | '►'
                | '➤'
                | '➜'
                | '→'
                | '❱'
                | '»'
                | '◆'
                | '●'
                | '◉'
                | '☛'
                | '👉'
        );
    if separated
        && (matches!(first, '|' | '/' | '-' | '\\' | '◐' | '◓' | '◑' | '◒')
            || ('\u{2800}'..='\u{28ff}').contains(&first))
    {
        return None;
    }
    if pointer {
        let marker = start;
        start += 1;
        start += cells[start..end].iter().position(|cell| !is_blank(cell))?;
        if start - marker > 4 {
            return None;
        }
    }
    let mut label = String::new();
    let mut label_chars = 0;
    let mut backgrounds = Vec::new();
    let mut inverse = 0;
    let mut appearance = Vec::new();
    for cell in &cells[start..end] {
        let text = visible_text(cell);
        for character in text.chars().filter(|character| !character.is_control()) {
            if label_chars == MAX_LABEL_CHARS {
                return None;
            }
            label.push(character);
            label_chars += 1;
        }
        if text.chars().any(char::is_alphanumeric) {
            backgrounds.push(cell.bg);
            inverse += usize::from(cell.flags.contains(Flags::INVERSE));
            appearance.push(Appearance {
                foreground: cell.fg,
                dim: cell.flags.contains(Flags::DIM),
            });
        }
    }
    if backgrounds.is_empty() {
        return None;
    }
    Some(MenuRow {
        item: Suggestion {
            label: label.trim_end().to_owned(),
            row,
        },
        column: start,
        pointer,
        description_column: description_column(cells, start, end),
        wrapped: false,
        backgrounds,
        inverse,
        appearance,
    })
}

#[derive(Clone, Copy)]
enum Evidence {
    None,
    Selected(usize),
    Ambiguous,
}

fn agree(evidence: impl IntoIterator<Item = Evidence>) -> Option<usize> {
    let mut selected = None;
    for item in evidence {
        match item {
            Evidence::None => {}
            Evidence::Ambiguous => return None,
            Evidence::Selected(row) => {
                if selected.is_some_and(|previous| previous != row) {
                    return None;
                }
                selected = Some(row);
            }
        }
    }
    selected
}

fn inverse_selection(rows: &[MenuRow]) -> Evidence {
    let mut inverse = rows.iter().enumerate().filter(|(_, row)| row.inverse >= 2);
    match (inverse.next(), inverse.next()) {
        (None, _) => Evidence::None,
        (Some((selected, _)), None)
            if rows
                .iter()
                .enumerate()
                .all(|(index, row)| index == selected || row.inverse == 0) =>
        {
            Evidence::Selected(selected)
        }
        _ => Evidence::Ambiguous,
    }
}

fn background_selection(rows: &[MenuRow]) -> Evidence {
    let mut colors = Vec::new();
    for color in rows.iter().flat_map(|row| &row.backgrounds) {
        if !colors.contains(color) {
            colors.push(*color);
        }
    }
    let mut choices = Vec::new();
    for normal in colors {
        if rows.len() == 2 && normal != Color::Named(NamedColor::Background) {
            continue;
        }
        let mut selected = None;
        let mut valid = true;
        for (index, row) in rows.iter().enumerate() {
            let changed: Vec<_> = row
                .backgrounds
                .iter()
                .filter(|color| **color != normal)
                .collect();
            if changed.is_empty() {
                valid &= row.backgrounds.len() >= 2;
            } else if changed.len() >= 2
                && changed.iter().all(|color| *color == changed[0])
                && selected.is_none()
            {
                selected = Some(index);
            } else {
                valid = false;
                break;
            }
        }
        if let Some(selected) = selected.filter(|_| valid) {
            choices.push(selected);
        }
    }
    match choices.as_slice() {
        [] => Evidence::None,
        [selected] => Evidence::Selected(*selected),
        _ => Evidence::Ambiguous,
    }
}

/// A standard native LISTBOX with source labels, never owner-drawn or a live
/// command dispatcher. The parent owns positioning/font/theme and routes keys.
pub struct Suggestions {
    pub hwnd: HWND,
    parent: HWND,
    items: Vec<Suggestion>,
    // Win32 child windows must be used and destroyed on their creating thread.
    _thread: PhantomData<Rc<()>>,
}

impl Suggestions {
    pub fn create(parent: HWND, instance: HINSTANCE, id: usize) -> Result<Self> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("LISTBOX"),
                w!("Подсказки TUI"),
                WS_CHILD
                    | WS_CLIPSIBLINGS
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | WS_BORDER
                    | WINDOW_STYLE((LBS_NOTIFY | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32),
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(id as *mut _)),
                Some(instance),
                None,
            )?
        };
        let result = Self {
            hwnd,
            parent,
            items: Vec::new(),
            _thread: PhantomData,
        };
        crate::accessibility::set_accessible_name(hwnd, "Подсказки TUI")?;
        Ok(result)
    }

    pub fn sync(&mut self, menu: &Menu) -> Result<()> {
        if menu.items.len() < 2
            || menu.items.len() > MAX_ITEMS
            || menu.selected >= menu.items.len()
            || menu.items.iter().any(|item| {
                item.label.chars().take(MAX_LABEL_CHARS + 1).count() > MAX_LABEL_CHARS
                    || !item.label.chars().any(char::is_alphanumeric)
                    || item.label.chars().any(char::is_control)
            })
        {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let labels_unchanged = self.items.len() == menu.items.len()
            && self
                .items
                .iter()
                .zip(&menu.items)
                .all(|(old, new)| old.label == new.label);
        if labels_unchanged {
            self.set_selection(menu.selected)?;
            self.items.clone_from(&menu.items);
            return Ok(());
        }
        let previous_selection = self.selection();
        let top = unsafe { SendMessageW(self.hwnd, LB_GETTOPINDEX, None, None).0 }.max(0) as usize;
        let prefix = self
            .items
            .iter()
            .zip(&menu.items)
            .take_while(|(old, new)| old.label == new.label)
            .count();
        let suffix = self.items[prefix..]
            .iter()
            .rev()
            .zip(menu.items[prefix..].iter().rev())
            .take_while(|(old, new)| old.label == new.label)
            .count();
        // WM_SETREDRAW(TRUE) can show a hidden HWND. Only suppress redraw on
        // an already visible list; a source sync must never make it visible.
        let visible = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        if visible {
            unsafe { SendMessageW(self.hwnd, WM_SETREDRAW, Some(WPARAM(0)), None) };
        }
        let result = (|| {
            for _ in prefix..self.items.len() - suffix {
                unsafe {
                    SendMessageW(self.hwnd, LB_DELETESTRING, Some(WPARAM(prefix)), None);
                }
            }
            for (index, item) in menu
                .items
                .iter()
                .enumerate()
                .skip(prefix)
                .take(menu.items.len() - prefix - suffix)
            {
                let label: Vec<u16> = item.label.encode_utf16().chain(Some(0)).collect();
                if unsafe {
                    SendMessageW(
                        self.hwnd,
                        LB_INSERTSTRING,
                        Some(WPARAM(index)),
                        Some(LPARAM(label.as_ptr() as isize)),
                    )
                    .0
                } < 0
                {
                    return Err(Error::from_hresult(E_OUTOFMEMORY));
                }
            }
            // The native count already reflects the new labels here.
            select(self.hwnd, menu.selected)?;
            if previous_selection == Some(menu.selected) {
                unsafe {
                    SendMessageW(
                        self.hwnd,
                        LB_SETTOPINDEX,
                        Some(WPARAM(top.min(menu.items.len() - 1))),
                        None,
                    );
                }
            }
            Ok(())
        })();
        if visible {
            unsafe {
                SendMessageW(self.hwnd, WM_SETREDRAW, Some(WPARAM(1)), None);
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
        if result.is_ok() {
            self.items.clone_from(&menu.items);
        } else {
            self.clear();
        }
        result
    }

    pub fn selection(&self) -> Option<usize> {
        let selection = unsafe { SendMessageW(self.hwnd, LB_GETCURSEL, None, None).0 };
        usize::try_from(selection).ok()
    }

    pub fn set_selection(&self, index: usize) -> Result<()> {
        if index >= self.items.len() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        select(self.hwnd, index)
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn clear(&mut self) {
        unsafe {
            SendMessageW(self.hwnd, LB_RESETCONTENT, None, None);
        }
        self.items.clear();
    }
}

fn select(hwnd: HWND, index: usize) -> Result<()> {
    unsafe {
        if SendMessageW(hwnd, LB_GETCURSEL, None, None).0 == index as isize {
            return Ok(());
        }
        if SendMessageW(hwnd, LB_SETCURSEL, Some(WPARAM(index)), None).0 < 0 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
    }
    Ok(())
}

impl Drop for Suggestions {
    fn drop(&mut self) {
        unsafe {
            // The parent may already have destroyed its children. Do not act
            // on a reused HWND belonging to another process or parent.
            let mut process = 0;
            GetWindowThreadProcessId(self.hwnd, Some(&mut process));
            if IsWindow(Some(self.hwnd)).as_bool()
                && process == std::process::id()
                && GetParent(self.hwnd).ok() == Some(self.parent)
            {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetDlgCtrlID, GetWindowLongW, MoveWindow, GWL_STYLE, LB_GETCOUNT, LB_GETTEXT,
        WS_EX_NOACTIVATE, WS_VISIBLE,
    };

    fn frame(rows: &[&str]) -> (TerminalSnapshot, (usize, usize)) {
        let mut terminal = TerminalModel::new(120, 32);
        let mut text = rows.join("\r\n");
        if !rows.is_empty() {
            text.push_str("\r\n");
        }
        text.push_str(
            "┌──────────────────┐\r\n│ > /m             │\r\n└──────────────────┘\x1b[?25l",
        );
        terminal.push_bytes(text.as_bytes());
        (terminal.live_snapshot(), (rows.len(), rows.len() + 2))
    }

    fn detect(rows: &[&str]) -> Option<Menu> {
        let (frame, composer) = frame(rows);
        detect_menu(&frame, Some(composer))
    }

    fn below(rows: &[&str]) -> Option<Menu> {
        let mut terminal = TerminalModel::new(120, 32);
        let mut text = String::from("Earlier output\r\n┌──────────────────┐\r\n│ > /m             │\r\n└──────────────────┘\r\n");
        text.push_str(&rows.join("\r\n"));
        text.push_str("\x1b[?25l");
        terminal.push_bytes(text.as_bytes());
        detect_menu(&terminal.live_snapshot(), Some((1, 3)))
    }

    #[test]
    fn attached_pointer_menu_omits_only_the_selector_and_keeps_source_rows() {
        let menu = detect(&[
            "Earlier output",
            "  Alpha  First choice",
            "→ Bravo  Second choice",
            "  Charlie  Third choice",
            "(2/20)",
            "",
        ])
        .unwrap();
        assert_eq!(menu.selected, 1);
        assert_eq!((menu.first_row, menu.last_row), (1, 5));
        assert_eq!(
            menu.items,
            vec![
                Suggestion {
                    label: "Alpha  First choice".into(),
                    row: 1
                },
                Suggestion {
                    label: "Bravo  Second choice".into(),
                    row: 2
                },
                Suggestion {
                    label: "Charlie  Third choice".into(),
                    row: 3
                },
            ]
        );
    }

    #[test]
    fn unicode_arguments_provider_and_content_arrows_remain_in_labels() {
        let label = "/mode [on|off] | <task>  Ж界e\u{301}😀 > 1 → result  Provider";
        let selected = format!("❯ {label}");
        let menu = detect(&["  First", &selected, "  Last"]).unwrap();
        assert_eq!(menu.items[menu.selected].label, label);
        let menu = detect(&["   First", "👉 Ж界e\u{301}😀", "   Last"]).unwrap();
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.items[1].label, "Ж界e\u{301}😀");
    }

    #[test]
    fn boxed_menu_chrome_is_in_the_projection_range_but_not_native_labels() {
        let menu = detect(&[
            "┌──────────────────────┐",
            "│   Alpha              │",
            "│ ◆ Bravo              │",
            "│   Charlie            │",
            "└──────────────────────┘",
            "(2/3)",
        ])
        .unwrap();
        assert_eq!((menu.first_row, menu.last_row), (0, 5));
        assert_eq!(menu.selected, 1);
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Bravo", "Charlie"]
        );
    }

    #[test]
    fn inverse_or_background_can_establish_a_unique_selection() {
        for selected in ["\x1b[7mAlpha\x1b[0m", "\x1b[44mAlpha\x1b[0m"] {
            let selected = format!("  {selected}  Provider");
            let menu = detect(&[&selected, "  Bravo  Provider", "  Charlie  Provider"]).unwrap();
            assert_eq!(menu.selected, 0);
            assert_eq!(menu.items[0].label, "Alpha  Provider");
        }
        let menu = detect(&[
            "  \x1b[40mAlpha\x1b[0m",
            "  \x1b[44mBravo\x1b[0m",
            "  \x1b[40mCharlie\x1b[0m",
        ])
        .unwrap();
        assert_eq!(menu.selected, 1);
        assert!(detect(&["  \x1b[40mAlpha\x1b[0m", "  \x1b[44mBravo\x1b[0m"]).is_none());
    }

    #[test]
    fn duplicate_or_conflicting_selection_evidence_is_rejected() {
        for rows in [
            vec!["→ Alpha", "→ Bravo", "  Charlie"],
            vec![
                "  \x1b[7mAlpha\x1b[0m",
                "  \x1b[7mBravo\x1b[0m",
                "  Charlie",
            ],
            vec!["→ Alpha", "  \x1b[44mBravo\x1b[0m", "  Charlie"],
            vec![
                "  \x1b[7mAlpha\x1b[0m",
                "  \x1b[44mBravo\x1b[0m",
                "  Charlie",
            ],
            vec!["   Alpha", "→ Bravo", "  Charlie"],
        ] {
            assert!(detect(&rows).is_none(), "{rows:?}");
        }
    }

    #[test]
    fn ordinary_output_lists_colors_and_spinners_are_not_suggestions() {
        for rows in [
            vec!["  Alpha", "  Bravo", "  Charlie"],
            vec!["  \x1b[36mAlpha\x1b[0m", "  Bravo", "  Charlie"],
            vec!["  \x1b[1mAlpha\x1b[0m", "  Bravo", "  Charlie"],
            vec!["1. Alpha", "2. Bravo", "3. Charlie"],
            vec!["> Alpha", "> Bravo", "> Charlie"],
            vec!["⠋ Alpha", "  Bravo", "  Charlie"],
            vec!["  Alpha", "/ Bravo"],
            vec!["  Alpha", "| Bravo"],
            vec!["  Alpha", "  \x1b[7mB\x1b[0mravo"],
            vec!["→ Alpha"],
            vec!["│"],
            vec!["││"],
        ] {
            assert!(detect(&rows).is_none(), "{rows:?}");
        }
    }

    #[test]
    fn detection_needs_a_live_nearby_composer_and_valid_geometry() {
        let (mut snapshot, composer) = frame(&["→ Alpha", "  Bravo"]);
        assert!(detect_menu(&snapshot, None).is_none());
        assert!(detect_menu(&snapshot, Some((composer.1, composer.0))).is_none());
        assert!(detect_menu(&snapshot, Some((snapshot.rows, snapshot.rows))).is_none());
        snapshot.display_offset = 1;
        assert!(detect_menu(&snapshot, Some(composer)).is_none());
        snapshot.display_offset = 0;
        snapshot.cursor = Some(bks_terminal::CursorSnapshot { row: 0, column: 0 });
        assert!(detect_menu(&snapshot, Some(composer)).is_none());
        snapshot.cursor = None;
        snapshot.cells.pop();
        assert!(detect_menu(&snapshot, Some(composer)).is_none());
        assert!(detect(&["→ Alpha", "  Bravo", "", "", "", ""]).is_none());
    }

    #[test]
    fn paging_rebuilds_source_mapping_without_turning_counters_into_items() {
        let before = detect(&["  Alpha", "❯ Bravo", "  Charlie", "(2/40)"]).unwrap();
        let after = detect(&["  Bravo", "❯ Charlie", "  Delta", "(3/40)"]).unwrap();
        assert_eq!(before.items.len(), 3);
        assert_eq!(after.items.len(), 3);
        assert_eq!(after.selected, 1);
        assert_eq!(
            after.items[1],
            Suggestion {
                label: "Charlie".into(),
                row: 1
            }
        );
        assert_eq!(after.last_row, 3);
    }

    #[test]
    fn source_menu_may_be_below_the_composer_while_native_layout_is_independent() {
        let menu = below(&["  Alpha", "→ Bravo", "  Charlie", "(2/40)", "Footer status"]).unwrap();
        assert_eq!(menu.selected, 1);
        assert_eq!((menu.first_row, menu.last_row), (4, 7));
        assert_eq!(
            menu.items[1],
            Suggestion {
                label: "Bravo".into(),
                row: 5
            }
        );
    }

    #[test]
    fn two_column_description_wraps_require_the_exact_description_indent() {
        let rows = [
            "  Alpha    First part",
            "           continued alpha",
            "→ Bravo    Second part",
            "           continued bravo",
            "  Charlie  Third part",
            "           continued charlie",
            "(2/40)",
        ];
        for (menu, source_start) in [(detect(&rows).unwrap(), 0), (below(&rows).unwrap(), 4)] {
            assert_eq!(menu.items.len(), 3);
            assert_eq!(menu.selected, 1);
            assert_eq!(menu.items[1].label, "Bravo    Second part continued bravo");
            assert_eq!(menu.items[1].row, source_start + 2);
            assert_eq!(
                (menu.first_row, menu.last_row),
                (source_start, source_start + 6)
            );
        }
        let bad = [
            "  Alpha    First part",
            "            incorrect indent",
            "→ Bravo    Second part",
        ];
        assert!(detect(&bad).is_none());
        assert!(below(&bad).is_none());
    }

    #[test]
    fn independent_menus_on_both_sides_are_ambiguous() {
        let mut terminal = TerminalModel::new(120, 32);
        terminal.push_bytes(
            concat!(
                "→ Above first\r\n  Above second\r\n",
                "┌──────────────────┐\r\n│ > /m             │\r\n└──────────────────┘\r\n",
                "  Below first\r\n→ Below second\x1b[?25l"
            )
            .as_bytes(),
        );
        assert!(detect_menu(&terminal.live_snapshot(), Some((2, 4))).is_none());
    }

    #[test]
    fn hidden_labels_and_unbounded_lists_cannot_supply_selection() {
        let (mut snapshot, composer) = frame(&["→ Alpha", "  Bravo", "  Charlie"]);
        for cell in &mut snapshot.cells {
            cell.flags.insert(Flags::HIDDEN);
        }
        assert!(detect_menu(&snapshot, Some(composer)).is_none());
        let rows: Vec<_> = (0..=MAX_ITEMS)
            .map(|index| format!("{} Choice {index}", if index == 5 { "→" } else { " " }))
            .collect();
        assert!(detect(&rows.iter().map(String::as_str).collect::<Vec<_>>()).is_none());
        let (mut snapshot, composer) = frame(&["  Alpha", "→ Bravo", "  Charlie"]);
        let selected = snapshot.cols + 2;
        snapshot.cells[selected].text = format!("B{}", "\u{301}".repeat(MAX_LABEL_CHARS));
        assert!(detect_menu(&snapshot, Some(composer)).is_none());
    }

    fn foreground_frame(
        selected: usize,
        dim: bool,
        wrapped: bool,
    ) -> (TerminalSnapshot, (usize, usize)) {
        let mut rows = Vec::new();
        for (index, label) in ["/alpha", "/bravo", "/third"].iter().enumerate() {
            let style = if selected == index {
                "\x1b[36m"
            } else if dim {
                "\x1b[2m"
            } else {
                ""
            };
            rows.push(format!(
                "  {style}{label}    Ж界e\u{301}😀 description\x1b[0m"
            ));
            if wrapped {
                rows.push(format!("            {style}continued {index}\x1b[0m"));
            }
        }
        frame(&rows.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[test]
    fn temporal_foreground_menu_requires_an_accepted_arrow_and_source_ack() {
        let (before, composer) = foreground_frame(0, true, false);
        let (after, _) = foreground_frame(1, true, false);
        let unchanged_source = after.clone();
        assert!(detect_menu(&before, Some(composer)).is_none());
        assert!(detect_menu(&after, Some(composer)).is_none());
        let mut observer = MenuObserver::default();
        assert!(observer.observe(&before, 10, Some(composer)).is_none());
        observer.accepted(b"\x1b[B", 10);
        assert!(observer.observe(&before, 10, Some(composer)).is_none());
        // A source cursor-only update can precede the real selection response.
        assert!(observer.observe(&before, 11, Some(composer)).is_none());
        let menu = observer.observe(&after, 12, Some(composer)).unwrap();
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.items[1].label, "/bravo    Ж界e\u{301}😀 description");
        assert_eq!(after, unchanged_source);
        assert_eq!(observer.observe(&after, 13, Some(composer)), Some(menu));
        observer.accepted(b"\x1bOA", 13);
        assert_eq!(
            observer
                .observe(&before, 14, Some(composer))
                .unwrap()
                .selected,
            0
        );
    }

    #[test]
    fn temporal_foreground_suggestions_below_horizontal_composer_have_no_pointer() {
        let scene = |selected| {
            let mut terminal = TerminalModel::new(113, 32);
            let rule = "─".repeat(113);
            let mut text = format!("Prior output\r\n\r\n{rule}\r\n❯\u{a0}/\r\n{rule}\r\n");
            for (index, label) in ["/help", "/model", "/clear"].iter().enumerate() {
                text.push_str(&format!(
                    "  {}{label:<10}Description {index}\x1b[0m\r\n",
                    if index == selected {
                        "\x1b[38;2;110;190;240m"
                    } else {
                        "\x1b[2m"
                    }
                ));
            }
            text.push_str("\x1b[4;4H");
            terminal.push_bytes(text.as_bytes());
            terminal.live_snapshot()
        };
        let before = scene(0);
        let after = scene(1);
        let composer = Some((2, 4));
        assert!(detect_menu(&after, composer).is_none());
        let mut observer = MenuObserver::default();
        assert!(observer.observe(&before, 8, composer).is_none());
        observer.accepted(b"\x1b[B", 8);
        let menu = observer.observe(&after, 9, composer).unwrap();
        assert_eq!((menu.first_row, menu.last_row, menu.selected), (5, 7, 1));
        assert_eq!(menu.items[1].label, "/model    Description 1");
    }

    #[test]
    fn temporal_foreground_static_lists_repeated_keys_and_wrong_direction_are_rejected() {
        let (before, composer) = foreground_frame(1, true, false);
        let (up, _) = foreground_frame(0, true, false);
        let (down, _) = foreground_frame(2, true, false);
        for input in [
            None,
            Some(&b"\x1b[A"[..]),
            Some(&b"\x1b[B\x1b[B"[..]),
            Some(&b"x"[..]),
        ] {
            let mut observer = MenuObserver::default();
            observer.observe(&before, 1, Some(composer));
            if let Some(input) = input {
                observer.accepted(input, 1);
            }
            assert!(observer.observe(&down, 2, Some(composer)).is_none());
        }
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&up, 2, Some(composer)).is_none());
        observer.accepted(b"\x1bOB", 2);
        observer.accepted(b"\x1bOB", 2);
        assert!(observer.observe(&before, 3, Some(composer)).is_none());
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 0);
        assert!(observer.observe(&down, 2, Some(composer)).is_none());
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&down, 1, Some(composer)).is_none());
        assert!(observer.observe(&down, 2, Some(composer)).is_none());
    }

    #[test]
    fn temporal_foreground_query_layout_page_and_unknown_selection_changes_invalidate_proof() {
        let (before, composer) = foreground_frame(0, true, false);
        let (after, _) = foreground_frame(1, true, false);
        for scenario in 0..5 {
            let mut changed = after.clone();
            let rows = match scenario {
                0 => {
                    changed.cells[composer.0 * changed.cols + 4].text = "z".into();
                    Some(composer)
                }
                1 => {
                    changed.alternate_screen = !changed.alternate_screen;
                    Some(composer)
                }
                2 => {
                    changed.history_size += 1;
                    Some(composer)
                }
                3 => {
                    changed.cells[2].text = "D".into();
                    Some(composer)
                }
                _ => None,
            };
            let mut observer = MenuObserver::default();
            observer.observe(&before, 1, Some(composer));
            observer.accepted(b"\x1b[B", 1);
            assert!(
                observer.observe(&changed, 2, rows).is_none(),
                "scenario {scenario}"
            );
        }
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&after, 2, Some(composer)).is_some());
        assert!(observer.observe(&before, 3, Some(composer)).is_none());
        observer.accepted(b"\x1bOB", 3);
        assert!(observer.observe(&after, 4, Some(composer)).is_some());
        observer.accepted(b"/", 4);
        assert!(observer.observe(&after, 5, Some(composer)).is_none());
        observer.clear();
        assert!(observer.observe(&after, 6, Some(composer)).is_none());
    }

    #[test]
    fn temporal_foreground_handles_unicode_description_wrap_and_plain_foreground_transfer() {
        for dim in [false, true] {
            let (before, composer) = foreground_frame(0, dim, true);
            let (after, _) = foreground_frame(1, dim, true);
            let mut observer = MenuObserver::default();
            observer.observe(&before, 1, Some(composer));
            observer.accepted(b"\x1bOB", 1);
            let menu = observer.observe(&after, 2, Some(composer)).unwrap();
            assert_eq!(menu.selected, 1);
            assert_eq!(menu.items.len(), 3);
            assert_eq!(
                menu.items[1].label,
                "/bravo    Ж界e\u{301}😀 description continued 1"
            );
            assert_eq!(menu.items[1].row, 2);
        }
    }

    fn palette_frame(
        selected: usize,
        count: usize,
        selected_style: &str,
        normal_style: &str,
    ) -> (TerminalSnapshot, (usize, usize)) {
        let rows: Vec<_> = (0..count)
            .map(|index| {
                let style = if index == selected {
                    selected_style
                } else {
                    normal_style
                };
                format!("  {style}Choice {index}\x1b[0m")
            })
            .collect();
        frame(&rows.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[test]
    fn temporal_two_item_foreground_exchange_does_not_guess_default_color() {
        for (selected_style, normal_style) in [
            ("\x1b[39m", "\x1b[36m"),
            ("\x1b[36m", "\x1b[39m"),
            ("\x1b[31m", "\x1b[36m"),
        ] {
            for from in 0..2 {
                let (before, composer) = palette_frame(from, 2, selected_style, normal_style);
                let (after, _) = palette_frame(1 - from, 2, selected_style, normal_style);
                for input in [b"\x1b[A", b"\x1b[B"] {
                    let mut observer = MenuObserver::default();
                    assert!(observer.observe(&before, 1, Some(composer)).is_none());
                    observer.accepted(input, 1);
                    assert!(observer.observe(&after, 2, Some(composer)).is_none());
                    assert!(observer.observe(&after, 3, Some(composer)).is_none());
                }
            }
        }
    }

    #[test]
    fn temporal_palette_orientation_uses_dim_or_unchanged_rows() {
        for (count, selected_style, normal_style) in [
            (3, "\x1b[39m", "\x1b[36m"),
            (3, "\x1b[36m", "\x1b[39m"),
            (2, "\x1b[39m", "\x1b[2m"),
            (2, "\x1b[36m", "\x1b[2m"),
        ] {
            for from in 0..count {
                let (before, composer) = palette_frame(from, count, selected_style, normal_style);
                for (input, to) in [
                    (b"\x1b[A", (from + count - 1) % count),
                    (b"\x1b[B", (from + 1) % count),
                ] {
                    let (after, _) = palette_frame(to, count, selected_style, normal_style);
                    let mut observer = MenuObserver::default();
                    assert!(observer.observe(&before, 1, Some(composer)).is_none());
                    observer.accepted(input, 1);
                    assert_eq!(
                        observer
                            .observe(&after, 2, Some(composer))
                            .unwrap()
                            .selected,
                        to
                    );
                }
            }
        }
    }

    #[test]
    fn temporal_dim_only_and_sixteen_item_source_lists_are_supported() {
        let rows = |selected| {
            (0..16)
                .map(|index| {
                    format!(
                        "  {}command{index:02}    description\x1b[0m",
                        if index == selected { "" } else { "\x1b[2m" }
                    )
                })
                .collect::<Vec<_>>()
        };
        let old = rows(7);
        let new = rows(8);
        let (before, composer) = frame(&old.iter().map(String::as_str).collect::<Vec<_>>());
        let (after, _) = frame(&new.iter().map(String::as_str).collect::<Vec<_>>());
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        let menu = observer.observe(&after, 2, Some(composer)).unwrap();
        assert_eq!((menu.items.len(), menu.selected), (16, 8));
    }

    #[test]
    fn temporal_foreground_needs_two_reciprocal_rows_and_expires_without_response() {
        let (before, composer) = foreground_frame(0, true, false);
        let (after, _) = foreground_frame(1, true, false);
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        observer.pending.as_mut().unwrap().started = Instant::now() - NAVIGATION_LIFETIME;
        assert!(observer.observe(&after, 2, Some(composer)).is_none());
        let mut changed = after.clone();
        changed.cells[..changed.cols].clone_from_slice(&before.cells[..before.cols]);
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&changed, 2, Some(composer)).is_none());
    }

    #[test]
    fn temporal_two_transferring_lists_and_custom_two_color_rows_are_ambiguous() {
        let scene = |selected| {
            let rows = (0..3)
                .map(|index| {
                    format!(
                        "  {}Label {index}\x1b[0m",
                        if index == selected {
                            "\x1b[36m"
                        } else {
                            "\x1b[2m"
                        }
                    )
                })
                .collect::<Vec<_>>();
            let text = format!("{}\r\n┌──────────────────┐\r\n│ > /m             │\r\n└──────────────────┘\r\n{}\x1b[?25l", rows.join("\r\n"), rows.join("\r\n"));
            let mut terminal = TerminalModel::new(120, 32);
            terminal.push_bytes(text.as_bytes());
            terminal.live_snapshot()
        };
        let mut observer = MenuObserver::default();
        observer.observe(&scene(0), 1, Some((3, 5)));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&scene(1), 2, Some((3, 5))).is_none());
        let (before, composer) = frame(&["  \x1b[31mAlpha\x1b[0m", "  \x1b[36mBravo\x1b[0m"]);
        let (after, _) = frame(&["  \x1b[36mAlpha\x1b[0m", "  \x1b[31mBravo\x1b[0m"]);
        let mut observer = MenuObserver::default();
        observer.observe(&before, 1, Some(composer));
        observer.accepted(b"\x1b[B", 1);
        assert!(observer.observe(&after, 2, Some(composer)).is_none());
    }

    #[test]
    fn temporal_observer_keeps_the_strict_pointer_fast_path() {
        let (snapshot, composer) = frame(&["  Alpha", "❯ Bravo", "  Charlie"]);
        let mut observer = MenuObserver::default();
        assert_eq!(
            observer.observe(&snapshot, 1, Some(composer)),
            detect_menu(&snapshot, Some(composer))
        );
        observer.accepted(b"x", 1);
        assert_eq!(
            observer.observe(&snapshot, 2, Some(composer)),
            detect_menu(&snapshot, Some(composer))
        );
    }

    struct HiddenParent {
        hwnd: HWND,
        _apartment: crate::accessibility::ComApartment,
    }

    impl HiddenParent {
        fn new() -> Self {
            let apartment = crate::accessibility::ComApartment::initialize().unwrap();
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("BKS private inline suggestions test"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    640,
                    240,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            Self {
                hwnd,
                _apartment: apartment,
            }
        }
    }

    impl Drop for HiddenParent {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    fn native_menu(count: usize, selected: usize) -> Menu {
        Menu {
            items: (0..count)
                .map(|row| Suggestion {
                    label: format!("Ж界e\u{301}😀 choice {row}"),
                    row,
                })
                .collect(),
            selected,
            first_row: 0,
            last_row: count - 1,
        }
    }

    #[test]
    fn private_hidden_native_list_sync_preserves_selection_viewport_and_source_text() {
        let parent = HiddenParent::new();
        let focus = unsafe { GetFocus() };
        let mut list = Suggestions::create(parent.hwnd, HINSTANCE::default(), 1030).unwrap();
        unsafe {
            MoveWindow(list.hwnd, 0, 0, 300, 48, false).unwrap();
        }
        let mut menu = native_menu(10, 7);
        list.sync(&menu).unwrap();
        assert_eq!(list.selection(), Some(7));
        assert_eq!(unsafe { GetDlgCtrlID(list.hwnd) }, 1030);
        assert_eq!(
            unsafe { SendMessageW(list.hwnd, LB_GETCOUNT, None, None).0 },
            10
        );
        unsafe {
            SendMessageW(list.hwnd, LB_SETTOPINDEX, Some(WPARAM(6)), None);
        }
        let top = unsafe { SendMessageW(list.hwnd, LB_GETTOPINDEX, None, None).0 };
        list.sync(&menu).unwrap();
        assert_eq!(
            unsafe { SendMessageW(list.hwnd, LB_GETTOPINDEX, None, None).0 },
            top
        );
        menu.items[0].label = "Changed source item".into();
        list.sync(&menu).unwrap();
        assert_eq!(list.selection(), Some(7));
        assert_eq!(
            unsafe { SendMessageW(list.hwnd, LB_GETTOPINDEX, None, None).0 },
            top
        );
        let mut text = [0u16; 128];
        let length = unsafe {
            SendMessageW(
                list.hwnd,
                LB_GETTEXT,
                Some(WPARAM(7)),
                Some(LPARAM(text.as_mut_ptr() as isize)),
            )
            .0
        } as usize;
        assert_eq!(
            String::from_utf16(&text[..length]).unwrap(),
            menu.items[7].label
        );
        menu.selected = 8;
        list.sync(&menu).unwrap();
        assert_eq!(list.selection(), Some(8));
        assert!(list.set_selection(10).is_err());
        assert_eq!(unsafe { GetFocus() }, focus);
        assert_eq!(
            unsafe { GetWindowLongW(list.hwnd, GWL_STYLE) } as u32 & WS_VISIBLE.0,
            0
        );
        list.show();
        assert!(!unsafe { IsWindowVisible(parent.hwnd).as_bool() });
        assert_eq!(unsafe { GetFocus() }, focus);
        list.hide();
        list.clear();
        assert_eq!(list.selection(), None);
        assert_eq!(
            unsafe { SendMessageW(list.hwnd, LB_GETCOUNT, None, None).0 },
            0
        );
        let hwnd = list.hwnd;
        drop(list);
        assert!(!unsafe { IsWindow(Some(hwnd)).as_bool() });
    }

    #[test]
    fn invalid_native_source_menu_keeps_the_existing_list_intact() {
        let parent = HiddenParent::new();
        let mut list = Suggestions::create(parent.hwnd, HINSTANCE::default(), 1030).unwrap();
        let menu = native_menu(3, 1);
        list.sync(&menu).unwrap();
        for bad in [
            Menu {
                selected: 3,
                ..menu.clone()
            },
            Menu {
                items: vec![Suggestion {
                    label: "only one".into(),
                    row: 0,
                }],
                ..menu.clone()
            },
            Menu {
                items: vec![
                    Suggestion {
                        label: "→".into(),
                        row: 0
                    };
                    3
                ],
                ..menu.clone()
            },
            Menu {
                items: vec![
                    Suggestion {
                        label: "text\0hidden".into(),
                        row: 0
                    };
                    3
                ],
                ..menu.clone()
            },
        ] {
            assert!(list.sync(&bad).is_err());
            assert_eq!(list.selection(), Some(1));
            assert_eq!(
                unsafe { SendMessageW(list.hwnd, LB_GETCOUNT, None, None).0 },
                3
            );
        }
    }
}
