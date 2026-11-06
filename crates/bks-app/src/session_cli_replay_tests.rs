//! Actual CLI bytes through the session's production projection/input pipeline.
//! The owned child only consumes literal stdin; it cannot execute TUI text.

use super::*;
use crate::profile::ProfileKind;
use crate::tui_adapters::AdapterKind;
use crate::tui_turns::{AnswerTarget, TurnEvent};

#[path = "codex_session_replay_tests.rs"]
mod codex_production_tests;

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
    columns: u16,
    rows: u16,
    phases: Vec<Phase>,
    chunks: Vec<Chunk>,
}

struct OwnedReplay(TerminalSession);

impl Drop for OwnedReplay {
    fn drop(&mut self) {
        let _ = self.0.pty.request_terminate();
    }
}

fn owned_cli_session(size: PtySize, adapter: AdapterKind) -> OwnedReplay {
    let profile = Profile {
        id: "kimi-session-capture-replay".into(),
        name: "Kimi session capture replay".into(),
        kind: ProfileKind::Local,
        executable: PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/more.com")
            .to_string_lossy()
            .into_owned(),
        args: Vec::new(),
        working_directory: None,
        launch: Default::default(),
    };
    let mut session = TerminalSession::spawn(1, &profile, size).unwrap();
    session.direct_input = true;
    session.reading = false;
    session.vt_reading = true;
    session.terminal_capture = true;
    session.set_adapter(adapter);
    OwnedReplay(session)
}

// Follow main's acknowledgement path for input accepted in the real capture.
// In particular, Submission sees the current source composer before Enter;
// an accepted request is not injected directly from fixture metadata. The
// passive replay process does not interpret the recorded application's keys.
fn accept_input(session: &mut TerminalSession, data: &[u8], now: Instant) -> Option<String> {
    let revision = session.terminal.reading_revision();
    session.menu_observer.accepted(data, revision);
    let mut frame = session.terminal.live_snapshot();
    frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
    if session.tui_menu.is_none()
        && !session
            .tui_composer
            .as_ref()
            .is_some_and(|input| input.protected)
    {
        session
            .source_input
            .accepted(&frame, revision, data, session.tui_composer.as_ref(), now);
    } else {
        session.source_input.clear();
    }
    session.tui_submission.observe_at(
        session.tui_composer.as_ref(),
        session.tui_menu.is_some(),
        now,
    );
    let document = session.reading_document_snapshot();
    let request = session.tui_submission.accepted_request(
        data,
        now,
        Arc::clone(&document),
        session.turns.has_active_turn(),
    );
    session.record_output_submission(data, request.is_some(), &document, now);
    if let Some(request) = request.as_ref() {
        session.turn_speech.reset();
        session
            .turns
            .submitted_request(&request.text, &request.baseline, request.kind, now);
    }
    session.record_direct_input(data);
    request.map(|request| request.text)
}

fn poll_source(
    session: &mut TerminalSession,
    now: Instant,
    until_ms: u64,
    elapsed_ms: &mut u64,
    events: &mut Vec<TurnEvent>,
) {
    while *elapsed_ms < until_ms {
        *elapsed_ms = (*elapsed_ms + 10).min(until_ms);
        session.observe_tui_frame(now + Duration::from_millis(*elapsed_ms));
        events.extend(session.turns.take_events());
    }
}

fn replay_kimi_session(bytes: &[u8], metadata: &str, queued_repeat: bool) {
    let capture: Capture = serde_json::from_str(metadata).unwrap();
    let mut owned = owned_cli_session(
        PtySize::new(capture.columns, capture.rows),
        AdapterKind::Kimi,
    );
    let session = &mut owned.0;
    let now = Instant::now();
    let mut elapsed_ms = 0;
    let mut begin = 0;
    let mut submitted = Vec::new();
    let mut ids = Vec::new();
    let mut events = Vec::new();
    for (phase_index, phase) in capture.phases.iter().enumerate() {
        let chunks: Vec<_> = capture
            .chunks
            .iter()
            .filter(|chunk| chunk.phase == phase.name)
            .collect();
        let input_time = chunks[0].elapsed_ms.saturating_sub(1);
        poll_source(session, now, input_time, &mut elapsed_ms, &mut events);
        let draft = match phase.name.as_str() {
            "draft-one" => Some("audit-one"),
            "draft-two" => Some(if queued_repeat {
                "audit-one"
            } else {
                "audit-two"
            }),
            _ => None,
        };
        if let Some(text) = draft {
            let input = session.tui_composer.as_ref().unwrap();
            assert_eq!((input.text.as_str(), input.caret), ("", Some(0)));
            assert!(session.tui_menu.is_none());
            assert!(accept_input(
                session,
                text.as_bytes(),
                now + Duration::from_millis(input_time)
            )
            .is_none());
        }
        let answer = phase.name.starts_with("answer-");
        if answer {
            let expected = if phase.name == "answer-one" || queued_repeat {
                "audit-one"
            } else {
                "audit-two"
            };
            let request = accept_input(session, b"\r", now + Duration::from_millis(input_time));
            assert_eq!(request.as_deref(), Some(expected));
            submitted.push(request.unwrap());
        }
        for chunk in &chunks {
            poll_source(
                session,
                now,
                chunk.elapsed_ms.saturating_sub(1),
                &mut elapsed_ms,
                &mut events,
            );
            session
                .ingest_output(&bytes[begin..chunk.end_byte])
                .unwrap();
            begin = chunk.end_byte;
            elapsed_ms = chunk.elapsed_ms;
            session.reading_document_snapshot();
            session.observe_tui_frame(now + Duration::from_millis(elapsed_ms));
            events.extend(session.turns.take_events());
        }
        assert_eq!(begin, phase.end_byte);
        let output = session.reading_document_snapshot();
        let input = session
            .tui_composer
            .as_ref()
            .expect("actual Kimi source confirms its current composer");
        assert_eq!(input.text, draft.unwrap_or(""));
        assert_eq!(input.caret, Some(draft.map_or(0, |text| text.len() as u32)));
        assert!(!input.protected);
        assert!(session.tui_menu.is_none());
        assert!(!output
            .lines
            .iter()
            .any(|line| { (input.first_row as i32..=input.last_row as i32).contains(&line.line) }));

        // Replay actual quiet intervals before the next source input phase.
        // The final capture also waits after its last VT; a bounded idle poll
        // checks the production completion debounce without inventing bytes.
        let until = capture
            .phases
            .get(phase_index + 1)
            .and_then(|next| capture.chunks.iter().find(|chunk| chunk.phase == next.name))
            .map_or(elapsed_ms + 500, |next| next.elapsed_ms.saturating_sub(2));
        poll_source(session, now, until, &mut elapsed_ms, &mut events);
        if answer {
            let target = session.turns.latest_answer().unwrap();
            let waiting = queued_repeat && phase.name == "answer-one";
            assert_eq!(target.complete, !waiting, "{}", session.turns.status());
            assert!(!target.failed);
            assert_eq!(session.turns.has_active_turn(), waiting);
            ids.push(target.id);
        }
    }
    assert_eq!(begin, bytes.len());
    assert_eq!(submitted.len(), 2);
    assert_eq!(ids.len(), 2);
    assert!(ids[0] < ids[1]);
    let first = session.turns.navigate(None, false).unwrap();
    let last = session.turns.latest_answer().unwrap();
    assert_eq!((first.id, last.id, last.total), (ids[0], ids[1], 2));
    assert_eq!(session.turns.navigate(None, true), Some(last));
    assert_eq!(session.turns.navigate(Some(last.id), true), Some(first));
    assert_eq!(session.turns.navigate(Some(first.id), false), Some(last));
    assert!(last.complete && !session.turns.has_active_turn());
    for (target, expected) in [first, last].into_iter().zip([
        "Kimi audit answer one.\r\n\r\n   First retained result.",
        "Kimi audit answer two.\r\n\r\n   Second retained result.",
    ]) {
        assert_answer_source(session, target, expected);
    }
    let archive = session.turns.document();
    for chrome in [
        "Working…",
        "Welcome",
        "context:",
        "Tip:",
        "mention files",
        "│ >",
        "↑ to edit",
    ] {
        assert!(!archive.text.contains(chrome), "{chrome}");
    }
    assert_eq!(archive.text.matches("Ответ 1").count(), 1);
    assert_eq!(archive.text.matches("Ответ 2").count(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, TurnEvent::Ready { .. }))
            .count(),
        if queued_repeat { 1 } else { 2 },
        "{events:?}"
    );
    // Reader/view switching uses the retained archive, not a second parse.
    session.set_answers_view(true);
    assert_eq!(session.display_document_snapshot().text, archive.text);
    session.set_answers_view(false);
    assert!(session
        .reading_document_snapshot()
        .text
        .contains("Second retained result."));
    assert_request_archive(session, [first.id, last.id], &submitted);
}

fn assert_request_archive(session: &mut TerminalSession, ids: [u64; 2], submitted: &[String]) {
    let revision = session.input_revision();
    let answers: Vec<_> = ids
        .iter()
        .map(|id| session.turns.answer_text(*id).unwrap().to_owned())
        .collect();
    assert_eq!(
        session
            .turns
            .document()
            .text
            .matches("Ваш запрос, свёрнуто:")
            .count(),
        2
    );
    session.set_answers_view(true);
    for (id, request) in ids.into_iter().zip(submitted) {
        let header = session.turns.toggle_request(id).unwrap();
        assert!(header.expanded);
        let document = session.display_document_snapshot();
        let request = request
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n");
        let units: Vec<_> = document.text.encode_utf16().collect();
        let start = header.end as usize + 2;
        assert_eq!(
            String::from_utf16(&units[start..start + request.encode_utf16().count()]).unwrap(),
            request
        );
    }
    for (id, answer) in ids.into_iter().zip(answers) {
        assert_eq!(session.turns.answer_text(id), Some(answer.as_str()));
        assert!(!session.turns.toggle_request(id).unwrap().expanded);
    }
    assert_eq!(
        session.input_revision(),
        revision,
        "reader actions must not write to the CLI"
    );
    assert!(session.turns.take_events().is_empty());
    session.set_answers_view(false);
}

fn assert_answer_source(session: &mut TerminalSession, target: AnswerTarget, expected: &str) {
    assert_eq!(session.turns.answer_text(target.id), Some(expected));
    let archive = session.turns.document();
    let units: Vec<_> = archive.text.encode_utf16().collect();
    let end = target.start as usize + expected.encode_utf16().count();
    assert_eq!(
        String::from_utf16(&units[target.start as usize..end]).unwrap(),
        expected
    );
    let source = session.reading_document_snapshot();
    let first_line = expected.split("\r\n").next().unwrap();
    let byte = source.text.find(first_line).unwrap();
    let offset = source.text[..byte].encode_utf16().count() as u32;
    let style = source
        .styles
        .iter()
        .find(|span| span.start <= offset && span.end > offset)
        .unwrap()
        .style;
    assert_eq!(
        archive
            .styles
            .iter()
            .find(|span| span.start <= target.start && span.end > target.start)
            .unwrap()
            .style,
        style
    );
}

#[test]
fn actual_kimi_capture_uses_session_source_ack_submission_and_two_answer_navigation() {
    replay_kimi_session(
        include_bytes!("../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.vt"),
        include_str!("../tests/fixtures/tui-kimi-2.4.1-two-answers/capture.json"),
        false,
    );
}

#[test]
fn actual_kimi_queued_repeat_keeps_two_ids_through_the_session_pipeline() {
    replay_kimi_session(
        include_bytes!("../tests/fixtures/tui-kimi-2.4.1-two-answers/queued-repeat.vt"),
        include_str!("../tests/fixtures/tui-kimi-2.4.1-two-answers/queued-repeat.json"),
        true,
    );
}

#[derive(serde::Deserialize)]
struct CliLedger {
    columns: u16,
    rows: u16,
    expected: Vec<String>,
    events: Vec<CliLedgerEvent>,
    #[serde(default = "ordinary_ready_turns")]
    ready_turns: Vec<usize>,
    #[serde(default)]
    retained_first_tail: Option<String>,
}

fn ordinary_ready_turns() -> Vec<usize> {
    vec![1, 2]
}

#[derive(serde::Deserialize)]
struct CliLedgerEvent {
    kind: String,
    at_ms: u64,
    #[serde(default)]
    start_byte: usize,
    #[serde(default)]
    end_byte: usize,
    #[serde(default)]
    bytes: Vec<u8>,
    #[serde(default)]
    columns: u16,
    #[serde(default)]
    rows: u16,
}

#[derive(serde::Deserialize)]
struct OpenCodeEditorCapture {
    columns: u16,
    rows: u16,
    events: Vec<CliLedgerEvent>,
    phases: Vec<OpenCodeEditorPhase>,
}

#[derive(serde::Deserialize)]
struct OpenCodeEditorPhase {
    name: String,
    at_ms: u64,
    text: String,
    shell: bool,
    caret: u32,
}

#[test]
fn actual_opencode_profile_editor_preserves_newlines_large_first_paste_and_shell_mode() {
    replay_opencode_editor(
        include_bytes!("../tests/fixtures/tui-opencode-profile28/editor.vt"),
        include_str!("../tests/fixtures/tui-opencode-profile28/editor.json"),
    );
}

#[test]
fn actual_opencode_home_100x29_transfers_first_typed_letters_to_shared_input() {
    replay_opencode_editor(
        include_bytes!("../tests/fixtures/tui-opencode-input29/home.vt"),
        include_str!("../tests/fixtures/tui-opencode-input29/home.json"),
    );
}

#[test]
fn actual_opencode_resize_restores_shared_input_and_canonical_wrapped_text() {
    replay_opencode_editor(
        include_bytes!("../tests/fixtures/tui-opencode-input29/resize.vt"),
        include_str!("../tests/fixtures/tui-opencode-input29/resize.json"),
    );
}

#[test]
fn actual_opencode_first_visible_prompt_transfers_immediate_typing() {
    replay_opencode_editor(
        include_bytes!("../tests/fixtures/tui-opencode-input29/first-source.vt"),
        include_str!("../tests/fixtures/tui-opencode-input29/first-source.json"),
    );
}

fn replay_opencode_editor(bytes: &[u8], metadata: &str) {
    let capture: OpenCodeEditorCapture = serde_json::from_str(metadata).unwrap();
    let mut owned = owned_cli_session(
        PtySize::new(capture.columns, capture.rows),
        AdapterKind::OpenCode,
    );
    let session = &mut owned.0;
    let now = Instant::now();
    let mut elapsed_ms = 0;
    let mut events = Vec::new();
    let mut timeline = capture.events.into_iter().peekable();
    let mut end = 0;
    for phase in capture.phases {
        while timeline
            .peek()
            .is_some_and(|item| item.at_ms <= phase.at_ms)
        {
            let item = timeline.next().unwrap();
            poll_source(session, now, item.at_ms, &mut elapsed_ms, &mut events);
            if item.kind == "resize" {
                session
                    .resize_at(
                        PtySize::new(item.columns, item.rows),
                        now + Duration::from_millis(item.at_ms),
                    )
                    .unwrap();
                session.observe_tui_frame(now + Duration::from_millis(item.at_ms));
            } else if item.kind == "input" {
                assert!(
                    accept_input(
                        session,
                        &item.bytes,
                        now + Duration::from_millis(item.at_ms)
                    )
                    .is_none(),
                    "{} is an edit, not a submitted request",
                    phase.name
                );
            } else {
                assert_eq!(item.start_byte, end);
                while end < item.end_byte {
                    let (used, complete) = session
                        .terminal
                        .push_bytes_until_frame(&bytes[end..item.end_byte]);
                    assert!(used > 0);
                    end += used;
                    if complete {
                        session.observe_tui_frame(now + Duration::from_millis(item.at_ms));
                        events.extend(session.turns.take_events());
                    }
                }
                session.observe_tui_frame(now + Duration::from_millis(item.at_ms));
                events.extend(session.turns.take_events());
            }
        }
        poll_source(session, now, phase.at_ms, &mut elapsed_ms, &mut events);
        let projected_output =
            session.reading_document_snapshot_at(now + Duration::from_millis(phase.at_ms));
        let input = session.tui_composer.as_ref().unwrap_or_else(|| {
            panic!(
                "{} has no shared source field; pending={} size={:?} frame={:?}",
                phase.name,
                session.resized_composer.is_some(),
                session.size,
                session.terminal.live_snapshot().cursor,
            )
        });
        assert_eq!(input.text, phase.text, "{}", phase.name);
        assert_eq!(input.caret, Some(phase.caret), "{} caret", phase.name);
        assert_eq!(input.accepts_requests(), !phase.shell, "{}", phase.name);
        assert!(!input.protected);
        let (first, last) = (input.first_row, input.last_row);
        assert!(
            projected_output
                .lines
                .iter()
                .all(|line| !(first as i32..=last as i32).contains(&line.line)),
            "{} active composer must be excluded from upper output",
            phase.name
        );
        if phase.shell {
            let mut submission = crate::tui_submission::Submission::default();
            submission.observe(Some(input), false);
            assert!(
                submission.accepted(b"\r").is_none(),
                "shell text is not a model request"
            );
        }
        assert!(
            !session.turns.has_active_turn(),
            "{} must not arm a model request",
            phase.name
        );
        assert!(!session.turns.has_answers());
    }
    assert!(!events.iter().any(|event| matches!(
        event,
        TurnEvent::Ready { .. }
            | TurnEvent::Phase {
                kind: crate::tui_turns::PhaseKind::Request,
                ..
            }
    )));
}

#[test]
fn actual_opencode_queue_keeps_both_answers_without_fabricating_early_ready() {
    replay_cli_ledger(
        AdapterKind::OpenCode,
        include_bytes!("../tests/fixtures/tui-opencode-profile28/queued.vt"),
        include_str!("../tests/fixtures/tui-opencode-profile28/queued.json"),
    );
}

fn replay_cli_ledger(adapter: AdapterKind, bytes: &[u8], metadata: &str) {
    let capture: CliLedger = serde_json::from_str(metadata).unwrap();
    let mut owned = owned_cli_session(PtySize::new(capture.columns, capture.rows), adapter);
    let session = &mut owned.0;
    let now = Instant::now();
    let mut elapsed_ms = 0;
    let mut events = Vec::new();
    let mut submitted = Vec::new();
    let mut end = 0;
    for item in capture.events {
        poll_source(session, now, item.at_ms, &mut elapsed_ms, &mut events);
        if item.kind == "input" {
            if let Some(input) = accept_input(
                session,
                &item.bytes,
                now + Duration::from_millis(item.at_ms),
            ) {
                submitted.push(input);
            }
        } else {
            assert_eq!(item.start_byte, end, "capture must retain every VT byte");
            session
                .terminal
                .push_bytes(&bytes[item.start_byte..item.end_byte]);
            end = item.end_byte;
            session
                .terminal
                .expire_sync(now + Duration::from_millis(item.at_ms));
            session.observe_tui_frame(now + Duration::from_millis(item.at_ms));
            events.extend(session.turns.take_events());
        }
    }
    poll_source(session, now, elapsed_ms + 500, &mut elapsed_ms, &mut events);
    assert_eq!(end, bytes.len());
    assert_eq!(
        submitted.len(),
        2,
        "{adapter:?} request binding: {submitted:?}"
    );
    let ready: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let TurnEvent::Ready { id } = event {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    let latest = session.turns.latest_answer().unwrap();
    assert!(latest.complete);
    assert_eq!(latest.total, 2);
    let previous = session.turns.navigate(Some(latest.id), true).unwrap();
    let expected_ready: Vec<_> = capture
        .ready_turns
        .iter()
        .map(|number| match number {
            1 => previous.id,
            2 => latest.id,
            _ => panic!("invalid fixture ready turn"),
        })
        .collect();
    assert_eq!(ready, expected_ready, "{adapter:?}: {events:?}");
    assert_eq!(previous.complete, capture.ready_turns.contains(&1));
    let next = session.turns.navigate(Some(previous.id), false).unwrap();
    assert_eq!((previous.number, next.number), (1, 2));
    assert_eq!(next.id, latest.id);
    let first = session.turns.answer_text(previous.id).unwrap();
    let second = session.turns.answer_text(next.id).unwrap();
    assert!(first.contains(&capture.expected[0]), "{adapter:?}: {first}");
    assert!(
        second.contains(&capture.expected[1]),
        "{adapter:?}: {second}"
    );
    assert!(!first.contains(&capture.expected[1]));
    assert!(!second.contains(&capture.expected[0]));
    if let Some(tail) = capture.retained_first_tail.as_ref() {
        assert!(
            first.ends_with(tail),
            "first queued answer must retain its full tail: {first}"
        );
        assert!(!first.contains("1 queued") && !second.contains("1 queued"));
    }
    if adapter == AdapterKind::Claude {
        for text in [first, second] {
            assert!(!text.contains("/effort"), "{text}");
            assert!(!text.contains(" for 0s"), "{text}");
        }
        assert!(
            !events.iter().any(|event| matches!(event,
            TurnEvent::Phase { kind: crate::tui_turns::PhaseKind::Answer, text }
                if text.contains("/effort") || text.contains(" for 0s"))),
            "{events:?}"
        );
    }
    assert_request_archive(session, [previous.id, latest.id], &submitted);
}

#[test]
fn actual_codex_two_loopback_answers_keep_ready_and_navigation_through_session() {
    replay_cli_ledger(
        AdapterKind::Codex,
        include_bytes!("../tests/fixtures/tui-cli-audit26/codex.vt"),
        include_str!("../tests/fixtures/tui-cli-audit26/codex.json"),
    );
}

#[test]
fn actual_claude_two_loopback_answers_exclude_chrome_from_archive_and_speech() {
    replay_cli_ledger(
        AdapterKind::Claude,
        include_bytes!("../tests/fixtures/tui-cli-audit26/claude.vt"),
        include_str!("../tests/fixtures/tui-cli-audit26/claude.json"),
    );
}

#[test]
fn actual_opencode_two_loopback_answers_keep_ready_and_navigation_through_session() {
    replay_cli_ledger(
        AdapterKind::OpenCode,
        include_bytes!("../tests/fixtures/tui-cli-audit26/opencode.vt"),
        include_str!("../tests/fixtures/tui-cli-audit26/opencode.json"),
    );
}
