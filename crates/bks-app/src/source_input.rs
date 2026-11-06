//! Bounded evidence of explicit accepted input. A projector still has to prove
//! the exact new VT prefix; pending bytes never become displayed text.

use std::time::{Duration, Instant};

use crate::tui_projection::{
    AcceptedEditEvidence, AcceptedEditKind, AcceptedSourceEvidence, Composer,
};
use bks_terminal::TerminalSnapshot;

const MAX_CHARS: usize = 64;
const MAX_VERIFIED_PASTE_CHARS: usize = 32_768;
const MAX_AGE: Duration = Duration::from_millis(750);
// Native Codex processes a large Windows paste over several source redraws.
// Keep its literal intent private and bounded until those glyphs arrive.
const MAX_CODEX_PASTE_AGE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, Eq)]
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

/// A native selection is a user's edit request, not a selection in the CLI.
/// Move its real cursor and delete only after each preceding key is confirmed.
/// In particular, UTF-16 units are never converted to a count of terminal keys:
/// the source may move/delete a scalar, a whole glyph, or a CRLF at a time.
pub struct NativeSelectionEdit {
    source: Source,
    text: String,
    caret: u32,
    start: u32,
    end: u32,
    deleting: bool,
    sent_revision: u64,
    started: Instant,
    queued: Vec<Vec<u8>>,
    queued_bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectionProgress {
    Waiting,
    Send(Vec<u8>),
    Complete(Vec<Vec<u8>>),
    Cancelled,
}

impl NativeSelectionEdit {
    pub fn begin(
        frame: &TerminalSnapshot,
        revision: u64,
        input: &Composer,
        native_text: &str,
        selection: (u32, u32),
        bytes: &[u8],
        now: Instant,
    ) -> Option<Self> {
        let deleting = matches!(bytes, b"\x7f" | b"\x1b[3~");
        let payload = bytes
            .strip_prefix(b"\x1b[200~")
            .and_then(|payload| payload.strip_suffix(b"\x1b[201~"));
        let replacement = std::str::from_utf8(payload.unwrap_or(bytes))
            .ok()
            .is_some_and(|text| {
                !text.is_empty()
                    && text.chars().all(|ch| {
                        !ch.is_control() || (payload.is_some() && matches!(ch, '\r' | '\n'))
                    })
            });
        let caret = input.caret?;
        let (start, end) = (selection.0.min(selection.1), selection.0.max(selection.1));
        if (!deleting && !replacement)
            || input.protected
            || input.text != native_text
            || !input.matches_source(frame)
            || frame.cursor.is_none()
            || frame.display_offset != 0
            || start == end
            || input.text.encode_utf16().count() > 32_768
            || !selection_boundary(frame, &input.text, start)
            || !selection_boundary(frame, &input.text, end)
            || utf16_byte(&input.text, caret).is_none()
        {
            return None;
        }
        let mut edit = Self {
            source: Source::of(frame),
            text: input.text.clone(),
            caret,
            start,
            end,
            deleting: false,
            sent_revision: revision,
            started: now,
            queued: Vec::new(),
            queued_bytes: 0,
        };
        if !deleting && !edit.queue(bytes) {
            return None;
        }
        Some(edit)
    }

    /// Retain actual user events in order while the source cursor catches up.
    /// They are never reconstructed from screen contents or replayed on cancel.
    pub fn queue(&mut self, bytes: &[u8]) -> bool {
        if self.queued.len() >= 256 || self.queued_bytes + bytes.len() > 32_768 {
            return false;
        }
        self.queued.push(bytes.to_vec());
        self.queued_bytes += bytes.len();
        true
    }

    pub fn first_key(&mut self, application_cursor: bool) -> Vec<u8> {
        self.next_key(application_cursor)
    }

    pub fn observe(
        &mut self,
        frame: &TerminalSnapshot,
        revision: u64,
        input: Option<&Composer>,
        application_cursor: bool,
        now: Instant,
    ) -> SelectionProgress {
        if Source::of(frame) != self.source
            || frame.display_offset != 0
            || now.saturating_duration_since(self.started) > MAX_AGE
            || input.is_some_and(|input| input.protected || !input.matches_source(frame))
        {
            return SelectionProgress::Cancelled;
        }
        let Some(input) = input else {
            return SelectionProgress::Waiting;
        };
        let Some(caret) = input.caret.filter(|_| frame.cursor.is_some()) else {
            return SelectionProgress::Waiting;
        };
        if revision == self.sent_revision || (input.text == self.text && caret == self.caret) {
            return SelectionProgress::Waiting;
        }
        let confirmed = if self.deleting {
            // Different line editors delete different sized Unicode glyphs.
            // Accept only a deletion wholly inside the requested selection,
            // with the untouched prefix/suffix and new caret proven by VT.
            (self.start..self.caret).contains(&caret)
                && utf16_byte(&self.text, caret)
                    .zip(utf16_byte(&self.text, self.caret))
                    .is_some_and(|(start, end)| {
                        input.text == format!("{}{}", &self.text[..start], &self.text[end..])
                    })
        } else {
            input.text == self.text
                && utf16_byte(&input.text, caret).is_some()
                && if self.caret < self.end {
                    (self.caret + 1..=self.end).contains(&caret)
                } else {
                    (self.end..self.caret).contains(&caret)
                }
        };
        if !confirmed {
            return SelectionProgress::Cancelled;
        }
        self.text.clone_from(&input.text);
        self.caret = caret;
        self.sent_revision = revision;
        self.started = now;
        if self.deleting && caret == self.start {
            return SelectionProgress::Complete(std::mem::take(&mut self.queued));
        }
        SelectionProgress::Send(self.next_key(application_cursor))
    }

    fn next_key(&mut self, application_cursor: bool) -> Vec<u8> {
        use crate::terminal_input::{encode_key, SpecialKey};
        if !self.deleting && self.caret != self.end {
            encode_key(
                if self.caret < self.end {
                    SpecialKey::Right
                } else {
                    SpecialKey::Left
                },
                false,
                false,
                false,
                application_cursor,
            )
        } else {
            self.deleting = true;
            encode_key(SpecialKey::Backspace, false, false, false, false)
        }
    }
}

fn utf16_byte(text: &str, offset: u32) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            // A canonical CRLF is one source line break, never two keys.
            return (!(ch == '\n' && text[..byte].ends_with('\r'))).then_some(byte);
        }
        units += ch.len_utf16() as u32;
    }
    (units == offset).then_some(text.len())
}

fn selection_boundary(frame: &TerminalSnapshot, text: &str, offset: u32) -> bool {
    let Some(byte) = utf16_byte(text, offset) else {
        return false;
    };
    let before = text[..byte].chars().next_back();
    let after = text[byte..].chars().next();
    if before == Some('\u{200d}')
        || after.is_some_and(|ch| {
            matches!(ch, '\u{200d}' | '\u{fe0e}' | '\u{fe0f}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}')
        })
        || (before.is_some_and(|ch| matches!(ch, '\u{1f1e6}'..='\u{1f1ff}'))
            && after.is_some_and(|ch| matches!(ch, '\u{1f1e6}'..='\u{1f1ff}')))
    {
        return false;
    }
    // VT cells retain base + combining marks as a unit. A native offset inside
    // one of these rendered glyphs cannot safely be turned into source keys.
    !frame.cells.iter().any(|cell| {
        cell.text.chars().nth(1).is_some()
            && text
                .match_indices(&cell.text)
                .any(|(start, glyph)| start < byte && byte < start + glyph.len())
    })
}

struct Pending {
    source: Source,
    evidence: AcceptedSourceEvidence,
    started: Instant,
    // Only an accepted input's baseline, never a public/displayed composer.
    // ConPTY may publish a hidden-cursor redraw before its final cursor tail.
    composer: Option<Composer>,
    max_chars: usize,
    max_age: Duration,
}

struct PendingEdit {
    source: Source,
    revision: u64,
    evidence: AcceptedEditEvidence,
    composer: Composer,
    started: Instant,
    repeats: usize,
}

#[derive(Default)]
pub struct SourceInput {
    pending: Option<Pending>,
    edit: Option<PendingEdit>,
}

impl SourceInput {
    pub fn clear(&mut self) {
        self.pending = None;
        self.edit = None;
    }

    pub fn cancel_edit(&mut self) {
        self.edit = None;
    }

    pub fn expire(&mut self, now: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now.saturating_duration_since(pending.started) > pending.max_age)
            || self
                .edit
                .as_ref()
                .is_some_and(|pending| now.saturating_duration_since(pending.started) > MAX_AGE)
        {
            self.clear();
        }
    }

    pub fn accepted(
        &mut self,
        frame: &TerminalSnapshot,
        revision: u64,
        bytes: &[u8],
        composer: Option<&Composer>,
        now: Instant,
    ) {
        self.expire(now);
        let kind = match bytes {
            b"\x7f" => Some(AcceptedEditKind::Backspace),
            b"\x1b[3~" => Some(AcceptedEditKind::Delete),
            _ => None,
        };
        if let Some(kind) = kind {
            self.pending = None;
            if frame.display_offset != 0 || composer.is_some_and(|input| input.protected) {
                self.clear();
                return;
            }
            let source = Source::of(frame);
            if let Some(pending) = self
                .edit
                .as_mut()
                .filter(|pending| pending.source == source && pending.evidence.kind() == kind)
            {
                if pending.repeats < MAX_CHARS {
                    pending.repeats += 1;
                } else {
                    self.clear();
                }
                return;
            }
            self.edit = None;
            if let Some(composer) = composer {
                if let Some(evidence) =
                    AcceptedEditEvidence::record(kind, frame, revision, composer)
                {
                    self.edit = Some(PendingEdit {
                        source,
                        revision,
                        evidence,
                        composer: composer.clone(),
                        started: now,
                        repeats: 1,
                    });
                }
            }
            return;
        }
        self.edit = None;
        let pasted = bytes
            .strip_prefix(b"\x1b[200~")
            .and_then(|payload| payload.strip_suffix(b"\x1b[201~"));
        let newline = composer.is_some_and(|input| {
            !input.protected
                && input.matches_source(frame)
                && match input.adapter() {
                    crate::tui_adapters::AdapterKind::Codex => crate::codex::newline_key(bytes),
                    crate::tui_adapters::AdapterKind::OpenCode => {
                        crate::opencode::newline_key(bytes)
                    }
                    _ => false,
                }
        });
        let max_chars = if pasted.is_some()
            && composer.is_some_and(|input| {
                matches!(
                    input.adapter(),
                    crate::tui_adapters::AdapterKind::Codex
                        | crate::tui_adapters::AdapterKind::OpenCode
                ) && !input.protected
                    && input.caret.is_some()
                    && input.matches_source(frame)
            }) {
            MAX_VERIFIED_PASTE_CHARS
        } else {
            MAX_CHARS
        };
        let text = std::str::from_utf8(if newline {
            b"\r\n"
        } else {
            pasted.unwrap_or(bytes)
        })
        .ok()
        .filter(|text| {
            !text.is_empty()
                && text.chars().take(max_chars + 1).count() <= max_chars
                && (max_chars == MAX_CHARS
                    || text
                        .encode_utf16()
                        .take(MAX_VERIFIED_PASTE_CHARS + 1)
                        .count()
                        <= MAX_VERIFIED_PASTE_CHARS)
                && text.chars().all(|ch| {
                    !ch.is_control() || ((pasted.is_some() || newline) && matches!(ch, '\r' | '\n'))
                })
        });
        let Some((text, cursor)) = text.zip(frame.cursor) else {
            self.clear();
            return;
        };
        // Complete bracketed paste is literal user input, including line
        // breaks. The reader uses CRLF; raw Enter/navigation stay excluded.
        let text = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n");
        if max_chars != MAX_CHARS
            && text
                .encode_utf16()
                .take(MAX_VERIFIED_PASTE_CHARS + 1)
                .count()
                > MAX_VERIFIED_PASTE_CHARS
        {
            self.clear();
            return;
        }
        if frame.display_offset != 0 || cursor.row >= frame.rows || cursor.column >= frame.cols {
            self.clear();
            return;
        }
        let source = Source::of(frame);
        if self.pending.as_ref().is_some_and(|pending| {
            pending.source != source
                && !pending
                    .composer
                    .as_ref()
                    .is_some_and(|input| input.permits_codex_repaint(frame))
        }) {
            self.clear();
        }
        let max_age = if pasted.is_some()
            && text.encode_utf16().count() > MAX_CHARS
            && composer
                .is_some_and(|input| input.adapter() == crate::tui_adapters::AdapterKind::Codex)
            && max_chars == MAX_VERIFIED_PASTE_CHARS
        {
            MAX_CODEX_PASTE_AGE
        } else {
            MAX_AGE
        };
        if let Some(pending) = &mut self.pending {
            let max_chars = pending.max_chars.max(max_chars);
            if pending.evidence.text.chars().count() + text.chars().count() <= max_chars
                && (max_chars == MAX_CHARS
                    || pending.evidence.text.encode_utf16().count() + text.encode_utf16().count()
                        <= MAX_VERIFIED_PASTE_CHARS)
            {
                pending.evidence.text.push_str(&text);
                pending.max_chars = max_chars;
                pending.max_age = pending.max_age.max(max_age);
            } else {
                self.clear();
            }
        } else if text.chars().count() <= max_chars {
            self.pending = Some(Pending {
                max_chars,
                max_age,
                source,
                evidence: AcceptedSourceEvidence {
                    text,
                    row: cursor.row,
                    column: cursor.column,
                    before_revision: revision,
                },
                started: now,
                composer: composer
                    .filter(|input| !input.protected && input.caret.is_some())
                    .cloned(),
            });
        }
    }

    pub fn evidence(
        &mut self,
        frame: &TerminalSnapshot,
        revision: u64,
        now: Instant,
    ) -> Option<&AcceptedSourceEvidence> {
        self.expire(now);
        if self.pending.as_ref().is_some_and(|pending| {
            pending.source != Source::of(frame)
                && !pending
                    .composer
                    .as_ref()
                    .is_some_and(|input| input.permits_codex_repaint(frame))
        }) {
            self.clear();
        }
        self.pending
            .as_ref()
            .filter(|pending| revision != pending.evidence.before_revision)
            .map(|pending| &pending.evidence)
    }

    pub fn confirmation(
        &mut self,
        frame: &TerminalSnapshot,
        revision: u64,
        now: Instant,
    ) -> Option<(&AcceptedSourceEvidence, Option<&Composer>)> {
        // Perform the same source, revision and expiry checks before exposing
        // the original proof. No pending bytes become presentation text.
        self.evidence(frame, revision, now)?;
        let pending = self.pending.as_ref()?;
        Some((&pending.evidence, pending.composer.as_ref()))
    }

    pub fn acknowledge(&mut self, input: &Composer) {
        if self
            .edit
            .as_ref()
            .is_some_and(|pending| pending.evidence.matches_result(input))
        {
            self.clear();
            return;
        }
        if let Some(pending) = self.pending.as_mut() {
            if pending.composer.is_none()
                && input.adapter() == crate::tui_adapters::AdapterKind::Codex
                && input.request_text() == pending.evidence.text
            {
                if !input.is_partial()
                    && input.caret == Some(pending.evidence.text.encode_utf16().count() as u32)
                {
                    self.clear();
                    return;
                }
                pending.composer = Some(input.clone());
            }
        }
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let Some(baseline) = pending.composer.as_ref() else {
            return;
        };
        if input.accepts_requests() != baseline.accepts_requests() {
            // '!' is a mode switch in OpenCode, not a prefix of the next
            // shell draft. Do not append later typing to that old evidence.
            self.clear();
            return;
        }
        let Some(caret) = baseline.caret else { return };
        let units: Vec<_> = baseline.text.encode_utf16().collect();
        let Some(prefix) = units.get(..caret as usize) else {
            return;
        };
        let Some(suffix) = units.get(caret as usize..) else {
            return;
        };
        let insertion: Vec<_> = pending.evidence.text.encode_utf16().collect();
        // Keep the same bounded proof after a partial acknowledgement. A
        // subsequent hidden-cursor chunk may precede the rest of a paste.
        if !input.protected
            && input.caret == Some(caret + insertion.len() as u32)
            && input
                .text
                .encode_utf16()
                .eq(prefix.iter().chain(&insertion).chain(suffix).copied())
        {
            self.clear();
        }
    }

    /// Private proof of a deletion key, retained across hidden redraws only.
    /// The projector still needs fresh visible text/caret and an exact delta.
    pub fn edit_confirmation(
        &mut self,
        frame: &TerminalSnapshot,
        revision: u64,
        now: Instant,
    ) -> Option<(&AcceptedEditEvidence, &Composer)> {
        self.expire(now);
        if self
            .edit
            .as_ref()
            .is_some_and(|pending| pending.source != Source::of(frame))
        {
            self.clear();
            return None;
        }
        self.edit
            .as_ref()
            .filter(|pending| revision > pending.revision)
            .map(|pending| (&pending.evidence, &pending.composer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    fn bounded_edit_source() -> (TerminalSnapshot, Composer) {
        let mut terminal = TerminalModel::new(40, 10);
        terminal.push_bytes(b"+--------+\r\n| > abc  |\r\n+--------+\x1b[2;8H");
        let frame = terminal.live_snapshot();
        let input = crate::tui_projection::project(&frame, &terminal.tui_reading_document(), None)
            .input
            .unwrap();
        assert_eq!(input.text, "abc");
        assert_eq!(input.caret, Some(3));
        (frame, input)
    }

    #[test]
    fn deletion_evidence_expires_from_first_key_and_repeats_need_existing_proof() {
        let now = Instant::now();
        let (frame, input) = bounded_edit_source();
        let mut source = SourceInput::default();
        source.accepted(&frame, 1, b"\x7f", None, now);
        assert!(source.edit_confirmation(&frame, 2, now).is_none());
        source.accepted(&frame, 1, b"\x7f", Some(&input), now);
        assert!(source.edit_confirmation(&frame, 1, now).is_none());
        assert!(source.edit_confirmation(&frame, 2, now).is_some());
        let mut hidden = frame.clone();
        hidden.cursor = None;
        source.accepted(&hidden, 2, b"\x7f", None, now + Duration::from_millis(700));
        assert!(source
            .edit_confirmation(&hidden, 3, now + Duration::from_millis(700))
            .is_some());
        assert!(source
            .edit_confirmation(&frame, 4, now + Duration::from_millis(751))
            .is_none());
        source.accepted(&frame, 1, b"\x7f", Some(&input), now);
        for _ in 1..MAX_CHARS {
            source.accepted(&hidden, 2, b"\x7f", None, now);
        }
        assert!(source.edit_confirmation(&frame, 3, now).is_some());
        source.accepted(&hidden, 2, b"\x7f", None, now);
        assert!(source.edit_confirmation(&frame, 3, now).is_none());
    }

    #[test]
    fn deletion_evidence_cancels_on_controls_source_change_and_explicit_reset() {
        let now = Instant::now();
        let (frame, input) = bounded_edit_source();
        for bytes in [b"\r".as_slice(), b"\x1b", b"\x08", b"\x1b[3~", b"x"] {
            let mut source = SourceInput::default();
            source.accepted(&frame, 1, b"\x7f", Some(&input), now);
            source.accepted(&frame, 1, bytes, None, now);
            assert!(
                source.edit_confirmation(&frame, 2, now).is_none(),
                "{bytes:?}"
            );
        }
        let mut source = SourceInput::default();
        source.accepted(&frame, 1, b"\x7f", Some(&input), now);
        let mut changed = frame.clone();
        changed.cols += 1;
        assert!(source.edit_confirmation(&changed, 2, now).is_none());
        assert!(source.edit_confirmation(&frame, 3, now).is_none());
        source.accepted(&frame, 1, b"\x7f", Some(&input), now);
        source.clear();
        assert!(source.edit_confirmation(&frame, 2, now).is_none());
    }

    #[test]
    fn cancel_edit_preserves_pending_slash_insertion() {
        let now = Instant::now();
        let (frame, input) = bounded_edit_source();
        let mut source = SourceInput::default();
        source.accepted(&frame, 1, b"/", Some(&input), now);
        source.cancel_edit();
        assert_eq!(source.confirmation(&frame, 2, now).unwrap().0.text, "/");
        source.accepted(&frame, 1, b"\x7f", Some(&input), now);
        assert!(source.edit_confirmation(&frame, 2, now).is_some());
        source.cancel_edit();
        assert!(source.edit_confirmation(&frame, 2, now).is_none());
    }

    fn selection_source(text: &str, caret: u32) -> (TerminalSnapshot, Composer) {
        let mut terminal = TerminalModel::new(80, 8);
        terminal.push_bytes(b"> baseline");
        let mut input = crate::tui_projection::project(
            &terminal.live_snapshot(),
            &terminal.tui_reading_document(),
            None,
        )
        .input
        .unwrap();
        terminal.push_bytes(format!("\x1b[H\x1b[J> {text}").as_bytes());
        input.text = text.into();
        input.caret = Some(caret);
        (terminal.live_snapshot(), input)
    }

    #[test]
    fn native_backspace_and_delete_move_the_source_then_delete_only_the_selection() {
        let (frame, mut input) = selection_source("abcd", 4);
        let now = Instant::now();
        for bytes in [b"\x7f".as_slice(), b"\x1b[3~"] {
            input.text = "abcd".into();
            input.caret = Some(4);
            let mut edit =
                NativeSelectionEdit::begin(&frame, 10, &input, "abcd", (1, 3), bytes, now).unwrap();
            assert_eq!(edit.first_key(false), b"\x1b[D");
            assert_eq!(
                edit.observe(&frame, 11, Some(&input), false, now),
                SelectionProgress::Waiting,
                "an unrelated source redraw must not repeat a key"
            );
            input.caret = Some(3);
            assert_eq!(
                edit.observe(&frame, 10, Some(&input), false, now),
                SelectionProgress::Waiting,
                "a new source revision is required"
            );
            assert_eq!(
                edit.observe(&frame, 11, Some(&input), false, now),
                SelectionProgress::Send(b"\x7f".to_vec())
            );
            input.text = "abd".into();
            input.caret = Some(2);
            assert_eq!(
                edit.observe(&frame, 12, Some(&input), false, now),
                SelectionProgress::Send(b"\x7f".to_vec())
            );
            input.text = "ad".into();
            input.caret = Some(1);
            assert_eq!(
                edit.observe(&frame, 13, Some(&input), false, now),
                SelectionProgress::Complete(Vec::new())
            );
        }
    }

    #[test]
    fn native_selection_uses_source_unicode_and_line_break_acknowledgements() {
        let (frame, mut input) = selection_source("AЖ🙂\r\n界Z", 8);
        let now = Instant::now();
        let mut edit =
            NativeSelectionEdit::begin(&frame, 1, &input, &input.text, (1, 7), b"\x7f", now)
                .unwrap();
        assert_eq!(edit.first_key(true), b"\x1bOD");
        for (revision, text, caret) in [
            (2, "AЖ🙂\r\n界Z", 7),
            (3, "AЖ🙂\r\nZ", 6),
            (4, "AЖ🙂Z", 4),
            (5, "AЖZ", 2),
        ] {
            input.text = text.into();
            input.caret = Some(caret);
            assert_eq!(
                edit.observe(&frame, revision, Some(&input), true, now),
                SelectionProgress::Send(b"\x7f".to_vec())
            );
        }
        input.text = "AZ".into();
        input.caret = Some(1);
        assert_eq!(
            edit.observe(&frame, 6, Some(&input), true, now),
            SelectionProgress::Complete(Vec::new())
        );
    }

    #[test]
    fn native_selection_accepts_scalar_or_glyph_steps_without_counting_utf16_units() {
        let now = Instant::now();
        for scalar_steps in [false, true] {
            let (frame, mut input) = selection_source("Ae\u{301}Z", 0);
            let mut edit =
                NativeSelectionEdit::begin(&frame, 1, &input, &input.text, (1, 3), b"\x1b[3~", now)
                    .unwrap();
            assert_eq!(edit.first_key(false), b"\x1b[C");
            input.caret = Some(1);
            assert_eq!(
                edit.observe(&frame, 2, Some(&input), false, now),
                SelectionProgress::Send(b"\x1b[C".to_vec())
            );
            if scalar_steps {
                input.caret = Some(2);
                assert_eq!(
                    edit.observe(&frame, 3, Some(&input), false, now),
                    SelectionProgress::Send(b"\x1b[C".to_vec())
                );
            }
            input.caret = Some(3);
            assert_eq!(
                edit.observe(&frame, 4, Some(&input), false, now),
                SelectionProgress::Send(b"\x7f".to_vec())
            );
            if scalar_steps {
                input.text = "AeZ".into();
                input.caret = Some(2);
                assert_eq!(
                    edit.observe(&frame, 5, Some(&input), false, now),
                    SelectionProgress::Send(b"\x7f".to_vec())
                );
            }
            input.text = "AZ".into();
            input.caret = Some(1);
            assert_eq!(
                edit.observe(&frame, 6, Some(&input), false, now),
                SelectionProgress::Complete(Vec::new())
            );
        }
    }

    #[test]
    fn native_selection_replacement_and_fast_following_events_wait_and_preserve_order() {
        let (frame, mut input) = selection_source("abZ", 2);
        let now = Instant::now();
        for replacement in ["🙂".as_bytes(), b"\x1b[200~new\r\nline\x1b[201~"] {
            input.text = "abZ".into();
            input.caret = Some(2);
            let mut edit =
                NativeSelectionEdit::begin(&frame, 1, &input, "abZ", (1, 2), replacement, now)
                    .unwrap();
            assert_eq!(edit.first_key(false), b"\x7f");
            assert!(edit.queue(b"!"));
            assert!(edit.queue(b"\r"));
            let mut hidden = frame.clone();
            hidden.cursor = None;
            input.caret = None;
            assert_eq!(
                edit.observe(&hidden, 2, Some(&input), false, now),
                SelectionProgress::Waiting
            );
            input.text = "aZ".into();
            input.caret = Some(1);
            assert_eq!(
                edit.observe(&frame, 3, Some(&input), false, now),
                SelectionProgress::Complete(vec![
                    replacement.to_vec(),
                    b"!".to_vec(),
                    b"\r".to_vec()
                ])
            );
        }
    }

    #[test]
    fn native_selection_rejects_unproven_and_partial_unicode_ranges() {
        let (mut frame, mut input) = selection_source("A🙂\r\ne\u{301}Z", 8);
        let now = Instant::now();
        for selection in [(0, 0), (0, 2), (0, 4), (5, 6), (0, 99)] {
            assert!(NativeSelectionEdit::begin(
                &frame,
                1,
                &input,
                &input.text,
                selection,
                b"\x7f",
                now
            )
            .is_none());
        }
        for bytes in [b"\r".as_slice(), b"\x1b[D", b"\x1b[200~\x03\x1b[201~"] {
            assert!(
                NativeSelectionEdit::begin(&frame, 1, &input, &input.text, (0, 3), bytes, now)
                    .is_none()
            );
        }
        for case in ["stale", "protected", "hidden", "caret", "source", "scroll"] {
            let saved = input.clone();
            let original = frame.clone();
            match case {
                "protected" => input.protected = true,
                "hidden" => frame.cursor = None,
                "caret" => input.caret = None,
                "source" => frame.cols += 1,
                "scroll" => frame.display_offset = 1,
                _ => {}
            }
            assert!(
                NativeSelectionEdit::begin(
                    &frame,
                    1,
                    &input,
                    if case == "stale" { "old" } else { &input.text },
                    (0, 3),
                    b"\x7f",
                    now
                )
                .is_none(),
                "{case}"
            );
            input = saved;
            frame = original;
        }
    }

    #[test]
    fn native_selection_cancels_on_mismatched_source_expiry_or_cursor_overshoot() {
        let (frame, input) = selection_source("abcd", 4);
        let now = Instant::now();
        for case in ["text", "caret", "source", "expiry", "protected", "prefix"] {
            let mut frame = frame.clone();
            let mut current = input.clone();
            let mut edit =
                NativeSelectionEdit::begin(&frame, 1, &current, &current.text, (1, 3), b"Q", now)
                    .unwrap();
            assert_eq!(edit.first_key(false), b"\x1b[D");
            match case {
                "text" => current.text = "other".into(),
                "caret" => current.caret = Some(2),
                "source" => frame.history_size += 1,
                "protected" => current.protected = true,
                "prefix" => {
                    current.caret = Some(3);
                    assert_eq!(
                        edit.observe(&frame, 2, Some(&current), false, now),
                        SelectionProgress::Send(b"\x7f".to_vec())
                    );
                    current.text = "xb d".into();
                    current.caret = Some(2);
                }
                _ => {}
            }
            assert_eq!(
                edit.observe(
                    &frame,
                    3,
                    Some(&current),
                    false,
                    if case == "expiry" {
                        now + MAX_AGE + Duration::from_millis(1)
                    } else {
                        now
                    }
                ),
                SelectionProgress::Cancelled,
                "{case} must discard the queued replacement"
            );
        }
    }

    #[test]
    fn partial_source_acknowledgement_keeps_the_original_baseline_until_the_full_input_returns() {
        let bytes =
            include_bytes!("../tests/fixtures/tui-codex-claude-composers/conpty-hidden-menu.vt");
        let mut terminal = TerminalModel::new(100, 30);
        terminal.push_bytes(&bytes[..3707]);
        let frame = terminal.live_snapshot();
        let baseline =
            crate::tui_projection::project(&frame, &terminal.tui_reading_document(), None)
                .input
                .unwrap();
        let now = Instant::now();
        let revision = terminal.reading_revision();
        let mut source = SourceInput::default();
        source.accepted(&frame, revision, b"/he", Some(&baseline), now);
        let mut current = baseline.clone();
        current.text = "/".into();
        current.caret = Some(1);
        source.acknowledge(&current);
        let (accepted, proof) = source.confirmation(&frame, revision + 1, now).unwrap();
        assert_eq!(accepted.text, "/he");
        assert_eq!(proof.unwrap(), &baseline);
        current.text = "/he".into();
        current.caret = None;
        source.acknowledge(&current);
        assert!(source.confirmation(&frame, revision + 2, now).is_some());
        current.caret = Some(3);
        source.acknowledge(&current);
        assert!(source.confirmation(&frame, revision + 3, now).is_none());
    }

    #[test]
    fn accepted_input_waits_for_a_new_source_revision_and_keeps_the_original_origin() {
        let frame = TerminalModel::new(40, 8).live_snapshot();
        let now = Instant::now();
        let mut input = SourceInput::default();
        input.accepted(&frame, 1, "Ж".as_bytes(), None, now);
        input.accepted(&frame, 1, b"q", None, now);
        assert!(input.evidence(&frame, 1, now).is_none());
        let evidence = input.evidence(&frame, 2, now).unwrap();
        assert_eq!(evidence.text, "Жq");
        assert_eq!((evidence.row, evidence.column), (0, 0));
        assert_eq!(evidence.before_revision, 1);
        input.clear();
        assert!(input.evidence(&frame, 2, now).is_none());
    }

    #[test]
    fn a_complete_bracketed_paste_is_literal_evidence_but_control_payloads_are_rejected() {
        let frame = TerminalModel::new(40, 8).live_snapshot();
        let now = Instant::now();
        let mut input = SourceInput::default();
        input.accepted(
            &frame,
            1,
            "\x1b[200~Ж界 tail\x1b[201~".as_bytes(),
            None,
            now,
        );
        assert_eq!(input.evidence(&frame, 2, now).unwrap().text, "Ж界 tail");
        input.clear();
        input.accepted(
            &frame,
            2,
            "\x1b[200~Ж\n界\r🙂\r\ntail\x1b[201~".as_bytes(),
            None,
            now,
        );
        assert_eq!(
            input.evidence(&frame, 3, now).unwrap().text,
            "Ж\r\n界\r\n🙂\r\ntail"
        );
        for control in [
            b"\x1b[200~draft\x1b[D\x1b[201~".as_slice(),
            b"\x1b[200~draft\tvalue\x1b[201~",
            b"draft\rnext",
            b"\x1b[200~draft\n",
        ] {
            input.accepted(&frame, 3, control, None, now);
            assert!(input.evidence(&frame, 4, now).is_none());
        }
    }

    #[test]
    fn controls_geometry_changes_expiry_and_oversized_input_discard_pending_evidence() {
        let mut frame = TerminalModel::new(40, 8).live_snapshot();
        let now = Instant::now();
        for control in [b"\r".as_slice(), b"\x1b[D", b"\x7f", b"\n"] {
            let mut input = SourceInput::default();
            input.accepted(&frame, 1, b"q", None, now);
            input.accepted(&frame, 1, control, None, now);
            assert!(input.evidence(&frame, 2, now).is_none());
        }
        let mut input = SourceInput::default();
        input.accepted(&frame, 1, b"q", None, now);
        assert!(input
            .evidence(&frame, 2, now + MAX_AGE + Duration::from_millis(1))
            .is_none());
        input.accepted(&frame, 1, b"q", None, now);
        frame.history_size += 1;
        assert!(input.evidence(&frame, 2, now).is_none());
        input.accepted(&frame, 1, "🙂".repeat(MAX_CHARS).as_bytes(), None, now);
        input.accepted(&frame, 1, b"q", None, now);
        assert!(input.evidence(&frame, 2, now).is_none());
        frame.cursor = None;
        input.accepted(&frame, 1, b"q", None, now);
        assert!(input.evidence(&frame, 2, now).is_none());
    }
}
