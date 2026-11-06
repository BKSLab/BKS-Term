//! Installed Codex bytes through the production Session, source projection and
//! Submission. The passive owned PTY never interprets captured TUI commands.

use super::*;
use crate::tui_submission::RequestKind;

#[derive(serde::Deserialize)]
struct CodexCapture {
    columns: u16,
    rows: u16,
    events: Vec<Event>,
    phases: Vec<SourcePhase>,
    #[serde(default)]
    transport_contract: Option<Transport>,
    #[serde(default)]
    typed_contract: Option<Transport>,
}

#[derive(serde::Deserialize)]
struct Event {
    kind: String,
    at_ms: u64,
    #[serde(default)]
    start_byte: usize,
    #[serde(default)]
    end_byte: usize,
    #[serde(default)]
    bytes: Vec<u8>,
    #[serde(default)]
    logical_bytes: Option<Vec<u8>>,
    #[serde(default)]
    columns: u16,
    #[serde(default)]
    rows: u16,
}

#[derive(serde::Deserialize)]
struct SourcePhase {
    name: String,
    at_ms: u64,
    end_byte: usize,
}

#[derive(serde::Deserialize)]
struct Transport {
    payload: String,
    actual_request: String,
    exact_request: bool,
    #[serde(default)]
    canonical_exact: bool,
    #[serde(default)]
    cases: Vec<UnicodeCase>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UnicodeCase {
    case: String,
    exact: bool,
    expected: String,
    actual: String,
}

struct PhaseView {
    name: String,
    input: Option<crate::tui_projection::Composer>,
    menu: Option<crate::tui_suggestions::Menu>,
    output: Arc<ReadingDocument>,
    source: ReadingDocument,
    active: bool,
    size: PtySize,
    uses_answers: bool,
    answer_count: usize,
    output_count: usize,
    output_id: Option<u64>,
    working_turn: Option<u64>,
}

struct InputFrame {
    logical: Vec<u8>,
    input: Option<crate::tui_projection::Composer>,
    cursor: Option<bks_terminal::CursorSnapshot>,
}

struct CodexReplay {
    owned: OwnedReplay,
    now: Instant,
    elapsed_ms: u64,
    events: Vec<(u64, TurnEvent)>,
    submitted: Vec<(String, RequestKind)>,
    phases: Vec<PhaseView>,
    frames: usize,
    inputs: Vec<InputFrame>,
    source_echoes: Vec<(u64, String)>,
    input_echo: crate::tui_input_echo::InputEcho,
    spoken_input: Vec<String>,
    spoken_activity: Vec<(u64, String)>,
}

impl CodexReplay {
    fn observe(&mut self, at_ms: u64) {
        self.elapsed_ms = at_ms;
        let fresh = self.owned.0.reading_document_key
            != Some((self.owned.0.terminal.reading_revision(), true));
        self.owned
            .0
            .observe_tui_frame(self.now + Duration::from_millis(at_ms));
        if !self.owned.0.terminal.sync_pending() {
            if let Some(echo) = self.input_echo.observe_with_composer(
                &self.owned.0.terminal.live_snapshot(),
                self.owned.0.tui_composer.as_ref(),
                self.now + Duration::from_millis(at_ms),
            ) {
                self.spoken_input.push(echo.text);
            }
        }
        if fresh && !self.owned.0.terminal.sync_pending() {
            for text in self
                .owned
                .0
                .terminal
                .tui_reading_document()
                .text
                .split("\r\n")
                .filter_map(|line| line.strip_prefix("› "))
            {
                if !self.source_echoes.iter().any(|(_, seen)| seen == text) {
                    self.source_echoes.push((at_ms, text.to_owned()));
                }
            }
        }
        let events = self.owned.0.turns.take_events();
        let now = self.now + Duration::from_millis(at_ms);
        let session = &mut self.owned.0;
        for event in &events {
            match event {
                TurnEvent::Phase { kind, text } => {
                    use crate::tui_turns::PhaseKind;
                    let label = match kind {
                        PhaseKind::Request => "Запрос отправлен",
                        PhaseKind::Thinking => "Размышление",
                        PhaseKind::Search => "Поиск",
                        PhaseKind::Tool => "Выполнение",
                        PhaseKind::Answer => "Формируется ответ",
                    };
                    let visible = if matches!(kind, PhaseKind::Answer | PhaseKind::Request) {
                        ""
                    } else {
                        text
                    };
                    session.turn_speech.phase(label, visible, now);
                }
                TurnEvent::Ready { .. } => session.turn_speech.ready(None, now),
                TurnEvent::Failed { text, .. }
                | TurnEvent::Interrupted { text, .. }
                | TurnEvent::NeedsInput { text, .. } => session.turn_speech.important(text, now),
                TurnEvent::NewOutput { text } => session.turn_speech.new_output(
                    text,
                    crate::output_speech::SpeechMode::Important,
                    session.turns.has_semantic_turn(),
                    now,
                ),
            }
        }
        if !session.terminal.sync_pending() {
            session
                .turn_speech
                .working(session.turns.working_turn(), now);
            if let Some(text) = session.turn_speech.take_due(
                now,
                crate::output_speech::SpeechMode::Important,
                true,
                false,
            ) {
                self.spoken_activity.push((at_ms, text));
            }
        }
        self.events
            .extend(events.into_iter().map(|event| (at_ms, event)));
    }

    fn poll(&mut self, until_ms: u64) {
        while self.elapsed_ms < until_ms {
            self.observe((self.elapsed_ms + 10).min(until_ms));
        }
    }

    fn accepted(&mut self, logical: &[u8], at_ms: u64) {
        let now = self.now + Duration::from_millis(at_ms);
        let session = &mut self.owned.0;
        session.turn_speech.typing();
        let revision = session.terminal.reading_revision();
        session.menu_observer.accepted(logical, revision);
        let mut frame = session.terminal.live_snapshot();
        frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
        if let Ok(text) = std::str::from_utf8(logical) {
            if text.chars().all(|ch| !ch.is_control()) {
                self.input_echo.typed_with_composer(
                    &frame,
                    session.tui_composer.as_ref(),
                    text,
                    now,
                );
            } else {
                self.input_echo.cancel_pending();
            }
        }
        self.inputs.push(InputFrame {
            logical: logical.to_vec(),
            input: session.tui_composer.clone(),
            cursor: frame.cursor,
        });
        if session.tui_menu.is_none()
            && !session
                .tui_composer
                .as_ref()
                .is_some_and(|input| input.protected)
        {
            session.source_input.accepted(
                &frame,
                revision,
                logical,
                session.tui_composer.as_ref(),
                now,
            );
        } else {
            session.source_input.clear();
        }
        session.tui_submission.observe_at(
            session.tui_composer.as_ref(),
            session.tui_menu.is_some(),
            now,
        );
        let document = session.reading_document_snapshot_at(now);
        let request = session.tui_submission.accepted_request(
            logical,
            now,
            Arc::clone(&document),
            session.turns.has_active_turn(),
        );
        session.record_output_submission(logical, request.is_some(), &document, now);
        if let Some(request) = request {
            self.submitted.push((request.text.clone(), request.kind));
            session.turn_speech.reset();
            session
                .turns
                .submitted_request(&request.text, &request.baseline, request.kind, now);
        }
        session.record_direct_input(logical);
    }

    fn phase(&self, name: &str) -> &PhaseView {
        self.phases.iter().find(|phase| phase.name == name).unwrap()
    }
}

fn replay_codex(bytes: &[u8], metadata: &str, stop: &str, native_clipboard: bool) -> CodexReplay {
    let capture: CodexCapture = serde_json::from_str(metadata).unwrap();
    let logical_paste = native_clipboard.then(|| {
        let contract = capture.transport_contract.as_ref().unwrap();
        assert!(contract.exact_request && contract.canonical_exact);
        assert_eq!(contract.payload, contract.actual_request);
        for case in &contract.cases {
            assert!(case.exact, "{}", case.case);
            assert_eq!(case.actual, case.expected, "{}", case.case);
        }
        assert_eq!(contract.cases.len(), 23);
        crate::terminal_input::paste_bytes(&contract.payload, true)
    });
    let mut replay = CodexReplay {
        owned: owned_cli_session(
            PtySize::new(capture.columns, capture.rows),
            AdapterKind::Codex,
        ),
        now: Instant::now(),
        elapsed_ms: 0,
        events: Vec::new(),
        submitted: Vec::new(),
        phases: Vec::new(),
        frames: 0,
        inputs: Vec::new(),
        source_echoes: Vec::new(),
        input_echo: Default::default(),
        spoken_input: Vec::new(),
        spoken_activity: Vec::new(),
    };
    let mut timeline = capture.events.iter().peekable();
    let mut consumed = 0;
    let mut native_pastes = 0;
    for phase in &capture.phases {
        while timeline
            .peek()
            .is_some_and(|event| event.at_ms <= phase.at_ms)
        {
            let event = timeline.next().unwrap();
            replay.poll(event.at_ms);
            match event.kind.as_str() {
                "input" => {
                    if let Some(logical) = event.logical_bytes.as_ref() {
                        // The picker fixture used an explicit native Escape,
                        // independently of BKS's literal-text encoder. Replay
                        // its observed cancellation; do not call it a literal
                        // input transport conformance case.
                        let encoded = if logical == b"\x1b" && event.bytes.len() > 1 {
                            crate::codex::win32_key(27, 1, 27, false, false, false, false)
                        } else {
                            crate::codex::literal_input(logical)
                                .unwrap()
                                .unwrap_or_else(|| logical.clone())
                        };
                        assert_eq!(
                            encoded, event.bytes,
                            "recorded native input identity must match its encoder"
                        );
                        replay.accepted(logical, event.at_ms);
                    } else if native_clipboard && event.bytes.starts_with(b"\x1b[200~") {
                        let logical = logical_paste.as_ref().unwrap();
                        let encoded = crate::codex::literal_input(logical).unwrap().unwrap();
                        assert_eq!(
                            encoded, event.bytes,
                            "production transport must match the actual recorded wire"
                        );
                        assert!(replay.owned.0.terminal.modes().win32_input);
                        native_pastes += 1;
                        replay.accepted(logical, event.at_ms);
                    } else {
                        replay.accepted(&event.bytes, event.at_ms);
                    }
                }
                "resize" => {
                    assert!(event.columns > 0 && event.rows > 0);
                    replay
                        .owned
                        .0
                        .resize_at(
                            PtySize::new(event.columns, event.rows),
                            replay.now + Duration::from_millis(event.at_ms),
                        )
                        .unwrap();
                    replay.observe(event.at_ms);
                }
                "poll" => {
                    assert_eq!(
                        consumed, event.start_byte,
                        "every captured output byte must be retained"
                    );
                    while consumed < event.end_byte {
                        let (used, complete) = replay
                            .owned
                            .0
                            .terminal
                            .push_bytes_until_frame(&bytes[consumed..event.end_byte]);
                        assert!(used > 0);
                        consumed += used;
                        if complete {
                            replay.frames += 1;
                            replay.observe(event.at_ms);
                        }
                    }
                    // Source timestamps belong to the capture; parser sync
                    // deadlines belong to the host clock. Do not expire a
                    // just-started redraw with a future synthetic Instant.
                    replay.observe(event.at_ms);
                }
                other => panic!("unknown recorded event {other}"),
            }
        }
        replay.poll(phase.at_ms);
        assert_eq!(consumed, phase.end_byte, "{}", phase.name);
        let output = replay
            .owned
            .0
            .reading_document_snapshot_at(replay.now + Duration::from_millis(phase.at_ms));
        replay.phases.push(PhaseView {
            name: phase.name.clone(),
            input: replay.owned.0.tui_composer.clone(),
            menu: replay.owned.0.tui_menu.clone(),
            output,
            source: replay.owned.0.terminal.tui_reading_document(),
            active: replay.owned.0.turns.has_active_turn(),
            size: replay.owned.0.size,
            uses_answers: replay.owned.0.latest_output_uses_archive(),
            answer_count: replay
                .owned
                .0
                .turns
                .latest_answer()
                .map_or(0, |answer| answer.total),
            output_count: replay
                .owned
                .0
                .turns
                .latest_output()
                .map_or(0, |output| output.total),
            output_id: replay.owned.0.turns.latest_output().map(|output| output.id),
            working_turn: replay.owned.0.turns.working_turn(),
        });
        if phase.name == stop {
            break;
        }
    }
    assert_eq!(replay.phases.last().unwrap().name, stop);
    assert!(replay.frames > 10);
    assert_eq!(native_pastes, usize::from(native_clipboard));
    replay.poll(replay.elapsed_ms + 500);
    replay
}

fn canonical(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn literal_request(metadata: &str) -> String {
    let capture: CodexCapture = serde_json::from_str(metadata).unwrap();
    let bytes = capture
        .events
        .iter()
        .find_map(|event| {
            event
                .bytes
                .strip_prefix(b"\x1b[200~")
                .and_then(|text| text.strip_suffix(b"\x1b[201~"))
        })
        .unwrap();
    canonical(std::str::from_utf8(bytes).unwrap())
}

fn assert_source_editor(phase: &PhaseView, expected: &str) {
    let input = phase.input.as_ref().unwrap_or_else(|| {
        panic!(
            "{} has no source editor: {} UTF-16 units, head={:?}",
            phase.name,
            phase.output.text.encode_utf16().count(),
            phase.output.text.chars().take(160).collect::<String>()
        )
    });
    assert_eq!(input.request_text(), expected, "{} request", phase.name);
    assert_eq!(
        input.request_caret(),
        Some(expected.encode_utf16().count() as u32),
        "{} caret",
        phase.name
    );
    assert!(!input.protected);
    assert!(
        phase
            .output
            .lines
            .iter()
            .all(|line| !(input.first_row as i32..=input.last_row as i32).contains(&line.line)),
        "{} source editor must leave upper output",
        phase.name
    );
    assert!(!phase.active, "{} cannot submit an edit", phase.name);
}

fn assert_archive(
    replay: &mut CodexReplay,
    requests: &[&str],
    complete: &[bool],
    markers: &[&str],
) {
    assert_eq!(
        replay.submitted.len(),
        requests.len(),
        "{:?}",
        replay.submitted
    );
    let session = &mut replay.owned.0;
    let last = session.turns.latest_answer().unwrap();
    assert_eq!(last.total, requests.len());
    let mut target = session.turns.navigate(None, false).unwrap();
    let mut ready = Vec::new();
    let revision = session.input_revision();
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(target.number, index + 1);
        assert_eq!(target.complete, complete[index]);
        let body = session.turns.answer_text(target.id).unwrap().to_owned();
        assert!(body.contains(markers[index]), "{body}");
        for (other, marker) in markers.iter().enumerate() {
            if other != index {
                assert!(!body.contains(marker), "{body}");
            }
        }
        for chrome in [
            "Queued follow-up inputs",
            "Messages to be submitted",
            "↳ BKS_",
            "edit last queued",
            "Worked for",
        ] {
            assert!(!body.contains(chrome), "{body}");
        }
        session.turns.toggle_request(target.id).unwrap();
        assert!(session.turns.document().text.contains(request));
        assert_eq!(session.turns.answer_text(target.id), Some(body.as_str()));
        if complete[index] {
            ready.push(target.id);
        }
        target = session.turns.navigate(Some(target.id), false).unwrap();
    }
    assert_eq!(
        session.input_revision(),
        revision,
        "reading/disclosure cannot write terminal input"
    );
    let actual: Vec<_> = replay
        .events
        .iter()
        .filter_map(|(_, event)| match event {
            TurnEvent::Ready { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(actual, ready, "{:?}", replay.events);
}

fn command_captures() -> [(&'static [u8], &'static str); 3] {
    [
        (
            include_bytes!("../tests/fixtures/tui-codex-command32/status-manual.vt"),
            include_str!("../tests/fixtures/tui-codex-command32/status-manual.json"),
        ),
        (
            include_bytes!("../tests/fixtures/tui-codex-command32/status-menu-enter.vt"),
            include_str!("../tests/fixtures/tui-codex-command32/status-menu-enter.json"),
        ),
        (
            include_bytes!("../tests/fixtures/tui-codex-command32/status-tab-enter.vt"),
            include_str!("../tests/fixtures/tui-codex-command32/status-tab-enter.json"),
        ),
    ]
}

#[test]
fn codex_local_command_is_retained_after_next_reply() {
    for (bytes, metadata) in command_captures() {
        let mut replay = replay_codex(bytes, metadata, "second-ready", false);
        let first = replay.phase("first-ready");
        assert!(first.uses_answers);
        assert_eq!(first.answer_count, 1);
        for name in ["status-after-enter", "status-quiet", "second-before-enter"] {
            let phase = replay.phase(name);
            assert!(phase.uses_answers, "{name}");
            assert_eq!(phase.answer_count, 1, "{name}");
            assert!(phase.output.text.contains("Model provider:"), "{name}");
            assert!(phase.output.text.contains("Token usage:"), "{name}");
        }
        let second = replay.phase("second-ready");
        assert!(second.uses_answers);
        assert_eq!(second.answer_count, 2);
        assert_archive(
            &mut replay,
            &["BKS_COMMAND32_FIRST", "BKS_COMMAND32_SECOND"],
            &[false, true],
            &["BKS_CODEX_RESPONSE_1_BEGIN", "BKS_CODEX_RESPONSE_2_BEGIN"],
        );
        let session = &replay.owned.0;
        let latest = session.turns.latest_output().unwrap();
        assert_eq!(latest.total, 3);
        let command = session
            .turns
            .navigate_output(Some(latest.id), true)
            .unwrap();
        assert_eq!(command.kind, crate::tui_turns::OutputKind::Command);
        assert!(session
            .turns
            .output_text(command.id)
            .unwrap()
            .contains("Token usage:"));
        assert!(session.turns.answer_text(command.id).is_none());
        let first = session
            .turns
            .navigate_output(Some(command.id), true)
            .unwrap();
        assert_eq!(first.kind, crate::tui_turns::OutputKind::Answer);
        assert!(session
            .turns
            .answer_text(first.id)
            .unwrap()
            .contains("BKS_CODEX_RESPONSE_1_BEGIN"));
        assert!(session
            .turns
            .document()
            .text
            .contains("Вывод команды: /status"));
    }
}

#[test]
fn codex_local_command_archive_and_source_both_allow_status_search() {
    for (bytes, metadata) in command_captures() {
        let mut replay = replay_codex(bytes, metadata, "status-quiet", false);
        let session = &mut replay.owned.0;
        let answer = session.turns.latest_answer().unwrap();
        session.set_answers_view(true);
        session.answer_cursor = Some(answer.id);
        let archived = session.display_document_snapshot();
        assert!(archived.text.contains("Token usage:"));
        let input_revision = session.input_revision();
        let (source, start) = session.source_output_for_reading();
        assert!(!session.answers_view);
        assert_eq!(session.display_document_snapshot().text, source.text);
        // Codex redraws and removes some earlier rows. If the generic change
        // anchor cannot align that replacement, the live page remains readable.
        let tail = String::from_utf16(
            &source
                .text
                .encode_utf16()
                .skip(start as usize)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(tail.contains("/status"));
        assert!(tail.contains("Token usage:"));
        assert!(source.lines.iter().any(|line| line.start == start));
        session.search.set_query("Token usage:".into(), &source);
        assert_eq!(session.search.matches().len(), 1);
        assert_eq!(session.turns.navigate(None, true).unwrap().id, answer.id);
        assert_eq!(session.input_revision(), input_revision);
    }
}

#[test]
fn codex_local_command_blocks_survive_typing_repeated_status_picker_and_later_answers() {
    use crate::tui_turns::OutputKind::{Answer, Command};
    for (bytes, metadata) in [
        (
            &include_bytes!("../tests/fixtures/tui-codex-command33/status-repeated.vt")[..],
            include_str!("../tests/fixtures/tui-codex-command33/status-repeated.json"),
        ),
        (
            &include_bytes!(
                "../tests/fixtures/tui-codex-command33/status-repeated-model-picker.vt"
            )[..],
            include_str!("../tests/fixtures/tui-codex-command33/status-repeated-model-picker.json"),
        ),
    ] {
        let mut replay = replay_codex(bytes, metadata, "second-ready", false);
        for (name, count) in [
            ("status1-quiet", 1),
            ("first-unsent-500ms", 1),
            ("first-ready", 2),
            ("status2-quiet", 3),
            ("status3-quiet", 4),
            ("second-ready", 5),
        ] {
            let phase = replay.phase(name);
            assert_eq!(phase.output_count, count, "{name}");
        }
        let first_command_id = replay.phase("status1-quiet").output_id;
        assert_eq!(
            first_command_id,
            replay.phase("first-unsent-500ms").output_id
        );
        assert_ne!(
            replay.phase("status2-quiet").output_id,
            replay.phase("status3-quiet").output_id
        );
        if let Some(phase) = replay
            .phases
            .iter()
            .find(|phase| phase.name == "empty-enter-quiet")
        {
            assert_eq!(
                phase.output_count, 4,
                "picker and empty Enter cannot invent a result"
            );
        }
        assert_eq!(
            replay
                .submitted
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>(),
            ["BKS_COMMAND33_FIRST", "BKS_COMMAND33_SECOND"]
        );
        let session = &mut replay.owned.0;
        let mut target = session.turns.navigate_output(None, false).unwrap();
        let mut commands = Vec::new();
        for kind in [Command, Answer, Command, Command, Answer] {
            assert_eq!(target.kind, kind);
            assert_eq!(target.total, 5);
            let body = session.turns.output_text(target.id).unwrap().to_owned();
            if kind == Command {
                assert!(body.contains("Token usage:"));
                assert!(body.contains("Model provider:"));
                for chrome in [
                    "Ask Codex",
                    "for shortcuts",
                    "warnings",
                    "BKS_CODEX_RESPONSE",
                    "Select Model",
                ] {
                    assert!(!body.contains(chrome), "{body}");
                }
                assert!(session.turns.answer_text(target.id).is_none());
                assert!(session.turns.toggle_request(target.id).is_none());
                assert!(!replay.events.iter().any(|(_, event)| matches!(event,
                    TurnEvent::Ready { id } | TurnEvent::Failed { id, .. } if *id == target.id)));
                commands.push((target.id, target.start, body));
            } else {
                assert!(!body.contains("Token usage:"));
            }
            target = session
                .turns
                .navigate_output(Some(target.id), false)
                .unwrap();
        }
        assert_eq!(
            commands[1].2, commands[2].2,
            "identical results need separate entries"
        );
        assert_ne!(commands[1].0, commands[2].0);
        assert_eq!(session.turns.latest_answer().unwrap().total, 2);
        let archive = session.turns.document();
        assert_eq!(archive.text.matches("Вывод команды: /status").count(), 3);
        let start = commands[0].1;
        session
            .answer_state
            .capture(&archive, (start, start), start, false);
        let input_revision = session.input_revision();
        // The terminal no longer owns history retention once a block was saved.
        session
            .terminal
            .push_bytes(b"\x1b[2J\x1b[Hlater terminal screen");
        session.observe_tui_frame(replay.now + Duration::from_millis(replay.elapsed_ms + 1_000));
        session
            .resize_at(
                PtySize::new(80, 20),
                replay.now + Duration::from_millis(replay.elapsed_ms + 1_100),
            )
            .unwrap();
        let retained = session.turns.document();
        assert_eq!(retained.text, archive.text);
        assert_eq!(
            session.answer_state.restore(&retained).selection,
            (start, start)
        );
        session.set_answers_view(true);
        let displayed = session.display_document_snapshot();
        session.search.set_query("Token usage:".into(), &displayed);
        assert_eq!(session.search.matches().len(), 3);
        assert_eq!(session.input_revision(), input_revision);
        session.turns.clear();
        session.observe_tui_frame(replay.now + Duration::from_millis(replay.elapsed_ms + 2_000));
        assert!(session.turns.latest_output().is_none());
        assert!(session.turns.document().text.is_empty());
    }
}

#[test]
fn codex_tool_work_speaks_during_real_quiet_polls_and_archives_only_the_final_answer() {
    let mut replay = replay_codex(
        include_bytes!("../tests/fixtures/tui-codex-tools34/tool-workflow.vt"),
        include_str!("../tests/fixtures/tui-codex-tools34/tool-workflow.json"),
        "prose-ready",
        false,
    );
    for name in [
        "work-observation-05s",
        "work-observation-10s",
        "work-observation-15s",
    ] {
        assert!(
            replay.phase(name).working_turn.is_some(),
            "{name} input={:?} active={}\n{}\nevents={:?}\nspeech={:?}",
            replay
                .phase(name)
                .input
                .as_ref()
                .map(|c| (&c.text, c.first_row, c.last_row)),
            replay.phase(name).active,
            replay.phase(name).output.text,
            replay.events,
            replay.spoken_activity
        );
    }
    assert!(replay.phase("main-ready").working_turn.is_none());
    assert!(replay.phase("prose-ready").working_turn.is_none());
    assert_eq!(
        replay.phase("collapsed-tool-stdout-visible").answer_count,
        0,
        "commentary and tool cells cannot masquerade as the final answer"
    );
    assert!(
        replay
            .spoken_activity
            .iter()
            .any(|(at, text)| (5_493..20_726).contains(at) && text == "Работа продолжается"),
        "{:?}",
        replay.spoken_activity
    );
    assert!(
        replay
            .spoken_activity
            .iter()
            .any(|(_, text)| text.starts_with("Выполнение: Ran Get-Content")),
        "{:?}",
        replay.spoken_activity
    );
    assert!(replay
        .spoken_activity
        .iter()
        .all(|(_, text)| !text.contains("TOOL_STDOUT34")));
    assert_archive(
        &mut replay,
        &["BKS_TOOLS34_MAIN", "BKS_TOOLS34_FINAL_RAN_PROSE"],
        &[true, true],
        &["BKS_FINAL34_BEGIN", "BKS_FINAL34_PROSE_BEGIN"],
    );
    let log = &replay.owned.0.turns;
    let first = log.navigate(None, false).unwrap();
    let answer = log.answer_text(first.id).unwrap();
    for activity in [
        "Ran Get-Content",
        "Start-Sleep",
        "TOOL_STDOUT34",
        "BKS_COMMENTARY34",
        "ctrl+t",
    ] {
        assert!(!answer.contains(activity), "{answer}");
    }
    assert!(answer.starts_with("BKS_FINAL34_BEGIN"), "{answer}");
    assert!(answer.contains("BKS_FINAL34_END"));
    let last = log.latest_answer().unwrap();
    assert!(log
        .answer_text(last.id)
        .unwrap()
        .starts_with("Ran verification is ordinary final prose"));
    assert_eq!(
        crate::direct_accessibility::current_line(&log.document(), first.start).as_deref(),
        Some("BKS_FINAL34_BEGIN")
    );
}

#[test]
fn codex_production_session_paste_80_and_400_keep_source_requests_and_one_ready() {
    for (bytes, metadata) in [
        (
            &include_bytes!("../tests/fixtures/tui-codex-production30/paste-80.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/paste-80.json"),
        ),
        (
            &include_bytes!("../tests/fixtures/tui-codex-production30/paste-400.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/paste-400.json"),
        ),
    ] {
        let request = literal_request(metadata);
        let mut replay = replay_codex(bytes, metadata, "submitted", false);
        assert_source_editor(replay.phase("paste"), &request);
        assert_archive(
            &mut replay,
            &[&request],
            &[true],
            &["BKS_CODEX_RESPONSE_1_BEGIN"],
        );
    }
}

#[test]
fn codex_production_session_4k_binds_whole_history_echo_after_partial_viewport() {
    let metadata =
        include_str!("../tests/fixtures/tui-codex-production30/paste-4k-unique-echo.json");
    let request = literal_request(metadata);
    assert_eq!(request.len(), 4_096);
    let mut replay = replay_codex(
        include_bytes!("../tests/fixtures/tui-codex-production30/paste-4k-unique-echo.vt"),
        metadata,
        "submitted",
        false,
    );
    let paste = replay.phase("paste");
    assert_source_editor(paste, &request);
    let input = paste.input.as_ref().unwrap();
    assert!(input.is_partial());
    assert!(!input.text.contains("BKS_UNIQUE_4K_BEGIN"));
    assert!(input.text.contains("BKS_UNIQUE_4K_END_MARK"));
    assert_archive(
        &mut replay,
        &[&request],
        &[true],
        &["BKS_CODEX_RESPONSE_1_BEGIN"],
    );
    for (_, event) in &replay.events {
        if let TurnEvent::Phase {
            kind: crate::tui_turns::PhaseKind::Answer,
            text,
        }
        | TurnEvent::NewOutput { text } = event
        {
            assert!(
                !text.contains("w0345") && !text.contains("BKS_UNIQUE_4K_BEGIN"),
                "{event:?}"
            );
        }
    }
}

#[test]
fn codex_production_session_native_unicode_proves_transport_and_rejects_renderer_glyph_loss() {
    let metadata =
        include_str!("../tests/fixtures/tui-codex-production30/native-clipboard-unicode.json");
    let capture: CodexCapture = serde_json::from_str(metadata).unwrap();
    let request = canonical(&capture.transport_contract.as_ref().unwrap().payload);
    let replay = replay_codex(
        include_bytes!("../tests/fixtures/tui-codex-production30/native-clipboard-unicode.vt"),
        metadata,
        "submitted",
        true,
    );
    let source = replay.phase("unicode-input");
    assert!(
        source.input.is_none(),
        "a renderer which changed glyphs cannot prove the original editor"
    );
    assert!(source.output.text.contains("CASE_200B=[ab]"));
    assert!(source.output.text.contains("CASE_emoji-zwj=[👩]💻"));
    assert!(!source.output.text.contains(&request));
    assert!(replay.submitted.iter().all(|(text, _)| text == &request));
    assert!(!replay.owned.0.turns.has_answers());
    assert!(!replay.events.iter().any(|(_, event)| matches!(
        event,
        TurnEvent::Ready { .. }
            | TurnEvent::Phase {
                kind: crate::tui_turns::PhaseKind::Request,
                ..
            }
    )));
}

#[test]
fn codex_production_session_combining_cursor_gap_waits_for_full_echo_before_archiving_literal_intent(
) {
    for (paced, bytes, metadata) in [
        (
            false,
            &include_bytes!("../tests/fixtures/tui-codex-production30/typed-native-fast.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/typed-native-fast.json"),
        ),
        (
            true,
            &include_bytes!("../tests/fixtures/tui-codex-production30/typed-native-paced.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/typed-native-paced.json"),
        ),
    ] {
        let capture: CodexCapture = serde_json::from_str(metadata).unwrap();
        let contract = capture.typed_contract.as_ref().unwrap();
        assert!(contract.exact_request);
        assert_eq!(contract.actual_request, contract.payload);
        assert_eq!(contract.payload, "Ж界🙂 e\u{301} []");
        let logical: Vec<_> = capture
            .events
            .iter()
            .filter_map(|event| event.logical_bytes.as_ref())
            .flatten()
            .copied()
            .collect();
        assert_eq!(logical, contract.payload.as_bytes());
        let mut replay = replay_codex(bytes, metadata, "submitted", false);
        let typed = replay.phase("typed");
        let input = typed.input.as_ref().unwrap();
        let observed = "Ж界🙂 e\u{301}[]";
        assert_eq!(input.text, observed);
        assert_eq!(input.request_text(), observed);
        assert_eq!(input.caret, Some(9));
        assert!(typed
            .source
            .text
            .split("\r\n")
            .any(|line| line == format!("› {observed}")));
        assert!(replay
            .phase("submitted")
            .source
            .text
            .split("\r\n")
            .any(|line| line == format!("› {}", contract.payload)));
        assert!(!typed.active);
        let enter = replay
            .inputs
            .iter()
            .find(|event| event.logical == b"\r")
            .unwrap();
        let proof = |input: &crate::tui_projection::Composer| {
            (
                input.text.clone(),
                input.caret,
                input.request_text().to_owned(),
                input.is_partial(),
                input.accepts_requests(),
            )
        };
        assert_eq!(
            replay.submitted.len(),
            1,
            "typed={:?}; before Enter={:?}",
            proof(input),
            enter.input.as_ref().map(proof)
        );
        assert_eq!(replay.submitted[0].0, contract.payload);
        if paced {
            let mark = replay
                .inputs
                .iter()
                .find(|event| event.logical == "\u{301}".as_bytes())
                .unwrap();
            assert_eq!(mark.input.as_ref().unwrap().caret, Some(6));
            let space = replay
                .inputs
                .iter()
                .find(|event| {
                    event.logical == b" "
                        && event
                            .input
                            .as_ref()
                            .is_some_and(|input| input.text.ends_with("e\u{301}"))
                })
                .unwrap();
            assert_eq!(space.input.as_ref().unwrap().caret, Some(5));
            assert_eq!(space.cursor.unwrap().column, 8);
            let bracket = replay
                .inputs
                .iter()
                .find(|event| event.logical == b"[")
                .unwrap();
            assert_eq!(bracket.input.as_ref().unwrap().caret, Some(7));
            assert_eq!(bracket.cursor.unwrap().column, 9);
            assert_eq!(
                space.input.as_ref().unwrap().text,
                bracket.input.as_ref().unwrap().text
            );
        }
        let echo_at = replay
            .source_echoes
            .iter()
            .find(|(_, text)| text == &contract.payload)
            .unwrap()
            .0;
        assert!(replay.events.iter().all(|(at, event)| !matches!(
            event,
            TurnEvent::Ready { .. }
                | TurnEvent::Phase {
                    kind: crate::tui_turns::PhaseKind::Request,
                    ..
                }
        ) || *at >= echo_at));
        assert_archive(
            &mut replay,
            &[&contract.payload],
            &[true],
            &["BKS_CODEX_RESPONSE_1_BEGIN"],
        );
    }
}

#[test]
fn codex_production_session_supported_fast_and_paced_unicode_keep_exact_source_and_archive() {
    for (bytes, metadata) in [
        (
            &include_bytes!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-fast.vt"
            )[..],
            include_str!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-fast.json"
            ),
        ),
        (
            &include_bytes!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-paced.vt"
            )[..],
            include_str!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-paced.json"
            ),
        ),
    ] {
        let capture: CodexCapture = serde_json::from_str(metadata).unwrap();
        let contract = capture.typed_contract.as_ref().unwrap();
        assert!(contract.exact_request);
        assert_eq!(contract.payload, "Ж界🙂 []");
        assert_eq!(contract.actual_request, contract.payload);
        let logical: Vec<_> = capture
            .events
            .iter()
            .filter_map(|event| event.logical_bytes.as_ref())
            .flatten()
            .copied()
            .collect();
        assert_eq!(logical, contract.payload.as_bytes());
        let mut replay = replay_codex(bytes, metadata, "submitted", false);
        assert_source_editor(replay.phase("typed"), &contract.payload);
        assert_archive(
            &mut replay,
            &[&contract.payload],
            &[true],
            &["BKS_CODEX_RESPONSE_1_BEGIN"],
        );
    }
}

#[test]
fn codex_production_session_shift_enter_is_a_source_newline_until_plain_enter() {
    let mut replay = replay_codex(
        include_bytes!("../tests/fixtures/tui-codex-production30/key-win-shift.vt"),
        include_str!("../tests/fixtures/tui-codex-production30/key-win-shift.json"),
        "submitted",
        false,
    );
    assert_source_editor(replay.phase("before-key"), "BEFORE");
    assert_source_editor(replay.phase("after-key"), "BEFORE\r\n");
    assert_source_editor(replay.phase("after-tail"), "BEFORE\r\nAFTER");
    assert_archive(
        &mut replay,
        &["BEFORE\r\nAFTER"],
        &[true],
        &["BKS_CODEX_RESPONSE_1_BEGIN"],
    );
}

#[test]
fn codex_first_typed_character_replacing_the_hint_is_echoed_once() {
    for (bytes, metadata) in [
        (
            &include_bytes!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-paced.vt"
            )[..],
            include_str!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-paced.json"
            ),
        ),
        (
            &include_bytes!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-fast.vt"
            )[..],
            include_str!(
                "../tests/fixtures/tui-codex-production30/typed-native-supported-fast.json"
            ),
        ),
    ] {
        let replay = replay_codex(bytes, metadata, "typed", false);
        assert_eq!(replay.spoken_input.concat(), "Ж界🙂 []");
        assert!(replay
            .spoken_input
            .first()
            .is_some_and(|text| text.starts_with('Ж')));
    }
}

#[test]
fn codex_production_session_resize_and_slash_menu_preserve_source_input_without_requests() {
    let metadata = include_str!("../tests/fixtures/tui-codex-production30/resize-menu.json");
    let request = literal_request(metadata);
    let replay = replay_codex(
        include_bytes!("../tests/fixtures/tui-codex-production30/resize-menu.vt"),
        metadata,
        "slash-dismiss",
        false,
    );
    for (name, columns, rows) in [
        ("draft-wide", 100, 29),
        ("draft-narrow", 48, 19),
        ("draft-wider", 120, 35),
    ] {
        let phase = replay.phase(name);
        assert_eq!(phase.size, PtySize::new(columns, rows));
        assert_source_editor(phase, &request);
    }
    assert_source_editor(replay.phase("draft-clear"), "");
    let first = replay
        .phase("slash-menu")
        .menu
        .as_ref()
        .expect("slash menu must be source-confirmed");
    let moved = replay
        .phase("slash-menu-down")
        .menu
        .as_ref()
        .expect("navigation must retain the same source menu");
    assert_eq!(first.items, moved.items);
    assert_ne!(first.selected, moved.selected);
    assert!(replay.phase("slash-dismiss").menu.is_none());
    assert!(replay.submitted.is_empty());
    assert!(!replay.owned.0.turns.has_answers() && !replay.owned.0.turns.has_active_turn());
    assert!(!replay.events.iter().any(|(_, event)| matches!(
        event,
        TurnEvent::Ready { .. }
            | TurnEvent::Phase {
                kind: crate::tui_turns::PhaseKind::Request,
                ..
            }
    )));
}

#[test]
fn codex_production_session_queue_and_steering_follow_source_order_and_unique_ready() {
    for (bytes, metadata, requests, complete, markers, kinds) in [
        (
            &include_bytes!("../tests/fixtures/tui-codex-production30/queue-repeat.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/queue-repeat.json"),
            vec!["BKS_SAME_REQUEST", "BKS_SAME_REQUEST"],
            vec![true, true],
            vec!["BKS_CODEX_RESPONSE_1_BEGIN", "BKS_CODEX_RESPONSE_2_BEGIN"],
            vec![RequestKind::Submit, RequestKind::Queue],
        ),
        (
            &include_bytes!("../tests/fixtures/tui-codex-production30/steer-repeat.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/steer-repeat.json"),
            vec!["BKS_SAME_REQUEST", "BKS_SAME_REQUEST"],
            vec![false, true],
            vec!["BKS_CODEX_RESPONSE_1_BEG", "BKS_CODEX_RESPONSE_2_BEGIN"],
            vec![RequestKind::Submit, RequestKind::Steer],
        ),
        (
            &include_bytes!("../tests/fixtures/tui-codex-production30/queue-then-steer.vt")[..],
            include_str!("../tests/fixtures/tui-codex-production30/queue-then-steer.json"),
            vec!["BKS_SAME_REQUEST", "BKS_THIRD_STEER", "BKS_SAME_REQUEST"],
            vec![false, true, true],
            vec![
                "BKS_CODEX_RESPONSE_1_BEG",
                "BKS_CODEX_RESPONSE_2_BEGIN",
                "BKS_CODEX_RESPONSE_3_BEGIN",
            ],
            vec![RequestKind::Submit, RequestKind::Queue, RequestKind::Steer],
        ),
    ] {
        let mut replay = replay_codex(bytes, metadata, "after-release", false);
        assert_eq!(
            replay
                .submitted
                .iter()
                .map(|(_, kind)| *kind)
                .collect::<Vec<_>>(),
            kinds
        );
        assert_archive(&mut replay, &requests, &complete, &markers);
    }
}
