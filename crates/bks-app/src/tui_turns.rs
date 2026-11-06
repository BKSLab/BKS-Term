//! Conservative, session-local TUI turn evidence and a retained answer archive.
//! Visible text describes observations; it never authorizes terminal actions.

use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bks_terminal::{
    Flags, ReadingCell, ReadingDocument, ReadingLine, StyleSpan, TerminalSnapshot, TextStyle,
};

use crate::tui_projection::Composer;
use crate::tui_submission::RequestKind;
use crate::tui_transcript::ReplyAnchor;

#[path = "tui_command_output.rs"]
mod command_output;

#[path = "tui_turns_codex_activity.rs"]
mod codex_activity;

#[cfg(test)]
#[path = "tui_turns_chrome_tests.rs"]
mod chrome_tests;

#[cfg(test)]
#[path = "tui_turns_request_tests.rs"]
mod request_tests;

#[cfg(test)]
#[path = "tui_turns_codex_tests.rs"]
mod codex_tests;

const STABLE_FOR: Duration = Duration::from_millis(150);
// A VT redraw can transiently omit a footer; completion needs a longer,
// independently stable structural boundary than ordinary phase text.
const COMPLETION_STABLE_FOR: Duration = Duration::from_millis(400);
const MAX_TURNS: usize = 128;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 128 * 1024;
const TURN_OVERHEAD: usize = 512;
const MAX_SOURCE_LINES: usize = 4_096;
const MAX_SPEECH_LINES: usize = 256;
const MAX_EVENT_BYTES: usize = 8_192;
const MAX_EVENTS: usize = 128;
const MAX_QUEUED_REQUESTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhaseKind {
    Request,
    Thinking,
    Search,
    Tool,
    Answer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnEvent {
    Phase { kind: PhaseKind, text: String },
    Ready { id: u64 },
    Failed { id: u64, text: String },
    Interrupted { id: u64, text: String },
    NeedsInput { id: u64, text: String },
    NewOutput { text: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Answer,
    Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnswerTarget {
    pub id: u64,
    pub start: u32,
    pub number: usize,
    pub total: usize,
    pub complete: bool,
    pub failed: bool,
    pub kind: OutputKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestTarget {
    pub id: u64,
    pub start: u32,
    pub end: u32,
    pub expanded: bool,
}

pub struct TurnLog {
    adapter: crate::tui_adapters::AdapterKind,
    turns: VecDeque<Turn>,
    candidate: Option<Candidate>,
    queued: VecDeque<Candidate>,
    active: Option<Active>,
    next_id: u64,
    revision: u64,
    archive: Arc<ReadingDocument>,
    targets: Vec<AnswerTarget>,
    output_targets: Vec<AnswerTarget>,
    command_capture: command_output::Capture,
    requests: Vec<RequestTarget>,
    events: VecDeque<TurnEvent>,
    status: String,
    source: Option<SourceKey>,
    view: View,
    passive: SemanticAnchor,
    passive_initialized: bool,
    pending_lines: Vec<PendingLine>,
    announced: HashSet<(Option<PhaseKind>, u64)>,
    source_input: Option<SourceInputBoundary>,
}

struct Turn {
    id: u64,
    kind: OutputKind,
    request: String,
    request_expanded: bool,
    answer: RichText,
    complete: bool,
    failed: bool,
}
struct Candidate {
    input: String,
    request: String,
    anchor: SemanticAnchor,
    status_lines: HashSet<u64>,
    stale_status: HashSet<u64>,
    kind: RequestKind,
    editor: Option<(usize, usize)>,
}

struct SourceInputBoundary {
    text: String,
    first: usize,
    last: usize,
}
struct Active {
    id: u64,
    input: String,
    anchor: SemanticAnchor,
    stale_status: HashSet<u64>,
    summary_baseline: HashSet<u64>,
    panel_margin: Option<usize>,
    seen_busy: bool,
    was_busy: bool,
    busy_gone: bool,
    input_unavailable: bool,
    seen_control: bool,
    was_control: bool,
    control_gone: bool,
    control_returned: bool,
    uncertain_end_announced: bool,
    semantic: bool,
    answer_changed: Instant,
    answer_signature: Option<u64>,
    answer_origin: Option<usize>,
    answer_history: usize,
    answer_regions: HashSet<usize>,
    answer_aliases: HashSet<usize>,
    completion_since: Option<Instant>,
    terminal: Option<(SignalKind, u64)>,
    codex_tools_seen: bool,
    provisional_headers: HashSet<(usize, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceKey {
    revision: u64,
    columns: usize,
    rows: usize,
    history: usize,
    alternate: bool,
    composer: bool,
    protected: bool,
    editable: bool,
    menu: bool,
}

#[derive(Default)]
struct View {
    phases: Vec<(PhaseKind, RichText)>,
    fallback: RichText,
    footer: Option<Footer>,
    distinct_footer: bool,
    live_status: HashSet<u64>,
    answer_headers: Vec<AnswerHeader>,
    retained_headers: HashSet<usize>,
    answer_snapshot: bool,
    answer_overlap: Option<usize>,
    panel_margin: Option<usize>,
    control_busy: bool,
    codex_tool_output: bool,
}

// A turn can grow above an unchanged status/footer row. ReplyAnchor remains a
// useful history alignment, but an append-only suffix is insufficient here.
// Fingerprints are bounded and never used to infer terminal actions.
#[derive(Default)]
struct SemanticAnchor {
    reply: ReplyAnchor,
    baseline: HashSet<u64>,
    ordered: Vec<(u64, i32)>,
    boundary: Option<u64>,
    codex: bool,
}

impl SemanticAnchor {
    fn submitted(document: &ReadingDocument) -> Self {
        let mut reply = ReplyAnchor::default();
        reply.submitted(document);
        Self {
            reply,
            baseline: document
                .text
                .rsplit("\r\n")
                .take(MAX_SOURCE_LINES)
                .map(fingerprint)
                .collect(),
            ordered: evidence_lines(document)
                .iter()
                .map(|line| (line.key, line.row))
                .collect(),
            boundary: None,
            codex: false,
        }
    }

    fn offset(&self, document: &ReadingDocument) -> Option<u32> {
        let lines: Vec<_> = document
            .lines
            .iter()
            .zip(document.text.split("\r\n"))
            .collect();
        if let Some(boundary) = self.boundary {
            if let Some((line, _)) = lines
                .iter()
                .rev()
                .find(|(_, text)| fingerprint(text) == boundary)
            {
                return Some(line.start);
            }
        }
        let changed = lines
            .iter()
            .skip(lines.len().saturating_sub(MAX_SOURCE_LINES))
            .find(|(_, text)| !text.trim().is_empty() && !self.contains(text))
            .map(|(line, _)| line.start);
        self.reply.offset(document).into_iter().chain(changed).min()
    }

    fn contains(&self, text: &str) -> bool {
        self.baseline.contains(&fingerprint(text))
    }

    fn fresh_echo(&self, document: &ReadingDocument, input: &str) -> Option<u64> {
        let lines = evidence_lines(document);
        let source: Vec<_> = document.text.split("\r\n").collect();
        lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                if line.row < 0 || line.code {
                    return None;
                }
                let echo_end = if exact_user_echo(line.text.trim(), input) {
                    line.index + 1
                } else {
                    panel_echo_end(&source, line.index, input)?
                };
                if !self.contains(line.text) {
                    return Some(line.key);
                }
                // Repeating an accepted request is valid. The new occurrence must
                // follow the retained old source body in order and precede a new
                // source occurrence of content. A copy in negative history plus an
                // ED2 repaint of the same request/body is not such a boundary.
                // The accepted panel can be inserted above an unchanged
                // application footer. Its trailing rows still belong below
                // the new echo, provided both their text and live source row
                // remain identical. They are not the previous answer's body.
                let mut owned_end = self.ordered.len();
                while owned_end > 0 {
                    let (key, row) = self.ordered[owned_end - 1];
                    if row < 0
                        || !lines[index + 1..]
                            .iter()
                            .any(|next| next.key == key && next.row == row)
                    {
                        break;
                    }
                    owned_end -= 1;
                }
                let mut remaining = owned_end;
                let mut retained_live_body = false;
                for prior in lines[..index].iter().rev() {
                    if remaining > 0 && self.ordered[remaining - 1].0 == prior.key {
                        if remaining == owned_end {
                            retained_live_body = prior.row >= 0;
                        }
                        remaining -= 1;
                    }
                }
                if remaining != 0 || owned_end < 2 || !retained_live_body {
                    return None;
                }
                lines[index + 1..]
                    .iter()
                    .skip_while(|next| next.index < echo_end)
                    .take_while(|next| next.code || !user_echo(next.text.trim_start(), input))
                    .any(|next| {
                        !self.baseline.contains(&next.key)
                            || lines.iter().filter(|line| line.key == next.key).count()
                                > self
                                    .ordered
                                    .iter()
                                    .filter(|(key, _)| *key == next.key)
                                    .count()
                    })
                    .then_some(line.key)
            })
            .next_back()
    }

    fn source_echo(
        &self,
        document: &ReadingDocument,
        input: &str,
        adapter: crate::tui_adapters::AdapterKind,
        history: usize,
        editor: Option<(usize, usize)>,
    ) -> Option<FreshEcho> {
        let lines = evidence_lines(document);
        let source: Vec<_> = document.text.split("\r\n").collect();
        let key = self.fresh_echo(document, input).or_else(|| {
            if adapter != crate::tui_adapters::AdapterKind::Codex {
                return None;
            }
            if let Some(line) = lines.iter().rev().find(|line| {
                !line.code
                    && !self.baseline.contains(&line.key)
                    && codex_request_echo_end(&source, line.index, input).is_some()
            }) {
                // A complete accepted request can begin in retained history
                // after a large paste. The entire canonical payload must be
                // present; a matching viewport suffix cannot claim ownership.
                return Some(line.key);
            }
            // Steering can delete the interrupted assistant body before the
            // new request is drawn. Codex still retains the old request and
            // appends a second exact live echo. Count only live occurrences:
            // an ED2 copy of the old slice into history is not a new request.
            let matching: Vec<_> = lines
                .iter()
                .filter(|line| !line.code && exact_user_echo(line.text, input))
                .collect();
            let latest = matching.iter().rev().find(|line| line.row >= 0)?;
            let previous = self
                .ordered
                .iter()
                .filter(|(key, row)| *row >= 0 && *key == latest.key)
                .count();
            let previous_total = self
                .ordered
                .iter()
                .filter(|(key, _)| *key == latest.key)
                .count();
            (matching.iter().filter(|line| line.row >= 0).count() > previous
                && matching.len() > previous_total)
                .then_some(latest.key)
        })?;
        let line = lines.iter().rev().find(|line| line.key == key)?;
        if editor.is_some_and(|(first, last)| {
            history
                .checked_add_signed(line.row as isize)
                .is_some_and(|row| (first..=last).contains(&row))
        }) {
            return None;
        }
        Some(FreshEcho {
            key,
            start: document.lines.get(line.index)?.start,
            end: document
                .lines
                .get(
                    codex_request_echo_end(&source, line.index, input)
                        .unwrap_or(line.index + 1)
                        .saturating_sub(1),
                )?
                .end,
        })
    }
}

struct EvidenceLine<'a> {
    index: usize,
    text: &'a str,
    row: i32,
    code: bool,
    key: u64,
}

#[derive(Clone, Copy)]
struct FreshEcho {
    key: u64,
    start: u32,
    end: u32,
}

fn evidence_lines(document: &ReadingDocument) -> Vec<EvidenceLine<'_>> {
    let mut code = false;
    let first = document.lines.len().saturating_sub(MAX_SOURCE_LINES);
    document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .enumerate()
        .filter_map(|(index, (line, text))| {
            let trimmed = text.trim();
            if fence(trimmed) {
                code = !code;
            }
            let quoted = trimmed.starts_with(['>', '❯', '›', '"', '\'']);
            let chrome = !code
                && !quoted
                && (footer_kind(trimmed).is_some()
                    || cancellation_control(trimmed)
                    || footer_metadata(trimmed)
                    || recorded_completion(document, text, line.start));
            (index >= first && !trimmed.is_empty() && !chrome).then(|| EvidenceLine {
                index,
                text,
                row: line.line,
                code,
                key: fingerprint(text),
            })
        })
        .collect()
}
struct AnswerHeader {
    phase: usize,
    absolute: usize,
    live: bool,
}
#[derive(Clone)]
struct Footer {
    kind: SignalKind,
    text: String,
    key: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignalKind {
    Busy,
    Complete,
    Failed,
    Interrupted,
    NeedsInput,
}

struct PendingLine {
    kind: Option<PhaseKind>,
    text: String,
    key: u64,
    since: Instant,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct RichText {
    text: String,
    styles: Vec<StyleSpan>,
    glyphs: Vec<Glyph>,
    length: u32,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Glyph {
    start: u32,
    end: u32,
    width: usize,
}

impl Default for TurnLog {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnLog {
    pub fn new() -> Self {
        Self {
            adapter: crate::tui_adapters::AdapterKind::Universal,
            turns: VecDeque::new(),
            candidate: None,
            queued: VecDeque::new(),
            active: None,
            next_id: 1,
            revision: 1,
            archive: Arc::new(empty_document(1)),
            targets: Vec::new(),
            output_targets: Vec::new(),
            command_capture: command_output::Capture::default(),
            requests: Vec::new(),
            events: VecDeque::new(),
            status: "Готовность TUI не подтверждена".into(),
            source: None,
            view: View::default(),
            passive: SemanticAnchor::default(),
            passive_initialized: false,
            pending_lines: Vec::new(),
            announced: HashSet::new(),
            source_input: None,
        }
    }

    /// The caller has accepted an explicit plain Enter and excluded menus and
    /// protected input. An overlapping request waits for a fresh, exact source
    /// user echo; accepting Enter alone cannot divide queued/streaming output.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn submitted(&mut self, input: &str, baseline: &ReadingDocument, now: Instant) {
        self.submitted_request(input, baseline, RequestKind::Submit, now);
    }

    pub fn submitted_request(
        &mut self,
        input: &str,
        baseline: &ReadingDocument,
        kind: RequestKind,
        _now: Instant,
    ) {
        let codex = self.adapter == crate::tui_adapters::AdapterKind::Codex;
        if input.trim().is_empty()
            || input.trim_start().starts_with('/')
            || (codex && input.len() > MAX_REQUEST_BYTES)
        {
            return;
        }
        let mut anchor = SemanticAnchor::submitted(baseline);
        anchor.codex = self.adapter == crate::tui_adapters::AdapterKind::Codex;
        let candidate = Candidate {
            input: bounded(
                input,
                if codex {
                    MAX_REQUEST_BYTES
                } else {
                    MAX_EVENT_BYTES
                },
            ),
            request: saved_request(input),
            anchor,
            status_lines: baseline
                .text
                .rsplit("\r\n")
                .take(MAX_SOURCE_LINES)
                .map(|line| fingerprint(line.trim()))
                .collect(),
            stale_status: live_status_keys(baseline),
            kind,
            editor: self
                .source_input
                .as_ref()
                .filter(|source| {
                    self.adapter == crate::tui_adapters::AdapterKind::Codex
                        && source.text == input
                        && self.has_active_turn()
                })
                .map(|source| (source.first, source.last)),
        };
        if self.active.is_some() || self.candidate.is_some() {
            if self.queued.len() < MAX_QUEUED_REQUESTS {
                self.queued.push_back(candidate);
            }
            self.source = None;
            return;
        }
        self.candidate = Some(candidate);
        self.pending_lines.clear();
        self.announced.clear();
        self.source = None;
        self.status = "Запрос отправлен; ответ ещё не подтверждён".into();
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn observe(
        &mut self,
        frame: &TerminalSnapshot,
        output: &ReadingDocument,
        composer: Option<&Composer>,
        menu_open: bool,
        now: Instant,
    ) {
        self.observe_for_adapter(
            crate::tui_adapters::AdapterKind::Universal,
            frame,
            output,
            composer,
            menu_open,
            now,
        );
    }

    /// A changed parser policy cannot inherit an in-flight turn's ownership.
    /// Retained answers and their navigation IDs remain available for reading.
    pub fn set_adapter(&mut self, adapter: crate::tui_adapters::AdapterKind) {
        if self.adapter != adapter {
            self.cancel_pending();
            self.view = View::default();
            self.passive = SemanticAnchor::default();
            self.passive_initialized = false;
            self.adapter = adapter;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn observe_for_adapter(
        &mut self,
        adapter: crate::tui_adapters::AdapterKind,
        frame: &TerminalSnapshot,
        output: &ReadingDocument,
        composer: Option<&Composer>,
        menu_open: bool,
        now: Instant,
    ) {
        self.set_adapter(adapter);
        self.observe_source(frame, output, composer, menu_open, now, true);
        if let Some((id, command, body)) = self
            .command_capture
            .observe(frame, output, composer, menu_open, now)
        {
            let id = if let Some(id) = id {
                let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == id) else {
                    return;
                };
                if turn.answer.signature() == body.signature() {
                    return;
                }
                turn.answer = body;
                id
            } else {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                self.turns.push_back(Turn {
                    id,
                    kind: OutputKind::Command,
                    request: command,
                    request_expanded: false,
                    answer: body,
                    complete: true,
                    failed: false,
                });
                id
            };
            self.command_capture.committed(id);
            self.retain();
            self.rebuild();
        }
    }

    /// Only the successful explicit user-input path arms a local reading
    /// snapshot. Source text cannot submit commands or create model events.
    pub fn accepted_output_input(
        &mut self,
        bytes: &[u8],
        frame: &TerminalSnapshot,
        output: &ReadingDocument,
        composer: Option<&Composer>,
        menu: Option<&crate::tui_suggestions::Menu>,
        now: Instant,
    ) {
        self.command_capture
            .accepted(bytes, frame, output, composer, menu, now);
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_source(
        &mut self,
        frame: &TerminalSnapshot,
        output: &ReadingDocument,
        composer: Option<&Composer>,
        menu_open: bool,
        now: Instant,
        handoff: bool,
    ) {
        if frame.display_offset != 0
            || frame.alternate_screen != output.alternate_screen
            || frame.cols == 0
            || frame.rows == 0
            || frame.cols.checked_mul(frame.rows) != Some(frame.cells.len())
        {
            return;
        }
        if !menu_open {
            for candidate in &mut self.queued {
                if composer.is_some_and(|input| input.text != candidate.input) {
                    // The source has cleared or replaced the submitted field.
                    // Its former rows can now host a real accepted echo.
                    candidate.editor = None;
                }
            }
            let fresh_boundary = handoff
                .then(|| {
                    self.queued
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| {
                            *index == 0 || self.adapter == crate::tui_adapters::AdapterKind::Codex
                        })
                        .filter_map(|(index, candidate)| {
                            candidate
                                .anchor
                                .source_echo(
                                    output,
                                    &candidate.input,
                                    self.adapter,
                                    frame.history_size,
                                    candidate.editor,
                                )
                                .map(|echo| (index, echo))
                        })
                        .min_by_key(|(_, echo)| echo.start)
                })
                .flatten();
            if let Some((index, boundary)) = fresh_boundary {
                let mut candidate = self.queued.remove(index).unwrap();
                if self.adapter == crate::tui_adapters::AdapterKind::Codex && self.active.is_some()
                {
                    // A drain can publish the old tail, its duration and the
                    // next request together. Parse the exact owned prefix
                    // before switching; the next turn must not absorb it.
                    let prefix = source_prefix(output, boundary.start);
                    self.observe_source(frame, &prefix, composer, false, now, false);
                    if let Some(active) = self.active.as_ref() {
                        let complete = self.view.footer.as_ref().is_some_and(|footer| {
                            footer.kind == SignalKind::Complete
                                && !active.stale_status.contains(&footer.key)
                        });
                        if complete {
                            self.emit(TurnEvent::Ready { id: active.id });
                        } else if candidate.kind == RequestKind::Steer {
                            self.emit(TurnEvent::Interrupted {
                                id: active.id,
                                text: "Ответ прерван новым запросом".into(),
                            });
                        }
                        self.close_active(complete);
                    }
                } else {
                    // Other renderers retain their existing conservative
                    // handoff contract, without inferring completion.
                    self.close_active(false);
                }
                if self.adapter == crate::tui_adapters::AdapterKind::Codex {
                    let accepted_prefix = source_prefix(output, boundary.end);
                    for waiting in &mut self.queued {
                        // Steering can erase the body in the original
                        // queue baseline. Rebase only through the newly
                        // observed accepted echo, preserving any later
                        // echoes in this same redraw as fresh evidence.
                        waiting.anchor = SemanticAnchor::submitted(&accepted_prefix);
                        waiting.anchor.codex = true;
                    }
                }
                candidate.anchor.boundary = Some(boundary.key);
                self.candidate = Some(candidate);
                self.pending_lines.clear();
                self.announced.clear();
                self.source = None;
            }
            if let Some(candidate) = self.candidate.as_mut() {
                if let Some(boundary) = candidate.anchor.source_echo(
                    output,
                    &candidate.input,
                    self.adapter,
                    frame.history_size,
                    None,
                ) {
                    candidate.anchor.boundary = Some(boundary.key);
                }
            }
            if let Some(active) = self.active.as_mut() {
                if let Some(boundary) = active.anchor.source_echo(
                    output,
                    &active.input,
                    self.adapter,
                    frame.history_size,
                    None,
                ) {
                    // A delayed echo or prompt reflow updates the same turn's
                    // source boundary; only the queued branch creates a turn.
                    active.anchor.boundary = Some(boundary.key);
                }
            }
        }
        let key = SourceKey {
            revision: output.revision,
            columns: frame.cols,
            rows: frame.rows,
            history: frame.history_size,
            alternate: frame.alternate_screen,
            composer: composer.is_some(),
            protected: composer.is_some_and(|input| input.protected),
            editable: editable(composer),
            menu: menu_open,
        };
        let source_changed = self.source != Some(key);
        if source_changed {
            let mut offset = self
                .active
                .as_ref()
                .and_then(|active| active.anchor.offset(output))
                .or_else(|| {
                    self.candidate
                        .as_ref()
                        .and_then(|candidate| candidate.anchor.offset(output))
                })
                .or_else(|| {
                    (self.active.is_none() && self.candidate.is_none() && self.passive_initialized)
                        .then(|| self.passive.offset(output))
                        .flatten()
                })
                .or_else(|| {
                    self.active.as_ref().and_then(|active| {
                        self.turns
                            .iter()
                            .find(|turn| turn.id == active.id)
                            .filter(|turn| !turn.answer.text.is_empty())
                            .and_then(|_| {
                                output
                                    .lines
                                    .iter()
                                    .find(|line| line.line >= 0)
                                    .map(|line| line.start)
                            })
                    })
                });
            if let Some(origin) = self.active.as_ref().and_then(|active| active.answer_origin) {
                if let Some(line) = output.lines.iter().find(|line| {
                    frame.history_size.checked_add_signed(line.line as isize) == Some(origin)
                }) {
                    // A still-visible owned header supplies the full current
                    // answer, including corrections below an unchanged label.
                    offset = offset.map(|offset| offset.min(line.start));
                }
            }
            let input = self
                .active
                .as_ref()
                .map(|active| active.input.as_str())
                .or_else(|| {
                    self.candidate
                        .as_ref()
                        .map(|candidate| candidate.input.as_str())
                })
                .unwrap_or("");
            let context = self
                .active
                .as_ref()
                .is_some_and(|active| active.seen_busy || active.semantic);
            let anchor = self
                .active
                .as_ref()
                .map(|active| &active.anchor)
                .or_else(|| self.candidate.as_ref().map(|candidate| &candidate.anchor));
            let codex_start = (self.adapter == crate::tui_adapters::AdapterKind::Codex)
                .then(|| {
                    let boundary = anchor?.boundary?;
                    output
                        .lines
                        .iter()
                        .zip(output.text.split("\r\n"))
                        .filter(|(_, text)| fingerprint(text) == boundary)
                        .last()
                        .map(|(line, _)| line.start)
                })
                .flatten();
            if let Some(start) = codex_start {
                offset = Some(offset.unwrap_or(start).max(start));
            }
            self.view = parse_view(
                frame,
                output,
                offset,
                composer,
                input,
                context,
                anchor,
                self.adapter.policy().semantics,
            );
            if codex_start.is_some() && !self.view.answer_headers.is_empty() {
                // A retained accepted Codex echo bounds a complete owned
                // source slice, even when ED2/steering reset absolute rows.
                // Corrections replace that slice rather than append stale ink.
                self.view.answer_snapshot = true;
            }
            if !self.view.answer_snapshot {
                if let Some((active, margin, answer)) = self.active.as_ref().and_then(|active| {
                    Some((
                        active,
                        active.panel_margin?,
                        &self.turns.iter().find(|turn| turn.id == active.id)?.answer,
                    ))
                }) {
                    panel_continuation(
                        frame,
                        output,
                        composer,
                        PanelContinuation {
                            previous: answer,
                            margin,
                            baseline: &active.summary_baseline,
                        },
                        &mut self.view,
                    );
                }
            }
            if self.view.phases.is_empty() && !self.view.fallback.text.is_empty() {
                let continuation = self
                    .active
                    .as_ref()
                    .and_then(|active| self.turns.iter().find(|turn| turn.id == active.id))
                    .is_some_and(|turn| {
                        text_overlap(&turn.answer.text, &self.view.fallback.text).is_some()
                    });
                if continuation {
                    self.view
                        .phases
                        .push((PhaseKind::Answer, std::mem::take(&mut self.view.fallback)));
                }
            }
            self.passive = SemanticAnchor::submitted(output);
            self.passive_initialized = true;
            self.source = Some(key);
        }
        let mut view = std::mem::take(&mut self.view);
        if self.active.is_none() && self.candidate.is_none() {
            if source_changed {
                for (_, part) in view.phases.drain(..) {
                    view.fallback.join(&part);
                }
            } else {
                view.phases.clear();
            }
        }
        if menu_open {
            view.phases.clear();
            view.answer_headers.clear();
            view.fallback = RichText::default();
        }
        let fresh_footer = view.footer.as_ref().is_some_and(|footer| {
            self.candidate
                .as_ref()
                .is_some_and(|candidate| !candidate.status_lines.contains(&footer.key))
        });
        if self.active.is_none()
            && self.candidate.is_some()
            && (self.adapter != crate::tui_adapters::AdapterKind::Codex
                || self
                    .candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.anchor.boundary.is_some()))
            && (!view
                .phases
                .iter()
                .all(|(_, text)| text.text.trim().is_empty())
                || !view.fallback.text.trim().is_empty()
                || fresh_footer)
        {
            let candidate = self.candidate.take().unwrap();
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.turns.push_back(Turn {
                id,
                kind: OutputKind::Answer,
                request: candidate.request,
                request_expanded: false,
                answer: RichText::default(),
                complete: false,
                failed: false,
            });
            self.emit(TurnEvent::Phase {
                kind: PhaseKind::Request,
                text: bounded(&candidate.input, MAX_EVENT_BYTES),
            });
            self.active = Some(Active {
                id,
                input: candidate.input,
                anchor: candidate.anchor,
                stale_status: candidate.stale_status,
                summary_baseline: candidate.status_lines,
                panel_margin: None,
                seen_busy: false,
                was_busy: false,
                busy_gone: false,
                input_unavailable: false,
                seen_control: false,
                was_control: false,
                control_gone: false,
                control_returned: false,
                uncertain_end_announced: false,
                semantic: false,
                answer_changed: now,
                answer_signature: None,
                answer_origin: None,
                answer_history: frame.history_size,
                answer_regions: HashSet::new(),
                answer_aliases: HashSet::new(),
                completion_since: None,
                terminal: None,
                codex_tools_seen: false,
                provisional_headers: HashSet::new(),
            });
            self.retain();
            self.rebuild();
        }
        if self.adapter == crate::tui_adapters::AdapterKind::Codex
            && self.active.is_none()
            && self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.anchor.boundary.is_none())
        {
            // Enter may first publish the still-visible editor viewport.
            // Its suffix is not assistant output and cannot arm a request.
            view.phases.clear();
            view.fallback = RichText::default();
            view.answer_headers.clear();
        }
        let mut changed = false;
        if let Some(active) = self.active.as_mut() {
            if view.codex_tool_output {
                let final_body = view
                    .phases
                    .iter()
                    .any(|(kind, _)| *kind == PhaseKind::Answer);
                if !active.codex_tools_seen || !final_body {
                    active.answer_origin = None;
                    active.answer_signature = None;
                    active.answer_aliases.clear();
                    active.answer_regions.clear();
                }
                active.codex_tools_seen = true;
                if !final_body {
                    let turn = self
                        .turns
                        .iter_mut()
                        .find(|turn| turn.id == active.id)
                        .unwrap();
                    if !turn.answer.text.is_empty() {
                        turn.answer = RichText::default();
                        active.answer_changed = now;
                        changed = true;
                    }
                }
            }
            if view.panel_margin.is_some() {
                active.panel_margin = view.panel_margin;
            }
            if !menu_open {
                active
                    .stale_status
                    .retain(|key| view.live_status.contains(key));
                if self.adapter == crate::tui_adapters::AdapterKind::Codex && view.distinct_footer {
                    if let Some(footer) = view.footer.as_ref() {
                        active.stale_status.remove(&footer.key);
                    }
                }
            }
            let busy = view.control_busy
                || view
                    .footer
                    .as_ref()
                    .is_some_and(|footer| footer.kind == SignalKind::Busy);
            let codex_busy = self.adapter == crate::tui_adapters::AdapterKind::Codex && busy;
            if codex_busy && !menu_open && composer.is_none_or(|input| !input.protected) {
                // Codex keeps its busy UI for commentary and removes it at
                // the first final-answer token. Remember only current source
                // headers, so later final snapshots do not reintroduce that
                // visible but provisional commentary as the answer prefix.
                active.provisional_headers.clear();
                active
                    .provisional_headers
                    .extend(view.answer_headers.iter().filter_map(|header| {
                        let (_, part) = view.phases.get(header.phase)?;
                        Some((
                            header.absolute,
                            fingerprint(part.text.split("\r\n").next().unwrap_or("")),
                        ))
                    }));
                let turn = self
                    .turns
                    .iter_mut()
                    .find(|turn| turn.id == active.id)
                    .unwrap();
                if !turn.answer.text.is_empty() {
                    turn.answer = RichText::default();
                    active.answer_signature = None;
                    active.answer_origin = None;
                    active.answer_regions.clear();
                    active.answer_aliases.clear();
                    active.answer_changed = now;
                    changed = true;
                }
            }
            if !menu_open {
                if busy {
                    active.seen_busy = true;
                    active.busy_gone = false;
                    active.input_unavailable |= !editable(composer);
                    active.uncertain_end_announced = false;
                } else if active.was_busy {
                    active.busy_gone = true;
                }
                active.was_busy = busy;
                if view.control_busy {
                    active.seen_control = true;
                    active.control_gone = false;
                    active.control_returned = false;
                } else if active.was_control {
                    active.control_gone = true;
                }
                active.was_control = view.control_busy;
                if source_changed && active.control_gone && editable(composer) {
                    active.control_returned = true;
                }
            }
            if !view.phases.is_empty() {
                active.semantic = true;
            }
            if let Some((kind, _)) = view.phases.last() {
                self.status = phase_status(*kind).into();
            } else if busy {
                self.status = "TUI выполняет запрос".into();
            }
            let mut answer = RichText::default();
            let final_headers: Vec<_> = view
                .answer_headers
                .iter()
                .filter(|header| {
                    let text = &view.phases[header.phase].1.text;
                    !active.provisional_headers.contains(&(
                        header.absolute,
                        fingerprint(text.split("\r\n").next().unwrap_or("")),
                    ))
                })
                .collect();
            let header = final_headers
                .iter()
                .copied()
                .find(|header| header.live)
                .or_else(|| final_headers.first().copied());
            let repaint = header.is_some_and(|header| {
                header.live
                    && frame.history_size > active.answer_history
                    && active.answer_origin.is_some_and(|old| {
                        view.answer_headers
                            .iter()
                            .any(|prior| prior.absolute == old && !prior.live)
                            || view.retained_headers.contains(&old)
                    })
            });
            if repaint {
                active.answer_aliases.extend(&active.answer_regions);
                if let Some(old) = active.answer_origin {
                    active.answer_aliases.insert(old);
                }
            }
            if source_changed {
                for (index, (_, part)) in view
                    .phases
                    .iter()
                    .enumerate()
                    .filter(|(_, (kind, _))| *kind == PhaseKind::Answer)
                {
                    if codex_busy
                        || view.answer_headers.iter().any(|header| {
                            header.phase == index
                                && active.provisional_headers.contains(&(
                                    header.absolute,
                                    fingerprint(part.text.split("\r\n").next().unwrap_or("")),
                                ))
                        })
                    {
                        continue;
                    }
                    if view.answer_headers.iter().any(|header| {
                        header.phase == index && active.answer_aliases.contains(&header.absolute)
                    }) {
                        continue;
                    }
                    // Distinct source headers within one snapshot are distinct
                    // paragraphs, including identical or prefix-related text.
                    // Only subsequent source snapshots use overlap merging.
                    answer.join(part);
                }
            }
            if !answer.text.trim().is_empty() && active.answer_signature != Some(answer.signature())
            {
                active.answer_signature = Some(answer.signature());
                let turn = self
                    .turns
                    .iter_mut()
                    .find(|turn| turn.id == active.id)
                    .unwrap();
                let same_text = turn.answer.text == answer.text;
                let full_snapshot = view.answer_snapshot
                    || header.is_some_and(|header| active.answer_origin == Some(header.absolute))
                    || repaint;
                if full_snapshot {
                    changed = turn.answer != answer;
                    turn.answer = answer;
                } else if let Some(overlap) = view.answer_overlap {
                    changed = turn.answer.merge_suffix(answer, overlap);
                } else {
                    changed = turn.answer.merge(answer);
                }
                if let Some(header) = header {
                    active.answer_origin = Some(header.absolute);
                    active.answer_history = frame.history_size;
                    active.answer_regions = final_headers
                        .iter()
                        .copied()
                        .filter(|header| !active.answer_aliases.contains(&header.absolute))
                        .map(|header| header.absolute)
                        .collect();
                }
                if changed && !same_text {
                    active.answer_changed = now;
                }
            }
        }
        if !menu_open {
            self.observe_lines(&view, now);
        }
        self.observe_terminal(&view, composer, menu_open, now);
        if changed {
            self.retain();
            self.rebuild();
        }
        self.view = view;
        if let Some(input) =
            composer.filter(|input| input.matches_source(frame) && !input.protected)
        {
            self.source_input = Some(SourceInputBoundary {
                text: input.text.clone(),
                first: frame.history_size + input.first_row,
                last: frame.history_size + input.last_row,
            });
        }
    }

    fn observe_terminal(
        &mut self,
        view: &View,
        composer: Option<&Composer>,
        menu_open: bool,
        now: Instant,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let id = active.id;
        let footer = view.footer.as_ref();
        let terminal = footer.filter(|footer| {
            footer.kind != SignalKind::Busy && !active.stale_status.contains(&footer.key)
        });
        let current_signal = terminal.map(|footer| (footer.kind, footer.key));
        if current_signal != active.terminal {
            self.active.as_mut().unwrap().terminal = current_signal;
            if let Some(footer) = terminal {
                match footer.kind {
                    SignalKind::Failed => {
                        if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == id) {
                            turn.failed = true;
                        }
                        self.status = "Ошибка TUI".into();
                        self.emit(TurnEvent::Failed {
                            id,
                            text: bounded(&footer.text, MAX_EVENT_BYTES),
                        });
                        self.close_active(false);
                        return;
                    }
                    SignalKind::Interrupted => {
                        self.status = "Ответ прерван".into();
                        self.emit(TurnEvent::Interrupted {
                            id,
                            text: bounded(&footer.text, MAX_EVENT_BYTES),
                        });
                        self.close_active(false);
                        return;
                    }
                    SignalKind::NeedsInput => {
                        self.status = "TUI ожидает ввода или подтверждения".into();
                        self.emit(TurnEvent::NeedsInput {
                            id,
                            text: bounded(&footer.text, MAX_EVENT_BYTES),
                        });
                    }
                    _ => {}
                }
            }
        }
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let answer = self
            .turns
            .iter()
            .find(|turn| turn.id == id)
            .is_some_and(|turn| !turn.answer.text.trim().is_empty());
        let stable = now.saturating_duration_since(active.answer_changed) >= STABLE_FOR;
        let explicit = terminal.is_some_and(|footer| footer.kind == SignalKind::Complete);
        let returned =
            active.seen_busy && active.input_unavailable && active.busy_gone && editable(composer);
        // A source cancellation control or a proven adapter working indicator
        // is a busy UI boundary. Its removal and a fresh editable composer work for TUIs
        // whose input remains available for queued messages while working.
        let control_returned = active.seen_control
            && active.control_gone
            && active.control_returned
            && editable(composer);
        let waiting = terminal.is_some_and(|footer| footer.kind == SignalKind::NeedsInput);
        if waiting {
            self.status = "TUI ожидает ввода или подтверждения".into();
        }
        // Codex removes its interruption control at the first answer token,
        // while the response stream can still be open. Its current duration
        // annotation supplies completion; returning input alone does not.
        let source_complete = !menu_open
            && answer
            && !active.was_busy
            && !waiting
            && (explicit
                || (self.adapter != crate::tui_adapters::AdapterKind::Codex
                    && active.panel_margin.is_none()
                    && (returned || control_returned)));
        if source_complete {
            active.completion_since.get_or_insert(now);
        } else {
            active.completion_since = None;
        }
        let completion_stable = active
            .completion_since
            .is_some_and(|since| now.saturating_duration_since(since) >= COMPLETION_STABLE_FOR);
        if source_complete && stable && completion_stable {
            self.status = "Ответ готов".into();
            self.emit(TurnEvent::Ready { id });
            self.close_active(true);
        } else if !menu_open
            && answer
            && stable
            && active.busy_gone
            && !active.input_unavailable
            && !explicit
            && !control_returned
            && !waiting
            && !active.uncertain_end_announced
        {
            active.uncertain_end_announced = true;
            self.status = "Новый вывод; завершение не подтверждено".into();
            self.emit(TurnEvent::NewOutput {
                text: "Новый вывод TUI; завершение не подтверждено".into(),
            });
        }
    }

    fn close_active(&mut self, complete: bool) {
        if let Some(active) = self.active.take() {
            if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == active.id) {
                turn.complete = complete;
            }
            self.pending_lines.clear();
            self.announced.clear();
            self.source = None;
            self.rebuild();
        }
    }

    fn observe_lines(&mut self, view: &View, now: Instant) {
        let mut current = Vec::new();
        for (kind, content) in &view.phases {
            for text in content
                .text
                .split("\r\n")
                .filter(|text| !text.trim().is_empty())
            {
                current.push((Some(*kind), bounded(text, MAX_EVENT_BYTES)));
            }
        }
        for text in view
            .fallback
            .text
            .split("\r\n")
            .filter(|text| !text.trim().is_empty())
        {
            current.push((None, bounded(text, MAX_EVENT_BYTES)));
        }
        current.truncate(MAX_SPEECH_LINES);
        self.pending_lines.retain(|line| {
            current
                .iter()
                .any(|(kind, text)| *kind == line.kind && fingerprint(text) == line.key)
        });
        for (kind, text) in current {
            let key = fingerprint(&text);
            if !self.announced.contains(&(kind, key))
                && !self
                    .pending_lines
                    .iter()
                    .any(|line| line.kind == kind && line.key == key)
            {
                self.pending_lines.push(PendingLine {
                    kind,
                    text,
                    key,
                    since: now,
                });
            }
        }
        let mut due = Vec::new();
        self.pending_lines.retain(|line| {
            if now.saturating_duration_since(line.since) < STABLE_FOR {
                return true;
            }
            due.push((line.kind, line.text.clone(), line.key));
            false
        });
        for (kind, text, key) in due {
            self.announced.insert((kind, key));
            if let Some(kind) = kind {
                self.emit(TurnEvent::Phase { kind, text });
            } else {
                if self.active.as_ref().is_none_or(|active| !active.semantic) {
                    self.status = "Новый вывод; готовность не подтверждена".into();
                }
                self.emit(TurnEvent::NewOutput { text });
            }
        }
        if self.announced.len() > MAX_SOURCE_LINES {
            self.announced.clear();
        }
    }

    fn emit(&mut self, event: TurnEvent) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn retain(&mut self) {
        // A command may follow a still-streaming answer. Bound every changed
        // entry and keep the active answer alive until its lifecycle ends.
        for turn in &mut self.turns {
            turn.answer
                .keep_tail(MAX_BYTES - turn.request.len() - TURN_OVERHEAD - 128);
        }
        while self.turns.len() > MAX_TURNS
            || (self.turns.len() > 1
                && self
                    .turns
                    .iter()
                    .map(|turn| turn.answer.text.len() + turn.request.len() + TURN_OVERHEAD)
                    .sum::<usize>()
                    > MAX_BYTES)
        {
            let active = self.active.as_ref().map(|active| active.id);
            let newest = self.turns.back().map(|turn| turn.id);
            if let Some(index) = self
                .turns
                .iter()
                .position(|turn| Some(turn.id) != active && Some(turn.id) != newest)
            {
                self.turns.remove(index);
            } else if let Some(index) = self.turns.iter().position(|turn| Some(turn.id) == active) {
                // With only the active answer and newest command left, keep
                // both; trim the answer's old text instead of losing the result.
                let others: usize = self
                    .turns
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .map(|(_, turn)| turn.answer.text.len() + turn.request.len() + TURN_OVERHEAD)
                    .sum();
                let turn = &mut self.turns[index];
                let budget =
                    MAX_BYTES.saturating_sub(others + turn.request.len() + TURN_OVERHEAD + 128);
                let previous = turn.answer.text.len();
                turn.answer.keep_tail(budget);
                if turn.answer.text.len() >= previous {
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn rebuild(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let mut all = RichText::default();
        let total = self
            .turns
            .iter()
            .filter(|turn| turn.kind == OutputKind::Answer && !turn.answer.text.is_empty())
            .count();
        let output_total = self
            .turns
            .iter()
            .filter(|turn| !turn.answer.text.is_empty())
            .count();
        self.targets.clear();
        self.output_targets.clear();
        self.requests.clear();
        for turn in self
            .turns
            .iter()
            .filter(|turn| !turn.answer.text.is_empty())
        {
            if !all.text.is_empty() {
                all.join(&RichText::plain(""));
            }
            if turn.kind == OutputKind::Command {
                let mut header = RichText::plain(&format!("Вывод команды: {}", turn.request));
                for span in &mut header.styles {
                    span.style.flags |= Flags::BOLD;
                }
                all.join(&header);
                let start = all.units() + 2;
                all.join(&turn.answer);
                self.output_targets.push(AnswerTarget {
                    id: turn.id,
                    start,
                    number: self.output_targets.len() + 1,
                    total: output_total,
                    complete: true,
                    failed: false,
                    kind: OutputKind::Command,
                });
                continue;
            }
            let number = self.targets.len() + 1;
            let preview = turn
                .request
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let mut characters = preview.chars();
            let mut preview: String = characters.by_ref().take(80).collect();
            if characters.next().is_some() {
                preview.push('…');
            }
            let state = if turn.request_expanded {
                "развёрнуто"
            } else {
                "свёрнуто"
            };
            let mut header = RichText::plain(&format!("Ваш запрос, {state}: {preview}"));
            for span in &mut header.styles {
                span.style.flags |= Flags::BOLD;
            }
            all.join(&header);
            self.requests.push(RequestTarget {
                id: turn.id,
                start: all.units() - header.units(),
                end: all.units(),
                expanded: turn.request_expanded,
            });
            if turn.request_expanded {
                all.join(&RichText::plain(&turn.request));
            }
            all.join(&RichText::plain(&format!("Ответ {number}")));
            let start = all.units() + 2;
            all.join(&turn.answer);
            self.targets.push(AnswerTarget {
                id: turn.id,
                start,
                number,
                total,
                complete: turn.complete,
                failed: turn.failed,
                kind: OutputKind::Answer,
            });
            self.output_targets.push(AnswerTarget {
                number: self.output_targets.len() + 1,
                total: output_total,
                ..*self.targets.last().unwrap()
            });
        }
        self.archive = Arc::new(all.document(self.revision));
    }

    pub fn take_events(&mut self) -> Vec<TurnEvent> {
        self.events.drain(..).collect()
    }
    pub fn status(&self) -> String {
        self.status.clone()
    }
    pub fn document(&self) -> Arc<ReadingDocument> {
        Arc::clone(&self.archive)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn has_answers(&self) -> bool {
        !self.targets.is_empty()
    }
    pub fn has_active_turn(&self) -> bool {
        self.candidate.is_some() || self.active.is_some() || !self.queued.is_empty()
    }
    pub fn has_semantic_turn(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.semantic)
    }
    /// A heartbeat describes only the current visible Codex busy UI of an
    /// owned request. A prior busy frame, phase text or elapsed time alone
    /// cannot keep it alive after a menu, protected input or terminal signal.
    pub fn working_turn(&self) -> Option<u64> {
        let active = self.active.as_ref()?;
        let source = self.source?;
        if self.adapter != crate::tui_adapters::AdapterKind::Codex
            || source.menu
            || source.protected
            || active.terminal.is_some()
            || self
                .view
                .footer
                .as_ref()
                .is_some_and(|footer| footer.kind != SignalKind::Busy)
        {
            return None;
        }
        (self.view.control_busy
            || self
                .view
                .footer
                .as_ref()
                .is_some_and(|footer| footer.kind == SignalKind::Busy))
        .then_some(active.id)
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn latest_answer(&self) -> Option<AnswerTarget> {
        self.targets.last().copied()
    }

    pub fn latest_output(&self) -> Option<AnswerTarget> {
        self.output_targets.last().copied()
    }

    pub fn navigate_output(&self, current: Option<u64>, previous: bool) -> Option<AnswerTarget> {
        let index = current.and_then(|id| {
            self.output_targets
                .iter()
                .position(|target| target.id == id)
        });
        let index = match index {
            Some(index) if previous => index.saturating_sub(1),
            Some(index) => (index + 1).min(self.output_targets.len().saturating_sub(1)),
            None if previous => self.output_targets.len().checked_sub(1)?,
            None => 0,
        };
        self.output_targets.get(index).copied()
    }
    /// Only generated request headings are actions. Selected text, request
    /// bodies and answer prose remain ordinary read-only text, even if they
    /// contain an identical-looking heading.
    pub fn request_at(&self, selection: (u32, u32)) -> Option<RequestTarget> {
        (selection.0 == selection.1)
            .then(|| {
                self.requests
                    .iter()
                    .find(|target| (target.start..=target.end).contains(&selection.0))
                    .copied()
            })
            .flatten()
    }
    pub fn toggle_request(&mut self, id: u64) -> Option<RequestTarget> {
        let turn = self.turns.iter_mut().find(|turn| {
            turn.kind == OutputKind::Answer && turn.id == id && !turn.answer.text.is_empty()
        })?;
        turn.request_expanded = !turn.request_expanded;
        self.rebuild();
        self.requests.iter().find(|target| target.id == id).copied()
    }
    pub fn answer_text(&self, id: u64) -> Option<&str> {
        self.turns
            .iter()
            .find(|turn| turn.id == id)
            .filter(|turn| turn.kind == OutputKind::Answer && !turn.answer.text.is_empty())
            .map(|turn| turn.answer.text.as_str())
    }
    #[cfg(test)]
    pub fn output_text(&self, id: u64) -> Option<&str> {
        self.turns
            .iter()
            .find(|turn| turn.id == id)
            .map(|turn| turn.answer.text.as_str())
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn navigate(&self, current_answer: Option<u64>, previous: bool) -> Option<AnswerTarget> {
        let index =
            current_answer.and_then(|id| self.targets.iter().position(|target| target.id == id));
        let target = match index {
            Some(index) if previous => index.saturating_sub(1),
            Some(index) => (index + 1).min(self.targets.len().saturating_sub(1)),
            None if previous => self.targets.len().checked_sub(1)?,
            None => 0,
        };
        self.targets.get(target).copied()
    }
    pub fn cancel_pending(&mut self) {
        self.command_capture = command_output::Capture::default();
        self.candidate = None;
        self.queued.clear();
        self.active = None;
        self.source_input = None;
        self.source = None;
        self.pending_lines.clear();
        self.announced.clear();
        self.events.clear();
        self.status = "Готовность TUI не подтверждена".into();
    }

    pub fn cancel_command_capture(&mut self) {
        self.command_capture = command_output::Capture::default();
    }
    pub fn clear(&mut self) {
        self.cancel_pending();
        self.turns.clear();
        self.rebuild();
        self.passive = SemanticAnchor::default();
        self.passive_initialized = false;
    }
}

fn phase_status(kind: PhaseKind) -> &'static str {
    match kind {
        PhaseKind::Request => "Запрос отправлен",
        PhaseKind::Thinking => "Размышление",
        PhaseKind::Search => "Поиск",
        PhaseKind::Tool => "Выполнение инструмента",
        PhaseKind::Answer => "Ответ формируется",
    }
}

fn fingerprint(text: &str) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}
fn bounded(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn saved_request(input: &str) -> String {
    let mut request = String::new();
    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if request.len() + character.len_utf8() + 2 > MAX_REQUEST_BYTES - 128 {
            request.push_str("\r\n[Конец запроса не сохранён: ограничение размера]");
            break;
        }
        match character {
            '\r' => {
                request.push_str("\r\n");
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
            }
            '\n' => request.push_str("\r\n"),
            '\t' => request.push(character),
            character if !character.is_control() => request.push(character),
            _ => {}
        }
    }
    request
}
fn empty_document(revision: u64) -> ReadingDocument {
    ReadingDocument {
        revision,
        alternate_screen: false,
        text: String::new(),
        cells: Vec::new(),
        lines: Vec::new(),
        styles: Vec::new(),
    }
}

/// An exact retained source segment, used only while transferring ownership
/// to a later observed echo. Coordinates/styles stay in the original frame.
fn source_prefix(document: &ReadingDocument, end: u32) -> ReadingDocument {
    let mut prefix = document.clone();
    let mut units = 0;
    let byte = document
        .text
        .char_indices()
        .find_map(|(byte, ch)| {
            if units == end {
                return Some(byte);
            }
            units += ch.len_utf16() as u32;
            None
        })
        .unwrap_or(document.text.len());
    prefix.text.truncate(byte);
    prefix.lines.retain(|line| line.start < end);
    prefix.cells.retain(|cell| cell.end <= end);
    prefix.styles.retain(|span| span.start < end);
    if let Some(span) = prefix.styles.last_mut() {
        span.end = span.end.min(end);
    }
    prefix
}

struct SourceLine<'a> {
    text: &'a str,
    start: u32,
    row: i32,
}

#[allow(clippy::too_many_arguments)]
fn parse_view(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    offset: Option<u32>,
    composer: Option<&Composer>,
    input: &str,
    context: bool,
    anchor: Option<&SemanticAnchor>,
    policy: crate::tui_adapters::Semantics,
) -> View {
    let lines: Vec<_> = document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .map(|(line, text)| SourceLine {
            text,
            start: line.start,
            row: line.line,
        })
        .collect();
    let mut footer = None;
    let mut controls = vec![false; lines.len()];
    let mut control_busy = false;
    let live_last = lines
        .iter()
        .filter(|line| line.row >= 0 && !line.text.trim().is_empty())
        .map(|line| line.row)
        .max()
        .unwrap_or(0);
    let mut code = false;
    let mut tool_metadata = false;
    let source: Vec<_> = lines.iter().map(|line| line.text).collect();
    let mut echo_end = 0;
    let owned_start = anchor.filter(|anchor| anchor.codex).and_then(|anchor| {
        let boundary = anchor.boundary?;
        lines
            .iter()
            .rev()
            .find(|line| fingerprint(line.text) == boundary)
            .map(|line| line.start)
    });
    for (index, line) in lines.iter().enumerate() {
        if index < echo_end {
            continue;
        }
        if !code && anchor.is_some_and(|anchor| anchor.codex) {
            if let Some(end) = codex_request_echo_end(&source, index, input) {
                echo_end = end;
                tool_metadata = false;
                continue;
            }
            // This pass only shields structurally styled tool continuations
            // from the fence grammar. Do not require busy before its footer
            // has been scanned: Running stdout can itself contain fences.
            // The role pass below still requires actual busy for Running.
            if codex_activity::prefix(document, &lines, index, true).is_some() {
                tool_metadata = true;
            } else if codex_activity::outer(line.text) {
                tool_metadata = false;
            }
            if tool_metadata && (line.text.starts_with("    ") || line.text.starts_with("  └ ")) {
                continue;
            }
        }
        let text = line.text.trim();
        if fence(text) {
            code = !code;
            continue;
        }
        if code
            || line.row < 0
            || line.row >= frame.rows as i32
            || text.starts_with(['>', '❯', '›', '"', '\''])
        {
            continue;
        }
        let adapter_chrome = policy.chrome.is_some_and(|filter| {
            filter(frame, document, composer, line.row, line.start, line.text)
        });
        controls[index] = adapter_chrome;
        let codex_busy = anchor.is_some_and(|anchor| anchor.codex)
            && codex_activity::busy(frame, document, line, composer);
        let area = line.row >= live_last - 2
            || composer.is_some_and(|input| line.row >= input.first_row.saturating_sub(3) as i32)
            || codex_busy;
        if area && owned_start.is_none_or(|start| line.start >= start) {
            let cancellation = cancellation_control(text);
            let bounded_busy = policy.bounded_busy.is_some_and(|marker| {
                marker(line.text) && bounded_busy_source(frame, document, line, composer)
            });
            let busy_boundary = cancellation || bounded_busy || codex_busy;
            controls[index] = adapter_chrome || busy_boundary || footer_metadata(text);
            control_busy |= busy_boundary;
            if let Some(kind) = busy_boundary
                .then_some(SignalKind::Busy)
                .or_else(|| footer_kind(text))
                .or_else(|| {
                    (policy.recorded_duration
                        && recorded_completion(document, line.text, line.start))
                    .then_some(SignalKind::Complete)
                })
            {
                footer = Some(Footer {
                    kind,
                    text: text.into(),
                    key: fingerprint(text),
                });
            }
        }
    }
    let distinct_footer = footer.as_ref().is_some_and(|footer| {
        owned_start.is_some_and(|start| {
            lines.iter().any(|line| {
                line.start < start
                    && fingerprint(line.text.trim()) == footer.key
                    && (footer_kind(line.text).is_some()
                        || recorded_completion(document, line.text, line.start))
            })
        })
    });
    let mut view = View {
        footer,
        distinct_footer,
        control_busy,
        live_status: live_status_keys(document),
        ..View::default()
    };
    let Some(offset) = offset else { return view };
    if policy.panel_responses
        && panel_response(frame, document, &lines, composer, input, anchor, &mut view)
    {
        return view;
    }
    let first = lines.partition_point(|line| line.start < offset);
    let first = first.max(lines.len().saturating_sub(MAX_SOURCE_LINES));
    let mut current = None;
    let mut captured_phase = None;
    let mut codex_tool_block = false;
    let mut pending_blank = 0;
    let mut semantic = context
        || view.control_busy
        || view
            .footer
            .as_ref()
            .is_some_and(|footer| matches!(footer.kind, SignalKind::Busy | SignalKind::Complete));
    let mut bytes = 0;
    let mut code = false;
    echo_end = 0;
    // Scan preceding role/fence structure without copying its text. A changed
    // continuation can start below an unchanged Thinking/Assistant label, and
    // a fence begun before the changed offset still protects code examples.
    for (index, line) in lines.iter().enumerate() {
        if index < echo_end {
            continue;
        }
        let capture = index >= first;
        if capture {
            bytes += line.text.len();
            if bytes > MAX_BYTES {
                break;
            }
        }
        if composer.is_some_and(|input| line.row > input.last_row as i32) {
            // Projection removes the editor rows, not the application's
            // shortcut/status footer below it. That footer is not answer prose.
            current = None;
            captured_phase = None;
            pending_blank = 0;
            continue;
        }
        // Tool stdout is already renderer-prefixed. Its quotes, fences and
        // role-looking text cannot change the assistant-message grammar.
        if codex_tool_block && !codex_activity::outer(line.text) {
            continue;
        }
        let text = line.text.trim_start();
        if fence(text) {
            code = !code;
        }
        let terminal = !code
            && (footer_kind(text).is_some()
                || (policy.recorded_duration
                    && recorded_completion(document, line.text, line.start)));
        let metadata =
            !code && (controls[index] || static_setting(text) || decorative_footer(text));
        let full_codex_echo = (!code && anchor.is_some_and(|anchor| anchor.codex))
            .then(|| codex_request_echo_end(&source, index, input))
            .flatten();
        if terminal || metadata || full_codex_echo.is_some() || (!code && user_echo(text, input)) {
            if let Some(end) = full_codex_echo {
                echo_end = end;
                semantic = true;
            }
            if !code && exact_user_echo(text, input) {
                semantic = true;
            }
            current = None;
            captured_phase = None;
            pending_blank = 0;
            continue;
        }
        let activity = (!code && anchor.is_some_and(|anchor| anchor.codex))
            .then(|| {
                codex_activity::prefix(
                    document,
                    &lines,
                    index,
                    view.control_busy
                        || view
                            .footer
                            .as_ref()
                            .is_some_and(|footer| footer.kind == SignalKind::Busy),
                )
            })
            .flatten();
        let prefix = activity.or_else(|| {
            (!code)
                .then(|| phase_prefix(document, line, semantic, policy.bullet_roles))
                .flatten()
        });
        if let Some((kind, skipped)) = prefix {
            codex_tool_block = activity.is_some();
            current = Some(kind);
            semantic = true;
            captured_phase = None;
            pending_blank = 0;
            if !capture {
                if kind == PhaseKind::Answer
                    && line.row < 0
                    && index >= lines.len().saturating_sub(MAX_SOURCE_LINES)
                {
                    if let Some(absolute) = frame.history_size.checked_add_signed(line.row as isize)
                    {
                        view.retained_headers.insert(absolute);
                    }
                }
                continue;
            }
            if activity.is_some() {
                // Plain assistant commentary before an actual tool call is
                // provisional. The final message follows the last activity
                // cell; tool headers/stdout do not belong to its answer body.
                view.phases.retain(|(kind, _)| *kind != PhaseKind::Answer);
                view.answer_headers.clear();
                view.codex_tool_output = true;
                view.answer_snapshot = true;
            }
            view.phases
                .push((kind, RichText::from_line(document, line, skipped)));
            captured_phase = Some(view.phases.len() - 1);
            if kind == PhaseKind::Answer {
                if let Some(absolute) = frame.history_size.checked_add_signed(line.row as isize) {
                    view.answer_headers.push(AnswerHeader {
                        phase: view.phases.len() - 1,
                        absolute,
                        live: line.row >= 0,
                    });
                }
            }
        } else if let Some(kind) = current {
            if !capture {
                continue;
            }
            if line.text.is_empty() && !code {
                // Empty physical rows between the last body line and footer
                // chrome are layout. Keep paragraph gaps only if another body
                // line follows in the same phase.
                pending_blank += 1;
                continue;
            }
            let part = RichText::from_line(document, line, 0);
            if let Some((_, text)) = captured_phase.and_then(|index| view.phases.get_mut(index)) {
                for _ in 0..pending_blank {
                    text.join(&RichText::default());
                }
                text.join(&part);
            } else {
                view.phases.push((kind, part));
                captured_phase = Some(view.phases.len() - 1);
            }
            pending_blank = 0;
        } else if capture
            && !text.trim().is_empty()
            && !static_setting(text)
            && !spinner(text)
            && !text.starts_with('/')
            && !text.starts_with(['❯', '›'])
            && anchor.is_none_or(|anchor| !anchor.contains(line.text))
        {
            view.fallback.join(&RichText::from_line(document, line, 0));
        }
    }
    view
}

fn bounded_busy_source(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    line: &SourceLine<'_>,
    composer: Option<&Composer>,
) -> bool {
    let Some(input) = composer.filter(|input| {
        editable(Some(input))
            && input.matches_source(frame)
            && crate::direct_accessibility::reading_cursor(frame, None).is_some_and(|cursor| {
                cursor.row > input.first_row
                    && cursor.row < input.last_row
                    && cursor.column > input.first_column
                    && cursor.column < input.last_column
            })
            && line.row >= 0
            && input.first_row.checked_sub(1) == Some(line.row as usize)
            && input.first_column == 1
            && input.last_column.checked_add(2) == Some(frame.cols)
    }) else {
        return false;
    };
    for (row, column, corner) in [
        (input.first_row, input.first_column, "╭"),
        (input.first_row, input.last_column, "╮"),
        (input.last_row, input.first_column, "╰"),
        (input.last_row, input.last_column, "╯"),
    ] {
        if frame
            .cells
            .get(row * frame.cols + column)
            .is_none_or(|cell| cell.text != corner)
        {
            return false;
        }
    }
    // The captured indicator has a separate accent; the Working label uses
    // coherent plain body styling. Color alone elsewhere in prose is no proof.
    let style = |at| {
        document
            .styles
            .iter()
            .find(|span| span.start <= at && span.end > at)
            .map(|span| span.style)
    };
    let indicator = line.start + 2;
    let label = indicator + 2;
    let Some((indicator_style, label_style)) = style(indicator).zip(style(label)) else {
        return false;
    };
    indicator_style.flags.is_empty()
        && label_style.flags.is_empty()
        && indicator_style.foreground != label_style.foreground
        && indicator_style.background == label_style.background
        && (label..label + 8).all(|at| style(at) == Some(label_style))
}

fn panel_text(text: &str) -> Option<&str> {
    let text = text.trim_start();
    text.strip_prefix('┃')
        .or_else(|| text.strip_prefix('│'))
        .map(str::trim)
}

// A full, framed echo of an accepted submission is stronger evidence than a
// model/agent name. Match all panel rows, including wrapped/multiline prompts;
// a matching suffix or a still-editable composer is not a new user message.
fn panel_echo_end(lines: &[&str], start: usize, input: &str) -> Option<usize> {
    if input.trim().is_empty()
        || start == 0
        || panel_text(lines[start - 1]) != Some("")
        || panel_text(lines[start])?.is_empty()
    {
        return None;
    }
    let mut words = Vec::new();
    let mut end = start;
    while let Some(text) = lines.get(end).and_then(|text| panel_text(text)) {
        words.extend(text.split_whitespace());
        if words.len() > MAX_EVENT_BYTES {
            return None;
        }
        end += 1;
    }
    (end > start + 1
        && panel_text(lines[end - 1]) == Some("")
        && words.into_iter().eq(input.split_whitespace()))
    .then_some(end)
}

fn duration_token(text: &str) -> bool {
    fn number(text: &str, unit: &str) -> bool {
        text.strip_suffix(unit).is_some_and(|value| {
            !value.is_empty()
                && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                && value.parse::<f64>().is_ok_and(|value| value.is_finite())
        })
    }
    let parts: Vec<_> = text.split_whitespace().collect();
    match parts.as_slice() {
        [value] => number(value, "ms") || number(value, "s"),
        [first, second] => {
            (number(first, "m") && number(second, "s"))
                || (number(first, "h") && number(second, "m"))
                || (number(first, "d") && number(second, "h"))
        }
        _ => false,
    }
}

// The captured renderer emits elapsed time only after message.time.completed.
// Agent/model strings are deliberately opaque. This grammar is only consulted
// at the end of a response tied to a confirmed source prompt panel, never as a
// global "done" keyword or an instruction to act on terminal text.
fn response_summary(text: &str) -> Option<SignalKind> {
    let mut parts: Vec<_> = text.trim().split('·').map(str::trim).collect();
    let interrupted = parts.last() == Some(&"interrupted");
    if interrupted {
        parts.pop();
    }
    if parts.last().is_some_and(|part| part.ends_with(" tok/s")) {
        let value = parts.pop()?.strip_suffix(" tok/s")?;
        if value.is_empty()
            || !value.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
            || !value.parse::<f64>().is_ok_and(|value| value.is_finite())
        {
            return None;
        }
    }
    (parts.len() == 3
        && !parts[0].is_empty()
        && !parts[0].starts_with(['>', '"', '\'', '┃', '│'])
        && !parts[1].is_empty()
        && duration_token(parts[2]))
    .then_some(if interrupted {
        SignalKind::Interrupted
    } else {
        SignalKind::Complete
    })
}

fn collapsed_thought(text: &str) -> bool {
    let Some(tail) = text.trim().strip_prefix("+ Thought") else {
        return false;
    };
    let tail = tail.trim_start_matches([':', ' ', '·']).trim();
    tail.is_empty() || duration_token(tail.rsplit('·').next().unwrap_or("").trim())
}

fn panel_response(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    lines: &[SourceLine<'_>],
    composer: Option<&Composer>,
    input: &str,
    anchor: Option<&SemanticAnchor>,
    view: &mut View,
) -> bool {
    let Some(boundary) = anchor.and_then(|anchor| anchor.boundary) else {
        return false;
    };
    let Some(composer) = composer else {
        return false;
    };
    let source: Vec<_> = lines.iter().map(|line| line.text).collect();
    let mut code = false;
    let mut start = None;
    for (index, line) in lines.iter().enumerate() {
        if fence(line.text.trim()) {
            code = !code;
        }
        if !code && fingerprint(line.text) == boundary {
            if let Some(end) = panel_echo_end(&source, index, input) {
                start = Some(end);
            }
        }
    }
    let Some(first) = start else { return false };
    if composer.caret.is_none() {
        // A retained field without a current cursor is useful for layout, but
        // can survive an unfinished repaint. Do not archive that mixed page.
        return true;
    }
    let mut code = false;
    let mut end = lines.len();
    for index in first..lines.len() {
        if fence(lines[index].text.trim()) {
            code = !code;
        }
        if lines[index].row >= composer.first_row as i32
            || (!code
                && panel_text(lines[index].text) == Some("")
                && lines
                    .get(index + 1)
                    .is_some_and(|line| panel_text(line.text).is_some()))
        {
            end = index;
            break;
        }
    }
    read_panel_body(frame, document, lines, first..end, None, false, view);
    // The full accepted panel is still visible: this is the whole answer so
    // far, even if prompt wrapping moved its first row since the last frame.
    view.answer_snapshot = !view.phases.is_empty()
        && view
            .phases
            .iter()
            .all(|(kind, _)| *kind == PhaseKind::Answer);
    true
}

fn read_panel_body(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    lines: &[SourceLine<'_>],
    range: std::ops::Range<usize>,
    known_margin: Option<usize>,
    code_at_start: bool,
    view: &mut View,
) {
    let mut first = range.start;
    let mut end = range.end;
    // Only a summary belonging to this body can finish this response. Generic
    // status chrome elsewhere in the viewport does not acquire that ownership.
    view.footer = view
        .footer
        .take()
        .filter(|footer| footer.kind == SignalKind::Busy);
    while first < end && lines[first].text.trim().is_empty() {
        first += 1;
    }
    // Collapsed reasoning is chrome. Expanded reasoning has no reliable text
    // boundary to the answer; leave it unclassified until the source collapses.
    while !code_at_start && first < end && collapsed_thought(lines[first].text) {
        first += 1;
        while first < end && lines[first].text.trim().is_empty() {
            first += 1;
        }
    }
    if !code_at_start && first < end && lines[first].text.trim_start().starts_with("- Thought") {
        return;
    }
    if !code_at_start && first < end {
        let text = lines[first].text.trim_start();
        if text == "Thinking" || text.starts_with("Thinking:") {
            view.phases.push((
                PhaseKind::Thinking,
                RichText::from_line(
                    document,
                    &lines[first],
                    lines[first].text.len() - text.len(),
                ),
            ));
            return;
        }
    }
    while end > first && lines[end - 1].text.trim().is_empty() {
        end -= 1;
    }
    let mut code = code_at_start;
    for line in &lines[first..end] {
        if fence(line.text.trim()) {
            code = !code;
        }
    }
    if end > first && !code {
        let last = lines[end - 1].text.trim();
        if let Some(mut kind) = response_summary(last) {
            // Renderer errors precede the summary. Retain the entire failure
            // as this turn's readable result, while its terminal signal remains
            // Failed even when the renderer records an elapsed duration.
            let mut in_code = code_at_start;
            let mut error = None;
            for (index, line) in lines.iter().enumerate().take(end - 1).skip(first) {
                if fence(line.text.trim()) {
                    in_code = !in_code;
                } else if !in_code && response_error_block(document, lines, index, end - 1) {
                    error = Some(index);
                }
            }
            if error.is_some() {
                kind = SignalKind::Failed;
            }
            view.footer = Some(Footer {
                kind,
                text: error.map_or_else(
                    || last.into(),
                    |first| {
                        lines[first..end - 1]
                            .iter()
                            .map(|line| line.text.trim())
                            .collect::<Vec<_>>()
                            .join("\r\n")
                            .trim_end()
                            .into()
                    },
                ),
                key: fingerprint(last),
            });
            view.live_status.insert(fingerprint(last));
            end -= 1;
        }
    }
    while end > first && lines[end - 1].text.trim().is_empty() {
        end -= 1;
    }
    if first == end {
        return;
    }
    // Remove only the common layout margin, preserving Markdown indentation,
    // list markers, paragraph gaps, source glyph widths and UTF-16 mappings.
    let margin = known_margin.unwrap_or_else(|| {
        lines[first..end]
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .map(|line| line.text.len() - line.text.trim_start_matches(' ').len())
            .min()
            .unwrap_or(0)
    });
    let mut answer = RichText::default();
    for line in &lines[first..end] {
        if answer.text.len() + line.text.len() > MAX_BYTES {
            break;
        }
        answer.join(&RichText::from_line(
            document,
            line,
            margin.min(line.text.len()),
        ));
    }
    view.phases.push((PhaseKind::Answer, answer));
    view.panel_margin = Some(margin);
    if let Some(absolute) = frame
        .history_size
        .checked_add_signed(lines[first].row as isize)
    {
        view.answer_headers.push(AnswerHeader {
            phase: 0,
            absolute,
            live: lines[first].row >= 0,
        });
    }
}

fn response_error_block(
    document: &ReadingDocument,
    lines: &[SourceLine<'_>],
    first: usize,
    summary: usize,
) -> bool {
    use bks_terminal::{Color, NamedColor};

    // The renderer puts failures in a uniformly colored final block, aligned
    // with its summary and separated from that summary by a blank row. Merely
    // mentioning "Error:" in an answer is not a failure signal.
    let header = lines[first].text;
    let margin = header.len() - header.trim_start_matches(' ').len();
    if !header.trim_start().starts_with("Error: ")
        || margin != lines[summary].text.len() - lines[summary].text.trim_start_matches(' ').len()
        || summary <= first + 1
        || !lines[summary - 1].text.trim().is_empty()
    {
        return false;
    }
    let start = lines[first].start + header[..margin].encode_utf16().count() as u32;
    let Some(style) = document
        .styles
        .iter()
        .find(|span| span.start <= start && start < span.end)
        .map(|span| span.style)
    else {
        return false;
    };
    let red = |r: u8, g: u8, b: u8| r.saturating_sub(g) >= 24 && r.saturating_sub(b) >= 12;
    let error_ink = match style.foreground {
        Color::Named(NamedColor::Red | NamedColor::BrightRed | NamedColor::DimRed)
        | Color::Indexed(1 | 9) => true,
        Color::Spec(color) => red(color.r, color.g, color.b),
        Color::Indexed(index @ 16..=231) => {
            let component = |value: u8| if value == 0 { 0 } else { 55 + 40 * value };
            let index = index - 16;
            red(
                component(index / 36),
                component(index / 6 % 6),
                component(index % 6),
            )
        }
        _ => false,
    };
    if !error_ink {
        return false;
    }
    lines[first..summary].iter().all(|line| {
        let text = line.text.trim();
        if text.is_empty() {
            return true;
        }
        let leading = line.text.len() - line.text.trim_start_matches(' ').len();
        if leading < margin || text.starts_with(['>', '❯', '›', '"', '\'']) || fence(text) {
            return false;
        }
        let first = line.start + line.text[..leading].encode_utf16().count() as u32;
        let end = first + text.encode_utf16().count() as u32;
        let styled: u32 = document
            .styles
            .iter()
            .filter(|span| {
                span.style.foreground == style.foreground
                    && !span
                        .style
                        .flags
                        .intersects(Flags::DIM | Flags::ITALIC | Flags::INVERSE)
            })
            .map(|span| span.end.min(end).saturating_sub(span.start.max(first)))
            .sum();
        styled == end - first
    })
}

struct PanelContinuation<'a> {
    previous: &'a RichText,
    margin: usize,
    baseline: &'a HashSet<u64>,
}

fn panel_continuation(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    composer: Option<&Composer>,
    owned: PanelContinuation<'_>,
    view: &mut View,
) {
    // Once the confirmed prompt scrolls out, retain only a source chunk that
    // overlaps this turn's retained body. Never reinterpret the whole viewport
    // (tab title, old responses, toolbars) as an assistant message.
    let mut next = View {
        control_busy: view.control_busy,
        live_status: std::mem::take(&mut view.live_status),
        ..View::default()
    };
    let lines: Vec<_> = document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .map(|(line, text)| SourceLine {
            text,
            start: line.start,
            row: line.line,
        })
        .collect();
    let end = composer
        .filter(|input| input.caret.is_some())
        .map_or(0, |composer| {
            lines.partition_point(|line| line.row < composer.first_row as i32)
        });
    let first = (0..end).find(|&index| {
        lines[index].row >= 0
            && !lines[index].text.trim().is_empty()
            && lines[index]
                .text
                .bytes()
                .take_while(|byte| *byte == b' ')
                .count()
                >= owned.margin
    });
    if let Some(first) = first {
        // A different panel before the composer is a different source region.
        let end = (first..end)
            .find(|&index| {
                !lines[index].text.trim().is_empty()
                    && lines[index]
                        .text
                        .bytes()
                        .take_while(|byte| *byte == b' ')
                        .count()
                        < owned.margin
            })
            .unwrap_or(end);
        let raw = lines[first..end]
            .iter()
            .map(|line| line.text.get(owned.margin..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\r\n");
        if let Some(overlap) = panel_overlap(&owned.previous.text, &raw) {
            let prefix = &owned.previous.text[..owned.previous.text.len() - overlap];
            let code_at_start = prefix
                .split("\r\n")
                .filter(|line| fence(line.trim()))
                .count()
                % 2
                != 0;
            read_panel_body(
                frame,
                document,
                &lines,
                first..end,
                Some(owned.margin),
                code_at_start,
                &mut next,
            );
        }
        let overlap = next
            .phases
            .first()
            .filter(|(kind, _)| *kind == PhaseKind::Answer)
            .and_then(|(_, body)| panel_overlap(&owned.previous.text, &body.text));
        if let Some(overlap) = overlap {
            // A viewport suffix must merge, even when it occupies the old
            // first row. A footer present before submission cannot complete it.
            next.answer_headers.clear();
            next.answer_overlap = Some(overlap);
            if next
                .footer
                .as_ref()
                .is_some_and(|footer| owned.baseline.contains(&footer.key))
            {
                next.footer = None;
            }
            *view = next;
            return;
        }
    }
    view.phases.clear();
    view.answer_headers.clear();
    view.fallback = RichText::default();
    view.footer = None;
}

fn panel_overlap(previous: &str, next: &str) -> Option<usize> {
    // A Unicode viewport can exceed the legacy short-overlap limit. Search
    // complete physical line anchors, then verify the entire suffix exactly;
    // repeated lines alone never supply evidence that two bodies are equal.
    let first_line = next.split("\r\n").next().filter(|line| !line.is_empty())?;
    let normalized_next = next.split_whitespace().collect::<Vec<_>>().join(" ");
    previous
        .match_indices(first_line)
        .map(|(start, _)| previous.len() - start)
        .filter(|&length| length <= next.len().saturating_mul(2))
        .filter(|&length| {
            let tail = &previous[previous.len() - length..];
            let exact =
                length <= next.len() && next.is_char_boundary(length) && tail == &next[..length];
            // During word wrapping a partial last word moves from the end of
            // one physical row to the next. The retained tail must still be a
            // prefix of the new prose with only whitespace differences. Replace
            // that tail from source, instead of appending a duplicate page.
            let code_at_start = previous[..previous.len() - length]
                .split("\r\n")
                .filter(|line| fence(line.trim()))
                .count()
                % 2
                != 0;
            let rewrapped = !code_at_start
                && !tail.split("\r\n").any(|line| fence(line.trim()))
                && normalized_next
                    .starts_with(&tail.split_whitespace().collect::<Vec<_>>().join(" "));
            (exact || rewrapped) && tail.chars().count() >= 32
        })
        .chain(text_overlap(previous, next))
        .filter(|&bytes| previous[previous.len() - bytes..].chars().count() >= 32)
        .max()
}

fn editable(composer: Option<&Composer>) -> bool {
    composer.is_some_and(|input| !input.protected && input.caret.is_some())
}

fn fence(text: &str) -> bool {
    text.starts_with("```") || text.starts_with("~~~")
}

fn exact_user_echo(text: &str, input: &str) -> bool {
    if input.trim().is_empty() {
        return false;
    }
    let text = text.trim();
    text.strip_prefix('❯')
        .or_else(|| {
            text.strip_prefix('›')
                .filter(|echo| echo.starts_with(char::is_whitespace))
        })
        .or_else(|| text.strip_prefix("User:"))
        .or_else(|| text.strip_prefix("Пользователь:"))
        .or_else(|| text.strip_prefix("> "))
        .is_some_and(|echo| echo.trim() == input.trim())
}

/// The accepted request is the exact comparison target. Only Codex's recorded
/// two-column continuation indent and physical word/character wrapping may be
/// removed; a suffix, unordered words or a changed glyph cannot match.
fn codex_request_echo_end(lines: &[&str], start: usize, input: &str) -> Option<usize> {
    let expected = input;
    if expected.trim().is_empty() {
        return None;
    }
    let first = lines.get(start)?.trim_start();
    let first = first.strip_prefix('›')?;
    let first = first
        .strip_prefix(' ')
        .or_else(|| first.strip_prefix('\u{00a0}'))?;
    let mut remaining = expected.strip_prefix(first)?;
    let mut end = start + 1;
    while !remaining.is_empty() && end - start < MAX_SOURCE_LINES {
        let text = *lines.get(end)?;
        if text.is_empty() && !remaining.starts_with(['\r', '\n']) {
            return None;
        }
        let fragment = if text.is_empty() {
            ""
        } else {
            text.strip_prefix("  ")?
        };
        remaining = remaining
            .strip_prefix("\r\n")
            .or_else(|| remaining.strip_prefix('\n'))
            .or_else(|| remaining.strip_prefix(' '))
            .unwrap_or(remaining)
            .strip_prefix(fragment)?;
        end += 1;
    }
    remaining.is_empty().then_some(end)
}

fn user_echo(text: &str, input: &str) -> bool {
    if text.starts_with(['❯', '›'])
        || text.starts_with("User:")
        || text.starts_with("Пользователь:")
    {
        return true;
    }
    !input.is_empty()
        && (text.trim() == input.trim()
            || text
                .strip_prefix("> ")
                .is_some_and(|text| text.trim() == input.trim()))
}

// A retained completion annotation is distinct from prose that happens to say
// "worked" or "done": the source renderer dims the entire typed duration row.
// Its location and stale-status checks are still enforced by parse_view.
fn recorded_completion(document: &ReadingDocument, text: &str, start: u32) -> bool {
    let trimmed = text.trim();
    let Some(value) = trimmed.strip_prefix("Worked for ") else {
        return false;
    };
    let duration = value.split(" • ").next().unwrap_or("");
    let parts: Vec<_> = duration.split_whitespace().collect();
    let hours = matches!(parts.as_slice(), [h, m, s]
        if h.strip_suffix('h').is_some_and(|value| value.parse::<u64>().is_ok())
        && m.strip_suffix('m').is_some_and(|value| value.parse::<u64>().is_ok())
        && s.strip_suffix('s').is_some_and(|value| value.parse::<u64>().is_ok()));
    if duration != "<1s" && !duration_token(duration) && !hours {
        return false;
    }
    let leading = text.len() - text.trim_start().len();
    let first = start + text[..leading].encode_utf16().count() as u32;
    let end = first + trimmed.encode_utf16().count() as u32;
    let dim: u32 = document
        .styles
        .iter()
        .filter(|span| span.style.flags.contains(Flags::DIM))
        .map(|span| span.end.min(end).saturating_sub(span.start.max(first)))
        .sum();
    dim == end - first
}

fn live_status_keys(document: &ReadingDocument) -> HashSet<u64> {
    let mut code = false;
    document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .filter_map(|(line, text)| {
            let trimmed = text.trim();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                code = !code;
                return None;
            }
            (line.line >= 0
                && !code
                && !trimmed.starts_with(['>', '❯', '›', '"', '\''])
                && (footer_kind(trimmed).is_some()
                    || recorded_completion(document, text, line.start)))
            .then(|| fingerprint(trimmed))
        })
        .collect()
}

fn phase_prefix(
    document: &ReadingDocument,
    line: &SourceLine<'_>,
    semantic: bool,
    bullet_roles: bool,
) -> Option<(PhaseKind, usize)> {
    let trimmed = line.text.trim_start();
    let leading = line.text.len() - trimmed.len();
    let lower = trimmed.to_lowercase();
    for (label, kind) in [
        ("assistant:", PhaseKind::Answer),
        ("ответ:", PhaseKind::Answer),
        ("thinking:", PhaseKind::Thinking),
        ("размышление:", PhaseKind::Thinking),
        ("searching:", PhaseKind::Search),
        ("поиск:", PhaseKind::Search),
        ("running:", PhaseKind::Tool),
        ("tool:", PhaseKind::Tool),
        ("инструмент:", PhaseKind::Tool),
    ] {
        if lower.starts_with(label) {
            if kind == PhaseKind::Thinking && static_setting(trimmed) {
                return None;
            }
            let mut skipped = leading + label.len();
            if line.text[skipped..].starts_with(' ') {
                skipped += 1;
            }
            return Some((kind, skipped));
        }
    }
    if bullet_roles
        && trimmed.starts_with(['●', '•'])
        && trimmed.chars().nth(1).is_some_and(char::is_whitespace)
    {
        let skipped =
            leading + trimmed.chars().next()?.len_utf8() + trimmed.chars().nth(1)?.len_utf8();
        let start = line.start + line.text[..skipped].encode_utf16().count() as u32;
        let end = line.start + line.text.encode_utf16().count() as u32;
        let leading_thought = document.styles.iter().any(|span| {
            span.start <= start
                && span.end > start
                && span.style.flags.intersects(Flags::ITALIC | Flags::DIM)
        });
        let styled: u32 = document
            .styles
            .iter()
            .filter(|span| span.style.flags.intersects(Flags::ITALIC | Flags::DIM))
            .map(|span| span.end.min(end).saturating_sub(span.start.max(start)))
            .sum();
        // Inline emphasis in an ordinary answer is not a role change. The
        // paired bullet convention needs coherent styling from its first word.
        let thinking = leading_thought && styled.saturating_mul(2) >= end - start;
        if thinking {
            return Some((PhaseKind::Thinking, skipped));
        }
        if semantic {
            return Some((PhaseKind::Answer, skipped));
        }
    }
    None
}

fn static_setting(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower
        .strip_prefix("thinking:")
        .or_else(|| lower.strip_prefix("размышление:"))
        .is_some_and(|value| {
            matches!(
                value.trim(),
                "high"
                    | "medium"
                    | "low"
                    | "auto"
                    | "on"
                    | "off"
                    | "true"
                    | "false"
                    | "enabled"
                    | "disabled"
                    | "none"
            )
        })
}
fn spinner(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().all(|ch| {
            ('\u{2800}'..='\u{28ff}').contains(&ch)
                || matches!(ch, '|' | '/' | '-' | '\\' | '.' | '…' | ' ')
        })
}
fn footer_kind(text: &str) -> Option<SignalKind> {
    let lower = text.trim_start_matches(['●', '•', ' ']).to_lowercase();
    if [
        "login expired",
        "authentication required",
        "please sign in",
        "please log in",
        "требуется аутентификация",
        "пожалуйста, войдите",
        "необходимо войти",
        "сессия входа истекла",
        "срок действия входа истёк",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return Some(SignalKind::NeedsInput);
    }
    let labelled = lower
        .strip_prefix("status:")
        .or_else(|| lower.strip_prefix("состояние:"));
    if labelled.is_none() {
        let busy = lower.trim_start_matches(|ch: char| {
            ('\u{2800}'..='\u{28ff}').contains(&ch) || matches!(ch, '|' | '/' | '-' | '\\' | ' ')
        });
        if [
            "thinking",
            "searching",
            "working",
            "running",
            "processing",
            "размышление",
            "поиск",
            "выполнение",
        ]
        .iter()
        .any(|word| {
            busy.strip_prefix(word).is_some_and(|rest| {
                let rest = rest.trim_start();
                rest.starts_with("...") || rest.starts_with('…') || rest.starts_with("(esc ")
            })
        }) {
            return Some(SignalKind::Busy);
        }
    }
    let value = labelled?.trim();
    if matches!(
        value,
        "busy"
            | "working"
            | "running"
            | "thinking"
            | "searching"
            | "processing"
            | "выполняется"
            | "поиск"
            | "размышление"
    ) {
        return Some(SignalKind::Busy);
    }
    if matches!(
        value,
        "done" | "complete" | "completed" | "finished" | "готово" | "завершено"
    ) {
        return Some(SignalKind::Complete);
    }
    if value.starts_with("error") || value.starts_with("failed") || value.starts_with("ошибка")
    {
        return Some(SignalKind::Failed);
    }
    if value.starts_with("interrupted")
        || value.starts_with("cancelled")
        || value.starts_with("canceled")
        || value.starts_with("прервано")
        || value.starts_with("отменено")
    {
        return Some(SignalKind::Interrupted);
    }
    if value.starts_with("waiting")
        || value.starts_with("needs input")
        || value.starts_with("approval")
        || value.starts_with("awaiting")
        || value.starts_with("ожидание")
        || value.starts_with("подтверждение")
    {
        return Some(SignalKind::NeedsInput);
    }
    None
}

fn decorative_footer(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.starts_with("footer:")
        || lower.strip_prefix("baked for ").is_some_and(|value| {
            value
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, ' ' | '.' | 's'))
        })
}

fn cancellation_control(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    for hint in [
        "ctrl+c: cancel",
        "ctrl+c to cancel",
        "ctrl+c: interrupt",
        "ctrl+c to interrupt",
        "esc to interrupt",
        "esc to cancel",
        "esc: interrupt",
        "esc: cancel",
        "escape to interrupt",
        "escape to cancel",
        "ctrl+c: отмена",
        "ctrl+c: прервать",
        "esc: отмена",
        "esc для отмены",
    ] {
        for (at, _) in lower.match_indices(hint) {
            let before = &lower[..at];
            let after = lower[at + hint.len()..].trim_start();
            // A control occupies its own footer segment. A body sentence such
            // as "use ctrl+c: cancel in this example" is not structural UI.
            let separate = before.is_empty()
                || before.ends_with("  ")
                || before.ends_with(['(', '[', '│', '|', '·', '•'])
                || before.trim_end().ends_with(['│', '|', '·', '•']);
            let ending = after.is_empty()
                || after.starts_with([')', ']', '│', '|', '·', '•'])
                || after.starts_with("context=")
                || after.starts_with("context:");
            if separate && ending {
                return true;
            }
        }
    }
    false
}

fn footer_metadata(text: &str) -> bool {
    if static_setting(text) || decorative_footer(text) {
        return true;
    }
    let lower = text.trim().to_lowercase();
    if lower
        .strip_prefix("context:")
        .or_else(|| lower.strip_prefix("context="))
        .or_else(|| lower.strip_prefix("context "))
        .is_some_and(|value| {
            value.ends_with('%')
                && value[..value.len() - 1]
                    .trim()
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '.')
        })
    {
        return true;
    }
    // Generic configuration/status chrome, independently of model, path, or
    // process names. A single body occurrence of "thinking: high" is retained.
    let setting = lower.split_once("thinking:").is_some_and(|(_, tail)| {
        let value = tail
            .trim_start()
            .split(|ch: char| !ch.is_alphanumeric())
            .next()
            .unwrap_or("");
        static_setting(&format!("thinking: {value}"))
    });
    setting
        && (lower.contains(['·', '│', '|'])
            || (lower.starts_with('(') && lower.ends_with(')'))
            || lower.contains("model:")
            || lower.contains("cwd")
            || lower.contains("branch"))
}

impl RichText {
    fn signature(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut hash);
        self.glyphs.hash(&mut hash);
        for span in &self.styles {
            span.start.hash(&mut hash);
            span.end.hash(&mut hash);
            color_key(span.style.foreground).hash(&mut hash);
            color_key(span.style.background).hash(&mut hash);
            span.style.flags.bits().hash(&mut hash);
        }
        hash.finish()
    }
    fn units(&self) -> u32 {
        self.length
    }
    fn plain(text: &str) -> Self {
        let mut rich = Self {
            text: text.into(),
            ..Self::default()
        };
        let mut offset = 0;
        for ch in text.chars() {
            let end = offset + ch.len_utf16() as u32;
            if !matches!(ch, '\r' | '\n') {
                rich.glyphs.push(Glyph {
                    start: offset,
                    end,
                    width: 1,
                });
            }
            offset = end;
        }
        StyleSpan::append(&mut rich.styles, 0, offset, TextStyle::default());
        rich.length = offset;
        rich
    }
    fn from_line(document: &ReadingDocument, line: &SourceLine<'_>, skipped: usize) -> Self {
        let text = &line.text[skipped..];
        let start = line.start + line.text[..skipped].encode_utf16().count() as u32;
        let end = start + text.encode_utf16().count() as u32;
        let mut rich = Self {
            text: text.into(),
            length: end - start,
            ..Self::default()
        };
        let first = document.cells.partition_point(|cell| cell.end <= start);
        for cell in document.cells[first..]
            .iter()
            .take_while(|cell| cell.start < end)
        {
            if cell.start >= start && cell.end <= end {
                rich.glyphs.push(Glyph {
                    start: cell.start - start,
                    end: cell.end - start,
                    width: cell.width,
                });
            }
        }
        let first = document.styles.partition_point(|span| span.end <= start);
        for span in document.styles[first..]
            .iter()
            .take_while(|span| span.start < end)
        {
            StyleSpan::append(
                &mut rich.styles,
                span.start.max(start) - start,
                span.end.min(end) - start,
                span.style,
            );
        }
        rich
    }
    fn join(&mut self, other: &Self) {
        if self.text.is_empty() {
            *self = other.clone();
            return;
        }
        let before = self.units();
        self.text.push_str("\r\n");
        self.length += 2;
        StyleSpan::append(&mut self.styles, before, before + 2, TextStyle::default());
        self.append(other);
    }
    fn append(&mut self, other: &Self) {
        let base = self.units();
        self.text.push_str(&other.text);
        self.length += other.length;
        for span in &other.styles {
            StyleSpan::append(
                &mut self.styles,
                base + span.start,
                base + span.end,
                span.style,
            );
        }
        self.glyphs.extend(other.glyphs.iter().map(|glyph| Glyph {
            start: base + glyph.start,
            end: base + glyph.end,
            ..*glyph
        }));
    }
    fn suffix(&self, start: u32) -> Self {
        let mut units = 0;
        let byte = self
            .text
            .char_indices()
            .find_map(|(byte, ch)| {
                let found = (units == start).then_some(byte);
                units += ch.len_utf16() as u32;
                found
            })
            .unwrap_or(self.text.len());
        let mut rich = Self {
            text: self.text[byte..].into(),
            length: self.length - start,
            ..Self::default()
        };
        for span in &self.styles {
            if span.end > start {
                StyleSpan::append(
                    &mut rich.styles,
                    span.start.saturating_sub(start),
                    span.end - start,
                    span.style,
                );
            }
        }
        rich.glyphs.extend(
            self.glyphs
                .iter()
                .filter(|glyph| glyph.start >= start)
                .map(|glyph| Glyph {
                    start: glyph.start - start,
                    end: glyph.end - start,
                    ..*glyph
                }),
        );
        rich
    }
    fn merge(&mut self, next: Self) -> bool {
        if self.text == next.text {
            let changed = *self != next;
            *self = next;
            return changed;
        }
        if self.text.is_empty() || next.text.starts_with(&self.text) {
            *self = next;
            return true;
        }
        if self.text.starts_with(&next.text)
            || self.text.ends_with(&next.text)
            || self.text.split("\r\n").any(|line| line == next.text)
        {
            return false;
        }
        let overlap = text_overlap(&self.text, &next.text).filter(|bytes| {
            let start = next.text[..*bytes].encode_utf16().count() as u32;
            next.glyphs
                .iter()
                .any(|glyph| glyph.start == start || glyph.end == start)
        });
        if let Some(bytes) = overlap {
            self.append(&next.suffix(next.text[..bytes].encode_utf16().count() as u32));
        } else {
            self.join(&next);
        }
        true
    }
    fn merge_suffix(&mut self, next: Self, overlap: usize) -> bool {
        let before = self.signature();
        let byte = self.text.len() - overlap;
        let units = self.text[..byte].encode_utf16().count() as u32;
        self.text.truncate(byte);
        self.length = units;
        self.glyphs.retain(|glyph| glyph.end <= units);
        self.styles.retain(|span| span.start < units);
        if let Some(last) = self.styles.last_mut() {
            last.end = last.end.min(units);
        }
        self.append(&next);
        self.signature() != before
    }
    fn keep_tail(&mut self, bytes: usize) {
        self.keep_tail_with_notice(bytes, "[Начало ответа удалено]");
    }
    fn keep_tail_with_notice(&mut self, bytes: usize, notice: &str) {
        if self.text.len() <= bytes {
            return;
        }
        let mut byte = self.text.len() - bytes;
        while !self.text.is_char_boundary(byte) {
            byte += 1;
        }
        let target = self.text[..byte].encode_utf16().count() as u32;
        let start = self
            .glyphs
            .iter()
            .find(|glyph| glyph.start >= target)
            .map_or(self.units(), |glyph| glyph.start);
        let tail = self.suffix(start);
        *self = Self::plain(notice);
        self.join(&tail);
    }
    fn document(&self, revision: u64) -> ReadingDocument {
        let mut document = empty_document(revision);
        document.text = self.text.clone();
        document.styles = self.styles.clone();
        if self.text.is_empty() {
            return document;
        }
        let mut start = 0;
        for (index, text) in self.text.split("\r\n").enumerate() {
            let end = start + text.encode_utf16().count() as u32;
            document.lines.push(ReadingLine {
                start,
                end,
                line: index as i32,
            });
            let mut column = 0;
            let first = self.glyphs.partition_point(|glyph| glyph.end <= start);
            for glyph in self.glyphs[first..]
                .iter()
                .take_while(|glyph| glyph.start < end)
            {
                document.cells.push(ReadingCell {
                    start: glyph.start,
                    end: glyph.end,
                    line: index as i32,
                    column,
                    width: glyph.width,
                });
                column += glyph.width;
            }
            start = end + 2;
        }
        document
    }
}

fn color_key(color: bks_terminal::Color) -> (u8, u32) {
    match color {
        bks_terminal::Color::Named(color) => (0, color as u32),
        bks_terminal::Color::Indexed(index) => (1, index as u32),
        bks_terminal::Color::Spec(color) => (
            2,
            ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32,
        ),
    }
}

fn text_overlap(previous: &str, next: &str) -> Option<usize> {
    let max = previous.len().min(next.len()).min(4_096);
    (8..=max).rev().find(|&length| {
        previous.is_char_boundary(previous.len() - length)
            && next.is_char_boundary(length)
            && previous[previous.len() - length..] == next[..length]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{Color, NamedColor, TerminalModel};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn kimi_bullets_keep_unowned_lists_quotes_code_and_changed_markers_unclassified() {
        use crate::tui_adapters::AdapterKind;
        for body in [
            "● unowned plain list item",
            "❯ another request\r\n● unowned response",
            "❯ q\r\n> ● quoted role",
            "❯ q\r\n```text\r\n● role inside code\r\n```",
            "❯ q\r\n○ changed marker",
            "❯ q\r\nAn inline ● is ordinary prose.",
        ] {
            let baseline = scene("", true);
            let source = scene(body, true);
            let now = Instant::now();
            let mut log = TurnLog::new();
            log.set_adapter(AdapterKind::Kimi);
            log.submitted("q", &baseline.output, now);
            for time in [now, now + Duration::from_secs(1)] {
                log.observe_for_adapter(
                    AdapterKind::Kimi,
                    &source.frame,
                    &source.output,
                    source.input.as_ref(),
                    false,
                    time,
                );
            }
            assert!(!log.has_answers(), "{body}");
            assert_eq!(ready_events(&log.take_events()), 0, "{body}");
            assert!(source.output.text.contains(body));
        }
    }

    #[test]
    fn captured_kimi_working_boundary_requires_live_composer_geometry_and_styles() {
        use crate::tui_adapters::AdapterKind;
        let bytes = include_bytes!("../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.vt");
        let capture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.json"
        ))
        .unwrap();
        let mut model = TerminalModel::new(100, 29);
        let mut begin = 0;
        for chunk in capture["chunks"].as_array().unwrap() {
            let end = chunk["end_byte"].as_u64().unwrap() as usize;
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let mut frame = model.live_snapshot();
            frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
            let document = model.tui_reading_document();
            let projection = crate::tui_projection::project_for_adapter(
                AdapterKind::Kimi,
                &frame,
                &document,
                None,
                None,
            );
            let Some(input) = projection.input else {
                continue;
            };
            let Some((line, text)) =
                document
                    .lines
                    .iter()
                    .zip(document.text.split("\r\n"))
                    .find(|(line, text)| {
                        line.line >= 0
                            && AdapterKind::Kimi.policy().semantics.bounded_busy.unwrap()(text)
                    })
            else {
                continue;
            };
            let source_line = SourceLine {
                text,
                start: line.start,
                row: line.line,
            };
            assert!(bounded_busy_source(
                &frame,
                &document,
                &source_line,
                Some(&input)
            ));
            assert!(!bounded_busy_source(&frame, &document, &source_line, None));
            let mut protected = input.clone();
            protected.protected = true;
            assert!(!bounded_busy_source(
                &frame,
                &document,
                &source_line,
                Some(&protected)
            ));
            let mut hidden = frame.clone();
            hidden.cursor = None;
            for cell in &mut hidden.cells {
                cell.flags.remove(Flags::INVERSE);
            }
            assert!(!bounded_busy_source(
                &hidden,
                &document,
                &source_line,
                Some(&input)
            ));
            let mut foreign = frame.clone();
            foreign.alternate_screen = !foreign.alternate_screen;
            assert!(!bounded_busy_source(
                &foreign,
                &document,
                &source_line,
                Some(&input)
            ));
            let mut shifted = input.clone();
            shifted.first_row -= 1;
            assert!(!bounded_busy_source(
                &frame,
                &document,
                &source_line,
                Some(&shifted)
            ));
            let mut plain = document.clone();
            for span in &mut plain.styles {
                span.style = TextStyle::default();
            }
            assert!(!bounded_busy_source(
                &frame,
                &plain,
                &source_line,
                Some(&input)
            ));
            let mut broken_border = frame.clone();
            broken_border.cells[input.first_row * frame.cols + input.first_column].text =
                "+".into();
            assert!(!bounded_busy_source(
                &broken_border,
                &document,
                &source_line,
                Some(&input)
            ));
            return;
        }
        panic!("recorded Kimi working boundary was not found");
    }

    #[test]
    fn captured_kimi_two_answers_keep_source_boundaries_and_navigation() {
        replay_kimi_capture(
            include_bytes!("../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.vt"),
            include_str!("../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.json"),
            false,
        );
    }

    #[test]
    fn captured_kimi_repeated_request_queued_while_editable_keeps_two_answer_ids() {
        replay_kimi_capture(
            include_bytes!("../tests/fixtures/tui-kimi-2.4.1-two-answers/queued-repeat.vt"),
            include_str!("../tests/fixtures/tui-kimi-2.4.1-two-answers/queued-repeat.json"),
            true,
        );
    }

    fn replay_kimi_capture(bytes: &[u8], metadata: &str, queued_repeat: bool) {
        use crate::{
            source_input::SourceInput,
            tui_adapters::AdapterKind,
            tui_projection::{project_for_adapter, project_recovered_for_adapter, ComposerHistory},
        };
        #[derive(serde::Deserialize)]
        struct Phase {
            name: String,
            end_byte: usize,
        }
        #[derive(serde::Deserialize)]
        struct Chunk {
            phase: String,
            end_byte: usize,
            elapsed_ms: u64,
        }
        #[derive(serde::Deserialize)]
        struct Capture {
            columns: usize,
            rows: usize,
            phases: Vec<Phase>,
            chunks: Vec<Chunk>,
        }
        let capture: Capture = serde_json::from_str(metadata).unwrap();
        let mut model = TerminalModel::new(capture.columns, capture.rows);
        let mut log = TurnLog::new();
        let mut source = SourceInput::default();
        let mut history = ComposerHistory::new();
        let mut previous = None;
        let mut baseline = model.tui_reading_document();
        let mut begin = 0;
        let mut targets = Vec::new();
        let mut events = Vec::new();
        let now = Instant::now();
        for phase in &capture.phases {
            let chunks: Vec<_> = capture
                .chunks
                .iter()
                .filter(|chunk| chunk.phase == phase.name)
                .collect();
            let phase_time = now + Duration::from_millis(chunks[0].elapsed_ms.saturating_sub(1));
            let draft = match phase.name.as_str() {
                "draft-one" => Some("audit-one"),
                "draft-two" => Some(if queued_repeat {
                    "audit-one"
                } else {
                    "audit-two"
                }),
                _ => None,
            };
            if let Some(input) = draft {
                source.accepted(
                    &model.live_snapshot(),
                    model.reading_revision(),
                    input.as_bytes(),
                    previous.as_ref(),
                    phase_time,
                );
            }
            let accepted = match phase.name.as_str() {
                "answer-one" => Some("audit-one"),
                "answer-two" => Some(if queued_repeat {
                    "audit-one"
                } else {
                    "audit-two"
                }),
                _ => None,
            };
            if let Some(input) = accepted {
                log.submitted(input, &baseline, phase_time);
                source.clear();
            }
            for chunk in &chunks {
                model.push_bytes(&bytes[begin..chunk.end_byte]);
                begin = chunk.end_byte;
                let time = now + Duration::from_millis(chunk.elapsed_ms);
                let mut frame = model.live_snapshot();
                frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                let raw = model.tui_reading_document();
                let document = history.filter(&frame, &raw);
                let evidence = source.confirmation(&frame, model.reading_revision(), time);
                let mut projection = project_for_adapter(
                    AdapterKind::Kimi,
                    &frame,
                    &document,
                    previous.as_ref(),
                    evidence.map(|(evidence, _)| evidence),
                );
                if projection.input.is_none() {
                    if let Some((evidence, Some(baseline))) = evidence {
                        projection = project_recovered_for_adapter(
                            AdapterKind::Kimi,
                            &frame,
                            &document,
                            baseline,
                            evidence,
                        );
                    }
                }
                log.observe_for_adapter(
                    AdapterKind::Kimi,
                    &frame,
                    &projection.output,
                    projection.input.as_ref(),
                    false,
                    time,
                );
                events.extend(log.take_events());
                if let Some(input) = projection.input.as_ref() {
                    source.acknowledge(input);
                    history.remember(&frame, &raw, input);
                }
                previous = projection.input;
                baseline = projection.output;
            }
            assert_eq!(begin, phase.end_byte);
            let composer = previous
                .as_ref()
                .expect("real source composer remains available");
            assert_eq!(composer.text, draft.unwrap_or(""));
            assert_eq!(
                composer.caret,
                Some(draft.map_or(0, |text| text.len() as u32))
            );
            if accepted.is_some() {
                let waiting_in_stream = queued_repeat && phase.name == "answer-one";
                let time = now
                    + Duration::from_millis(chunks.last().unwrap().elapsed_ms)
                    + if waiting_in_stream {
                        Duration::ZERO
                    } else {
                        COMPLETION_STABLE_FOR + Duration::from_millis(1)
                    };
                log.observe_for_adapter(
                    AdapterKind::Kimi,
                    &model.live_snapshot(),
                    &baseline,
                    previous.as_ref(),
                    false,
                    time,
                );
                events.extend(log.take_events());
                let target = log
                    .latest_answer()
                    .expect("captured ● response must be an answer");
                assert!(
                    target.complete != waiting_in_stream && !target.failed,
                    "{}; control={:?}; source={}",
                    log.status(),
                    log.active.as_ref().map(|active| (
                        active.seen_busy,
                        active.input_unavailable,
                        active.busy_gone,
                        active.seen_control,
                        active.control_gone,
                        active.control_returned,
                        active.was_busy,
                    )),
                    baseline.text
                );
                assert_eq!(log.has_active_turn(), waiting_in_stream);
                targets.push(target);
            }
        }
        assert_eq!(begin, bytes.len());
        assert_eq!(targets.len(), 2);
        assert!(targets[0].id < targets[1].id);
        assert_eq!(log.navigate(None, false).unwrap().id, targets[0].id);
        assert_eq!(log.navigate(None, true), Some(targets[1]));
        assert_eq!(
            log.navigate(Some(targets[1].id), true).unwrap().id,
            targets[0].id
        );
        assert_eq!(log.navigate(Some(targets[0].id), false), Some(targets[1]));
        for (target, expected) in targets.iter().zip([
            "Kimi audit answer one.\r\n\r\n   First retained result.",
            "Kimi audit answer two.\r\n\r\n   Second retained result.",
        ]) {
            assert_eq!(log.answer_text(target.id), Some(expected));
            let first_line = expected.split("\r\n").next().unwrap();
            let source_byte = baseline.text.find(first_line).unwrap();
            let source_offset = baseline.text[..source_byte].encode_utf16().count() as u32;
            let source_style = baseline
                .styles
                .iter()
                .find(|span| span.start <= source_offset && span.end > source_offset)
                .unwrap()
                .style;
            let archive = log.document();
            let archived_style = archive
                .styles
                .iter()
                .find(|span| span.start <= target.start && span.end > target.start)
                .unwrap()
                .style;
            assert_eq!(archived_style, source_style);
        }
        assert_eq!(
            ready_events(&events),
            if queued_repeat { 1 } else { 2 },
            "{events:?}"
        );
        let unclassified: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::NewOutput { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            unclassified,
            if queued_repeat {
                vec!["   ↑ to edit · ctrl-s to steer immediately"]
            } else {
                Vec::new()
            },
            "{events:?}"
        );
        valid_document(&log.document());
    }

    pub(super) struct Scene {
        frame: TerminalSnapshot,
        pub(super) output: ReadingDocument,
        input: Option<Composer>,
    }

    pub(super) fn scene(text: &str, composer: bool) -> Scene {
        scene_columns(text, composer, 80)
    }

    fn scene_columns(text: &str, composer: bool, columns: usize) -> Scene {
        scene_size(text, composer, columns, 20)
    }

    fn scene_size(text: &str, composer: bool, columns: usize, rows: usize) -> Scene {
        let mut model = TerminalModel::new(columns, rows);
        model.push_bytes(text.as_bytes());
        if composer {
            let row = text.split("\r\n").count() + 1;
            model.push_bytes(b"\r\n+--------+\r\n| >      |\r\n+--------+");
            model.push_bytes(format!("\x1b[{};5H", row + 1).as_bytes());
        }
        let frame = model.live_snapshot();
        let document = model.reading_document();
        let projection = crate::tui_projection::project(&frame, &document, None);
        let mut output = if composer {
            projection.output
        } else {
            document
        };
        output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        Scene {
            frame,
            output,
            input: projection.input,
        }
    }

    fn footer_scene(body: &str, hint: Option<&str>, composer: bool, columns: usize) -> Scene {
        let mut model = TerminalModel::new(columns, 24);
        model.push_bytes(body.as_bytes());
        model.push_bytes(b"\x1b[18;1H(model: local \xc2\xb7 thinking: high)");
        if let Some(hint) = hint {
            model.push_bytes(format!("\x1b[18;{}H{hint}", columns - 18).as_bytes());
        }
        model.push_bytes(b"\x1b[19;1Hcontext: 2%");
        if composer {
            model.push_bytes(b"\x1b[21;1H+--------+\r\n| >      |\r\n+--------+\x1b[22;5H");
        }
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let projection = crate::tui_projection::project(&frame, &raw, None);
        let mut output = projection.output;
        output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        assert_eq!(projection.input.is_some(), composer);
        Scene {
            frame,
            output,
            input: projection.input,
        }
    }

    fn observe(log: &mut TurnLog, scene: &Scene, now: Instant) {
        log.observe(
            &scene.frame,
            &scene.output,
            scene.input.as_ref(),
            false,
            now,
        );
    }

    pub(super) fn settle(log: &mut TurnLog, scene: &Scene, now: Instant) {
        observe(log, scene, now);
        observe(
            log,
            scene,
            now + COMPLETION_STABLE_FOR + Duration::from_millis(1),
        );
    }

    fn ready_events(events: &[TurnEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, TurnEvent::Ready { .. }))
            .count()
    }

    fn complete_turn(
        log: &mut TurnLog,
        baseline: &ReadingDocument,
        answer: &str,
        now: Instant,
    ) -> Scene {
        log.submitted("synthetic request", baseline, now);
        let busy = scene("Thinking: synthetic reasoning\r\nStatus: busy", true);
        settle(log, &busy, now);
        let answer = scene(&format!("Assistant: {answer}\r\nStatus: done"), true);
        settle(log, &answer, now + Duration::from_secs(1));
        answer
    }

    pub(super) fn valid_document(document: &ReadingDocument) {
        let units: Vec<_> = document.text.encode_utf16().collect();
        let mut covered = vec![false; units.len()];
        for cell in &document.cells {
            assert!(cell.start < cell.end && cell.end as usize <= units.len());
            assert!(String::from_utf16(&units[cell.start as usize..cell.end as usize]).is_ok());
            for unit in &mut covered[cell.start as usize..cell.end as usize] {
                *unit = true;
            }
        }
        for (index, unit) in units.iter().enumerate() {
            assert!(covered[index] || matches!(*unit, 13 | 10));
        }
        for line in &document.lines {
            assert!(line.start <= line.end && line.end as usize <= units.len());
        }
        for pair in document.styles.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        if !units.is_empty() {
            assert_eq!(document.styles.first().unwrap().start, 0);
            assert_eq!(document.styles.last().unwrap().end as usize, units.len());
        }
    }

    fn physical_scene(model: &TerminalModel) -> Scene {
        let frame = model.live_snapshot();
        let projection =
            crate::tui_projection::project(&frame, &model.tui_reading_document(), None);
        let mut output = projection.output;
        output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        Scene {
            frame,
            output,
            input: projection.input,
        }
    }

    #[test]
    fn retained_composer_does_not_archive_a_hidden_cursor_repaint() {
        // The captured renderer has a synchronized intermediate update at
        // byte 52801 with a hidden cursor and a partly overwritten body. The
        // production projection correctly retains the old field for layout;
        // that caret-less field must not grant new body ownership.
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-long-scroll.vt");
        let mut model = TerminalModel::new(110, 36);
        let mut history = crate::tui_projection::ComposerHistory::new();
        let mut menus = crate::tui_suggestions::MenuObserver::default();
        let mut previous = None;
        let mut projected = |model: &TerminalModel| {
            let original = model.live_snapshot();
            let mut frame = original.clone();
            frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
            let raw = model.tui_reading_document();
            let filtered = history.filter(&frame, &raw);
            let projection = crate::tui_projection::project(&frame, &filtered, previous.as_ref());
            previous = projection.input;
            if let Some(input) = &previous {
                history.remember(&frame, &raw, input);
            }
            let menu = menus.observe(
                &frame,
                model.reading_revision(),
                previous
                    .as_ref()
                    .map(|input| (input.first_row, input.last_row)),
            );
            let mut output = projection.output;
            if let Some(menu) = menu {
                history.remember_rows(&frame, &raw, menu.first_row, menu.last_row);
                output =
                    crate::tui_projection::exclude_rows(&output, menu.first_row, menu.last_row);
            }
            output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
            Scene {
                frame: original,
                output,
                input: previous.clone(),
            }
        };
        let now = Instant::now();
        let mut begin = 12_435;
        model.push_bytes(&bytes[..begin]);
        let mut pure = TurnLog::new();
        let mut actual = TurnLog::new();
        pure.submitted(
            "Привет, кто ты и что умеешь?",
            &physical_scene(&model).output,
            now,
        );
        actual.submitted(
            "Привет, кто ты и что умеешь?",
            &projected(&model).output,
            now,
        );
        let marker = b"\x1b[?2026l";
        for (index, end) in bytes
            .windows(marker.len())
            .enumerate()
            .filter(|(_, part)| *part == marker)
            .map(|(offset, _)| offset + marker.len())
            .filter(|&end| end > 12_435)
            .enumerate()
        {
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let expected = physical_scene(&model);
            let source = projected(&model);
            if end == 52_801 {
                assert!(expected.input.is_none());
                assert!(source
                    .input
                    .as_ref()
                    .is_some_and(|input| input.caret.is_none()));
                assert!(source.output.text.contains("MARKER_034 Ж🙂оверяем"));
            }
            observe(&mut pure, &expected, now);
            observe(&mut actual, &source, now);
            let body = |log: &TurnLog| {
                log.latest_answer()
                    .and_then(|target| log.answer_text(target.id))
                    .unwrap_or("")
                    .to_owned()
            };
            assert_eq!(
                body(&actual),
                body(&pure),
                "archive frame {index} endpoint {end}"
            );
        }
        let completed = projected(&model);
        settle(&mut actual, &completed, now + Duration::from_secs(60));
        let answer = actual.latest_answer().unwrap();
        assert!(answer.complete, "{}", actual.status());
        for number in 1..=120 {
            assert_eq!(
                actual
                    .answer_text(answer.id)
                    .unwrap()
                    .matches(&format!("MARKER_{number:03} Ж🙂."))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn paired_renderer_source_fixtures_keep_two_answers_and_real_composer_boundaries() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            columns: usize,
            rows: usize,
            phase_ends: Vec<usize>,
            accepted_inputs: Vec<String>,
        }
        // Formats are taken from exact installed/versioned renderer source;
        // these fixtures do not claim an authenticated model session capture.
        for (bytes, metadata) in [
            (
                &include_bytes!("../tests/fixtures/tui-turns-paired-renderers/codex-two-turns.vt")
                    [..],
                include_str!("../tests/fixtures/tui-turns-paired-renderers/codex-two-turns.json"),
            ),
            (
                &include_bytes!("../tests/fixtures/tui-turns-paired-renderers/claude-two-turns.vt")
                    [..],
                include_str!("../tests/fixtures/tui-turns-paired-renderers/claude-two-turns.json"),
            ),
        ] {
            let spec: Fixture = serde_json::from_str(metadata).unwrap();
            let mut model = TerminalModel::new(spec.columns, spec.rows);
            let mut begin = spec.phase_ends[0];
            model.push_bytes(&bytes[..begin]);
            let mut baseline = physical_scene(&model);
            assert!(baseline.input.is_some());
            let mut log = TurnLog::new();
            let mut ids = Vec::new();
            let now = Instant::now();
            for turn in 0..2 {
                let time = now + Duration::from_secs(turn as u64 * 5);
                log.submitted(&spec.accepted_inputs[turn], &baseline.output, time);
                for phase in 0..2 {
                    let end = spec.phase_ends[turn * 2 + phase + 1];
                    model.push_bytes(&bytes[begin..end]);
                    begin = end;
                    let next = physical_scene(&model);
                    let input = next.input.as_ref().expect("source-equivalent composer");
                    assert!(input.caret.is_some() && !input.protected);
                    assert!(!next.output.lines.iter().any(|line| {
                        (input.first_row as i32..=input.last_row as i32).contains(&line.line)
                    }));
                    settle(&mut log, &next, time + Duration::from_secs(phase as u64));
                    let events = log.take_events();
                    assert_eq!(ready_events(&events), phase, "{events:?}");
                    let target = log.latest_answer().unwrap();
                    if phase == 0 {
                        ids.push(target.id);
                        assert!(!target.complete);
                        assert!(events.iter().any(|event| matches!(
                            event,
                            TurnEvent::Phase {
                                kind: PhaseKind::Answer,
                                ..
                            }
                        )));
                        assert!(
                            !events.iter().any(|event| matches!(
                                event,
                                TurnEvent::Phase {
                                    kind: PhaseKind::Thinking,
                                    ..
                                }
                            )),
                            "a dim bullet alone is not reasoning"
                        );
                    } else {
                        assert_eq!(target.id, ids[turn]);
                        assert!(target.complete);
                        let body = log.answer_text(target.id).unwrap();
                        assert!(body.contains("Ж🙂."));
                        for chrome in [
                            "Worked for",
                            "Working (",
                            "esc to interrupt",
                            "? for shortcuts",
                            "accepted question",
                            "─",
                        ] {
                            assert!(!body.contains(chrome), "{body}");
                        }
                    }
                    baseline = next;
                }
            }
            let first = log.navigate(None, false).unwrap();
            let last = log.latest_answer().unwrap();
            assert_eq!([first.id, last.id], ids.as_slice());
            assert_eq!(last.total, 2);
            assert_eq!(log.navigate(Some(first.id), false), Some(last));
            assert_eq!(log.navigate(Some(last.id), true), Some(first));
            assert_eq!(
                log.answer_text(first.id),
                Some("First source answer Ж🙂.\r\n  Preserved paragraph and 1. item.")
            );
            assert_eq!(
                log.answer_text(last.id),
                Some("Second source answer Ж🙂.\r\n  Preserved paragraph and 2. item.")
            );
            valid_document(&log.document());
        }
    }

    #[test]
    fn thin_prompt_marker_and_nbsp_require_an_exact_accepted_echo() {
        for separator in [' ', '\u{00a0}'] {
            assert!(exact_user_echo(
                &format!("›{separator}accepted Ж🙂"),
                "accepted Ж🙂"
            ));
            assert!(!exact_user_echo(
                &format!("›{separator}other"),
                "accepted Ж🙂"
            ));
            assert!(!exact_user_echo(
                &format!("> ›{separator}accepted Ж🙂"),
                "accepted Ж🙂"
            ));
        }
        assert!(!exact_user_echo("›accepted", "accepted"));
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("accepted", &baseline.output, now);
        let unaccepted = scene("› unrelated\r\n• unclassified body", true);
        settle(&mut log, &unaccepted, now);
        assert!(!log.has_answers());
        let quoted = scene("› Status: done\r\n› Working (1s • esc to interrupt)", true);
        settle(&mut log, &quoted, now + Duration::from_secs(1));
        assert_eq!(ready_events(&log.take_events()), 0);
        assert!(!log.view.control_busy && log.view.footer.is_none());
    }

    #[test]
    fn recorded_duration_needs_source_style_and_a_fresh_unquoted_boundary() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (body, completed) in [
            ("Worked for 2s", false),
            ("\x1b[2mWorked for forever\x1b[0m", false),
            ("\x1b[2m> Worked for 2s\x1b[0m", false),
            ("```\r\n\x1b[2mWorked for 2s\x1b[0m\r\n```", false),
            ("\x1b[2mWorked for <1s • 20:45\x1b[0m", true),
            ("\x1b[2mWorked for 1h 2m 3s • 20:45\x1b[0m", true),
        ] {
            let mut log = TurnLog::new();
            log.submitted("q", &baseline.output, now);
            let frame = scene(&format!("› q\r\n• answer\r\n{body}"), true);
            settle(&mut log, &frame, now);
            assert_eq!(
                ready_events(&log.take_events()),
                usize::from(completed),
                "{body}"
            );
        }
        let baseline = scene("\x1b[2mWorked for 2s • 20:45\x1b[0m", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let stale = scene(
            "› q\r\n• new body\r\n\x1b[2mWorked for 2s • 20:45\x1b[0m",
            true,
        );
        settle(&mut log, &stale, now);
        assert!(log.has_answers());
        assert_eq!(ready_events(&log.take_events()), 0);
    }

    #[test]
    fn actual_unlabelled_two_turn_capture_retains_only_answers_with_stable_navigation() {
        // Byte-exact OpenCode 2.0.32 renderer output from two explicit fixture
        // submissions to an isolated loopback provider, no real model/network.
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-two-answers.vt");
        let now = Instant::now();
        let mut model = TerminalModel::new(110, 36);
        model.push_bytes(&bytes[..13_400]);
        let mut baseline = physical_scene(&model);
        let mut log = TurnLog::new();
        let mut previous_end = 13_400;
        let mut ids = Vec::new();
        for (index, (prompt, endpoints, selected)) in [
            (
                "Привет, кто ты и что умеешь?",
                [16_241, 21_524, 78_185],
                "Ж🙂",
            ),
            (
                "Понял. На какой модели ты работаешь?",
                [81_591, 93_551, 121_923],
                "DeepSeek V4.1 Flash Free.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let turn_time = now + Duration::from_secs(index as u64 * 10);
            log.submitted(prompt, &baseline.output, turn_time);
            let mut streaming_id = None;
            for (phase, end) in endpoints.into_iter().enumerate() {
                model.push_bytes(&bytes[previous_end..end]);
                previous_end = end;
                let frame = physical_scene(&model);
                assert!(frame.input.is_some(), "actual composer at phase {phase}");
                settle(
                    &mut log,
                    &frame,
                    turn_time + Duration::from_secs(phase as u64),
                );
                let events = log.take_events();
                assert_eq!(ready_events(&events), usize::from(phase == 2), "{events:?}");
                if phase == 1 {
                    streaming_id = log.latest_answer().map(|target| target.id);
                    assert!(streaming_id.is_some());
                }
                if phase == 2 {
                    let target = log.latest_answer().unwrap();
                    assert_eq!(Some(target.id), streaming_id);
                    assert!(target.complete);
                    assert_eq!(target.total, index + 1);
                    ids.push(target.id);
                    let answer = log.answer_text(target.id).unwrap();
                    assert!(answer.contains(selected), "{answer}");
                    for chrome in [
                        prompt,
                        "┃",
                        "Build ·",
                        "tok/s",
                        "New session",
                        "ctrl+p",
                        "cwd",
                    ] {
                        assert!(!answer.contains(chrome), "unexpected {chrome}: {answer}");
                    }
                    let before = log.document();
                    let revision = log.revision();
                    // An unchanged source repaint cannot append a body, emit a
                    // second Ready, or invalidate the reader's answer offsets.
                    let repaint = physical_scene(&model);
                    settle(&mut log, &repaint, turn_time + Duration::from_secs(4));
                    assert_eq!(log.revision(), revision);
                    assert_eq!(log.document().as_ref(), before.as_ref());
                    assert!(log.take_events().is_empty());
                }
                baseline = frame;
            }
        }
        let first = log.navigate(None, false).unwrap();
        let second = log.latest_answer().unwrap();
        assert_eq!([first.id, second.id], ids.as_slice());
        assert_eq!(log.navigate(Some(second.id), true), Some(first));
        assert_eq!(log.navigate(Some(first.id), false), Some(second));
        let first_text = log.answer_text(first.id).unwrap();
        assert!(first_text.contains("Где я сейчас\r\n\r\n"));
        assert!(first_text.contains("- Объясняю сложные вещи простыми словами."));
        assert!(first_text.contains("Длинная строка для проверки переноса:"));
        assert!(first_text.contains("знаков препинания или Unicode-символов."));
        assert_eq!(first_text.matches("Привет! Я помощник").count(), 1);
        assert!(!log
            .answer_text(second.id)
            .unwrap()
            .contains("Привет! Я помощник"));
        let archive = log.document();
        valid_document(&archive);
        assert!(archive
            .styles
            .iter()
            .any(|span| span.style.flags.contains(Flags::BOLD)));
        assert!(archive.cells.iter().any(|cell| cell.width == 2));
        for target in [first, second] {
            let units: Vec<_> = archive.text.encode_utf16().collect();
            let rest = String::from_utf16(&units[target.start as usize..]).unwrap();
            assert!(rest.starts_with(log.answer_text(target.id).unwrap()));
        }
    }

    #[test]
    fn actual_retry_then_two_errors_remain_readable_and_terminal_in_archive() {
        // Installed OpenCode 2.0.32 at 100x29: two equal accepted prompts,
        // loopback HTTP 503 twice followed by a permanent HTTP 403 failure.
        // Replay every renderer commit, including its temporary failure toast.
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-errors.vt");
        let mut model = TerminalModel::new(100, 29);
        let mut history = crate::tui_projection::ComposerHistory::new();
        let mut menus = crate::tui_suggestions::MenuObserver::default();
        let mut previous = None;
        let mut project =
            |model: &TerminalModel,
             accepted: Option<&crate::tui_projection::AcceptedSourceEvidence>| {
                let original = model.live_snapshot();
                let mut frame = original.clone();
                frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                let raw = model.tui_reading_document();
                let filtered = history.filter(&frame, &raw);
                let projection = crate::tui_projection::project_confirmed(
                    &frame,
                    &filtered,
                    previous.as_ref(),
                    accepted,
                );
                previous = projection.input;
                if let Some(input) = &previous {
                    history.remember(&frame, &raw, input);
                }
                let menu = menus.observe(
                    &frame,
                    model.reading_revision(),
                    previous
                        .as_ref()
                        .map(|input| (input.first_row, input.last_row)),
                );
                let mut output = projection.output;
                if let Some(menu) = menu.as_ref() {
                    history.remember_rows(&frame, &raw, menu.first_row, menu.last_row);
                    output =
                        crate::tui_projection::exclude_rows(&output, menu.first_row, menu.last_row);
                }
                output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
                (
                    Scene {
                        frame: original,
                        output,
                        input: previous.clone(),
                    },
                    menu.is_some(),
                )
            };
        let now = Instant::now();
        let mut log = TurnLog::new();
        let mut begin = 0;
        let mut ids = Vec::new();
        let marker = b"\x1b[?2026l";
        let expected = "Error: Free usage limit reached. Free models work in OpenCode (https://opencode.ai);\r\nsubscribe to OpenCode Go to keep going: https://opencode.ai/go";
        for (turn, (empty, submitted, completed)) in
            [(11_067, 11_138, 126_426), (126_426, 126_492, 131_129)]
                .into_iter()
                .enumerate()
        {
            model.push_bytes(&bytes[begin..empty]);
            let (empty_source, _) = project(&model, None);
            assert!(empty_source.input.is_some());
            let cursor = empty_source.frame.cursor.unwrap();
            let accepted = crate::tui_projection::AcceptedSourceEvidence {
                text: "привет".into(),
                row: cursor.row,
                column: cursor.column,
                before_revision: model.reading_revision(),
            };
            begin = empty;
            model.push_bytes(&bytes[begin..submitted]);
            begin = submitted;
            let (baseline, menu) = project(&model, Some(&accepted));
            assert!(!menu);
            assert!(baseline.input.is_some());
            let at = now + Duration::from_secs(turn as u64 * 20);
            log.submitted("привет", &baseline.output, at);
            let mut failures = 0;
            let mut ready = 0;
            let ends = bytes[..completed]
                .windows(marker.len())
                .enumerate()
                .filter(|(_, part)| *part == marker)
                .map(|(offset, _)| offset + marker.len())
                .filter(|&end| end > submitted);
            for (index, end) in ends.enumerate() {
                model.push_bytes(&bytes[begin..end]);
                begin = end;
                let (source, menu) = project(&model, None);
                log.observe(
                    &source.frame,
                    &source.output,
                    source.input.as_ref(),
                    menu,
                    at + Duration::from_millis(index as u64 * 16),
                );
                let events = log.take_events();
                ready += ready_events(&events);
                failures += events
                    .iter()
                    .filter(|event| matches!(event, TurnEvent::Failed { .. }))
                    .count();
            }
            model.push_bytes(&bytes[begin..completed]);
            begin = completed;
            let (source, menu) = project(&model, None);
            assert!(!menu);
            settle(&mut log, &source, at + Duration::from_secs(15));
            let events = log.take_events();
            ready += ready_events(&events);
            failures += events
                .iter()
                .filter(|event| matches!(event, TurnEvent::Failed { .. }))
                .count();
            assert_eq!(ready, 0, "turn {turn}: {}", log.document().text);
            assert_eq!(failures, 1, "turn {turn}: {}", log.document().text);
            let target = log.latest_answer().unwrap();
            assert!(target.failed && !target.complete);
            assert_eq!(target.total, turn + 1);
            assert_eq!(log.answer_text(target.id), Some(expected));
            assert!(!log.has_active_turn());
            assert_eq!(log.status(), "Ошибка TUI");
            ids.push(target.id);
        }
        assert_ne!(ids[0], ids[1]);
        let archive = log.document();
        assert_eq!(
            archive
                .text
                .matches("Error: Free usage limit reached.")
                .count(),
            2
        );
        for chrome in [
            "ECONNRESET",
            "Retry",
            "Build ·",
            "Session failed",
            "New session",
        ] {
            assert!(!archive.text.contains(chrome), "{chrome}: {}", archive.text);
        }
        valid_document(&archive);
    }

    #[test]
    fn actual_long_response_survives_prompt_scrolling_out_with_every_marker_once() {
        // Prefix through complete1 of the isolated 120-paragraph loopback
        // capture. Replay renderer frames, including the early frames before
        // the prompt disappears; coarse 350 ms snapshots miss those frames.
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-long-scroll.vt");
        let now = Instant::now();
        let mut model = TerminalModel::new(110, 36);
        let mut begin = 12_435;
        model.push_bytes(&bytes[..begin]);
        let mut log = TurnLog::new();
        log.submitted(
            "Привет, кто ты и что умеешь?",
            &physical_scene(&model).output,
            now,
        );
        let marker = b"\x1b[?2026l";
        let ends: Vec<_> = bytes
            .windows(marker.len())
            .enumerate()
            .filter(|(_, part)| *part == marker)
            .map(|(offset, _)| offset + marker.len())
            .filter(|&end| end > begin)
            .collect();
        let mut ready = 0;
        let mut answer_id = None;
        for (index, end) in ends.into_iter().enumerate() {
            model.push_bytes(&bytes[begin..end]);
            begin = end;
            let source = physical_scene(&model);
            observe(
                &mut log,
                &source,
                now + Duration::from_millis(index as u64 * 16),
            );
            if let Some(target) = log.latest_answer() {
                if let Some(id) = answer_id {
                    assert_eq!(target.id, id);
                } else {
                    answer_id = Some(target.id);
                }
            }
            ready += ready_events(&log.take_events());
        }
        model.push_bytes(&bytes[begin..]);
        let completed = physical_scene(&model);
        assert!(!completed
            .output
            .text
            .contains("Привет, кто ты и что умеешь?"));
        settle(&mut log, &completed, now + Duration::from_secs(60));
        ready += ready_events(&log.take_events());
        if ready != 1 {
            let body = log
                .latest_answer()
                .and_then(|target| log.answer_text(target.id))
                .unwrap_or("");
            eprintln!(
                "retained {} chars; tail {:?}; panel {:?}; footer {:?}; final {:?}",
                body.chars().count(),
                body.chars()
                    .skip(body.chars().count().saturating_sub(220))
                    .collect::<String>(),
                log.active.as_ref().and_then(|active| active.panel_margin),
                log.view.footer.as_ref().map(|footer| footer.kind),
                completed.output.text.lines().take(8).collect::<Vec<_>>()
            );
        }
        assert_eq!(ready, 1, "{}", log.status());
        let target = log.latest_answer().unwrap();
        assert_eq!(target.id, answer_id.unwrap());
        assert!(target.complete);
        let answer = log.answer_text(target.id).unwrap();
        assert!(
            answer.starts_with("Длинный фиксированный ответ"),
            "{answer}"
        );
        assert!(answer.ends_with("КОНЕЦ ДЛИННОГО ОТВЕТА — все 120 отмеченных строк завершены."));
        let mut previous = 0;
        for number in 1..=120 {
            let marker = format!("MARKER_{number:03} Ж🙂.");
            assert_eq!(answer.matches(&marker).count(), 1, "{marker}: {answer}");
            let at = answer.find(&marker).unwrap();
            assert!(at > previous);
            previous = at;
        }
        for chrome in [
            "New session",
            "┃",
            "Build ·",
            "tok/s",
            "ctrl+p",
            "esc interrupt",
        ] {
            assert!(!answer.contains(chrome), "{chrome}");
        }
        valid_document(&log.document());
        assert!(log.document().cells.iter().any(|cell| cell.width == 2));
    }

    #[test]
    fn scrolled_panel_requires_owned_body_and_a_fresh_summary() {
        let now = Instant::now();
        let baseline = scene("     Agent · Model · 1.2s", true);
        let mut log = TurnLog::new();
        log.submitted("request", &baseline.output, now);
        let tail = "This retained paragraph identifies the owned response Ж🙂.";
        let first = scene(
            &format!("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n     First paragraph.\r\n     {tail}"),
            true,
        );
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        let original = log.answer_text(id).unwrap().to_owned();
        log.take_events();
        let unrelated = scene(
            " • Other viewport\r\n     A different response without an owned source overlap.\r\n\r\n     Agent · Model · 9.9s",
            true,
        );
        settle(&mut log, &unrelated, now + Duration::from_secs(1));
        assert_eq!(log.answer_text(id), Some(original.as_str()));
        assert!(log.take_events().is_empty());
        for (index, summary) in ["Agent · Model · 1.2s", "Agent · Model · 2.3s"]
            .into_iter()
            .enumerate()
        {
            let next = scene(
                &format!(
                    " • New session\r\n     {tail}\r\n\r\n     New final paragraph.\r\n\r\n     {summary}"
                ),
                true,
            );
            settle(&mut log, &next, now + Duration::from_secs(2 + index as u64));
            assert_eq!(log.latest_answer().unwrap().id, id);
            assert_eq!(ready_events(&log.take_events()), index);
        }
        assert_eq!(
            log.answer_text(id),
            Some(format!("{original}\r\n\r\nNew final paragraph.").as_str())
        );
        valid_document(&log.document());
    }

    #[test]
    fn scrolled_code_keeps_a_literal_summary_until_its_source_fence_closes() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("request", &baseline.output, now);
        let code = "An owned code example with at least thirty two characters.";
        let first = scene(
            &format!("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n     ```text\r\n     {code}"),
            true,
        );
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        log.take_events();
        let still_code = scene(
            &format!(" • New session\r\n     {code}\r\n     Agent · Model · 1.2s"),
            true,
        );
        settle(&mut log, &still_code, now + Duration::from_secs(1));
        assert_eq!(ready_events(&log.take_events()), 0);
        assert!(log
            .answer_text(id)
            .unwrap()
            .ends_with("Agent · Model · 1.2s"));
        let done = scene(
            &format!(
                " • New session\r\n     {code}\r\n     Agent · Model · 1.2s\r\n     ```\r\n\r\n     Agent · Model · 2.3s"
            ),
            true,
        );
        settle(&mut log, &done, now + Duration::from_secs(2));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert!(log
            .answer_text(id)
            .unwrap()
            .ends_with("Agent · Model · 1.2s\r\n```"));
        valid_document(&log.document());
    }

    #[test]
    fn accepted_multiline_panel_preserves_markdown_but_excludes_thought_and_summary() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted(
            "Explain these two things:\nfirst and second",
            &baseline.output,
            now,
        );
        let thinking = scene(
            "  ┃\r\n  ┃  Explain these two things:\r\n  ┃  first and second\r\n  ┃\r\n\r\n     Thinking: local reasoning",
            true,
        );
        settle(&mut log, &thinking, now);
        assert!(!log.has_answers());
        assert!(log.take_events().iter().any(|event| matches!(
            event,
            TurnEvent::Phase {
                kind: PhaseKind::Thinking,
                ..
            }
        )));
        let answer = scene(
            "  ┃\r\n  ┃  Explain these two things:\r\n  ┃  first and second\r\n  ┃\r\n\r\n     + Thought: local reasoning · 886ms\r\n\r\n     \x1b[1mDetails\x1b[0m\r\n\r\n     1. First numbered paragraph.\r\n     2. Second numbered paragraph.\r\n        - Nested bullet Ж🙂.\r\n\r\n     Any agent · Any model · 1m 2s · 75.5 tok/s",
            true,
        );
        settle(&mut log, &answer, now + Duration::from_secs(1));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some(
                "Details\r\n\r\n1. First numbered paragraph.\r\n2. Second numbered paragraph.\r\n   - Nested bullet Ж🙂."
            )
        );
        valid_document(&log.document());
    }

    #[test]
    fn panel_duration_requires_accepted_full_echo_and_an_unquoted_response_boundary() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (prompt, body, footer, ready, answers) in [
            (
                "different",
                "plain answer",
                "Agent · Model · 1.2s",
                false,
                false,
            ),
            ("request", "plain answer", "Agent · Model", false, true),
            (
                "request",
                "plain answer",
                "> Agent · Model · 1.2s",
                false,
                true,
            ),
            (
                "request",
                "```text\r\nAgent · Model · 1.2s",
                "```",
                false,
                true,
            ),
            (
                "request",
                "plain answer",
                "Agent · Model · nope · 5 tok/s",
                false,
                true,
            ),
            (
                "request",
                "plain answer",
                "Agent · Model · 1.2s",
                true,
                true,
            ),
        ] {
            let mut log = TurnLog::new();
            log.submitted(prompt, &baseline.output, now);
            let frame = scene(
                &format!("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n{body}\r\n\r\n{footer}"),
                true,
            );
            settle(&mut log, &frame, now);
            assert_eq!(log.has_answers(), answers, "{prompt}: {body}: {footer}");
            assert_eq!(
                ready_events(&log.take_events()),
                usize::from(ready),
                "{footer}"
            );
        }
    }

    #[test]
    fn panel_response_keeps_fenced_gutters_and_never_completes_errors_or_interruptions() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (body, footer, expected, retained) in [
            (
                "partial answer",
                "Agent · Model · 1.2s · interrupted",
                SignalKind::Interrupted,
                "partial answer",
            ),
            (
                "partial answer\r\n\r\n\x1b[38;2;240;113;120mError: fixture failure\r\nwrapped failure details\x1b[0m",
                "Agent · Model · 1.2s",
                SignalKind::Failed,
                "partial answer\r\n\r\nError: fixture failure\r\nwrapped failure details",
            ),
        ] {
            let mut log = TurnLog::new();
            log.submitted("request", &baseline.output, now);
            let frame = scene(
                &format!("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n{body}\r\n\r\n{footer}"),
                true,
            );
            settle(&mut log, &frame, now);
            let events = log.take_events();
            assert_eq!(ready_events(&events), 0, "{events:?}");
            assert!(events.iter().any(|event| matches!(
                (expected, event),
                (SignalKind::Interrupted, TurnEvent::Interrupted { .. })
                    | (SignalKind::Failed, TurnEvent::Failed { .. })
            )));
            assert_eq!(
                log.answer_text(log.latest_answer().unwrap().id),
                Some(retained)
            );
            assert!(!log.latest_answer().unwrap().complete);
            assert_eq!(
                log.latest_answer().unwrap().failed,
                expected == SignalKind::Failed
            );
        }
        let mut log = TurnLog::new();
        log.submitted("request", &baseline.output, now);
        let frame = scene(
            "  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n```text\r\n┃\r\n┃ a quoted panel\r\n┃\r\n```\r\nAfter code.\r\n\r\nAgent · Model · 1.2s",
            true,
        );
        settle(&mut log, &frame, now);
        assert_eq!(ready_events(&log.take_events()), 1);
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("```text\r\n┃\r\n┃ a quoted panel\r\n┃\r\n```\r\nAfter code.")
        );
    }

    #[test]
    fn final_panel_errors_replace_retry_and_keep_separate_failed_answers() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("привет", &baseline.output, now);
        let prompt = "  ┃\r\n  ┃  привет\r\n  ┃\r\n\r\n";
        let retry = scene(
            &format!("{prompt}     \x1b[33m⚠ Retry due · attempt 2 · ECONNRESET\x1b[0m\r\n\r\n     Agent · First model"),
            true,
        );
        settle(&mut log, &retry, now);
        let first = log.latest_answer().unwrap();
        assert!(log.answer_text(first.id).unwrap().contains("Retry due"));
        assert!(!first.complete && !first.failed);
        assert!(log.has_active_turn());
        log.take_events();

        // Source-equivalent Wc element from the installed renderer: a uniformly
        // colored wrapped error block, blank row, then a completion duration.
        let error = "Error: Free usage limit reached. Free models work in OpenCode\r\n(https://opencode.ai); subscribe: https://opencode.ai/go Ж🙂";
        let source = format!(
            "{prompt}\x1b[38;2;240;113;120m{error}\x1b[0m\r\n\r\nAgent · First model · 3.0s"
        );
        let final_first = scene(&source, true);
        settle(&mut log, &final_first, now + Duration::from_secs(1));
        assert_eq!(log.answer_text(first.id), Some(error));
        assert!(log.latest_answer().unwrap().failed);
        assert!(!log.has_active_turn());
        assert_eq!(log.status(), "Ошибка TUI");
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert!(events.iter().any(|event| matches!(
            event, TurnEvent::Failed { id, text } if *id == first.id && text == error
        )));

        // A model switch and a repeated accepted prompt have their own source
        // boundary. Neither the old error nor its retry may take that turn.
        log.submitted("привет", &final_first.output, now + Duration::from_secs(2));
        let second_source = format!(
            "{source}\r\n\r\nSwitched model to Second model\r\n\r\n{prompt}\x1b[38;2;240;113;120m{error}\x1b[0m\r\n\r\nAgent · Second model · 660ms"
        );
        let final_second = scene_size(&second_source, true, 100, 29);
        assert!(final_second.input.is_some());
        settle(&mut log, &final_second, now + Duration::from_secs(3));
        let second = log.latest_answer().unwrap();
        assert_ne!(
            second.id,
            first.id,
            "{}\nstatus={}",
            final_second.output.text,
            log.status()
        );
        assert!(second.failed && !second.complete);
        assert_eq!(second.total, 2);
        assert_eq!(log.answer_text(first.id), Some(error));
        assert_eq!(log.answer_text(second.id), Some(error));
        assert_eq!(log.navigate(Some(second.id), true).unwrap().id, first.id);
        let archive = log.document();
        assert_eq!(archive.text.matches("Error: Free usage").count(), 2);
        for chrome in [
            "Retry due",
            "ECONNRESET",
            "Agent",
            "First model",
            "Second model",
            "Switched",
        ] {
            assert!(
                !archive.text.contains(chrome),
                "unexpected {chrome}: {}",
                archive.text
            );
        }
        assert!(archive.styles.iter().any(|span| matches!(
            span.style.foreground,
            bks_terminal::Color::Spec(rgb) if (rgb.r, rgb.g, rgb.b) == (240, 113, 120)
        )));
        assert!(archive.cells.iter().any(|cell| cell.width == 2));
        valid_document(&archive);
        log.take_events();
        let revision = log.revision();
        settle(&mut log, &final_second, now + Duration::from_secs(4));
        assert_eq!(log.revision(), revision);
        assert!(log.take_events().is_empty());
        let erased = scene("", true);
        settle(&mut log, &erased, now + Duration::from_secs(5));
        assert_eq!(log.document(), archive);
    }

    #[test]
    fn colored_error_needs_accepted_body_and_terminal_source_summary() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (prompt, body) in [
            (
                "different request",
                "\x1b[31mError: unrelated\x1b[0m\r\n\r\nAgent · Model · 1.2s",
            ),
            ("request", "\x1b[31mError: still streaming\x1b[0m"),
            ("request", "> \x1b[31mError: quoted\x1b[0m"),
            ("request", "```text\r\n\x1b[31mError: literal\x1b[0m\r\n```"),
            ("request", "\x1b[31mError:\x1b[0m highlighted label"),
        ] {
            let mut log = TurnLog::new();
            log.submitted("request", &baseline.output, now);
            let source = scene(&format!("  ┃\r\n  ┃  {prompt}\r\n  ┃\r\n\r\n{body}"), true);
            settle(&mut log, &source, now);
            observe(&mut log, &source, now + Duration::from_secs(30));
            let events = log.take_events();
            assert_eq!(ready_events(&events), 0, "{prompt}: {body}");
            assert!(!events
                .iter()
                .any(|event| matches!(event, TurnEvent::Failed { .. })));
            assert!(log.has_active_turn());
            assert!(log
                .latest_answer()
                .is_none_or(|answer| !answer.failed && !answer.complete));
        }
    }

    #[test]
    fn repeated_echo_can_precede_unchanged_source_footer_but_not_old_history() {
        let initial = "❯ q\r\n● original body\x1b[18;1Hunchanged application footer";
        let baseline = scene(initial, true);
        let anchor = SemanticAnchor::submitted(&baseline.output);
        let inserted = scene(
            "❯ q\r\n● original body\r\n❯ q\r\n● new body\x1b[18;1Hunchanged application footer",
            true,
        );
        assert!(anchor.fresh_echo(&inserted.output, "q").is_some());
        let repeated = scene(
            "❯ q\r\n● original body\r\n❯ q\r\n● original body\x1b[18;1Hunchanged application footer",
            true,
        );
        assert!(anchor.fresh_echo(&repeated.output, "q").is_some());
        let empty = scene(
            "❯ q\r\n● original body\r\n❯ q\x1b[18;1Hunchanged application footer",
            true,
        );
        assert_eq!(anchor.fresh_echo(&empty.output, "q"), None);
        let displaced = scene(
            "❯ q\r\n● original body\r\n❯ q\r\n● new body\x1b[19;1Hunchanged application footer",
            true,
        );
        assert_eq!(anchor.fresh_echo(&displaced.output, "q"), None);
        let mut model = TerminalModel::new(80, 20);
        model.push_bytes(initial.as_bytes());
        model.push_bytes(
            format!(
                "\x1b[20;1H{}\x1b[2J\x1b[H❯ q\r\n● new body\x1b[18;1Hunchanged application footer",
                "\r\n".repeat(20)
            )
            .as_bytes(),
        );
        assert_eq!(anchor.fresh_echo(&model.reading_document(), "q"), None);
    }

    #[test]
    fn panel_error_prose_quotes_and_code_remain_complete_answer_text() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (source, expected) in [
            (
                "Причина:\r\n\r\nError: connection refused\r\n\r\nИсправление: проверьте адрес.",
                "Причина:\r\n\r\nError: connection refused\r\n\r\nИсправление: проверьте адрес.",
            ),
            (
                "Последняя строка примера:\r\nError: connection refused",
                "Последняя строка примера:\r\nError: connection refused",
            ),
            (
                "```text\r\n\x1b[38;2;240;113;120mError: connection refused\x1b[0m\r\n```",
                "```text\r\nError: connection refused\r\n```",
            ),
            (
                "> \x1b[38;2;240;113;120mError: quoted failure\x1b[0m",
                "> Error: quoted failure",
            ),
            (
                "\x1b[38;2;240;113;120mError: red example\x1b[0m\r\n\r\nThe answer continues.",
                "Error: red example\r\n\r\nThe answer continues.",
            ),
            (
                "\x1b[31mError:\x1b[0m a highlighted label",
                "Error: a highlighted label",
            ),
            (
                "An indented example:\r\n  \x1b[31mError: indentation\x1b[0m",
                "An indented example:\r\n  Error: indentation",
            ),
        ] {
            let mut log = TurnLog::new();
            log.submitted("request", &baseline.output, now);
            let frame = scene(
                &format!("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n{source}\r\n\r\nAgent · Model · 1.2s"),
                true,
            );
            settle(&mut log, &frame, now);
            let events = log.take_events();
            assert_eq!(ready_events(&events), 1, "{expected}: {events:?}");
            assert!(!events
                .iter()
                .any(|event| matches!(event, TurnEvent::Failed { .. })));
            let answer = log.latest_answer().unwrap();
            assert!(answer.complete);
            assert_eq!(log.answer_text(answer.id), Some(expected));
        }
    }

    #[test]
    fn accepted_panel_reflow_corrects_the_same_answer_without_appending_old_layout() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("Explain first and second topics", &baseline.output, now);
        let first = scene(
            "  ┃\r\n  ┃  Explain first and second topics\r\n  ┃\r\n\r\n     First paragraph.\r\n     Old body.\r\n\r\n     Agent · Model",
            true,
        );
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        assert_eq!(ready_events(&log.take_events()), 0);
        let next = scene(
            "  ┃\r\n  ┃  Explain first and\r\n  ┃  second topics\r\n  ┃\r\n\r\n     First paragraph.\r\n     Corrected body Ж🙂.\r\n\r\n     Agent · Model · 1.2s",
            true,
        );
        settle(&mut log, &next, now + Duration::from_secs(1));
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert_eq!(ready_events(&log.take_events()), 1);
        assert_eq!(
            log.answer_text(id),
            Some("First paragraph.\r\nCorrected body Ж🙂.")
        );
        valid_document(&log.document());
    }

    #[test]
    fn streaming_phases_make_one_styled_answer_and_ready_requires_source_completion() {
        let now = Instant::now();
        let baseline = scene("thinking: high", true);
        let mut log = TurnLog::new();
        log.submitted("unique request", &baseline.output, now);
        assert!(log.has_active_turn() && !log.has_answers());
        assert!(log.take_events().is_empty());
        for (index, text) in [
            "Thinking: analyze the input",
            "Searching: visible documents",
            "Running: synthetic work",
        ]
        .into_iter()
        .enumerate()
        {
            let busy = scene(&format!("{text}\r\nStatus: busy"), false);
            settle(&mut log, &busy, now + Duration::from_secs(index as u64));
            assert!(!log.has_answers());
        }
        let first = scene(
            "Assistant: \x1b[31;1mЖ🙂e\u{301}\x1b[0m first\r\nStatus: busy",
            false,
        );
        settle(&mut log, &first, now + Duration::from_secs(3));
        let id = log.latest_answer().unwrap().id;
        let grown = scene(
            "Assistant: \x1b[31;1mЖ🙂e\u{301}\x1b[0m first line\r\nsecond paragraph\r\nStatus: busy",
            false,
        );
        settle(&mut log, &grown, now + Duration::from_secs(4));
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert_eq!(
            log.answer_text(id),
            Some("Ж🙂e\u{301} first line\r\nsecond paragraph")
        );
        observe(&mut log, &grown, now + Duration::from_secs(20));
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        for kind in [
            PhaseKind::Request,
            PhaseKind::Thinking,
            PhaseKind::Search,
            PhaseKind::Tool,
            PhaseKind::Answer,
        ] {
            assert!(events.iter().any(|event| matches!(event, TurnEvent::Phase {kind: observed, ..} if *observed == kind)), "{kind:?}");
        }
        let done = scene(
            "Assistant: \x1b[31;1mЖ🙂e\u{301}\x1b[0m first line\r\nsecond paragraph\r\nStatus: done",
            true,
        );
        settle(&mut log, &done, now + Duration::from_secs(21));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert!(log.latest_answer().unwrap().complete);
        assert!(!log.has_active_turn());
        let archive = log.document();
        assert!(archive
            .text
            .starts_with("Ваш запрос, свёрнуто: unique request\r\nОтвет 1\r\nЖ🙂"));
        for unwanted in ["analyze", "documents", "synthetic work", "Status:"] {
            assert!(!archive.text.contains(unwanted));
        }
        assert!(archive.styles.iter().any(|span| span.style.foreground
            == Color::Named(NamedColor::Red)
            && span.style.flags.contains(Flags::BOLD)));
        valid_document(&archive);
        observe(&mut log, &done, now + Duration::from_secs(30));
        observe(&mut log, &done, now + Duration::from_secs(31));
        assert!(log.take_events().is_empty());
    }

    #[test]
    fn time_pause_composer_and_done_word_alone_do_not_finish() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let answer = scene("Assistant: still streaming\r\nDone", true);
        settle(&mut log, &answer, now);
        observe(&mut log, &answer, now + Duration::from_secs(10_000));
        assert_eq!(ready_events(&log.take_events()), 0);
        assert!(log.has_active_turn());
        assert!(!log.latest_answer().unwrap().complete);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let busy = scene("Assistant: a partial answer\r\nStatus: busy", false);
        settle(&mut log, &busy, now);
        let gone = scene("Assistant: a partial answer", true);
        observe(&mut log, &gone, now + Duration::from_secs(1));
        observe(&mut log, &gone, now + Duration::from_millis(1_200));
        assert_eq!(ready_events(&log.take_events()), 0);
        observe(&mut log, &busy, now + Duration::from_millis(1_250));
        observe(&mut log, &busy, now + Duration::from_secs(2));
        assert_eq!(ready_events(&log.take_events()), 0);
        settle(&mut log, &gone, now + Duration::from_secs(3));
        assert_eq!(ready_events(&log.take_events()), 1);
    }

    #[test]
    fn generic_live_busy_footer_can_finish_without_a_program_specific_done_word() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let busy = scene("Thinking: inspect input\r\n⠋ Thinking...", false);
        settle(&mut log, &busy, now);
        let answer = scene("Assistant: visible answer\r\n⠙ Working...", false);
        settle(&mut log, &answer, now + Duration::from_secs(1));
        observe(&mut log, &answer, now + Duration::from_secs(5));
        assert_eq!(ready_events(&log.take_events()), 0);
        let done = scene("Assistant: visible answer\r\nBaked for 0s", true);
        settle(&mut log, &done, now + Duration::from_secs(6));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("visible answer")
        );
        assert!(!log.document().text.contains("Baked"));
    }

    #[test]
    fn selected_semantics_do_not_borrow_another_renderers_completion_annotation() {
        use crate::tui_adapters::AdapterKind;
        let now = Instant::now();
        let baseline = scene("", true);
        let output = scene(
            "› q\r\nAssistant: source answer\r\n\x1b[2mWorked for 1s\x1b[0m",
            true,
        );
        for adapter in [
            AdapterKind::Codex,
            AdapterKind::Claude,
            AdapterKind::Kimi,
            AdapterKind::Gemini,
        ] {
            let mut log = TurnLog::new();
            log.set_adapter(adapter);
            log.submitted("q", &baseline.output, now);
            for at in [now, now + Duration::from_secs(1)] {
                log.observe_for_adapter(
                    adapter,
                    &output.frame,
                    &output.output,
                    output.input.as_ref(),
                    false,
                    at,
                );
            }
            assert_eq!(
                ready_events(&log.take_events()),
                usize::from(adapter == AdapterKind::Codex),
                "{adapter:?}"
            );
            assert_eq!(
                log.latest_answer().unwrap().complete,
                adapter == AdapterKind::Codex
            );
            if adapter != AdapterKind::Codex {
                assert!(log
                    .answer_text(log.latest_answer().unwrap().id)
                    .unwrap()
                    .contains("Worked for 1s"));
            }
        }
    }

    #[test]
    fn selected_panel_policy_requires_the_owned_renderer_convention() {
        use crate::tui_adapters::AdapterKind;
        let now = Instant::now();
        let baseline = scene("", true);
        let output=scene("  ┃\r\n  ┃  request\r\n  ┃\r\n\r\n     unlabelled source answer\r\n\r\n     Agent · Model · 1.2s",true);
        for adapter in [
            AdapterKind::OpenCode,
            AdapterKind::Codex,
            AdapterKind::Gemini,
        ] {
            let mut log = TurnLog::new();
            log.set_adapter(adapter);
            log.submitted("request", &baseline.output, now);
            for at in [now, now + Duration::from_secs(1)] {
                log.observe_for_adapter(
                    adapter,
                    &output.frame,
                    &output.output,
                    output.input.as_ref(),
                    false,
                    at,
                );
            }
            assert_eq!(
                ready_events(&log.take_events()),
                usize::from(adapter == AdapterKind::OpenCode)
            );
            assert_eq!(log.has_answers(), adapter == AdapterKind::OpenCode);
        }
    }

    #[test]
    fn changing_adapter_discards_inflight_proof_but_retains_answer_archive() {
        use crate::tui_adapters::AdapterKind;
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        let completed = complete_turn(&mut log, &baseline.output, "retained answer", now);
        let archive = log.document();
        let target = log.latest_answer().unwrap();
        log.submitted("next", &completed.output, now + Duration::from_secs(2));
        assert!(log.has_active_turn());
        log.set_adapter(AdapterKind::Claude);
        assert!(!log.has_active_turn());
        assert_eq!(log.document(), archive);
        assert_eq!(log.latest_answer(), Some(target));
        log.observe_for_adapter(
            AdapterKind::Claude,
            &completed.frame,
            &completed.output,
            completed.input.as_ref(),
            false,
            now + Duration::from_secs(3),
        );
        assert_eq!(log.document(), archive);
        assert_eq!(ready_events(&log.take_events()), 0);
    }

    #[test]
    fn always_editable_composer_needs_a_fresh_explicit_completion_boundary() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let busy = scene("Assistant: partial answer\r\nStatus: busy", true);
        settle(&mut log, &busy, now);
        let quiet = scene("Assistant: partial answer", true);
        settle(&mut log, &quiet, now + Duration::from_secs(1));
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert!(events
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        assert!(!log.latest_answer().unwrap().complete);
        let done = scene("Assistant: partial answer\r\nStatus: done", true);
        settle(&mut log, &done, now + Duration::from_secs(2));
        assert_eq!(ready_events(&log.take_events()), 1);

        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let mut unavailable = busy;
        unavailable.input.as_mut().unwrap().caret = None;
        settle(&mut log, &unavailable, now);
        settle(&mut log, &quiet, now + Duration::from_secs(1));
        assert_eq!(ready_events(&log.take_events()), 1);
    }

    #[test]
    fn unchanged_old_done_is_not_fresh_just_because_busy_was_seen_elsewhere() {
        let now = Instant::now();
        let baseline = scene("Status: done", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let busy = scene(
            "Status: done\r\nAssistant: partial answer\r\nStatus: busy",
            true,
        );
        settle(&mut log, &busy, now);
        let stale = scene("Status: done\r\nAssistant: partial answer", true);
        settle(&mut log, &stale, now + Duration::from_secs(1));
        assert_eq!(ready_events(&log.take_events()), 0);
        assert!(!log.latest_answer().unwrap().complete);
        let fresh_busy = scene("Assistant: partial answer\r\nStatus: busy", true);
        settle(&mut log, &fresh_busy, now + Duration::from_secs(2));
        let done = scene("Assistant: partial answer\r\nStatus: done", true);
        settle(&mut log, &done, now + Duration::from_secs(3));
        assert_eq!(ready_events(&log.take_events()), 1);
    }

    #[test]
    fn user_like_lines_inside_answer_code_fences_remain_in_the_answer() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let source = scene(
            "Assistant: Example\r\n```\r\nUser: demo\r\n❯ sample\r\n```\r\nfollowing\r\nStatus: busy",
            false,
        );
        settle(&mut log, &source, now);
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("Example\r\n```\r\nUser: demo\r\n❯ sample\r\n```\r\nfollowing")
        );
        valid_document(&log.document());
    }

    #[test]
    fn same_source_answer_redraw_corrects_body_and_ed2_repaint_does_not_duplicate_it() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let first = scene("Assistant: Result value=1\r\nStatus: busy", false);
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        let corrected = scene("Assistant: Result value=2\r\nStatus: busy", false);
        settle(&mut log, &corrected, now + Duration::from_secs(1));
        assert_eq!(log.answer_text(id), Some("Result value=2"));
        assert!(!log.document().text.contains("value=1"));
        let mut model = TerminalModel::new(80, 20);
        model.push_bytes(b"Assistant: Result value=2\r\nStatus: busy");
        model.push_bytes(
            format!(
                "\x1b[20;1H{}\x1b[2J\x1b[HAssistant: Result value=3\r\nStatus: busy",
                "\r\n".repeat(20)
            )
            .as_bytes(),
        );
        let mut output = model.reading_document();
        output.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        let repaint = Scene {
            frame: model.live_snapshot(),
            output,
            input: None,
        };
        settle(&mut log, &repaint, now + Duration::from_secs(2));
        assert_eq!(log.answer_text(id), Some("Result value=3"));
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert!(!log.document().text.contains("value=2"));
        valid_document(&log.document());
    }

    #[test]
    fn ready_then_unchanged_does_not_replay_but_new_passive_output_is_reported() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        let ready = complete_turn(&mut log, &baseline.output, "known answer", now);
        log.take_events();
        let archive = log.document();
        settle(&mut log, &ready, now + Duration::from_secs(3));
        assert!(log.take_events().is_empty());
        let appended = scene(
            "Assistant: known answer\r\nStatus: done\r\nnew passive output",
            true,
        );
        settle(&mut log, &appended, now + Duration::from_secs(4));
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert!(events.iter().any(
            |event| matches!(event, TurnEvent::NewOutput {text} if text == "new passive output")
        ));
        assert!(!events.iter().any(
            |event| matches!(event, TurnEvent::NewOutput {text} if text.contains("known answer"))
        ));
        assert_eq!(log.document().text, archive.text);
        let mut passive = TurnLog::new();
        settle(&mut passive, &appended, now);
        assert!(passive.take_events().is_empty() && !passive.has_answers());
    }

    #[test]
    fn overlapping_continuation_after_a_header_is_trimmed_updates_the_same_answer() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let first = scene(
            "Assistant: original first line\r\nshared retained tail\r\nStatus: busy",
            false,
        );
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        let trimmed = scene(
            "shared retained tail\r\nnew tail line\r\nStatus: busy",
            false,
        );
        settle(&mut log, &trimmed, now + Duration::from_secs(1));
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert_eq!(
            log.answer_text(id),
            Some("original first line\r\nshared retained tail\r\nnew tail line")
        );
        valid_document(&log.document());
    }

    #[test]
    fn settings_and_timing_words_inside_a_fenced_answer_are_not_footer_metadata() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let answer = scene(
            "Assistant: example\r\n```\r\nthinking: high\r\nBaked for 0s\r\n```",
            true,
        );
        settle(&mut log, &answer, now);
        let text = log.answer_text(log.latest_answer().unwrap().id).unwrap();
        assert!(text.contains("thinking: high") && text.contains("Baked for 0s"));
        assert_eq!(ready_events(&log.take_events()), 0);
    }

    #[test]
    fn static_settings_menus_slash_commands_empty_enter_and_unknown_output_are_conservative() {
        let now = Instant::now();
        let baseline = scene("thinking: high\r\nBaked for 0s", true);
        let mut log = TurnLog::new();
        for input in ["", " \r\n", "/help"] {
            log.submitted(input, &baseline.output, now);
        }
        assert!(!log.has_active_turn());
        log.submitted("q", &baseline.output, now);
        let menu = scene("Assistant: menu item\r\n/help Commands", true);
        log.observe(&menu.frame, &menu.output, menu.input.as_ref(), true, now);
        log.observe(
            &menu.frame,
            &menu.output,
            menu.input.as_ref(),
            true,
            now + Duration::from_secs(1),
        );
        assert!(log.take_events().is_empty());
        settle(&mut log, &baseline, now + Duration::from_secs(2));
        assert!(log.take_events().is_empty());
        let unknown = scene("plain unlabelled visible output", true);
        settle(&mut log, &unknown, now + Duration::from_secs(3));
        assert!(log
            .take_events()
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        assert!(!log.has_answers() && !log.has_semantic_turn());
        observe(&mut log, &unknown, now + Duration::from_secs(999));
        assert_eq!(ready_events(&log.take_events()), 0);
    }

    #[test]
    fn quoted_and_fenced_status_is_answer_content_and_stale_done_is_not_completion() {
        let now = Instant::now();
        let baseline = scene("Status: done", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let answer = scene(
            "Assistant: Example\r\n```\r\nStatus: done\r\n```\r\n> Status: done\r\nStatus: done",
            true,
        );
        settle(&mut log, &answer, now);
        let id = log.latest_answer().unwrap().id;
        assert_eq!(ready_events(&log.take_events()), 0);
        assert!(log
            .answer_text(id)
            .unwrap()
            .contains("```\r\nStatus: done\r\n```"));
        assert!(log.answer_text(id).unwrap().contains("> Status: done"));
    }

    #[test]
    fn generic_auth_failure_error_interruption_and_approval_wait_are_not_ready() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (text, expected) in [
            (
                "● Login expired · Please run /login",
                SignalKind::NeedsInput,
            ),
            ("Status: awaiting approval", SignalKind::NeedsInput),
            ("Status: error: connection lost", SignalKind::Failed),
            ("Status: interrupted", SignalKind::Interrupted),
        ] {
            let mut log = TurnLog::new();
            log.submitted("q", &baseline.output, now);
            let failure = scene(text, true);
            settle(&mut log, &failure, now);
            let events = log.take_events();
            assert!(events.iter().any(|event| matches!(
                (expected, event),
                (SignalKind::NeedsInput, TurnEvent::NeedsInput { .. })
                    | (SignalKind::Failed, TurnEvent::Failed { .. })
                    | (SignalKind::Interrupted, TurnEvent::Interrupted { .. })
            )));
            assert_eq!(ready_events(&events), 0);
            assert!(!log.has_answers());
            observe(&mut log, &failure, now + Duration::from_secs(5));
            assert!(log.take_events().is_empty());
            assert!(!log.status().is_empty());
        }
    }

    #[test]
    fn paired_bullet_style_is_semantic_but_color_alone_is_not() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let thought = scene(
            "\x1b[2;3m● inspecting visible input\x1b[0m\r\nStatus: busy",
            false,
        );
        settle(&mut log, &thought, now);
        assert!(log.take_events().iter().any(|event| matches!(event, TurnEvent::Phase {kind: PhaseKind::Thinking, text} if text == "inspecting visible input")));
        let answer = scene("● normal visible answer\r\nStatus: done", true);
        settle(&mut log, &answer, now + Duration::from_secs(1));
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("normal visible answer")
        );
        assert_eq!(ready_events(&log.take_events()), 1);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let color = scene("\x1b[31m● colored unclassified text\x1b[0m", true);
        settle(&mut log, &color, now);
        assert!(!log.has_answers() && !log.has_semantic_turn());
    }

    #[test]
    fn two_answers_navigation_archive_and_ids_survive_screen_erase_and_cancel() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        let first = complete_turn(&mut log, &baseline.output, "first Ж🙂e\u{301}", now);
        let one = log.latest_answer().unwrap();
        assert!(one.complete);
        let second = complete_turn(
            &mut log,
            &first.output,
            "second answer",
            now + Duration::from_secs(3),
        );
        let two = log.latest_answer().unwrap();
        assert!(two.complete && two.id > one.id);
        assert_eq!(log.navigate(None, true), Some(two));
        assert_eq!(log.navigate(None, false).unwrap().id, one.id);
        assert_eq!(log.navigate(Some(two.id), true).unwrap().id, one.id);
        assert_eq!(log.navigate(Some(one.id), false).unwrap().id, two.id);
        let archive = log.document();
        let units: Vec<_> = archive.text.encode_utf16().collect();
        for target in [log.navigate(None, false).unwrap(), two] {
            let text = log.answer_text(target.id).unwrap();
            let end = target.start as usize + text.encode_utf16().count();
            assert_eq!(
                String::from_utf16(&units[target.start as usize..end]).unwrap(),
                text
            );
        }
        let erased = scene("", true);
        settle(&mut log, &erased, now + Duration::from_secs(7));
        assert_eq!(log.document().text, archive.text);
        valid_document(&log.document());
        log.submitted(
            "not acknowledged",
            &second.output,
            now + Duration::from_secs(8),
        );
        log.cancel_pending();
        assert!(!log.has_active_turn() && log.has_answers());
        log.clear();
        assert!(!log.has_answers() && log.document().text.is_empty());
        complete_turn(
            &mut log,
            &baseline.output,
            "after clear",
            now + Duration::from_secs(10),
        );
        assert!(log.latest_answer().unwrap().id > two.id);
    }

    #[test]
    fn redraw_reflow_and_style_updates_do_not_duplicate_phase_or_answer_blocks() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let wide = scene_columns(
            "Assistant: a long continuous paragraph with Ж🙂e\u{301}\r\nStatus: busy",
            false,
            80,
        );
        settle(&mut log, &wide, now);
        log.take_events();
        let id = log.latest_answer().unwrap().id;
        let revision = log.revision();
        let narrow = scene_columns(
            "Assistant: a long continuous paragraph with Ж🙂e\u{301}\r\nStatus: busy",
            false,
            30,
        );
        settle(&mut log, &narrow, now + Duration::from_secs(1));
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert_eq!(log.revision(), revision);
        assert!(log.take_events().is_empty());
        valid_document(&log.document());
        let color = scene_columns(
            "Assistant: \x1b[32ma long continuous paragraph with Ж🙂e\u{301}\x1b[0m\r\nStatus: busy",
            false,
            30,
        );
        settle(&mut log, &color, now + Duration::from_secs(2));
        assert!(log.revision() > revision);
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert!(log
            .document()
            .styles
            .iter()
            .any(|span| span.style.foreground == Color::Named(NamedColor::Green)));
        assert!(log.take_events().is_empty());
    }

    #[test]
    fn old_negative_history_busy_does_not_block_a_new_complete_answer() {
        let now = Instant::now();
        let mut model = TerminalModel::new(60, 8);
        model.push_bytes(b"Thinking: old\r\nStatus: busy\r\nAssistant: old answer");
        let baseline = model.reading_document();
        let mut log = TurnLog::new();
        log.submitted("new request", &baseline, now);
        model.push_bytes(format!("\x1b[8;1H{}\x1b[2J\x1b[HAssistant: new answer\r\nStatus: done\r\n+--------+\r\n| >      |\r\n+--------+\x1b[4;5H", "\r\n".repeat(8)).as_bytes());
        let frame = model.live_snapshot();
        let raw = model.reading_document();
        let projection = crate::tui_projection::project(&frame, &raw, None);
        log.observe(
            &frame,
            &projection.output,
            projection.input.as_ref(),
            false,
            now,
        );
        log.observe(
            &frame,
            &projection.output,
            projection.input.as_ref(),
            false,
            now + Duration::from_secs(1),
        );
        assert_eq!(ready_events(&log.take_events()), 1);
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("new answer")
        );
    }

    #[test]
    fn bullet_turns_inserted_before_a_retained_footer_keep_phases_and_two_answer_targets() {
        let now = Instant::now();
        let baseline = footer_scene("", None, true, 100);
        let mut log = TurnLog::new();
        log.submitted("first request", &baseline.output, now);
        let thought = footer_scene(
            "❯ first request\r\n\x1b[2;3m● first thought\x1b[0m",
            Some("ctrl+c: cancel"),
            false,
            100,
        );
        settle(&mut log, &thought, now);
        let events = log.take_events();
        assert!(events.iter().any(|event| matches!(event, TurnEvent::Phase {kind: PhaseKind::Thinking, text} if text == "first thought")));
        assert!(!events
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        let grown = footer_scene(
            "❯ first request\r\n\x1b[2;3m● first thought with a long soft wrapped continuation across the available physical columns and Ж🙂e\u{301}\x1b[0m",
            Some("ctrl+c: cancel"),
            false,
            60,
        );
        settle(&mut log, &grown, now + Duration::from_secs(1));
        assert!(!log
            .take_events()
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        let body = "❯ first request\r\n\x1b[2;3m● first thought\x1b[0m\r\n● First answer Ж🙂e\u{301} with \x1b[3memphasis\x1b[0m";
        let answer = footer_scene(body, Some("ctrl+c: cancel"), false, 100);
        settle(&mut log, &answer, now + Duration::from_secs(2));
        let first = log.latest_answer().unwrap();
        assert_eq!(
            log.answer_text(first.id),
            Some("First answer Ж🙂e\u{301} with emphasis")
        );
        assert!(!first.complete);
        let events = log.take_events();
        assert!(events.iter().any(|event| matches!(
            event,
            TurnEvent::Phase {
                kind: PhaseKind::Answer,
                ..
            }
        )));
        assert_eq!(ready_events(&events), 0);
        assert!(!events
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        observe(&mut log, &answer, now + Duration::from_secs(10_000));
        assert!(log.take_events().is_empty());
        let done = footer_scene(body, None, true, 100);
        settle(&mut log, &done, now + Duration::from_secs(10_001));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert!(log.latest_answer().unwrap().complete);

        log.submitted(
            "second request",
            &done.output,
            now + Duration::from_secs(10_002),
        );
        let second_body = "❯ second request\r\n\x1b[3m● second reasoning\x1b[0m\r\n● Second answer";
        let second = footer_scene(second_body, Some("ctrl+c: cancel"), false, 100);
        settle(&mut log, &second, now + Duration::from_secs(10_003));
        let second = footer_scene(second_body, None, true, 100);
        settle(&mut log, &second, now + Duration::from_secs(10_004));
        assert_eq!(ready_events(&log.take_events()), 1);
        let latest = log.latest_answer().unwrap();
        assert_eq!(latest.total, 2);
        assert!(latest.complete && latest.id != first.id);
        assert_eq!(log.navigate(Some(latest.id), true).unwrap().id, first.id);
        assert_eq!(
            log.answer_text(first.id),
            Some("First answer Ж🙂e\u{301} with emphasis")
        );
        assert_eq!(log.answer_text(latest.id), Some("Second answer"));
        let archive = log.document();
        for target in [log.navigate(None, false).unwrap(), latest] {
            let units: Vec<_> = archive.text.encode_utf16().collect();
            let body = log.answer_text(target.id).unwrap();
            let end = target.start as usize + body.encode_utf16().count();
            assert_eq!(
                String::from_utf16(&units[target.start as usize..end]).unwrap(),
                body
            );
        }
        assert!(!archive.text.contains("thinking: high") && !archive.text.contains("cancel"));
        valid_document(&archive);
    }

    #[test]
    fn cancellation_control_boundary_can_complete_with_a_persistently_editable_composer() {
        let now = Instant::now();
        let baseline = footer_scene("", None, true, 100);
        for hint in ["ctrl+c: cancel", "esc to interrupt", "ctrl+c to cancel"] {
            let mut log = TurnLog::new();
            log.submitted("q", &baseline.output, now);
            let busy = footer_scene(
                "❯ q\r\n● queued input remains editable",
                Some(hint),
                true,
                100,
            );
            settle(&mut log, &busy, now);
            observe(&mut log, &busy, now + Duration::from_secs(5_000));
            assert_eq!(ready_events(&log.take_events()), 0, "{hint}");
            let quiet = footer_scene("❯ q\r\n● queued input remains editable", None, true, 100);
            observe(&mut log, &quiet, now + Duration::from_secs(5_001));
            observe(&mut log, &quiet, now + Duration::from_millis(5_001_200));
            assert_eq!(ready_events(&log.take_events()), 0, "{hint}");
            observe(&mut log, &quiet, now + Duration::from_secs(5_002));
            let events = log.take_events();
            assert_eq!(ready_events(&events), 1, "{hint}");
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, TurnEvent::NewOutput { .. })),
                "{events:?}"
            );
        }
    }

    #[test]
    fn changed_continuations_keep_the_role_and_fence_context_before_the_offset() {
        let now = Instant::now();
        let baseline = scene(
            "Thinking: retained role\r\nold thought\r\nFooter: fixed",
            true,
        );
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let changed = scene(
            "Thinking: retained role\r\nold thought\r\nnew thought continuation\r\nFooter: fixed",
            true,
        );
        settle(&mut log, &changed, now);
        let events = log.take_events();
        assert!(events.iter().any(|event| matches!(event, TurnEvent::Phase {kind: PhaseKind::Thinking, text} if text == "new thought continuation")));
        assert!(!events
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
        let answer = scene(
            "Thinking: retained role\r\nold thought\r\nnew thought continuation\r\n● actual answer\r\nFooter: fixed",
            true,
        );
        settle(&mut log, &answer, now + Duration::from_secs(1));
        let id = log.latest_answer().unwrap().id;
        assert_eq!(log.answer_text(id), Some("actual answer"));
        let corrected = scene(
            "Thinking: retained role\r\nold thought\r\nnew thought continuation\r\n● actual answer\r\nvalue=1\r\nFooter: fixed",
            true,
        );
        settle(&mut log, &corrected, now + Duration::from_secs(2));
        let corrected = scene(
            "Thinking: retained role\r\nold thought\r\nnew thought continuation\r\n● actual answer\r\nvalue=2\r\nFooter: fixed",
            true,
        );
        settle(&mut log, &corrected, now + Duration::from_secs(3));
        assert_eq!(log.answer_text(id), Some("actual answer\r\nvalue=2"));
        assert!(!log
            .take_events()
            .iter()
            .any(|event| matches!(event, TurnEvent::NewOutput { .. })));
    }

    #[test]
    fn overlapping_accepted_request_waits_for_a_fresh_exact_source_echo() {
        let now = Instant::now();
        let baseline = footer_scene("", None, true, 100);
        let mut log = TurnLog::new();
        log.submitted("first request", &baseline.output, now);
        let first = footer_scene(
            "❯ first request\r\n● first incomplete answer",
            Some("ctrl+c: cancel"),
            true,
            100,
        );
        settle(&mut log, &first, now);
        let old = log.latest_answer().unwrap();
        log.take_events();
        log.submitted(
            "second request",
            &first.output,
            now + Duration::from_secs(1),
        );
        let continuing = footer_scene(
            "❯ first request\r\n● first incomplete answer grows",
            Some("ctrl+c: cancel"),
            true,
            100,
        );
        settle(&mut log, &continuing, now + Duration::from_secs(2));
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert!(!log.take_events().iter().any(|event| matches!(
            event,
            TurnEvent::Phase {
                kind: PhaseKind::Request,
                ..
            }
        )));

        let second = footer_scene(
            "❯ second request\r\n\x1b[3m● second thought\x1b[0m",
            Some("ctrl+c: cancel"),
            false,
            100,
        );
        settle(&mut log, &second, now + Duration::from_secs(3));
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert_eq!(events.iter().filter(|event| matches!(event, TurnEvent::Phase {kind: PhaseKind::Request, text} if text == "second request")).count(), 1);
        assert_eq!(
            log.answer_text(old.id),
            Some("first incomplete answer grows")
        );
        let second = footer_scene(
            "❯ second request\r\n● second answer",
            Some("ctrl+c: cancel"),
            true,
            100,
        );
        settle(&mut log, &second, now + Duration::from_secs(4));
        let new = log.latest_answer().unwrap();
        assert!(new.id != old.id && new.total == 2);
        assert!(!log.navigate(Some(new.id), true).unwrap().complete);
        assert_eq!(
            log.answer_text(old.id),
            Some("first incomplete answer grows")
        );
        assert_eq!(log.answer_text(new.id), Some("second answer"));
        let second = footer_scene("❯ second request\r\n● second answer", None, true, 100);
        settle(&mut log, &second, now + Duration::from_secs(5));
        assert_eq!(ready_events(&log.take_events()), 1);
        assert!(!log.navigate(Some(new.id), true).unwrap().complete);
        assert!(log.latest_answer().unwrap().complete);
        valid_document(&log.document());
    }

    #[test]
    fn repeated_current_user_echo_after_ed2_keeps_the_full_answer_repaint_proof() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let mut model = TerminalModel::new(80, 20);
        model.push_bytes("❯ q\r\nAssistant: Result value=1\r\nctrl+c: cancel".as_bytes());
        let first = Scene {
            frame: model.live_snapshot(),
            output: model.reading_document(),
            input: None,
        };
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        model.push_bytes(
            format!(
                "\x1b[20;1H{}\x1b[2J\x1b[H❯ q\r\nAssistant: Result value=2\r\nctrl+c: cancel",
                "\r\n".repeat(20)
            )
            .as_bytes(),
        );
        let repaint = Scene {
            frame: model.live_snapshot(),
            output: model.reading_document(),
            input: None,
        };
        settle(&mut log, &repaint, now + Duration::from_secs(1));
        assert_eq!(log.answer_text(id), Some("Result value=2"));
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert_eq!(ready_events(&log.take_events()), 0);
        valid_document(&log.document());
    }

    #[test]
    fn passive_unknown_output_inserted_before_a_static_footer_is_reported_once() {
        let now = Instant::now();
        let baseline = footer_scene("", None, true, 100);
        let mut log = TurnLog::new();
        settle(&mut log, &baseline, now);
        assert!(log.take_events().is_empty());
        let changed = footer_scene("new unknown visible text", None, true, 100);
        settle(&mut log, &changed, now + Duration::from_secs(1));
        assert_eq!(
            log.take_events(),
            vec![TurnEvent::NewOutput {
                text: "new unknown visible text".into(),
            }]
        );
        settle(&mut log, &changed, now + Duration::from_secs(2));
        assert!(log.take_events().is_empty());
        assert!(!log.has_active_turn() && !log.has_answers());
    }

    #[test]
    fn repeated_accepted_request_has_a_new_ordered_echo_after_retained_old_body() {
        let now = Instant::now();
        let baseline = footer_scene("", None, true, 100);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let first = footer_scene(
            "❯ q\r\n● First incomplete answer",
            Some("ctrl+c: cancel"),
            true,
            100,
        );
        settle(&mut log, &first, now);
        let old = log.latest_answer().unwrap();
        log.take_events();
        log.submitted("q", &first.output, now + Duration::from_secs(1));
        let second = footer_scene(
            "❯ q\r\n● First incomplete answer\r\n❯ q\r\n● Second answer",
            Some("ctrl+c: cancel"),
            true,
            100,
        );
        settle(&mut log, &second, now + Duration::from_secs(2));
        let latest = log.latest_answer().unwrap();
        assert_eq!(latest.total, 2);
        assert_ne!(latest.id, old.id);
        assert_eq!(log.answer_text(old.id), Some("First incomplete answer"));
        assert_eq!(log.answer_text(latest.id), Some("Second answer"));
        assert!(!log.navigate(Some(latest.id), true).unwrap().complete);
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert_eq!(events.iter().filter(|event| matches!(event, TurnEvent::Phase {kind: PhaseKind::Request, text} if text == "q")).count(), 1);
        valid_document(&log.document());
    }

    #[test]
    fn queued_repeated_request_rejects_an_ed2_old_slice_then_accepts_an_actual_appended_echo() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let mut model = TerminalModel::new(80, 20);
        model.push_bytes("❯ q\r\n● First incomplete answer\r\nctrl+c: cancel".as_bytes());
        let first = Scene {
            frame: model.live_snapshot(),
            output: model.reading_document(),
            input: None,
        };
        settle(&mut log, &first, now);
        let old = log.latest_answer().unwrap();
        log.take_events();
        log.submitted("q", &first.output, now + Duration::from_secs(1));
        model.push_bytes(
            format!(
                "\x1b[20;1H{}\x1b[2J\x1b[H❯ q\r\n● First incomplete answer\r\nctrl+c: cancel",
                "\r\n".repeat(20)
            )
            .as_bytes(),
        );
        let repaint = Scene {
            frame: model.live_snapshot(),
            output: model.reading_document(),
            input: None,
        };
        settle(&mut log, &repaint, now + Duration::from_secs(2));
        assert_eq!(log.latest_answer().unwrap().total, 1);
        assert_eq!(log.answer_text(old.id), Some("First incomplete answer"));
        let events = log.take_events();
        assert_eq!(ready_events(&events), 0);
        assert!(!events.iter().any(|event| matches!(
            event,
            TurnEvent::Phase {
                kind: PhaseKind::Request,
                ..
            }
        )));

        model.push_bytes("\x1b[4;1H❯ q\r\n● Second answer\r\nctrl+c: cancel".as_bytes());
        let next = Scene {
            frame: model.live_snapshot(),
            output: model.reading_document(),
            input: None,
        };
        settle(&mut log, &next, now + Duration::from_secs(3));
        assert_eq!(log.latest_answer().unwrap().total, 2);
        assert_eq!(log.answer_text(old.id), Some("First incomplete answer"));
        assert_eq!(
            log.answer_text(log.latest_answer().unwrap().id),
            Some("Second answer")
        );
        assert_eq!(ready_events(&log.take_events()), 0);
        valid_document(&log.document());
    }

    #[test]
    fn distinct_answer_headers_preserve_duplicate_and_prefix_paragraphs_across_redraw() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (body, expected) in [
            ("● Alpha\r\n● Alpha", "Alpha\r\nAlpha"),
            ("● Alpha\r\n● Alpha and Beta", "Alpha\r\nAlpha and Beta"),
        ] {
            let mut log = TurnLog::new();
            log.submitted("q", &baseline.output, now);
            let source = scene(&format!("❯ q\r\n{body}\r\nctrl+c: cancel"), true);
            settle(&mut log, &source, now);
            let id = log.latest_answer().unwrap().id;
            assert_eq!(log.answer_text(id), Some(expected));
            let redraw = scene(&format!("❯ q\r\n{body}\r\nctrl+c: cancel"), true);
            settle(&mut log, &redraw, now + Duration::from_secs(1));
            assert_eq!(log.answer_text(id), Some(expected));
            assert_eq!(log.latest_answer().unwrap().total, 1);
            let mut model = TerminalModel::new(80, 20);
            let rendered = format!("❯ q\r\n{body}\r\nctrl+c: cancel");
            model.push_bytes(rendered.as_bytes());
            model.push_bytes(
                format!("\x1b[20;1H{}\x1b[2J\x1b[H{rendered}", "\r\n".repeat(20)).as_bytes(),
            );
            let repaint = Scene {
                frame: model.live_snapshot(),
                output: model.reading_document(),
                input: None,
            };
            settle(&mut log, &repaint, now + Duration::from_secs(2));
            assert_eq!(log.answer_text(id), Some(expected));
            assert_eq!(log.latest_answer().unwrap().total, 1);
            assert_eq!(ready_events(&log.take_events()), 0);
            valid_document(&log.document());
        }
    }

    #[test]
    fn menu_code_quoted_controls_and_stale_echoes_do_not_create_turn_boundaries_or_ready() {
        let now = Instant::now();
        let baseline = scene("", true);
        for suffix in [
            "```\r\n❯ next\r\nctrl+c: cancel\r\nesc to interrupt\r\n```",
            "```\r\n›\u{00a0}next\r\nWorking (1s • esc to interrupt)\r\nWorked for 2s\r\n```",
            "> ❯ next\r\n> ctrl+c: cancel",
            "> › next\r\n> Working (1s • esc to interrupt)",
            "Use ctrl+c: cancel in this example",
        ] {
            let mut log = TurnLog::new();
            log.submitted("q", &baseline.output, now);
            let first = scene("Assistant: first answer\r\nStatus: busy", true);
            settle(&mut log, &first, now);
            let id = log.latest_answer().unwrap().id;
            log.take_events();
            log.submitted("next", &first.output, now + Duration::from_secs(1));
            let code = scene(&format!("Assistant: first answer\r\n{suffix}"), true);
            settle(&mut log, &code, now + Duration::from_secs(2));
            observe(&mut log, &code, now + Duration::from_secs(500));
            let events = log.take_events();
            assert_eq!(ready_events(&events), 0, "{suffix}");
            assert!(
                !events.iter().any(|event| matches!(
                    event,
                    TurnEvent::Phase {
                        kind: PhaseKind::Request,
                        ..
                    }
                )),
                "{events:?}"
            );
            assert_eq!(log.latest_answer().unwrap().id, id);
            assert!(log.answer_text(id).unwrap().contains(suffix));
        }
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let first = scene("❯ next\r\nAssistant: first answer\r\nStatus: busy", true);
        settle(&mut log, &first, now);
        let id = log.latest_answer().unwrap().id;
        log.submitted("next", &first.output, now + Duration::from_secs(1));
        let stale = scene(
            "❯ next\r\nAssistant: first answer grows\r\nStatus: busy",
            true,
        );
        settle(&mut log, &stale, now + Duration::from_secs(2));
        assert_eq!(log.latest_answer().unwrap().id, id);
        let menu = scene("❯ next\r\nAssistant: menu-only\r\nStatus: done", true);
        log.observe(
            &menu.frame,
            &menu.output,
            menu.input.as_ref(),
            true,
            now + Duration::from_secs(3),
        );
        log.observe(
            &menu.frame,
            &menu.output,
            menu.input.as_ref(),
            true,
            now + Duration::from_secs(4),
        );
        assert_eq!(log.latest_answer().unwrap().id, id);
        assert_eq!(ready_events(&log.take_events()), 0);
    }

    #[test]
    fn codex_busy_commentary_is_not_promoted_and_cannot_prefix_the_final_message() {
        let now = Instant::now();
        let baseline = scene("", true);
        for (menu, protected) in [(true, false), (false, true)] {
            let mut log = TurnLog::new();
            log.set_adapter(crate::tui_adapters::AdapterKind::Codex);
            log.submitted("q", &baseline.output, now);
            // This exercises the admitted observer lifecycle, independently of
            // native renderer/style admission covered by the actual CLI replay.
            let busy = scene(
                "› q\r\n\r\n• Interim visible commentary\r\nStatus: busy",
                true,
            );
            log.observe_for_adapter(
                crate::tui_adapters::AdapterKind::Codex,
                &busy.frame,
                &busy.output,
                busy.input.as_ref(),
                false,
                now,
            );
            assert!(log.has_active_turn());
            assert!(log.latest_answer().is_none());
            assert!(log.working_turn().is_some());
            let mut input = busy.input.clone();
            if let Some(input) = input.as_mut() {
                input.protected = protected;
            }
            log.observe_for_adapter(
                crate::tui_adapters::AdapterKind::Codex,
                &busy.frame,
                &busy.output,
                input.as_ref(),
                menu,
                now + Duration::from_millis(10),
            );
            assert!(log.working_turn().is_none());
            let final_source = scene(
            "› q\r\n\r\n• Interim visible commentary\r\n\r\n• Actual final answer\r\nStatus: done",
            true,
        );
            for elapsed in [1_000, 1_500] {
                log.observe_for_adapter(
                    crate::tui_adapters::AdapterKind::Codex,
                    &final_source.frame,
                    &final_source.output,
                    final_source.input.as_ref(),
                    false,
                    now + Duration::from_millis(elapsed),
                );
            }
            let answer = log.latest_answer().unwrap();
            assert_eq!(log.answer_text(answer.id), Some("Actual final answer"));
            assert!(answer.complete);
            assert!(log.working_turn().is_none());
            assert_eq!(ready_events(&log.take_events()), 1);
        }
    }

    #[test]
    fn retention_limits_turn_count_and_unicode_tail_without_reusing_ids() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        let mut previous = baseline.output;
        for index in 0..130 {
            previous = complete_turn(
                &mut log,
                &previous,
                &format!("answer {index}"),
                now + Duration::from_secs(index * 3),
            )
            .output;
            log.take_events();
        }
        assert_eq!(log.turns.len(), MAX_TURNS);
        assert_eq!(log.latest_answer().unwrap().total, MAX_TURNS);
        assert!(log.answer_text(1).is_none());
        assert!(log.latest_answer().unwrap().id >= 130);
        valid_document(&log.document());
        let mut rich = RichText::plain(&"Ж🙂e\u{301}".repeat(1_024));
        rich.keep_tail(512);
        assert!(rich.text.len() < 640);
        valid_document(&rich.document(1));
    }

    #[test]
    fn local_output_retention_keeps_active_answer_and_newest_command_within_limits() {
        let now = Instant::now();
        let baseline = scene("", true);
        let mut log = TurnLog::new();
        log.submitted("q", &baseline.output, now);
        let streaming = scene("Assistant: streaming\r\nStatus: busy", true);
        settle(&mut log, &streaming, now);
        let active = log.active.as_ref().unwrap().id;
        for _ in 0..130 {
            let id = log.next_id;
            log.next_id += 1;
            log.turns.push_back(Turn {
                id,
                kind: OutputKind::Command,
                request: "/status".into(),
                request_expanded: false,
                answer: RichText::plain("retained local output"),
                complete: true,
                failed: false,
            });
            log.retain();
        }
        assert_eq!(log.turns.len(), MAX_TURNS);
        assert_eq!(log.turns.front().unwrap().id, active);
        let newest = log.turns.back().unwrap().id;
        // An answer can continue growing after a command was inserted.
        log.turns
            .iter_mut()
            .find(|turn| turn.id == active)
            .unwrap()
            .answer = RichText::plain(&"🙂".repeat(MAX_BYTES / 4));
        log.retain();
        log.rebuild();
        assert!(log.answer_text(active).is_some());
        assert_eq!(log.latest_output().unwrap().id, newest);
        assert_eq!(log.latest_output().unwrap().kind, OutputKind::Command);
        assert_eq!(log.output_text(newest), Some("retained local output"));
        assert!(
            log.turns
                .iter()
                .map(|turn| turn.answer.text.len() + turn.request.len() + TURN_OVERHEAD)
                .sum::<usize>()
                <= MAX_BYTES
        );
        valid_document(&log.document());
    }
}
