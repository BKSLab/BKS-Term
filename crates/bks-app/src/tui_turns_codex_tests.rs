//! The installed Codex renderer, replayed through every retained poll and
//! complete synchronized redraw. Inputs are confirmed source fields, never
//! reconstructed requests or rewritten endpoint screenshots.

use super::*;
use crate::source_input::SourceInput;
use crate::tui_adapters::AdapterKind;
use crate::tui_projection::{self, ComposerHistory};
use bks_terminal::TerminalModel;

#[derive(serde::Deserialize)]
struct Capture {
    columns: usize,
    rows: usize,
    events: Vec<CaptureEvent>,
    phases: Vec<CapturePhase>,
}

#[derive(serde::Deserialize)]
struct CapturePhase {
    name: String,
    end_byte: usize,
    at_ms: u64,
}

#[derive(serde::Deserialize)]
struct CaptureEvent {
    kind: String,
    at_ms: u64,
    #[serde(default)]
    start_byte: usize,
    #[serde(default)]
    end_byte: usize,
    #[serde(default)]
    bytes: Vec<u8>,
}

struct Replay {
    model: TerminalModel,
    history: ComposerHistory,
    source: SourceInput,
    composer: Option<Composer>,
    document: ReadingDocument,
    log: TurnLog,
    events: Vec<(u64, TurnEvent)>,
    submitted: Vec<String>,
    now: Instant,
    at_ms: u64,
    complete_frames: usize,
    partial_input_seen: bool,
}

impl Replay {
    fn observe(&mut self, at_ms: u64) {
        self.at_ms = at_ms;
        let now = self.now + Duration::from_millis(at_ms);
        // Parser deadlines use the host's real clock. A synthetic at_ms must
        // not expire a just-started redraw. Captured complete sync boundaries
        // are published below; timeout behavior has separate terminal tests.
        if self.model.sync_pending() {
            return;
        }
        let mut frame = self.model.live_snapshot();
        frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
        let raw = self.model.tui_reading_document();
        let filtered = self.history.filter(&frame, &raw);
        let accepted = self.source.confirmation(&frame, raw.revision, now);
        let mut projection = tui_projection::project_for_adapter(
            AdapterKind::Codex,
            &frame,
            &filtered,
            self.composer.as_ref(),
            accepted.map(|(evidence, _)| evidence),
        );
        if let Some((evidence, Some(baseline))) = accepted {
            let recovered = tui_projection::project_recovered_for_adapter(
                AdapterKind::Codex,
                &frame,
                &filtered,
                baseline,
                evidence,
            );
            if recovered.input.is_some() {
                projection = recovered;
            }
        }
        self.composer = projection.input;
        self.document = projection.output;
        if let Some(input) = self.composer.as_ref() {
            self.partial_input_seen |= input.is_partial();
            self.history.remember(&frame, &raw, input);
            self.source.acknowledge(input);
        }
        self.log.observe_for_adapter(
            AdapterKind::Codex,
            &frame,
            &self.document,
            self.composer.as_ref(),
            false,
            now,
        );
        let events = self.log.take_events();
        self.events
            .extend(events.into_iter().map(|event| (at_ms, event)));
    }

    fn input(&mut self, event: &CaptureEvent) {
        let now = self.now + Duration::from_millis(event.at_ms);
        let mut frame = self.model.live_snapshot();
        frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
        self.source.accepted(
            &frame,
            self.model.reading_revision(),
            &event.bytes,
            self.composer.as_ref(),
            now,
        );
        if matches!(event.bytes.as_slice(), b"\r" | b"\t") {
            let input = self.composer.as_ref().unwrap_or_else(|| {
                panic!(
                    "accepted key needs a live source editor at {}; source has {} UTF-16 units, head={:?}",
                    event.at_ms, self.document.text.encode_utf16().count(), self.document.text.chars().take(160).collect::<String>()
                )
            });
            let text = input.request_text();
            if !text.trim().is_empty() && !text.starts_with('/') {
                assert!(!input.protected && input.request_caret().is_some());
                self.submitted.push(text.to_owned());
                let kind = if !self.log.has_active_turn() {
                    crate::tui_submission::RequestKind::Submit
                } else if event.bytes == b"\t" {
                    crate::tui_submission::RequestKind::Queue
                } else {
                    crate::tui_submission::RequestKind::Steer
                };
                self.log.submitted_request(text, &self.document, kind, now);
            }
        }
    }
}

fn replay(bytes: &[u8], metadata: &str) -> Replay {
    replay_phase(bytes, metadata, None)
}

fn replay_phase(bytes: &[u8], metadata: &str, phase_name: Option<&str>) -> Replay {
    let capture: Capture = serde_json::from_str(metadata).unwrap();
    let mut replay = Replay {
        model: TerminalModel::new(capture.columns, capture.rows),
        history: ComposerHistory::new(),
        source: SourceInput::default(),
        composer: None,
        document: empty_document(0),
        log: TurnLog::new(),
        events: Vec::new(),
        submitted: Vec::new(),
        now: Instant::now(),
        at_ms: 0,
        complete_frames: 0,
        partial_input_seen: false,
    };
    replay.log.set_adapter(AdapterKind::Codex);
    let phase = capture
        .phases
        .iter()
        .find(|phase| match phase_name {
            Some(name) => phase.name == name,
            None => phase.name == "after-release" || phase.name == "second-ready",
        })
        .unwrap();
    let (end, end_ms) = (phase.end_byte, phase.at_ms);
    let mut consumed = 0;
    for event in capture.events {
        if event.at_ms > end_ms {
            break;
        }
        while replay.at_ms < event.at_ms {
            replay.observe((replay.at_ms + 10).min(event.at_ms));
        }
        if event.kind == "input" {
            replay.input(&event);
            continue;
        }
        assert_eq!(event.kind, "poll");
        assert_eq!(
            event.start_byte, consumed,
            "every captured byte must be retained"
        );
        while consumed < event.end_byte {
            let (used, complete) = replay
                .model
                .push_bytes_until_frame(&bytes[consumed..event.end_byte]);
            assert!(used > 0);
            consumed += used;
            if complete {
                replay.complete_frames += 1;
                replay.observe(event.at_ms);
            }
        }
        replay.observe(event.at_ms);
    }
    assert_eq!(consumed, end);
    replay.observe(replay.at_ms + 500);
    assert!(replay.complete_frames > 10);
    replay
}

fn assert_turns(replay: &Replay, requests: &[&str], complete: &[bool], bodies: &[&str]) {
    assert_eq!(replay.submitted.len(), requests.len());
    let actual: Vec<_> = replay
        .log
        .turns
        .iter()
        .map(|turn| {
            (
                turn.request.as_str(),
                turn.complete,
                turn.answer.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        actual.len(),
        requests.len(),
        "{actual:#?}\nevents={:#?}",
        replay.events
    );
    let mut ready = Vec::new();
    for (index, turn) in replay.log.turns.iter().enumerate() {
        assert_eq!(turn.request, requests[index]);
        assert_eq!(
            turn.complete,
            complete[index],
            "turn {}: {actual:#?}",
            index + 1
        );
        assert!(
            turn.answer.text.contains(bodies[index]),
            "turn {}: {actual:#?}",
            index + 1
        );
        if complete[index] {
            ready.push(turn.id);
        }
        for chrome in [
            "Queued follow-up inputs",
            "↳ BKS_",
            "edit last queued",
            "Worked for",
            "esc to interrupt",
        ] {
            assert!(
                !turn.answer.text.contains(chrome),
                "turn {}: {actual:#?}",
                index + 1
            );
        }
        for (other, body) in bodies.iter().enumerate() {
            if other != index {
                assert!(
                    !turn.answer.text.contains(body),
                    "turn {} contains answer {}: {actual:#?}",
                    index + 1,
                    other + 1
                );
            }
        }
    }
    let actual_ready: Vec<_> = replay
        .events
        .iter()
        .filter_map(|(_, event)| match event {
            TurnEvent::Ready { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(actual_ready, ready, "{:#?}", replay.events);
    assert_eq!(
        replay
            .events
            .iter()
            .filter(|(_, event)| matches!(
                event,
                TurnEvent::Phase {
                    kind: PhaseKind::Request,
                    ..
                }
            ))
            .count(),
        requests.len()
    );
    for (_, event) in &replay.events {
        let text = match event {
            TurnEvent::Phase { text, .. } | TurnEvent::NewOutput { text } => text,
            _ => continue,
        };
        for chrome in ["Queued follow-up inputs", "↳ BKS_", "edit last queued"] {
            assert!(!text.contains(chrome), "{event:?}");
        }
    }
    assert_eq!(replay.log.targets.len(), requests.len());
    super::tests::valid_document(&replay.log.document());
}

#[test]
fn actual_codex_queue_preview_does_not_start_a_turn_or_announce_ready() {
    let replay = replay_phase(
        include_bytes!("../tests/fixtures/tui-codex-production30/queue-repeat.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/queue-repeat.json"),
        Some("queued-busy"),
    );
    assert_eq!(replay.log.turns.len(), 1);
    assert_eq!(replay.log.queued.len(), 1);
    assert!(!replay.log.turns[0].complete);
    assert!(!replay
        .events
        .iter()
        .any(|(_, event)| matches!(event, TurnEvent::Ready { .. })));
    assert!(!replay.log.turns[0].answer.text.contains("Queued follow-up"));
    assert!(replay.log.turns[0].answer.text.ends_with("RESPONSE_1_BEG"));
}

#[test]
fn codex_queue_metadata_requires_recorded_ink_and_the_current_source_slot() {
    let mut replay = replay_phase(
        include_bytes!("../tests/fixtures/tui-codex-production30/queue-repeat.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/queue-repeat.json"),
        Some("queued-busy"),
    );
    let metadata = AdapterKind::Codex.policy().semantics.chrome.unwrap();
    let input = replay.composer.clone().unwrap();
    let frame = replay.model.live_snapshot();
    let (header, text) = replay
        .document
        .lines
        .iter()
        .zip(replay.document.text.split("\r\n"))
        .find(|(_, text)| *text == "• Queued follow-up inputs")
        .unwrap();
    assert!(metadata(
        &frame,
        &replay.document,
        Some(&input),
        header.line,
        header.start,
        text
    ));
    assert!(metadata(
        &frame,
        &replay.document,
        None,
        header.line,
        header.start,
        text
    ));
    assert!(!metadata(
        &frame,
        &replay.document,
        Some(&input),
        header.line - 2,
        header.start,
        text
    ));
    let mut displaced = input.clone();
    displaced.first_row -= 1;
    assert!(!metadata(
        &frame,
        &replay.document,
        Some(&displaced),
        header.line,
        header.start,
        text
    ));
    let mut protected = input.clone();
    protected.protected = true;
    assert!(!metadata(
        &frame,
        &replay.document,
        Some(&protected),
        header.line,
        header.start,
        text
    ));
    let row = header.line;
    replay.model.push_bytes(
        format!(
            "\x1b[{};1H\x1b[0m• Queued follow-up inputs\x1b[{};3H",
            row + 1,
            input.first_row + 2
        )
        .as_bytes(),
    );
    replay.observe(replay.at_ms + 1);
    let frame = replay.model.live_snapshot();
    let (header, text) = replay
        .document
        .lines
        .iter()
        .zip(replay.document.text.split("\r\n"))
        .find(|(line, _)| line.line == row)
        .unwrap();
    assert!(!metadata(
        &frame,
        &replay.document,
        replay.composer.as_ref(),
        row,
        header.start,
        text
    ));
    assert!(replay.log.turns[0]
        .answer
        .text
        .contains("Queued follow-up inputs"));
}

#[test]
fn actual_codex_queue_repeat_retains_both_full_answers_and_ready_once() {
    let replay = replay(
        include_bytes!("../tests/fixtures/tui-codex-production30/queue-repeat.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/queue-repeat.json"),
    );
    assert_turns(
        &replay,
        &["BKS_SAME_REQUEST", "BKS_SAME_REQUEST"],
        &[true, true],
        &["BKS_CODEX_RESPONSE_1_BEGIN", "BKS_CODEX_RESPONSE_2_BEGIN"],
    );
    for (index, turn) in replay.log.turns.iter().enumerate() {
        assert!(
            turn.answer
                .text
                .ends_with(&format!("BKS_CODEX_RESPONSE_{}_END", index + 1)),
            "{}",
            turn.answer.text
        );
    }
}

#[test]
fn actual_codex_steer_repeat_retains_interrupted_answer_without_ready() {
    let replay = replay(
        include_bytes!("../tests/fixtures/tui-codex-production30/steer-repeat.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/steer-repeat.json"),
    );
    assert_turns(
        &replay,
        &["BKS_SAME_REQUEST", "BKS_SAME_REQUEST"],
        &[false, true],
        &["BKS_CODEX_RESPONSE_1_BEG", "BKS_CODEX_RESPONSE_2_BEGIN"],
    );
}

#[test]
fn actual_codex_steer_overtakes_queued_request_only_when_source_echoes_it() {
    let replay = replay(
        include_bytes!("../tests/fixtures/tui-codex-production30/queue-then-steer.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/queue-then-steer.json"),
    );
    assert_turns(
        &replay,
        &["BKS_SAME_REQUEST", "BKS_THIRD_STEER", "BKS_SAME_REQUEST"],
        &[false, true, true],
        &[
            "BKS_CODEX_RESPONSE_1_BEG",
            "BKS_CODEX_RESPONSE_2_BEGIN",
            "BKS_CODEX_RESPONSE_3_BEGIN",
        ],
    );
}

#[test]
fn actual_codex_long_answer_keeps_every_source_line_across_later_turn() {
    let replay = replay(
        include_bytes!("../tests/fixtures/tui-codex-production30/long-two-answers.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/long-two-answers.json"),
    );
    assert_turns(
        &replay,
        &["BKS_LONG_REQUEST", "BKS_SECOND_REQUEST"],
        &[true, true],
        &["BKS_CODEX_LONG_BEGIN", "BKS_CODEX_RESPONSE_2_BEGIN"],
    );
    assert!(replay.log.turns[0]
        .answer
        .text
        .ends_with("BKS_CODEX_LONG_END"));
    for number in 1..=75 {
        assert_eq!(replay.log.turns[0].answer.text.matches(&format!("Line {number:02}: retained long response, Unicode Ж界🙂, useful prose for the source transcript.")).count(), 1);
    }
}

#[test]
fn codex_full_request_echo_requires_every_known_glyph_in_source_order() {
    let capture: Capture = serde_json::from_str(include_str!(
        "../tests/fixtures/tui-codex-production30/paste-4k-unique-echo.json"
    ))
    .unwrap();
    let paste = capture
        .events
        .iter()
        .find_map(|event| {
            event
                .bytes
                .strip_prefix(b"\x1b[200~")
                .and_then(|bytes| bytes.strip_suffix(b"\x1b[201~"))
        })
        .unwrap();
    let expected = std::str::from_utf8(paste).unwrap();
    assert_eq!(expected.len(), 4_096);
    let source: Vec<_> =
        include_str!("../tests/fixtures/tui-codex-production30/paste-4k-unique-echo-source.txt")
            .lines()
            .collect();
    assert_eq!(
        codex_request_echo_end(&source, 0, expected),
        Some(source.len())
    );
    assert!(codex_request_echo_end(&source[..source.len() - 1], 0, expected).is_none());
    assert!(codex_request_echo_end(&source, 0, &expected.replace("w0345", "w0346")).is_none());
    assert!(codex_request_echo_end(&source[1..], 0, expected).is_none());
    assert!(
        codex_request_echo_end(&source, 0, &expected.replace("w0345 w0346", "w0346 w0345"))
            .is_none()
    );
    for (input, lines) in [
        (
            "  Ж🙂  first\r\n\r\n> quote",
            vec!["›   Ж🙂  first", "", "  > quote"],
        ),
        ("alpha beta", vec!["› alpha", "  beta"]),
        ("alphabeta", vec!["› alpha", "  beta"]),
    ] {
        assert_eq!(codex_request_echo_end(&lines, 0, input), Some(lines.len()));
    }
    assert!(codex_request_echo_end(&["› alpha", "", "  beta"], 0, "alpha beta").is_none());
}

#[test]
fn actual_codex_4k_request_keeps_full_history_echo_and_excludes_its_continuations() {
    let metadata =
        include_str!("../tests/fixtures/tui-codex-production30/paste-4k-unique-echo.json");
    let capture: Capture = serde_json::from_str(metadata).unwrap();
    let request = capture
        .events
        .iter()
        .find_map(|event| {
            event
                .bytes
                .strip_prefix(b"\x1b[200~")
                .and_then(|bytes| bytes.strip_suffix(b"\x1b[201~"))
        })
        .unwrap();
    let request = std::str::from_utf8(request).unwrap();
    let mut replay = replay_phase(
        include_bytes!("../tests/fixtures/tui-codex-production30/paste-4k-unique-echo.vt"),
        metadata,
        Some("submitted"),
    );
    assert!(replay.partial_input_seen);
    assert_turns(
        &replay,
        &[request],
        &[true],
        &["BKS_CODEX_RESPONSE_1_BEGIN"],
    );
    assert_eq!(replay.log.turns[0].request, request);
    let id = replay.log.latest_answer().unwrap().id;
    replay.log.toggle_request(id).unwrap();
    assert!(replay.log.document().text.contains(request));
    for (_, event) in &replay.events {
        let text = match event {
            TurnEvent::Phase {
                kind: PhaseKind::Answer,
                text,
            }
            | TurnEvent::NewOutput { text } => text,
            _ => continue,
        };
        assert!(
            !text.contains("w0345") && !text.contains("BKS_UNIQUE_4K_BEGIN"),
            "{event:?}"
        );
    }
}

#[test]
fn codex_large_request_identity_requires_the_entire_echo_and_keeps_speech_bounded() {
    // Source-equivalent renderer unit: the real captured 4K cases above cover
    // the installed CLI. This adversarial boundary exceeds the speech limit.
    fn paint(model: &mut TerminalModel, request: &str) {
        model.push_bytes(b"\x1b[3J\x1b[2J\x1b[H");
        for (index, row) in request.as_bytes().chunks(98).enumerate() {
            model.push_bytes(if index == 0 {
                "› ".as_bytes()
            } else {
                b"\r\n  "
            });
            model.push_bytes(row);
        }
        model.push_bytes(
            "\r\n\r\n• proven complete answer\r\n\x1b[2mWorked for <1s • 12:34\x1b[0m".as_bytes(),
        );
    }
    fn observe(log: &mut TurnLog, model: &TerminalModel, now: Instant) {
        log.observe_for_adapter(
            AdapterKind::Codex,
            &model.live_snapshot(),
            &model.tui_reading_document(),
            None,
            false,
            now,
        );
    }
    let mut model = TerminalModel::new(100, 29);
    let baseline = model.tui_reading_document();
    let request = format!("{}KNOWN_TAIL_END", "A".repeat(MAX_EVENT_BYTES + 256));
    let now = Instant::now();
    let mut log = TurnLog::new();
    log.set_adapter(AdapterKind::Codex);
    log.submitted_request(
        &request,
        &baseline,
        crate::tui_submission::RequestKind::Submit,
        now,
    );
    paint(&mut model, &request[..MAX_EVENT_BYTES]);
    observe(&mut log, &model, now);
    observe(&mut log, &model, now + Duration::from_secs(1));
    assert!(
        !log.has_answers(),
        "a speech-sized source prefix cannot prove the full literal request"
    );
    assert!(!log
        .take_events()
        .iter()
        .any(|event| matches!(event, TurnEvent::Ready { .. })));
    paint(&mut model, &request);
    observe(&mut log, &model, now + Duration::from_secs(2));
    observe(&mut log, &model, now + Duration::from_secs(3));
    let answer = log.latest_answer().unwrap();
    assert!(answer.complete);
    assert_eq!(log.answer_text(answer.id), Some("proven complete answer"));
    let events = log.take_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, TurnEvent::Ready { .. }))
            .count(),
        1
    );
    let announced = events
        .iter()
        .find_map(|event| match event {
            TurnEvent::Phase {
                kind: PhaseKind::Request,
                text,
            } => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(announced.len(), MAX_EVENT_BYTES);
    log.toggle_request(answer.id).unwrap();
    assert!(log.document().text.contains(&request));
    super::tests::valid_document(&log.document());
    let mut rejected = TurnLog::new();
    rejected.set_adapter(AdapterKind::Codex);
    rejected.submitted_request(
        &"A".repeat(MAX_REQUEST_BYTES + 1),
        &baseline,
        crate::tui_submission::RequestKind::Submit,
        now,
    );
    assert!(!rejected.has_active_turn());
}
