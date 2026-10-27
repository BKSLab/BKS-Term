//! A reading hint for generic terminal output after an explicit submission.
//! Text changes carry no meaning about replies, prompts, or permission to act.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};

use bks_terminal::ReadingDocument;

// Fingerprints retain no additional text and have a fixed memory bound even
// for a very long soft-wrapped logical line or a full 10,000-row history.
const MAX_BASELINE_LINES: usize = 4_096;

#[derive(Default)]
pub struct ReplyAnchor {
    baseline: Option<Baseline>,
}

struct Baseline {
    alternate_screen: bool,
    origin: usize,
    origin_offset: u32,
    prefix: u64,
    lines: Vec<LineKey>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct LineKey {
    hash: u64,
    bytes: usize,
}

impl LineKey {
    fn new(text: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        Self {
            hash: hasher.finish(),
            bytes: text.len(),
        }
    }
}

impl ReplyAnchor {
    /// Replace the baseline with the already visible native reading document.
    /// Call immediately before forwarding the user's explicit Enter to the PTY.
    pub fn submitted(&mut self, document: &ReadingDocument) {
        let origin = document.lines.len().saturating_sub(MAX_BASELINE_LINES);
        let mut prefix = DefaultHasher::new();
        let mut lines = Vec::with_capacity(document.lines.len().min(MAX_BASELINE_LINES));
        for (index, (_, text)) in document
            .lines
            .iter()
            .zip(document.text.split("\r\n"))
            .enumerate()
        {
            let key = LineKey::new(text);
            if index < origin {
                key.hash(&mut prefix);
            } else {
                lines.push(key);
            }
        }
        self.baseline = Some(Baseline {
            alternate_screen: document.alternate_screen,
            origin,
            origin_offset: document.lines.get(origin).map_or(0, |line| line.start),
            prefix: prefix.finish(),
            lines,
        });
    }

    /// First changed, nonblank logical line in the current document, in UTF-16.
    /// This query never advances the baseline or remembers a selection. The
    /// caller decides when to select it and whether to follow later output.
    /// An ambiguous history replacement, unchanged text, or clear alone returns
    /// None, so the caller can use its ordinary live-page reading fallback.
    pub fn offset(&self, document: &ReadingDocument) -> Option<u32> {
        let baseline = self.baseline.as_ref()?;
        let lines: Vec<_> = document
            .lines
            .iter()
            .zip(document.text.split("\r\n"))
            .collect();
        if baseline.alternate_screen != document.alternate_screen {
            return lines
                .iter()
                .find(|(_, text)| !text.trim().is_empty())
                .map(|(line, _)| line.start);
        }
        let keys: Vec<_> = lines.iter().map(|(_, text)| LineKey::new(text)).collect();
        let mut prefix = DefaultHasher::new();
        for key in keys.iter().take(baseline.origin) {
            key.hash(&mut prefix);
        }
        let (previous, current) = if baseline.origin <= keys.len()
            && prefix.finish() == baseline.prefix
            && document
                .lines
                .get(baseline.origin)
                .map_or(0, |line| line.start)
                == baseline.origin_offset
        {
            if baseline.origin == 0 {
                // A unique surviving line also aligns leading duplicate
                // rows. If only repeated rows survive, retain uncertainty.
                if let Some((previous, current)) = retained_alignment(&baseline.lines, &keys) {
                    if current != 0 {
                        return None;
                    }
                    (previous, current)
                } else if keys.first() != baseline.lines.first()
                    && keys.first().is_some_and(|key| baseline.lines.contains(key))
                {
                    return None;
                } else {
                    (0, 0)
                }
            } else {
                (0, baseline.origin)
            }
        } else {
            let alignment = retained_alignment(&baseline.lines, &keys)?;
            if alignment.1 > baseline.origin {
                return None;
            }
            alignment
        };
        for (index, key) in keys.iter().enumerate().skip(current) {
            let old = baseline.lines.get(previous + index - current);
            if old != Some(key) && !lines[index].1.trim().is_empty() {
                return Some(lines[index].0.start);
            }
        }
        None
    }
}

fn unique_positions(lines: &[LineKey]) -> HashMap<LineKey, Option<usize>> {
    let mut positions = HashMap::new();
    for (index, key) in lines.iter().enumerate() {
        positions
            .entry(*key)
            .and_modify(|position| *position = None)
            .or_insert(Some(index));
    }
    positions
}

fn retained_alignment(previous: &[LineKey], current: &[LineKey]) -> Option<(usize, usize)> {
    // One unique retained line is enough to align a trimmed suffix. Repeated
    // lines alone are deliberately inconclusive; no quadratic diff is needed.
    let old_positions = unique_positions(previous);
    let positions = unique_positions(current);
    for (old, key) in previous.iter().enumerate() {
        if let Some(Some(new)) = positions.get(key) {
            if old_positions.get(key) != Some(&Some(old)) {
                continue;
            }
            // Insertion of unrelated text before the retained window has no
            // reliable origin. A caller's live-page fallback is more useful.
            if *new >= old {
                return Some((0, *new - old));
            }
            return Some((old - *new, 0));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    fn document(text: &str) -> ReadingDocument {
        let mut terminal = TerminalModel::new(120, 8);
        terminal.push_bytes(text.as_bytes());
        terminal.reading_document()
    }

    fn line_offset(document: &ReadingDocument, needle: &str) -> u32 {
        document
            .lines
            .iter()
            .zip(document.text.split("\r\n"))
            .find(|(_, text)| text.contains(needle))
            .unwrap()
            .0
            .start
    }

    fn assert_valid_line_start(document: &ReadingDocument, offset: u32) {
        assert!(document.lines.iter().any(|line| line.start == offset));
        let units: Vec<_> = document.text.encode_utf16().collect();
        assert!(offset as usize <= units.len());
        assert!(String::from_utf16(&units[..offset as usize]).is_ok());
        assert!(String::from_utf16(&units[offset as usize..]).is_ok());
        assert!(document
            .cell_at(offset)
            .is_none_or(|cell| cell.start == offset));
    }

    #[test]
    fn absent_submission_and_unchanged_text_have_no_anchor() {
        let mut document = document("old output\r\n> visible input");
        let mut anchor = ReplyAnchor::default();
        assert_eq!(anchor.offset(&document), None);
        anchor.submitted(&document);
        assert_eq!(anchor.offset(&document), None);
        document.revision += 100;
        assert_eq!(anchor.offset(&document), None);
    }

    #[test]
    fn progressive_output_keeps_the_first_changed_logical_line() {
        let mut terminal = TerminalModel::new(120, 8);
        terminal.push_bytes("old 🙂\r\n> visible input".as_bytes());
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        terminal.push_bytes("\r\nЖ😀".as_bytes());
        let first = terminal.reading_document();
        let expected = line_offset(&first, "Ж😀");
        assert_eq!(anchor.offset(&first), Some(expected));
        assert_valid_line_start(&first, expected);
        terminal.push_bytes("e\u{301} more\r\nlater line".as_bytes());
        let next = terminal.reading_document();
        assert_eq!(anchor.offset(&next), Some(expected));
        assert_eq!(anchor.offset(&next), Some(expected));
        assert_valid_line_start(&next, expected);
    }

    #[test]
    fn inline_output_and_soft_wrap_return_the_whole_logical_line() {
        let mut terminal = TerminalModel::new(4, 4);
        terminal.push_bytes("A🙂e\u{301}".as_bytes());
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        terminal.push_bytes("界 more text".as_bytes());
        let next = terminal.reading_document();
        assert_eq!(next.lines.len(), 1);
        assert_eq!(anchor.offset(&next), Some(0));
        assert_valid_line_start(&next, 0);
    }

    #[test]
    fn soft_wrap_resize_and_cursor_motion_do_not_create_output() {
        let mut terminal = TerminalModel::new(4, 3);
        terminal.push_bytes(b"abcdefghij\r\nlast");
        let first = terminal.reading_document();
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&first);
        terminal.resize(8, 3);
        terminal.push_bytes(b"\x1b[H");
        let next = terminal.reading_document();
        assert_eq!(first.text, next.text);
        assert_eq!(anchor.offset(&next), None);
    }

    #[test]
    fn overwrite_finds_the_first_changed_line_without_reply_semantics() {
        let mut terminal = TerminalModel::new(40, 4);
        terminal.push_bytes("old header\r\nworking e\u{301}🙂\r\nunchanged".as_bytes());
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        terminal.push_bytes("\x1b[2;1Hchanged 界\x1b[K".as_bytes());
        let next = terminal.reading_document();
        let expected = line_offset(&next, "changed");
        assert_eq!(anchor.offset(&next), Some(expected));
        assert_valid_line_start(&next, expected);
    }

    #[test]
    fn clear_alone_is_empty_and_redraw_exposes_its_first_nonblank_line() {
        let mut terminal = TerminalModel::new(40, 4);
        terminal.push_bytes(b"old output\r\nold input");
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        terminal.push_bytes(b"\x1b[2J\x1b[H\x1b[3J");
        assert_eq!(anchor.offset(&terminal.reading_document()), None);
        terminal.push_bytes("\r\nnew Ж🙂e\u{301}".as_bytes());
        let next = terminal.reading_document();
        let expected = line_offset(&next, "new Ж");
        assert_eq!(anchor.offset(&next), Some(expected));
        assert_valid_line_start(&next, expected);
    }

    #[test]
    fn blank_progress_and_removing_text_alone_do_not_create_an_anchor() {
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&document("header\r\nsecond"));
        assert_eq!(anchor.offset(&document("header")), None);
        anchor.submitted(&document("header"));
        assert_eq!(anchor.offset(&document("header\r\n\r\n")), None);
        assert_eq!(anchor.offset(&document("header\r\n\r\ntext")), Some(10));
    }

    #[test]
    fn trimmed_leading_history_is_not_mistaken_for_new_output() {
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&document("old first\r\nretained second\r\nretained third"));
        let trimmed = document("retained second\r\nretained third");
        assert_eq!(anchor.offset(&trimmed), None);
        let next = document("retained second\r\nretained third\r\nnew output");
        assert_eq!(anchor.offset(&next), Some(line_offset(&next, "new output")));
    }

    #[test]
    fn bounded_baseline_aligns_a_full_retention_window() {
        let mut terminal = TerminalModel::new(32, 3);
        let history: String = (0..10_010)
            .map(|row| format!("old row {row}\r\n"))
            .collect();
        terminal.push_bytes(history.as_bytes());
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        let baseline = anchor.baseline.as_ref().unwrap();
        assert_eq!(baseline.lines.len(), MAX_BASELINE_LINES);
        assert!(baseline.origin > 0);
        assert!(baseline.origin_offset > 0);
        terminal.push_bytes("new output Ж🙂e\u{301}\r\nlast".as_bytes());
        let next = terminal.reading_document();
        let expected = line_offset(&next, "new output");
        assert_eq!(anchor.offset(&next), Some(expected));
        assert_valid_line_start(&next, expected);
    }

    #[test]
    fn ambiguous_repeated_retained_lines_use_the_callers_fallback() {
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&document("removed first\r\nrepeat\r\nrepeat"));
        assert_eq!(anchor.offset(&document("repeat\r\nrepeat\r\nnew")), None);
        let repeated: String = (0..MAX_BASELINE_LINES + 20).map(|_| "repeat\r\n").collect();
        anchor.submitted(&document(&repeated));
        let unrelated = document("unrelated reset\r\nnew output");
        assert_eq!(anchor.offset(&unrelated), None);
    }

    #[test]
    fn alternate_screen_transitions_do_not_compare_unrelated_histories() {
        let mut terminal = TerminalModel::new(40, 4);
        terminal.push_bytes(b"main old");
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&terminal.reading_document());
        terminal.push_bytes(b"\x1b[?1049h");
        assert_eq!(anchor.offset(&terminal.reading_document()), None);
        terminal.push_bytes("\r\nalternate 界🙂".as_bytes());
        let alternate = terminal.reading_document();
        assert_eq!(
            anchor.offset(&alternate),
            Some(line_offset(&alternate, "alternate"))
        );
        anchor.submitted(&alternate);
        terminal.push_bytes(b"\x1b[?1049l");
        assert_eq!(anchor.offset(&terminal.reading_document()), Some(0));
    }

    #[test]
    fn another_submission_replaces_the_baseline() {
        let mut anchor = ReplyAnchor::default();
        anchor.submitted(&document("first submission"));
        let next = document("first submission\r\nfirst output\r\nnext input");
        assert_eq!(
            anchor.offset(&next),
            Some(line_offset(&next, "first output"))
        );
        anchor.submitted(&next);
        assert_eq!(anchor.offset(&next), None);
        let last = document("first submission\r\nfirst output\r\nnext input\r\nsecond output");
        assert_eq!(
            anchor.offset(&last),
            Some(line_offset(&last, "second output"))
        );
    }
}
