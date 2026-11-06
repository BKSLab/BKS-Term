//! Accepted input is evidence for a request even before the next VT repaint.
//! This bounded hint is never displayed and never used to send input to a TUI.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bks_terminal::ReadingDocument;

use crate::tui_projection::Composer;

const MAX_UNITS: usize = 32_768;
const MAX_CHECKPOINTS: usize = 256;
const QUEUE_FOR: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Submit,
    Queue,
    Steer,
}

pub struct AcceptedRequest {
    pub text: String,
    pub baseline: Arc<ReadingDocument>,
    pub kind: RequestKind,
}

struct QueueIntent {
    text: Vec<u16>,
    started: Instant,
    baseline: Option<Arc<ReadingDocument>>,
}

#[derive(Clone, PartialEq, Eq)]
struct Source {
    text: Vec<u16>,
    caret: Option<usize>,
}

#[derive(Clone)]
struct Prediction {
    text: Vec<u16>,
    caret: usize,
}

#[derive(Default)]
pub struct Submission {
    adapter: crate::tui_adapters::AdapterKind,
    source_alternate: Option<bool>,
    queue: Option<QueueIntent>,
    submitted_baseline: Option<Arc<ReadingDocument>>,
    source: Option<Source>,
    predicted: Option<Prediction>,
    checkpoints: VecDeque<Vec<u16>>,
    submitted: bool,
    unacknowledged_edit: bool,
}

impl Submission {
    pub fn observe(&mut self, composer: Option<&Composer>, menu_open: bool) {
        self.observe_at(composer, menu_open, Instant::now());
    }

    pub fn observe_at(&mut self, composer: Option<&Composer>, menu_open: bool, now: Instant) {
        let queue = self.queue.take().filter(|intent| {
            !menu_open
                && now.saturating_duration_since(intent.started) <= QUEUE_FOR
                && composer.is_none_or(|input| {
                    !input.protected
                        && input.accepts_requests()
                        && input.adapter() == crate::tui_adapters::AdapterKind::OpenCode
                        && input.text.encode_utf16().eq(intent.text.iter().copied())
                })
        });
        let Some(composer) = composer.filter(|composer| !composer.protected && !menu_open) else {
            *self = Self::default();
            self.queue = queue;
            return;
        };
        if self.adapter != composer.adapter()
            || (self.adapter == crate::tui_adapters::AdapterKind::Codex
                && self
                    .source_alternate
                    .is_some_and(|alternate| alternate != composer.uses_alternate_screen()))
        {
            *self = Self::default();
            self.adapter = composer.adapter();
        }
        self.source_alternate = Some(composer.uses_alternate_screen());
        if !composer.accepts_requests() {
            // A contradictory source repaint cannot confirm the literal, but
            // it must not erase an already bounded user-input intent either.
            // Only Codex's later full fresh echo may confirm that private hint.
            if self.adapter == crate::tui_adapters::AdapterKind::Codex
                && !composer.text.is_empty()
                && self.predicted.is_some()
            {
                self.queue = queue;
                return;
            }
            *self = Self::default();
            self.queue = queue;
            return;
        }
        self.observe_input(
            composer.request_text(),
            composer.request_caret(),
            composer.protected,
            menu_open,
        );
        self.queue = queue;
    }

    fn observe_input(&mut self, text: &str, caret: Option<u32>, protected: bool, menu_open: bool) {
        if protected || menu_open {
            *self = Self::default();
            return;
        }
        let text: Vec<_> = text.encode_utf16().take(MAX_UNITS + 1).collect();
        if text.len() > MAX_UNITS {
            *self = Self::default();
            return;
        }
        let source = Source {
            caret: caret.map(|caret| (caret as usize).min(text.len())),
            text,
        };
        if self.source.as_ref() == Some(&source) {
            return;
        }
        if self.adapter == crate::tui_adapters::AdapterKind::Codex
            && source.text.is_empty()
            && self.source.as_ref().is_some_and(|old| !old.text.is_empty())
        {
            self.checkpoints.clear();
            self.predicted = prediction(&source);
            self.unacknowledged_edit = false;
            self.submitted = false;
            self.source = Some(source);
            return;
        }
        if (!self.checkpoints.is_empty()
            || (self.adapter == crate::tui_adapters::AdapterKind::Codex
                && self.predicted.is_some()))
            && self
                .source
                .as_ref()
                .is_some_and(|old| old.text == source.text)
        {
            // A blinking/hidden caret is not an acknowledgement of the suffix.
            // Codex's private literal caret also survives a source correction;
            // an explicit navigation key invalidates that hint in accepted().
            self.source = Some(source);
            return;
        }
        if let Some(index) = self
            .checkpoints
            .iter()
            .position(|text| *text == source.text)
        {
            self.checkpoints.drain(..=index);
            if self.checkpoints.is_empty()
                && !(self.adapter == crate::tui_adapters::AdapterKind::Codex
                    && self
                        .predicted
                        .as_ref()
                        .is_some_and(|input| input.text == source.text))
            {
                self.predicted = prediction(&source);
            }
        } else {
            // ConPTY can redraw a combining mark at a wrong cursor column.
            // Keep Codex's bounded literal intent private instead of replacing
            // it with altered VT text. Only the later full fresh request echo
            // can confirm it; navigation/unknown controls clear checkpoints.
            if self.adapter == crate::tui_adapters::AdapterKind::Codex
                && !self.checkpoints.is_empty()
            {
                self.submitted = false;
                self.unacknowledged_edit = true;
                self.source = Some(source);
                return;
            }
            // A source-side transformation or untracked edit invalidates the hint.
            self.checkpoints.clear();
            self.predicted = prediction(&source);
        }
        self.submitted = false;
        self.unacknowledged_edit = !self.checkpoints.is_empty();
        self.source = Some(source);
    }

    /// Call only after the PTY accepted the bytes. Unknown edits wait for a new
    /// acknowledged composer instead of inventing what the application accepted.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn accepted(&mut self, bytes: &[u8]) -> Option<String> {
        self.accepted_at(bytes, Instant::now())
    }

    pub fn accepted_at(&mut self, bytes: &[u8], now: Instant) -> Option<String> {
        self.accepted_input(bytes, now, false)
    }

    fn accepted_input(&mut self, bytes: &[u8], now: Instant, active_turn: bool) -> Option<String> {
        self.submitted_baseline = None;
        if let Some(intent) = self.queue.take() {
            if bytes == b"\r" && now.saturating_duration_since(intent.started) <= QUEUE_FOR {
                self.submitted = true;
                self.predicted = None;
                self.checkpoints.clear();
                self.submitted_baseline = intent.baseline;
                return String::from_utf16(&intent.text)
                    .ok()
                    .filter(|text| !text.trim().is_empty() && !text.trim_start().starts_with('/'));
            }
            self.predicted = None;
            self.checkpoints.clear();
            self.unacknowledged_edit = true;
        }
        // OpenCode's standard Ctrl+X leader does not edit the source prompt.
        // Enter can queue it; TurnLog still requires its fresh source echo
        // before dividing an active response. Menus/protected/shell input have
        // already cleared this state in observe(). Other controls invalidate it.
        if self.adapter == crate::tui_adapters::AdapterKind::OpenCode && bytes == b"\x18" {
            self.queue = self.source.as_ref().map(|source| QueueIntent {
                text: source.text.clone(),
                started: now,
                baseline: None,
            });
            return None;
        }
        if self.adapter == crate::tui_adapters::AdapterKind::Codex
            && crate::codex::keeps_submission_hint(bytes)
        {
            return None;
        }
        let codex_queue = self.adapter == crate::tui_adapters::AdapterKind::Codex
            && active_turn
            && bytes == b"\t";
        if bytes == b"\r" || codex_queue {
            if self.submitted {
                return None;
            }
            self.submitted = true;
            self.checkpoints.clear();
            let text = self.predicted.take().map(|input| input.text).or_else(|| {
                (!self.unacknowledged_edit)
                    .then(|| self.source.as_ref().map(|source| source.text.clone()))
                    .flatten()
            });
            return text.and_then(|units| {
                let text = String::from_utf16(&units).ok()?;
                (!text.trim().is_empty() && !text.trim_start().starts_with('/')).then_some(text)
            });
        }
        let codex_literal = (self.adapter == crate::tui_adapters::AdapterKind::Codex)
            .then(|| {
                bytes
                    .strip_prefix(b"\x1b[200~")
                    .and_then(|payload| payload.strip_suffix(b"\x1b[201~"))
                    .and_then(|payload| std::str::from_utf8(payload).ok())
                    .filter(|text| {
                        text.chars()
                            .all(|ch| !ch.is_control() || matches!(ch, '\r' | '\n'))
                    })
                    .map(|text| {
                        text.replace("\r\n", "\n")
                            .replace('\r', "\n")
                            .replace('\n', "\r\n")
                    })
                    .or_else(|| crate::codex::newline_key(bytes).then(|| "\r\n".to_owned()))
            })
            .flatten();
        let printable = codex_literal.as_deref().or_else(|| {
            std::str::from_utf8(bytes)
                .ok()
                .filter(|text| !text.is_empty() && text.chars().all(|ch| !ch.is_control()))
        });
        self.unacknowledged_edit = true;
        if let Some(text) = printable {
            // An empty, confirmed composer can accept the next fast request
            // even if both Enter repaints were coalesced into an empty field.
            if self.submitted
                && self
                    .source
                    .as_ref()
                    .is_some_and(|source| source.text.is_empty())
            {
                self.predicted = self.source.as_ref().and_then(prediction);
                self.submitted = false;
            }
            if let Some(input) = &mut self.predicted {
                let units: Vec<_> = text.encode_utf16().collect();
                if input.text.len() + units.len() <= MAX_UNITS
                    && self.checkpoints.len() < MAX_CHECKPOINTS
                {
                    let count = units.len();
                    input.text.splice(input.caret..input.caret, units);
                    input.caret += count;
                    self.checkpoints.push_back(input.text.clone());
                    return None;
                }
            }
        }
        self.predicted = None;
        self.checkpoints.clear();
        None
    }

    /// Retain the source-projected document from before OpenCode's leader
    /// hides its editor. Its transient raw input cannot become an earlier
    /// semantic echo and prevent later recognition of the queued prompt.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn accepted_with_document(
        &mut self,
        bytes: &[u8],
        now: Instant,
        document: Arc<ReadingDocument>,
    ) -> Option<(String, Arc<ReadingDocument>)> {
        self.accepted_request(bytes, now, document, false)
            .map(|request| (request.text, request.baseline))
    }

    /// Queue/steer identity is supplied by an accepted user key while the
    /// owning session has an active turn. It never infers a key from output.
    /// TurnLog must still prove the fresh source echo before dividing turns.
    pub fn accepted_request(
        &mut self,
        bytes: &[u8],
        now: Instant,
        document: Arc<ReadingDocument>,
        active_turn: bool,
    ) -> Option<AcceptedRequest> {
        let kind = match (self.adapter, active_turn, bytes) {
            (crate::tui_adapters::AdapterKind::Codex, true, b"\t") => RequestKind::Queue,
            (crate::tui_adapters::AdapterKind::Codex, true, b"\r") => RequestKind::Steer,
            _ => RequestKind::Submit,
        };
        let request = self.accepted_input(bytes, now, active_turn);
        if bytes == b"\x18" {
            if let Some(intent) = &mut self.queue {
                intent.baseline = Some(Arc::clone(&document));
            }
        }
        let baseline = self.submitted_baseline.take().unwrap_or(document);
        request.map(|text| AcceptedRequest {
            text,
            baseline,
            kind,
        })
    }
}

fn prediction(source: &Source) -> Option<Prediction> {
    let caret = source.caret?;
    // A cursor between a surrogate pair is not a valid insertion position.
    if caret > 0
        && caret < source.text.len()
        && (0xd800..=0xdbff).contains(&source.text[caret - 1])
        && (0xdc00..=0xdfff).contains(&source.text[caret])
    {
        return None;
    }
    Some(Prediction {
        text: source.text.clone(),
        caret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_ignored_modified_enter_does_not_discard_the_next_plain_submission() {
        let mut hint = Submission {
            adapter: crate::tui_adapters::AdapterKind::Codex,
            ..Default::default()
        };
        hint.observe_input("current draft", Some(13), false, false);
        let modified = crate::codex::win32_key(13, 28, 13, false, false, true, false);
        assert!(hint.accepted(&modified).is_none());
        hint.observe_input("current draft", Some(13), false, false);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("current draft"));
        assert!(hint.accepted(b"\r").is_none());

        let mut rebound = Submission {
            adapter: crate::tui_adapters::AdapterKind::Codex,
            ..Default::default()
        };
        rebound.observe_input("before", Some(6), false, false);
        assert!(rebound.accepted(&modified).is_none());
        rebound.observe_input("after", Some(5), false, false);
        assert_eq!(rebound.accepted(b"\r").as_deref(), Some("after"));
    }

    #[test]
    fn codex_renderer_disagreement_keeps_only_the_original_literal_submission_hint() {
        let mut hint = Submission {
            adapter: crate::tui_adapters::AdapterKind::Codex,
            ..Default::default()
        };
        hint.observe_input("abc", Some(3), false, false);
        assert!(hint.accepted(b" ").is_none());
        assert!(hint.accepted(b"[").is_none());
        hint.observe_input("abc[", Some(4), false, false);
        assert!(hint.accepted(b"]").is_none());
        hint.observe_input("abc[]", Some(5), false, false);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("abc []"));
        hint.observe_input("", Some(0), false, false);
        assert!(hint.accepted(b"next").is_none());
        hint.observe_input("next", Some(4), false, false);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("next"));
    }

    fn observe(hint: &mut Submission, text: &str, caret: u32) {
        hint.observe_input(text, Some(caret), false, false);
    }

    #[test]
    fn opencode_queue_keeps_the_pre_leader_baseline_through_hidden_editor_frames() {
        let now = Instant::now();
        let mut hint = Submission::default();
        observe(&mut hint, "question Ж🙂", 12);
        hint.adapter = crate::tui_adapters::AdapterKind::OpenCode;
        let mut terminal = bks_terminal::TerminalModel::new(80, 24);
        terminal.push_bytes(b"prior answer");
        let baseline = Arc::new(terminal.reading_document());
        assert!(hint
            .accepted_with_document(b"\x18", now, Arc::clone(&baseline))
            .is_none());
        hint.observe_at(None, false, now + Duration::from_secs(1));
        terminal.push_bytes("\r\neditable question Ж🙂".as_bytes());
        let (text, actual) = hint
            .accepted_with_document(
                b"\r",
                now + Duration::from_secs(1),
                Arc::new(terminal.reading_document()),
            )
            .unwrap();
        assert_eq!(text, "question Ж🙂");
        assert!(Arc::ptr_eq(&baseline, &actual));
        assert!(hint
            .accepted_at(b"\r", now + Duration::from_secs(1))
            .is_none());
    }

    #[test]
    fn queue_intent_expires_and_menus_or_other_leader_actions_cancel_it() {
        for case in ["expiry", "menu", "other", "other adapter"] {
            let now = Instant::now();
            let mut hint = Submission::default();
            observe(&mut hint, "question", 8);
            hint.adapter = if case == "other adapter" {
                crate::tui_adapters::AdapterKind::Kimi
            } else {
                crate::tui_adapters::AdapterKind::OpenCode
            };
            assert!(hint.accepted_at(b"\x18", now).is_none());
            if case == "other" {
                assert!(hint.accepted_at(b"m", now).is_none());
            }
            let later = now
                + if case == "expiry" {
                    QUEUE_FOR + Duration::from_millis(1)
                } else {
                    Duration::from_millis(100)
                };
            hint.observe_at(None, case == "menu", later);
            assert!(hint.accepted_at(b"\r", later).is_none(), "{case}");
        }
    }

    #[test]
    fn fast_request_before_echo_uses_only_accepted_input() {
        let mut hint = Submission::default();
        observe(&mut hint, "", 0);
        hint.accepted(b"q");
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("q"));
        assert!(hint.accepted(b"\r").is_none());
        hint.accepted(b"q");
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("q"));
    }

    #[test]
    fn partial_echo_does_not_lose_the_unacknowledged_suffix() {
        let mut hint = Submission::default();
        observe(&mut hint, "", 0);
        hint.accepted("Ж".as_bytes());
        hint.accepted("🙂".as_bytes());
        hint.accepted(b"x");
        observe(&mut hint, "Ж", 1);
        hint.observe_input("Ж", None, false, false);
        observe(&mut hint, "Ж🙂", 3);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("Ж🙂x"));
    }

    #[test]
    fn source_transform_and_unknown_edit_require_acknowledgement() {
        let mut hint = Submission::default();
        observe(&mut hint, "ab", 1);
        hint.accepted(b"z");
        observe(&mut hint, "aZb", 2);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("aZb"));
        observe(&mut hint, "abc", 3);
        hint.accepted(b"\x1b[D");
        assert!(hint.accepted(b"\r").is_none());
        observe(&mut hint, "abc", 2);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("abc"));
    }

    #[test]
    fn acknowledged_text_can_be_submitted_with_a_hidden_caret() {
        let mut hint = Submission::default();
        observe(&mut hint, "q", 1);
        hint.observe_input("q", None, false, false);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("q"));
        hint.observe_input("known text", None, false, false);
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("known text"));
        hint.observe_input("old", None, false, false);
        hint.accepted(b"x");
        assert!(hint.accepted(b"\r").is_none());
    }

    #[test]
    fn menus_slash_commands_protected_and_absent_input_are_not_requests() {
        for (protected, menu, present) in [
            (true, false, true),
            (false, true, true),
            (false, false, false),
        ] {
            let mut hint = Submission::default();
            if present {
                hint.observe_input("secret", Some(6), protected, menu);
            } else {
                hint.observe(None, menu);
            }
            hint.accepted(b"q");
            assert!(hint.accepted(b"\r").is_none());
        }
        let mut hint = Submission::default();
        observe(&mut hint, "", 0);
        hint.accepted(b"/model");
        assert!(hint.accepted(b"\r").is_none());
        hint.observe_input("", Some(0), false, true);
        observe(&mut hint, "", 0);
        hint.accepted(b"q");
        assert_eq!(hint.accepted(b"\r").as_deref(), Some("q"));
    }
}
