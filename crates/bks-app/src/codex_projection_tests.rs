use super::*;
use crate::source_input::SourceInput;
use crate::tui_adapters::AdapterKind;
use bks_terminal::TerminalModel;
use std::time::{Duration, Instant};

#[test]
fn codex_clipped_literal_input_publishes_only_source_but_binds_the_complete_user_intent() {
    let mut text = (0..700)
        .map(|index| format!("w{index:04} "))
        .collect::<String>();
    text.truncate(4_093);
    text.push_str("END");
    assert_eq!(text.len(), 4_096);
    let tail = &text[text.len() - 80..];
    let now = Instant::now();
    let (before, document) = textarea(100, &["hint"], (0, 0), true, 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
        .input
        .unwrap();
    let mut source = SourceInput::default();
    source.accepted(
        &before,
        1,
        &crate::terminal_input::paste_bytes(&text, true),
        Some(&initial),
        now,
    );
    let (after, output) = textarea(100, &[tail], (0, 80), false, 2);
    let projected = project_for_adapter(
        AdapterKind::Codex,
        &after,
        &output,
        Some(&initial),
        source.evidence(&after, 2, now),
    );
    let input = projected.input.unwrap();
    assert_eq!(input.text, tail);
    assert_eq!(input.caret, Some(80));
    assert!(input.is_partial());
    assert_eq!(input.request_text(), text);
    assert_eq!(input.request_caret(), Some(4_096));
    assert!(!projected.output.text.contains(tail));
    let retained = project_for_adapter(AdapterKind::Codex, &after, &output, Some(&input), None)
        .input
        .unwrap();
    assert_eq!(retained.text, tail);
    assert!(retained.is_partial());
    assert_eq!(retained.request_text(), text);
    let mut resized = after.clone();
    resized.rows -= 1;
    resized.cells.truncate(resized.cols * resized.rows);
    let resized_input =
        project_resized_for_adapter(AdapterKind::Codex, &resized, &output, &retained)
            .input
            .expect("height resize must retain a proven clipped draft");
    assert!(resized_input.is_partial());
    assert_eq!(resized_input.text, tail);
    assert_eq!(resized_input.caret, Some(80));
    assert_eq!(resized_input.request_text(), text);
    assert_eq!(resized_input.request_caret(), Some(4_096));
    let mut old_history = retained.clone();
    old_history.source.history = 5;
    assert!(
        project_resized_for_adapter(AdapterKind::Codex, &resized, &output, &old_history)
            .input
            .is_some()
    );
    let mut other_buffer = resized.clone();
    other_buffer.alternate_screen = !other_buffer.alternate_screen;
    assert!(
        project_resized_for_adapter(AdapterKind::Codex, &other_buffer, &output, &old_history)
            .input
            .is_none()
    );
    let mut submission = crate::tui_submission::Submission::default();
    submission.observe(Some(&retained), false);
    let request = submission
        .accepted_request(b"\r", now, std::sync::Arc::new(projected.output), false)
        .unwrap();
    assert_eq!(request.text, text);
    assert_eq!(request.kind, crate::tui_submission::RequestKind::Submit);

    let corrupted = format!("X{}", &tail[1..]);
    let (changed, changed_document) = textarea(100, &[&corrupted], (0, 80), false, 2);
    assert!(project_for_adapter(
        AdapterKind::Codex,
        &changed,
        &changed_document,
        Some(&initial),
        source.evidence(&changed, 2, now)
    )
    .input
    .is_none());
}

fn textarea(
    columns: usize,
    body: &[&str],
    cursor: (usize, usize),
    placeholder: bool,
    revision: u64,
) -> (TerminalSnapshot, ReadingDocument) {
    textarea_at(2, columns, body, cursor, placeholder, revision)
}

#[test]
fn codex_private_literal_hint_cannot_cross_an_empty_composer_or_buffer_switch() {
    let now = Instant::now();
    let (before, document) = textarea(100, &["hint"], (0, 0), true, 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
        .input
        .unwrap();
    let mut evidence = SourceInput::default();
    evidence.accepted(&before, 1, b"ol", Some(&initial), now);
    let (frame, document) = textarea(100, &["ol"], (0, 2), false, 2);
    let current = project_for_adapter(
        AdapterKind::Codex,
        &frame,
        &document,
        Some(&initial),
        evidence.evidence(&frame, 2, now),
    )
    .input
    .unwrap();
    for alternate_screen in [false, true] {
        let mut submission = crate::tui_submission::Submission::default();
        submission.observe(Some(&current), false);
        assert!(submission.accepted(b"d").is_none());
        let mut alternate = initial.clone();
        alternate.source.alternate = alternate_screen;
        submission.observe(Some(&alternate), false);
        assert!(submission.accepted(b"new").is_none());
        assert_eq!(submission.accepted(b"\r").as_deref(), Some("new"));
    }
}

fn textarea_at(
    top: usize,
    columns: usize,
    body: &[&str],
    cursor: (usize, usize),
    placeholder: bool,
    revision: u64,
) -> (TerminalSnapshot, ReadingDocument) {
    let mut model = TerminalModel::new(columns, 40);
    model.push_bytes(b"\x1b[31mprior answer\x1b[0m");
    let bottom = top + body.len() + 1;
    for row in top..=bottom {
        model.push_bytes(
            format!(
                "\x1b[{};1H\x1b[48;2;40;40;40m{}",
                row + 1,
                " ".repeat(columns)
            )
            .as_bytes(),
        );
    }
    model.push_bytes(format!("\x1b[{};1H\x1b[1m›\x1b[22m", top + 2).as_bytes());
    for (row, text) in body.iter().enumerate() {
        model.push_bytes(
            format!(
                "\x1b[{};3H{}{text}\x1b[22m",
                top + row + 2,
                if placeholder { "\x1b[2m" } else { "" }
            )
            .as_bytes(),
        );
    }
    model.push_bytes(
        format!(
            "\x1b[0m\x1b[{};3Hfooter\x1b[{};{}H",
            bottom + 3,
            top + cursor.0 + 2,
            cursor.1 + 3
        )
        .as_bytes(),
    );
    let mut document = model.tui_reading_document();
    document.revision = revision;
    (model.live_snapshot(), document)
}

#[test]
fn codex_startup_repaint_keeps_only_an_exact_accepted_insertion_across_new_history() {
    let now = Instant::now();
    let (before, document) = textarea_at(2, 100, &["hint"], (0, 0), true, 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
        .input
        .unwrap();
    let mut source = SourceInput::default();
    source.accepted(&before, 1, b"first request", Some(&initial), now);
    let (mut after, output) = textarea_at(5, 100, &["first request"], (0, 13), false, 2);
    after.history_size = 1;
    let (accepted, baseline) = source.confirmation(&after, 2, now).unwrap();
    let projection = project_recovered_for_adapter(
        AdapterKind::Codex,
        &after,
        &output,
        baseline.unwrap(),
        accepted,
    );
    let input = projection.input.unwrap();
    assert_eq!(input.text, "first request");
    assert_eq!(input.caret, Some(13));
    assert_eq!(input.first_row, 5);
    source.acknowledge(&input);
    assert!(source.evidence(&after, 3, now).is_none());

    let mut source = SourceInput::default();
    source.accepted(&before, 1, b"first request", Some(&initial), now);
    let (mut changed, changed_document) = textarea_at(5, 100, &["other input"], (0, 11), false, 2);
    changed.history_size = 1;
    let (accepted, baseline) = source.confirmation(&changed, 2, now).unwrap();
    assert!(project_recovered_for_adapter(
        AdapterKind::Codex,
        &changed,
        &changed_document,
        baseline.unwrap(),
        accepted,
    )
    .input
    .is_none());
}

#[test]
fn codex_typing_during_startup_history_growth_keeps_the_original_literal_origin() {
    let now = Instant::now();
    let (before, document) = textarea_at(2, 100, &["hint"], (0, 0), true, 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
        .input
        .unwrap();
    let mut source = SourceInput::default();
    source.accepted(&before, 1, b"first", Some(&initial), now);
    let (mut middle, _) = textarea_at(5, 100, &["first"], (0, 5), false, 2);
    middle.history_size = 1;
    source.accepted(
        &middle,
        2,
        b" request",
        None,
        now + Duration::from_millis(10),
    );
    let (mut after, output) = textarea_at(5, 100, &["first request"], (0, 13), false, 3);
    after.history_size = 1;
    let (evidence, baseline) = source
        .confirmation(&after, 3, now + Duration::from_millis(20))
        .unwrap();
    assert_eq!(evidence.text, "first request");
    let recovered = project_recovered_for_adapter(
        AdapterKind::Codex,
        &after,
        &output,
        baseline.unwrap(),
        evidence,
    )
    .input
    .unwrap();
    assert_eq!(recovered.text, "first request");
    assert_eq!(recovered.caret, Some(13));
}

#[test]
fn codex_initial_acquisition_needs_a_new_source_at_the_recorded_input_origin() {
    let now = Instant::now();
    let (before, _) = textarea(100, &["hint"], (0, 0), true, 1);
    let mut source = SourceInput::default();
    source.accepted(&before, 1, b"early input", None, now);
    let (after, output) = textarea(100, &["early input"], (0, 11), false, 2);
    assert!(
        project_for_adapter(AdapterKind::Codex, &after, &output, None, None)
            .input
            .is_none()
    );
    let input = project_for_adapter(
        AdapterKind::Codex,
        &after,
        &output,
        None,
        source.evidence(&after, 2, now),
    )
    .input
    .unwrap();
    assert_eq!(input.text, "early input");
    let mut wrong_origin = source.evidence(&after, 2, now).unwrap().clone();
    wrong_origin.row += 1;
    assert!(project_for_adapter(
        AdapterKind::Codex,
        &after,
        &output,
        None,
        Some(&wrong_origin)
    )
    .input
    .is_none());
}

#[test]
fn codex_first_literal_paste_over_64_requires_exact_new_source_before_projection() {
    for count in [63, 64, 65, 80] {
        let now = Instant::now();
        let (before, document) = textarea(100, &["unentered hint"], (0, 0), true, 1);
        let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
            .input
            .unwrap();
        let text = "a".repeat(count);
        let bytes = crate::terminal_input::paste_bytes(&text, true);
        let mut source = SourceInput::default();
        source.accepted(&before, 1, &bytes, Some(&initial), now);
        assert!(source.evidence(&before, 1, now).is_none());
        assert!(
            project_for_adapter(AdapterKind::Codex, &before, &document, Some(&initial), None)
                .input
                .unwrap()
                .text
                .is_empty()
        );
        let (after, output) = textarea(100, &[&text], (0, count), false, 2);
        let accepted = source.evidence(&after, 2, now);
        assert!(accepted.is_some(), "{count}");
        assert!(
            project_for_adapter(AdapterKind::Codex, &after, &output, Some(&initial), None)
                .input
                .is_none()
        );
        let projection = project_for_adapter(
            AdapterKind::Codex,
            &after,
            &output,
            Some(&initial),
            accepted,
        );
        let input = projection.input.unwrap();
        assert_eq!(input.text, text);
        assert_eq!(input.caret, Some(count as u32));
        assert!(!projection.output.text.contains(&text));
        assert!(projection.output.text.contains("prior answer"));
        source.acknowledge(&input);
        assert!(source.evidence(&after, 3, now).is_none());
    }
}

#[test]
fn codex_paste_keeps_unicode_real_newlines_and_verified_physical_wraps() {
    for (text, body, cursor) in [
        (
            "Ж界🙂\r\n  second line".to_owned(),
            vec!["Ж界🙂".to_owned(), "  second line".to_owned()],
            (1, 13),
        ),
        (
            "a".repeat(80),
            vec!["a".repeat(37), "a".repeat(37), "a".repeat(6)],
            (2, 6),
        ),
    ] {
        let now = Instant::now();
        let (before, document) = textarea(40, &["hint"], (0, 0), true, 1);
        let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
            .input
            .unwrap();
        let mut source = SourceInput::default();
        source.accepted(
            &before,
            1,
            &crate::terminal_input::paste_bytes(&text, true),
            Some(&initial),
            now,
        );
        let body: Vec<_> = body.iter().map(String::as_str).collect();
        let (after, output) = textarea(40, &body, cursor, false, 2);
        let projection = project_for_adapter(
            AdapterKind::Codex,
            &after,
            &output,
            Some(&initial),
            source.evidence(&after, 2, now),
        );
        let input = projection.input.unwrap();
        assert_eq!(input.text, text);
        assert_eq!(input.caret, Some(text.encode_utf16().count() as u32));
        assert_eq!(input.adapter(), AdapterKind::Codex);
    }
}

#[test]
fn codex_large_paste_proof_expires_and_cannot_cross_geometry_or_protected_input() {
    let now = Instant::now();
    let (before, document) = textarea(100, &["hint"], (0, 0), true, 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &document, None, None)
        .input
        .unwrap();
    let bytes = crate::terminal_input::paste_bytes(&"x".repeat(80), true);
    for case in [
        "expiry",
        "geometry",
        "protected",
        "other adapter",
        "utf16 limit",
    ] {
        let mut source = SourceInput::default();
        let mut baseline = initial.clone();
        if case == "protected" {
            baseline.protected = true;
        }
        if case == "other adapter" {
            baseline.adapter = AdapterKind::Universal;
        }
        let oversized = crate::terminal_input::paste_bytes(&"🙂".repeat(16_385), true);
        source.accepted(
            &before,
            1,
            if case == "utf16 limit" {
                &oversized
            } else {
                &bytes
            },
            Some(&baseline),
            now,
        );
        let mut after = before.clone();
        if case == "geometry" {
            after.cols += 1;
        }
        let observed = if case == "expiry" {
            now + Duration::from_secs(16)
        } else {
            now
        };
        assert!(source.evidence(&after, 2, observed).is_none(), "{case}");
    }
}
