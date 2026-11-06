use super::tests::{scene, settle, valid_document, Scene};
use super::*;

fn request_answer(
    log: &mut TurnLog,
    baseline: &ReadingDocument,
    request: &str,
    answer: &str,
    now: Instant,
) -> Scene {
    log.submitted(request, baseline, now);
    settle(log, &scene("Thinking: working\r\nStatus: busy", true), now);
    let done = scene(&format!("Assistant: {answer}\r\nStatus: done"), true);
    settle(log, &done, now + Duration::from_secs(1));
    done
}

fn text_at(document: &ReadingDocument, start: u32, text: &str) {
    let units: Vec<_> = document.text.encode_utf16().collect();
    assert_eq!(
        String::from_utf16(&units[start as usize..start as usize + text.encode_utf16().count()])
            .unwrap(),
        text
    );
}

#[test]
fn unicode_multiline_request_is_collapsed_until_explicitly_opened() {
    let now = Instant::now();
    let baseline = scene("", true);
    let request = format!(
        "Ж🙂e\u{301} {}\n\n  第二行\tend",
        "long request ".repeat(12)
    );
    let expected = request.replace('\n', "\r\n");
    let mut log = TurnLog::new();
    request_answer(&mut log, &baseline.output, &request, "source answer", now);
    let answer = log.latest_answer().unwrap();
    let header = log.request_at((0, 0)).unwrap();
    assert_eq!(header.id, answer.id);
    assert!(!header.expanded);
    assert!(log
        .document()
        .text
        .starts_with("Ваш запрос, свёрнуто: Ж🙂e\u{301} "));
    assert!(!log.document().text.contains("第二行"));
    assert!(log
        .document()
        .text
        .contains("…\r\nОтвет 1\r\nsource answer"));
    valid_document(&log.document());
    log.take_events();
    let opened = log.toggle_request(answer.id).unwrap();
    assert!(opened.expanded);
    assert!(log
        .document()
        .text
        .contains(&format!("\r\n{expected}\r\nОтвет 1")));
    assert_eq!(log.answer_text(answer.id), Some("source answer"));
    text_at(
        &log.document(),
        log.latest_answer().unwrap().start,
        "source answer",
    );
    assert!(log.take_events().is_empty());
    valid_document(&log.document());
    assert!(!log.toggle_request(answer.id).unwrap().expanded);
    assert!(!log.document().text.contains("第二行"));
    assert_eq!(log.latest_answer(), Some(answer));
}

#[test]
fn only_generated_headers_with_a_collapsed_caret_are_actions() {
    let baseline = scene("", true);
    let mut log = TurnLog::new();
    let fake = "Ваш запрос, свёрнуто: ordinary answer prose";
    request_answer(&mut log, &baseline.output, fake, fake, Instant::now());
    let header = log.request_at((0, 0)).unwrap();
    assert!(log.request_at((header.end, header.end)).is_some());
    assert!(log.request_at((header.start, header.end)).is_none());
    let answer = log.latest_answer().unwrap();
    assert!(log.request_at((answer.start, answer.start)).is_none());
    log.toggle_request(answer.id).unwrap();
    let header = log.requests[0];
    assert!(log.request_at((header.end + 2, header.end + 2)).is_none());
    let revision = log.revision();
    assert!(log.toggle_request(u64::MAX).is_none());
    assert_eq!(log.revision(), revision);
    valid_document(&log.document());
}

#[test]
fn disclosure_keeps_answer_ids_offsets_styles_and_other_requests_independent() {
    let mut log = TurnLog::new();
    let now = Instant::now();
    let baseline = scene("", true);
    let done = request_answer(
        &mut log,
        &baseline.output,
        "same 🙂\nrequest",
        "\x1b[31;1mfirst",
        now,
    );
    let first = log.latest_answer().unwrap();
    let original = log.document();
    let style = original
        .styles
        .iter()
        .find(|span| span.end > first.start)
        .unwrap()
        .style;
    request_answer(
        &mut log,
        &done.output,
        "same 🙂\nrequest",
        "second",
        now + Duration::from_secs(3),
    );
    let second = log.latest_answer().unwrap();
    log.toggle_request(first.id).unwrap();
    let previous = log.navigate(Some(second.id), true).unwrap();
    let next = log.navigate(Some(previous.id), false).unwrap();
    assert_eq!((previous.id, next.id), (first.id, second.id));
    text_at(&log.document(), previous.start, "first");
    text_at(&log.document(), next.start, "second");
    assert_eq!(
        log.document()
            .styles
            .iter()
            .find(|span| span.end > previous.start)
            .unwrap()
            .style,
        style
    );
    assert!(log.requests[0].expanded);
    assert!(!log.requests[1].expanded);
    log.toggle_request(second.id).unwrap();
    assert!(log.requests.iter().all(|header| header.expanded));
    assert_eq!(log.document().text.matches("same 🙂\r\nrequest").count(), 2);
    log.set_adapter(crate::tui_adapters::AdapterKind::Kimi);
    assert!(log.requests.iter().all(|header| header.expanded));
    valid_document(&log.document());
    log.clear();
    assert!(log.document().text.is_empty());
    assert!(log.request_at((0, 0)).is_none());
    assert!(log.toggle_request(first.id).is_none());
}

#[test]
fn archived_request_keeps_text_beyond_the_existing_semantic_evidence_limit() {
    let request = format!("{}\r\nЖ🙂 request tail", "x".repeat(MAX_EVENT_BYTES + 100));
    let baseline = scene("", true);
    let mut log = TurnLog::new();
    request_answer(
        &mut log,
        &baseline.output,
        &request,
        "answer",
        Instant::now(),
    );
    log.toggle_request(log.latest_answer().unwrap().id).unwrap();
    assert!(log.document().text.contains(&request));
    valid_document(&log.document());
}

#[test]
fn saved_requests_normalize_line_endings_and_explicitly_label_size_limits() {
    assert_eq!(
        saved_request("one\r\ntwo\rthree\nfour\t🙂\0\x1b"),
        "one\r\ntwo\r\nthree\r\nfour\t🙂"
    );
    let request = saved_request(&"Ж🙂".repeat(MAX_REQUEST_BYTES));
    assert!(request.len() <= MAX_REQUEST_BYTES);
    assert!(request.ends_with("[Конец запроса не сохранён: ограничение размера]"));
}

#[test]
fn hidden_requests_count_towards_retention_and_are_removed_with_their_answers() {
    let mut log = TurnLog::new();
    for id in 1..=24 {
        log.turns.push_back(Turn {
            kind: OutputKind::Answer,
            id,
            request: saved_request(&"Ж🙂".repeat(MAX_REQUEST_BYTES / 6)),
            request_expanded: false,
            answer: RichText::plain("small answer"),
            complete: true,
            failed: false,
        });
        log.retain();
    }
    log.rebuild();
    assert!(log.turns.len() < 24);
    assert!(log.toggle_request(1).is_none());
    let ids: Vec<_> = log.turns.iter().map(|turn| turn.id).collect();
    for id in ids {
        log.toggle_request(id).unwrap();
    }
    assert!(log.document().text.len() <= MAX_BYTES);
    assert_eq!(log.requests.len(), log.targets.len());
    assert_eq!(log.latest_answer().unwrap().id, 24);
    valid_document(&log.document());
}

#[test]
fn new_answer_output_preserves_an_expanded_request_bookmark() {
    let mut log = TurnLog::new();
    let now = Instant::now();
    let baseline = scene("", true);
    let request = "first line\nselected Ж🙂 text\nlast line";
    let done = request_answer(&mut log, &baseline.output, request, "first answer", now);
    let id = log.latest_answer().unwrap().id;
    log.toggle_request(id).unwrap();
    let old = log.document();
    let byte = old.text.find("selected Ж🙂 text").unwrap();
    let start = old.text[..byte].encode_utf16().count() as u32;
    let selected = (
        start,
        start + "selected Ж🙂 text".encode_utf16().count() as u32,
    );
    let mut reader = crate::reading_state::ReadingState::default();
    reader.capture(&old, selected, log.requests[0].start, false);
    request_answer(
        &mut log,
        &done.output,
        "next request",
        "second answer",
        now + Duration::from_secs(3),
    );
    let restored = reader.restore(&log.document());
    assert_eq!(restored.selection, selected);
    assert_eq!(restored.viewport_offset, log.requests[0].start);
    assert!(log.requests[0].expanded);
}
