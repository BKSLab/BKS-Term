use super::*;
use crate::source_input::SourceInput;
use crate::tui_projection::project_for_adapter;
use crate::tui_suggestions::Suggestion;
use bks_terminal::{Color, NamedColor, TerminalModel};

const COLUMNS: usize = 80;
const ROWS: usize = 24;
const EDITOR_TOP: usize = 12;

struct Scene {
    frame: TerminalSnapshot,
    document: ReadingDocument,
    composer: Composer,
}

// Codex's padded editor has a blank top/bottom row, a bold two-column
// prompt, a uniform background and a separate footer. Input still has to be
// proved by project_for_adapter; a drawn slash command alone is not intent.
fn draw(
    output: &[&str],
    field: &str,
    caret: usize,
    placeholder: bool,
    revision: u64,
) -> (TerminalSnapshot, ReadingDocument) {
    assert!(output.len() < EDITOR_TOP);
    let mut model = TerminalModel::new(COLUMNS, ROWS);
    for (row, text) in output.iter().enumerate() {
        model.push_bytes(format!("\x1b[0m\x1b[{};1H{text}", row + 1).as_bytes());
    }
    for row in EDITOR_TOP..=EDITOR_TOP + 2 {
        model.push_bytes(
            format!(
                "\x1b[0m\x1b[{};1H\x1b[48;2;40;40;40m{}",
                row + 1,
                " ".repeat(COLUMNS)
            )
            .as_bytes(),
        );
    }
    model.push_bytes(
        format!(
            "\x1b[{};1H\x1b[1m›\x1b[22m {}{field}\x1b[0m\x1b[{};3Hfooter: source only\x1b[{};{}H",
            EDITOR_TOP + 2,
            if placeholder { "\x1b[2m" } else { "" },
            EDITOR_TOP + 5,
            EDITOR_TOP + 2,
            caret + 3,
        )
        .as_bytes(),
    );
    let mut document = model.tui_reading_document();
    document.revision = revision;
    (model.live_snapshot(), document)
}

fn scene(output: &[&str], field: &str, revision: u64, now: Instant) -> Scene {
    let (before, before_document) = draw(output, "Ask anything", 0, true, revision - 1);
    let initial = project_for_adapter(AdapterKind::Codex, &before, &before_document, None, None)
        .input
        .expect("source placeholder must prove an empty Codex composer");
    assert!(initial.text.is_empty());
    if field.is_empty() {
        let mut document = before_document;
        document.revision = revision;
        return Scene {
            frame: before,
            document,
            composer: initial,
        };
    }
    let mut source = SourceInput::default();
    source.accepted(
        &before,
        before_document.revision,
        field.as_bytes(),
        Some(&initial),
        now,
    );
    let (frame, document) = draw(output, field, field.len(), false, revision);
    let composer = project_for_adapter(
        AdapterKind::Codex,
        &frame,
        &document,
        Some(&initial),
        source.evidence(&frame, revision, now),
    )
    .input
    .expect("exact accepted source field must project");
    assert_eq!(composer.text, field);
    Scene {
        frame,
        document,
        composer,
    }
}

fn submit(capture: &mut Capture, source: &Scene, bytes: &[u8], menu: Option<&Menu>, now: Instant) {
    capture.accepted(
        bytes,
        &source.frame,
        &source.document,
        Some(&source.composer),
        menu,
        now,
    );
}

fn observe(
    capture: &mut Capture,
    source: &Scene,
    now: Instant,
) -> Option<(Option<u64>, String, RichText)> {
    capture.observe(
        &source.frame,
        &source.document,
        Some(&source.composer),
        false,
        now,
    )
}

fn status_menu() -> Menu {
    Menu {
        items: vec![Suggestion {
            label: "/status  Show session configuration".into(),
            row: EDITOR_TOP + 4,
        }],
        selected: 0,
        first_row: EDITOR_TOP + 4,
        last_row: EDITOR_TOP + 4,
    }
}

#[test]
fn command_output_waits_for_stable_source_and_keeps_unicode_styles_and_blank_lines() {
    let now = Instant::now();
    let before = scene(&["earlier answer"], "/status", 2, now);
    let after = scene(
        &[
            "earlier answer",
            "/status",
            "\x1b[1;32mЖ界🙂 e\u{301} []\x1b[0m",
            "",
            "  limit: 42%",
        ],
        "",
        3,
        now,
    );
    let original = after.document.clone();
    let mut capture = Capture::default();
    submit(&mut capture, &before, b"\r", None, now);
    assert!(observe(&mut capture, &after, now).is_none());
    assert!(observe(&mut capture, &after, now + Duration::from_millis(149)).is_none());
    let (id, command, body) =
        observe(&mut capture, &after, now + Duration::from_millis(150)).unwrap();
    assert_eq!(id, None);
    assert_eq!(command, "/status");
    assert_eq!(body.text, "Ж界🙂 e\u{301} []\r\n\r\n  limit: 42%");
    let unicode_end = "Ж界🙂 e\u{301} []".encode_utf16().count() as u32;
    assert!(body.styles.iter().any(|span| {
        span.start == 0
            && span.end == unicode_end
            && span.style.flags.contains(Flags::BOLD)
            && span.style.foreground == Color::Named(NamedColor::Green)
    }));
    assert!(body.glyphs.iter().any(|glyph| glyph.width == 2));
    assert!(body.glyphs.iter().any(|glyph| glyph.end - glyph.start == 2));
    assert!(body.styles.iter().all(|span| span.end <= body.units()));
    assert!(!body.text.contains("footer"));
    assert!(!body.text.contains("Ask anything"));
    assert_eq!(after.document, original);
}

#[test]
fn command_output_requires_explicit_enter_and_an_exact_unfenced_echo() {
    let now = Instant::now();
    let before = scene(&["earlier answer"], "/status", 2, now);
    let valid = scene(&["/status", "Actual source result"], "", 3, now);
    for key in [None, Some(b"x".as_slice()), Some(b"\t".as_slice())] {
        let mut capture = Capture::default();
        if let Some(key) = key {
            submit(&mut capture, &before, key, None, now);
        }
        assert!(observe(&mut capture, &valid, now).is_none());
        assert!(observe(&mut capture, &valid, now + Duration::from_secs(1)).is_none());
    }
    for lines in [
        vec!["Actual source result"],
        vec!["quoted /status", "Actual source result"],
        vec!["/status-extra", "Actual source result"],
        vec!["```text", "/status", "```", "Actual source result"],
        vec!["~~~", "/status", "~~~", "Actual source result"],
        vec!["/status"],
    ] {
        let after = scene(&lines, "", 3, now);
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\r", None, now);
        assert!(observe(&mut capture, &after, now).is_none(), "{lines:?}");
        assert!(
            observe(&mut capture, &after, now + Duration::from_secs(1)).is_none(),
            "{lines:?}"
        );
    }
}

#[test]
fn command_output_does_not_copy_a_following_model_request_or_answer() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    for marker in ["› following request", "• following answer"] {
        let after = scene(&["/status", "Status body", marker], "", 3, now);
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\r", None, now);
        assert!(observe(&mut capture, &after, now).is_none());
        assert!(observe(&mut capture, &after, now + Duration::from_secs(1)).is_none());
    }
}

#[test]
fn command_output_requires_a_fresh_cleared_editor_and_closed_menu() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    let after = scene(&["/status", "Actual source result"], "", 3, now);
    let nonempty = scene(
        &["/status", "Actual source result"],
        "still editing",
        3,
        now,
    );
    let mut stale = after.document.clone();
    stale.revision = before.document.revision;
    for case in [
        "unchanged revision",
        "nonempty editor",
        "menu",
        "no composer",
    ] {
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\r", None, now);
        for elapsed in [0, 1_000] {
            let (frame, document, composer, menu) = match case {
                "unchanged revision" => (&after.frame, &stale, Some(&after.composer), false),
                "nonempty editor" => (
                    &nonempty.frame,
                    &nonempty.document,
                    Some(&nonempty.composer),
                    false,
                ),
                "menu" => (&after.frame, &after.document, Some(&after.composer), true),
                "no composer" => (&after.frame, &after.document, None, false),
                _ => unreachable!(),
            };
            assert!(
                capture
                    .observe(
                        frame,
                        document,
                        composer,
                        menu,
                        now + Duration::from_millis(elapsed)
                    )
                    .is_none(),
                "{case}"
            );
        }
    }
}

#[test]
fn command_output_never_accepts_a_protected_source_field() {
    let now = Instant::now();
    let (frame, document) = draw(&[], "\x1b[8m/status\x1b[28m", 7, false, 2);
    let protected = project_for_adapter(AdapterKind::Codex, &frame, &document, None, None)
        .input
        .expect("protected source composer remains readable with redaction");
    assert!(protected.protected);
    assert!(!document.text.contains("/status"));
    let after = scene(&["/status", "Actual source result"], "", 3, now);
    let mut capture = Capture::default();
    capture.accepted(b"\r", &frame, &document, Some(&protected), None, now);
    assert!(observe(&mut capture, &after, now).is_none());
    assert!(observe(&mut capture, &after, now + Duration::from_secs(1)).is_none());

    let before = scene(&[], "/status", 2, now);
    submit(&mut capture, &before, b"\r", None, now);
    assert!(capture
        .observe(&frame, &document, Some(&protected), false, now)
        .is_none());
    assert!(capture
        .observe(
            &frame,
            &document,
            Some(&protected),
            false,
            now + Duration::from_secs(1),
        )
        .is_none());
}

#[test]
fn command_output_cancels_on_expiry_buffer_switch_or_resize_and_cannot_revive() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    let after = scene(&["/status", "Actual source result"], "", 3, now);
    for case in ["expiry", "buffer", "resize"] {
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\r", None, now);
        assert!(observe(&mut capture, &after, now).is_none());
        let mut changed = after.frame.clone();
        let mut document = after.document.clone();
        let at = match case {
            "expiry" => now + Duration::from_secs(16),
            "buffer" => {
                changed.alternate_screen = !changed.alternate_screen;
                document.alternate_screen = changed.alternate_screen;
                now + Duration::from_millis(100)
            }
            "resize" => {
                changed.rows -= 1;
                changed.cells.truncate(changed.cols * changed.rows);
                now + Duration::from_millis(100)
            }
            _ => unreachable!(),
        };
        assert!(
            capture
                .observe(&changed, &document, Some(&after.composer), false, at)
                .is_none(),
            "{case}"
        );
        assert!(
            observe(&mut capture, &after, at + Duration::from_secs(1)).is_none(),
            "{case}"
        );
    }
}

#[test]
fn command_output_stability_restarts_after_source_changes_or_an_open_menu() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    let first = scene(&["/status", "Partial status"], "", 3, now);
    let final_source = scene(&["/status", "Full status"], "", 4, now);
    let mut capture = Capture::default();
    submit(&mut capture, &before, b"\r", None, now);
    assert!(observe(&mut capture, &first, now).is_none());
    assert!(observe(
        &mut capture,
        &final_source,
        now + Duration::from_millis(100)
    )
    .is_none());
    assert!(observe(
        &mut capture,
        &final_source,
        now + Duration::from_millis(200)
    )
    .is_none());
    assert!(capture
        .observe(
            &final_source.frame,
            &final_source.document,
            Some(&final_source.composer),
            true,
            now + Duration::from_millis(250),
        )
        .is_none());
    assert!(observe(
        &mut capture,
        &final_source,
        now + Duration::from_millis(400)
    )
    .is_none());
    let (_, _, body) = observe(
        &mut capture,
        &final_source,
        now + Duration::from_millis(550),
    )
    .unwrap();
    assert_eq!(body.text, "Full status");
}

#[test]
fn command_output_updates_the_committed_id_and_next_key_seals_it() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    let first = scene(&["/status", "Status first paint"], "", 3, now);
    let later = scene(&["/status", "Status final paint"], "", 4, now);
    let mut capture = Capture::default();
    submit(&mut capture, &before, b"\r", None, now);
    assert!(observe(&mut capture, &first, now).is_none());
    assert_eq!(
        observe(&mut capture, &first, now + Duration::from_millis(150))
            .unwrap()
            .0,
        None
    );
    capture.committed(42);
    assert!(observe(&mut capture, &later, now + Duration::from_millis(200)).is_none());
    let (id, command, body) =
        observe(&mut capture, &later, now + Duration::from_millis(350)).unwrap();
    assert_eq!(id, Some(42));
    assert_eq!(command, "/status");
    assert_eq!(body.text, "Status final paint");
    submit(
        &mut capture,
        &later,
        b"x",
        None,
        now + Duration::from_millis(400),
    );
    assert!(observe(&mut capture, &later, now + Duration::from_secs(1)).is_none());
}

#[test]
fn command_output_next_explicit_key_cancels_an_unconfirmed_result() {
    let now = Instant::now();
    let before = scene(&[], "/status", 2, now);
    let after = scene(&["/status", "Actual source result"], "", 3, now);
    for key in [b"x".as_slice(), b"\x1b[A", b"\r", b"\t"] {
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\r", None, now);
        assert!(observe(&mut capture, &after, now).is_none());
        submit(
            &mut capture,
            &after,
            key,
            None,
            now + Duration::from_millis(50),
        );
        assert!(observe(&mut capture, &after, now + Duration::from_secs(1)).is_none());
    }
}

#[test]
fn command_output_tab_hint_needs_the_exact_completed_source_field_and_caret() {
    let now = Instant::now();
    let before = scene(&[], "/sta", 2, now);
    let menu = status_menu();
    let after = scene(&["/status", "Actual source result"], "", 4, now);
    for trailing_space in [false, true] {
        let field = if trailing_space {
            "/status "
        } else {
            "/status"
        };
        let (completed, document) = draw(&[], field, field.len(), false, 3);
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\t", Some(&menu), now);
        assert!(observe(&mut capture, &after, now + Duration::from_millis(50)).is_none());
        capture.accepted(
            b"\r",
            &completed,
            &document,
            None,
            None,
            now + Duration::from_millis(100),
        );
        assert!(observe(&mut capture, &after, now + Duration::from_millis(200)).is_none());
        let (_, command, body) =
            observe(&mut capture, &after, now + Duration::from_millis(350)).unwrap();
        assert_eq!(command, "/status");
        assert_eq!(body.text, "Actual source result");
    }
}

#[test]
fn command_output_tab_hint_rejects_incomplete_hidden_unprompted_and_wrong_caret_fields() {
    let now = Instant::now();
    let before = scene(&[], "/sta", 2, now);
    let menu = status_menu();
    let after = scene(&["/status", "Actual source result"], "", 4, now);
    for case in [
        "prefix",
        "other command",
        "wrong caret",
        "no cursor",
        "hidden",
        "prompt",
        "menu",
    ] {
        let field = match case {
            "prefix" => "/sta",
            "other command" => "/status-extra",
            _ => "/status ",
        };
        let (mut completed, document) = draw(&[], field, field.len(), false, 3);
        match case {
            "wrong caret" => completed.cursor.as_mut().unwrap().column = 3,
            "no cursor" => completed.cursor = None,
            "hidden" => completed.cells[(EDITOR_TOP + 1) * COLUMNS + 2]
                .flags
                .insert(Flags::HIDDEN),
            "prompt" => completed.cells[(EDITOR_TOP + 1) * COLUMNS]
                .flags
                .remove(Flags::BOLD),
            _ => {}
        }
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\t", Some(&menu), now);
        capture.accepted(
            b"\r",
            &completed,
            &document,
            None,
            (case == "menu").then_some(&menu),
            now + Duration::from_millis(100),
        );
        assert!(
            observe(&mut capture, &after, now + Duration::from_millis(200)).is_none(),
            "{case}"
        );
        assert!(
            observe(&mut capture, &after, now + Duration::from_secs(1)).is_none(),
            "{case}"
        );
    }
}

#[test]
fn command_output_tab_hint_expires_cancels_and_cannot_cross_buffer_or_size() {
    let now = Instant::now();
    let before = scene(&[], "/sta", 2, now);
    let menu = status_menu();
    let after = scene(&["/status", "Actual source result"], "", 4, now);
    for case in ["expiry", "next key", "buffer", "resize"] {
        let mut capture = Capture::default();
        submit(&mut capture, &before, b"\t", Some(&menu), now);
        let (mut completed, document) = draw(&[], "/status ", 8, false, 3);
        let at = match case {
            "expiry" => now + Duration::from_secs(6),
            "next key" => {
                submit(
                    &mut capture,
                    &before,
                    b"x",
                    None,
                    now + Duration::from_millis(50),
                );
                now + Duration::from_millis(100)
            }
            "buffer" => {
                completed.alternate_screen = !completed.alternate_screen;
                now + Duration::from_millis(100)
            }
            "resize" => {
                completed.rows -= 1;
                completed.cells.truncate(completed.cols * completed.rows);
                now + Duration::from_millis(100)
            }
            _ => unreachable!(),
        };
        capture.accepted(b"\r", &completed, &document, None, None, at);
        assert!(
            observe(&mut capture, &after, at + Duration::from_millis(100)).is_none(),
            "{case}"
        );
        assert!(
            observe(&mut capture, &after, at + Duration::from_secs(1)).is_none(),
            "{case}"
        );
    }
}
