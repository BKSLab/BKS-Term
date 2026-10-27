use std::cmp::Reverse;
use std::ops::Range;

use bks_terminal::ReadingDocument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoredReading {
    pub selection: (u32, u32),
    pub viewport_offset: u32,
    pub follow: bool,
}

/// Main and alternate screens keep independent reading positions and text.
#[derive(Default)]
pub struct ReadingState {
    main: Bookmark,
    alternate: Bookmark,
}

#[derive(Default)]
struct Bookmark {
    document: Option<TextSnapshot>,
    selection: (u32, u32),
    viewport_offset: u32,
    follow: bool,
}

struct TextSnapshot {
    revision: u64,
    text: Vec<u16>,
    lines: Vec<Range<usize>>,
}

impl TextSnapshot {
    fn new(document: &ReadingDocument) -> Self {
        Self {
            revision: document.revision,
            text: document.text.encode_utf16().collect(),
            lines: document
                .lines
                .iter()
                .map(|line| line.start as usize..line.end as usize)
                .collect(),
        }
    }

    fn anchor(&self, offset: u32) -> (usize, usize) {
        let index = self
            .lines
            .partition_point(|line| line.start <= offset as usize)
            .saturating_sub(1);
        (index, offset as usize - self.lines[index].start)
    }

    fn line_text(&self, index: usize) -> &[u16] {
        &self.text[self.lines[index].clone()]
    }
}

impl ReadingState {
    /// Offsets refer to the current native document; the viewport offset is
    /// EM_LINEINDEX(first visible native line), not a logical VT line number.
    pub fn capture(
        &mut self,
        document: &ReadingDocument,
        selection: (u32, u32),
        viewport_offset: u32,
        at_bottom: bool,
    ) {
        let bookmark = self.bookmark(document);
        if bookmark.document.as_ref().map(|saved| saved.revision) != Some(document.revision) {
            bookmark.document = Some(TextSnapshot::new(document));
        }
        let end = bookmark.document.as_ref().unwrap().text.len() as u32;
        bookmark.selection = normalize_selection(document, selection);
        bookmark.viewport_offset = snap(document, viewport_offset, false);
        // Empty EOF after a clear is not an explicit request to follow output.
        // Keep the user's previous choice until a nonempty capture can change it.
        if end > 0 {
            bookmark.follow = at_bottom && bookmark.selection == (end, end);
        }
    }

    /// Also advances the stored snapshot when the user has no focus in the
    /// reader, so repeated updates never reuse offsets from an obsolete epoch.
    pub fn restore(&mut self, document: &ReadingDocument) -> RestoredReading {
        let bookmark = self.bookmark(document);
        if bookmark.document.as_ref().map(|saved| saved.revision) == Some(document.revision) {
            return bookmark.restored();
        }
        let next = TextSnapshot::new(document);
        if bookmark.follow {
            let end = next.text.len() as u32;
            bookmark.selection = (end, end);
            bookmark.viewport_offset = end;
        } else if let Some(previous) = &bookmark.document {
            let change = Change::new(previous, &next);
            let start = change.map_offset(bookmark.selection.0);
            let finish = change.map_offset(bookmark.selection.1);
            let old_selection = bookmark.selection.0 as usize..bookmark.selection.1 as usize;
            bookmark.selection = if old_selection.is_empty() {
                (start, start)
            } else if next.text.get(start as usize..finish as usize)
                == Some(&previous.text[old_selection.clone()])
            {
                (start, finish)
            } else if let Some(start) =
                find_selected_span(previous, &next, old_selection.clone(), start as usize)
            {
                (start as u32, (start + old_selection.len()) as u32)
            } else {
                // Removed/overwritten selection: keep its nearest logical
                // line and column, without selecting unrelated new output.
                (start, start)
            };
            bookmark.viewport_offset = change.map_offset(bookmark.viewport_offset);
        }
        bookmark.selection = normalize_selection(document, bookmark.selection);
        bookmark.viewport_offset = snap(document, bookmark.viewport_offset, false);
        bookmark.document = Some(next);
        bookmark.restored()
    }

    fn bookmark(&mut self, document: &ReadingDocument) -> &mut Bookmark {
        if document.alternate_screen {
            &mut self.alternate
        } else {
            &mut self.main
        }
    }
}

impl Bookmark {
    fn restored(&self) -> RestoredReading {
        RestoredReading {
            selection: self.selection,
            viewport_offset: self.viewport_offset,
            follow: self.follow,
        }
    }
}

fn normalize_selection(document: &ReadingDocument, selection: (u32, u32)) -> (u32, u32) {
    let start = selection.0.min(selection.1);
    let end = selection.0.max(selection.1);
    let start = snap(document, start, false);
    let end = snap(document, end, selection.0 != selection.1);
    (start, end)
}

fn snap(document: &ReadingDocument, offset: u32, forward: bool) -> u32 {
    let offset = offset.min(document.lines.last().map_or(0, |line| line.end));
    match document.cell_at(offset) {
        Some(cell) if offset > cell.start => {
            if forward {
                cell.end
            } else {
                cell.start
            }
        }
        _ => offset,
    }
}

struct Change<'a> {
    previous: &'a TextSnapshot,
    next: &'a TextSnapshot,
    prefix: usize,
    suffix: usize,
    line_shift: isize,
}

impl<'a> Change<'a> {
    fn new(previous: &'a TextSnapshot, next: &'a TextSnapshot) -> Self {
        let prefix = previous
            .text
            .iter()
            .zip(&next.text)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = previous.text[prefix..]
            .iter()
            .rev()
            .zip(next.text[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        Self {
            previous,
            next,
            prefix,
            suffix,
            line_shift: if prefix == previous.text.len() {
                0
            } else {
                guess_line_shift(previous, next)
            },
        }
    }

    fn map_offset(&self, offset: u32) -> u32 {
        let offset = offset as usize;
        if offset < self.prefix || self.prefix == self.previous.text.len() {
            return offset as u32;
        }
        if self.suffix > 0 && offset >= self.previous.text.len() - self.suffix {
            return (self.next.text.len() - (self.previous.text.len() - offset)) as u32;
        }
        let (old_line, column) = self.previous.anchor(offset as u32);
        let expected = old_line
            .saturating_add_signed(self.line_shift)
            .min(self.next.lines.len() - 1);
        let index = (0..self.next.lines.len())
            .filter(|&index| self.next.line_text(index) == self.previous.line_text(old_line))
            .filter(|&index| {
                index == expected || context_score(self.previous, old_line, self.next, index) > 0
            })
            .max_by_key(|&index| {
                (
                    context_score(self.previous, old_line, self.next, index),
                    Reverse(index.abs_diff(expected)),
                    Reverse(index),
                )
            })
            .unwrap_or(expected);
        let line = &self.next.lines[index];
        (line.start + column.min(line.len())) as u32
    }
}

fn context_score(
    old: &TextSnapshot,
    old_line: usize,
    new: &TextSnapshot,
    new_line: usize,
) -> usize {
    [-2, -1, 1, 2]
        .into_iter()
        .filter(|&delta| {
            let Some(a) = old_line
                .checked_add_signed(delta)
                .filter(|&i| i < old.lines.len())
            else {
                return false;
            };
            let Some(b) = new_line
                .checked_add_signed(delta)
                .filter(|&i| i < new.lines.len())
            else {
                return false;
            };
            old.line_text(a) == new.line_text(b)
        })
        .count()
}

fn guess_line_shift(old: &TextSnapshot, new: &TextSnapshot) -> isize {
    // A few surviving leading lines detect history trimming as well as
    // insertion/removal above a bookmark. Prefer nearby matches on ambiguity.
    (0..new.lines.len().min(3))
        .flat_map(|new_line| (0..old.lines.len()).map(move |old_line| (old_line, new_line)))
        .filter(|&(a, b)| {
            !new.line_text(b).is_empty()
                && old.line_text(a) == new.line_text(b)
                && (b == 0 || context_score(old, a, new, b) > 0)
        })
        .max_by_key(|&(a, b)| {
            (
                context_score(old, a, new, b),
                Reverse(a.abs_diff(b)),
                Reverse(a),
            )
        })
        .map_or(0, |(a, b)| b as isize - a as isize)
}

/// Linear KMP search preserves an unchanged selected substring even when the
/// rest of its line was overwritten. Context and distance disambiguate repeats.
fn find_selected_span(
    previous: &TextSnapshot,
    next: &TextSnapshot,
    selected: Range<usize>,
    expected: usize,
) -> Option<usize> {
    let needle = &previous.text[selected.clone()];
    let mut prefix = vec![0; needle.len()];
    let mut matched = 0;
    for index in 1..needle.len() {
        while matched > 0 && needle[index] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    let mut best = None;
    let expected_line = next.anchor(expected as u32).0;
    let old_line = previous.anchor(selected.start as u32).0;
    matched = 0;
    for (index, unit) in next.text.iter().enumerate() {
        while matched > 0 && *unit != needle[matched] {
            matched = prefix[matched - 1];
        }
        if *unit == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            let start = index + 1 - needle.len();
            let candidate_line = next.anchor(start as u32).0;
            // An isolated duplicate in unrelated later output is insufficient
            // evidence that the old occurrence survived. Stay near its anchor.
            if candidate_line != expected_line
                && context_score(previous, old_line, next, candidate_line) == 0
            {
                matched = prefix[matched - 1];
                continue;
            }
            let before = previous.text[..selected.start]
                .iter()
                .rev()
                .zip(next.text[..start].iter().rev())
                .take(24)
                .take_while(|(a, b)| a == b)
                .count();
            let after = previous.text[selected.end..]
                .iter()
                .zip(&next.text[index + 1..])
                .take(24)
                .take_while(|(a, b)| a == b)
                .count();
            let key = (
                before + after,
                Reverse(start.abs_diff(expected)),
                Reverse(start),
            );
            if best.as_ref().is_none_or(|&(old_key, _)| key > old_key) {
                best = Some((key, start));
            }
            matched = prefix[matched - 1];
        }
    }
    best.map(|(_, start)| start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    #[test]
    fn removed_selection_does_not_jump_to_an_isolated_duplicate_in_new_tail() {
        let first = document("before\r\nchosen value\r\nlast", 1, false);
        let mut state = ReadingState::default();
        state.capture(&first, selected(&first, "chosen value"), 8, false);
        let next = document("before\r\nXXXX\r\nlast\r\nchosen value", 2, false);
        let restored = state.restore(&next);
        assert_eq!(restored.selection, (8, 8));
        assert_eq!(restored.viewport_offset, 8);
        assert!(!restored.follow);
    }

    fn document(text: &str, revision: u64, alternate: bool) -> ReadingDocument {
        let mut terminal = TerminalModel::new(120, 60);
        terminal.push_bytes(text.as_bytes());
        let mut document = terminal.reading_document();
        document.revision = revision;
        document.alternate_screen = alternate;
        document
    }

    fn selected(document: &ReadingDocument, needle: &str) -> (u32, u32) {
        let index = document.text.find(needle).unwrap();
        let start = document.text[..index].encode_utf16().count() as u32;
        (start, start + needle.encode_utf16().count() as u32)
    }

    fn end(document: &ReadingDocument) -> u32 {
        document.text.encode_utf16().count() as u32
    }

    #[test]
    fn append_keeps_selection_and_native_viewport_across_repeated_restore() {
        let first = document("zero\r\nselected text\r\nthird", 1, false);
        let mut state = ReadingState::default();
        let selection = selected(&first, "selected");
        state.capture(&first, selection, selection.0, false);
        for (revision, suffix) in [(2, "\r\nfourth"), (3, "\r\nfourth\r\nfifth")] {
            let next = document(&format!("{}{suffix}", first.text), revision, false);
            let restored = state.restore(&next);
            assert_eq!(restored.selection, selection);
            assert_eq!(restored.viewport_offset, selection.0);
            assert!(!restored.follow);
            assert_eq!(state.restore(&next), restored);
        }
    }

    #[test]
    fn follow_is_opt_in_at_collapsed_end_and_bottom() {
        let first = document("one", 1, false);
        let mut state = ReadingState::default();
        assert_eq!(
            state.restore(&first),
            RestoredReading {
                selection: (0, 0),
                viewport_offset: 0,
                follow: false,
            }
        );
        state.capture(&first, (end(&first), end(&first)), 0, false);
        let second = document("one\r\ntwo", 2, false);
        assert_eq!(state.restore(&second).selection, (3, 3));
        assert!(!state.restore(&second).follow);
        state.capture(&second, (end(&second), end(&second)), 0, true);
        let third = document("one\r\ntwo\r\nthree", 3, false);
        let restored = state.restore(&third);
        assert!(restored.follow);
        assert_eq!(restored.selection, (end(&third), end(&third)));
        assert_eq!(restored.viewport_offset, end(&third));
        state.capture(&third, (end(&third) - 1, end(&third)), 0, true);
        assert!(!state.restore(&third).follow);
    }

    #[test]
    fn empty_capture_after_clear_does_not_enable_follow_for_new_output() {
        let first = document("before\r\nselected fragment\r\nafter", 1, false);
        let mut state = ReadingState::default();
        let selection = selected(&first, "selected fragment");
        state.capture(&first, selection, selection.0, false);

        let cleared = document("", 2, false);
        assert_eq!(
            state.restore(&cleared),
            RestoredReading {
                selection: (0, 0),
                viewport_offset: 0,
                follow: false,
            }
        );
        state.capture(&cleared, (0, 0), 0, true);

        let text: String = (0..80).map(|row| format!("later {row}\r\n")).collect();
        let later = document(&text, 3, false);
        assert_eq!(
            state.restore(&later),
            RestoredReading {
                selection: (0, 0),
                viewport_offset: 0,
                follow: false,
            }
        );
    }

    #[test]
    fn explicit_follow_survives_empty_capture_and_later_append() {
        let first = document("before\r\nafter", 1, false);
        let mut state = ReadingState::default();
        state.capture(&first, (end(&first), end(&first)), 0, true);

        let cleared = document("", 2, false);
        assert_eq!(
            state.restore(&cleared),
            RestoredReading {
                selection: (0, 0),
                viewport_offset: 0,
                follow: true,
            }
        );
        state.capture(&cleared, (0, 0), 0, true);

        let text: String = (0..80).map(|row| format!("later {row}\r\n")).collect();
        let later = document(&text, 3, false);
        assert_eq!(
            state.restore(&later),
            RestoredReading {
                selection: (end(&later), end(&later)),
                viewport_offset: end(&later),
                follow: true,
            }
        );
    }

    #[test]
    fn trimmed_history_keeps_surviving_text_then_clamps_deleted_selection_nearby() {
        let first = document("discard0\r\ndiscard1\r\nkeep me\r\nafter", 1, false);
        let mut state = ReadingState::default();
        let selection = selected(&first, "keep me");
        state.capture(&first, selection, selection.0, false);
        let second = document("keep me\r\nafter\r\nnewtail", 2, false);
        let restored = state.restore(&second);
        assert_eq!(restored.selection, (0, 7));
        assert_eq!(restored.viewport_offset, 0);
        let third = document("after\r\nnewtail\r\nlater", 3, false);
        let restored = state.restore(&third);
        assert_eq!(restored.selection, (0, 0));
        assert_eq!(restored.viewport_offset, 0);
        assert!(!restored.follow);
    }

    #[test]
    fn overwrite_before_selection_moves_line_anchor_without_following_tail() {
        let first = document("header\r\nstable selection\r\nfooter", 1, false);
        let mut state = ReadingState::default();
        let selection = selected(&first, "stable selection");
        state.capture(&first, selection, selection.0, false);
        let next = document(
            "rewritten heading\r\nstable selection\r\nfooter\r\nappended",
            2,
            false,
        );
        let restored = state.restore(&next);
        assert_eq!(restored.selection, selected(&next, "stable selection"));
        assert_eq!(restored.viewport_offset, restored.selection.0);
        assert!(!restored.follow);
    }

    #[test]
    fn selected_substring_survives_partial_line_overwrite_and_append() {
        let first = document("short prefix TARGET unchanged tail", 1, false);
        let mut state = ReadingState::default();
        state.capture(&first, selected(&first, "TARGET"), 0, false);
        let next = document("a much longer prefix TARGET unchanged tail plus", 2, false);
        assert_eq!(state.restore(&next).selection, selected(&next, "TARGET"));
    }

    #[test]
    fn overwritten_selection_collapses_at_nearest_line_and_cls_clamps_to_start() {
        let first = document("before\r\nchosen value\r\nlast", 1, false);
        let mut state = ReadingState::default();
        let selection = selected(&first, "chosen value");
        state.capture(&first, selection, selection.0, false);
        let next = document("before\r\nXXXX\r\nlast\r\nextra", 2, false);
        let restored = state.restore(&next);
        assert_eq!(restored.selection, (8, 8));
        assert_eq!(restored.viewport_offset, 8);
        let cleared = document("", 3, false);
        assert_eq!(
            state.restore(&cleared),
            RestoredReading {
                selection: (0, 0),
                viewport_offset: 0,
                follow: false,
            }
        );
    }

    #[test]
    fn main_and_alternate_keep_separate_bookmarks_and_text_epochs() {
        let main = document("main one\r\nmain two", 1, false);
        let alternate = document("alt one\r\nalt two", 2, true);
        let mut state = ReadingState::default();
        let main_selection = selected(&main, "main two");
        state.capture(&main, main_selection, main_selection.0, false);
        assert_eq!(state.restore(&alternate).selection, (0, 0));
        state.capture(&alternate, (end(&alternate), end(&alternate)), 0, true);
        let main_next = document("main one\r\nmain two\r\nmain three", 3, false);
        let restored = state.restore(&main_next);
        assert_eq!(restored.selection, main_selection);
        assert_eq!(restored.viewport_offset, main_selection.0);
        assert!(!restored.follow);
        let alternate_next = document("alt one\r\nalt two\r\nalt three", 4, true);
        let restored = state.restore(&alternate_next);
        assert_eq!(
            restored.selection,
            (end(&alternate_next), end(&alternate_next))
        );
        assert!(restored.follow);
    }

    #[test]
    fn soft_wrap_resize_preserves_logical_offsets_and_native_line_anchor() {
        let mut terminal = TerminalModel::new(4, 3);
        terminal.push_bytes(b"abcdefghij\r\nlast");
        let first = terminal.reading_document();
        let selection = selected(&first, "fghi");
        let mut state = ReadingState::default();
        state.capture(&first, selection, 4, false);
        terminal.resize(8, 3);
        let next = terminal.reading_document();
        assert_eq!(next.text, first.text);
        let restored = state.restore(&next);
        assert_eq!(restored.selection, selection);
        assert_eq!(restored.viewport_offset, 4);
    }

    #[test]
    fn offsets_snap_to_whole_surrogate_and_combining_glyphs() {
        let first = document("A🙂e\u{301}界Z", 1, false);
        let mut state = ReadingState::default();
        state.capture(&first, (2, 4), 2, false);
        let restored = state.restore(&first);
        assert_eq!(restored.selection, (1, 5));
        assert_eq!(restored.viewport_offset, 1);
        state.capture(&first, (4, 4), 4, false);
        assert_eq!(state.restore(&first).selection, (3, 3));
        state.capture(&first, (1, 5), 0, false);
        let next = document("prefix A🙂e\u{301}界Z!", 2, false);
        assert_eq!(
            state.restore(&next).selection,
            selected(&next, "🙂e\u{301}")
        );
    }

    #[test]
    fn duplicate_lines_choose_nearest_matching_context_and_same_revision_reuses_text() {
        let first = document("left\r\ndup\r\nmiddle\r\ndup\r\nright", 1, false);
        let second_dup = first.text.rfind("dup").unwrap() as u32;
        let mut state = ReadingState::default();
        state.capture(&first, (second_dup, second_dup + 3), second_dup, false);
        let next = document("dup\r\nmiddle\r\ndup\r\nright\r\nnew", 2, false);
        let expected = next.text.rfind("dup").unwrap() as u32;
        let restored = state.restore(&next);
        assert_eq!(restored.selection, (expected, expected + 3));
        assert_eq!(restored.viewport_offset, expected);
        let storage = state.main.document.as_ref().unwrap().text.as_ptr();
        state.capture(&next, restored.selection, restored.viewport_offset, false);
        assert_eq!(state.restore(&next), restored);
        assert_eq!(state.main.document.as_ref().unwrap().text.as_ptr(), storage);
    }
}
