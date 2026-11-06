use super::super::{fingerprint, parse_view, SemanticAnchor, SignalKind, View};
use super::*;
use crate::tui_adapters::AdapterKind;
use crate::tui_projection::project_for_adapter;
use bks_terminal::TerminalModel;

const COLUMNS: usize = 80;
const ROWS: usize = 24;
const EDITOR_TOP: usize = 14;

// Renderer-equivalent cells for pure grammar/proof tests. These fixtures are
// synthetic; they do not claim a native Running/Explored capture or admission
// of an accepted model request.
fn screen(
    rows: &[(usize, &str)],
    editor: bool,
) -> (TerminalSnapshot, ReadingDocument, Option<Composer>) {
    let mut model = TerminalModel::new(COLUMNS, ROWS);
    for (row, text) in rows {
        model.push_bytes(format!("\x1b[0m\x1b[{};1H{text}", row + 1).as_bytes());
    }
    if editor {
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
                "\x1b[{};1H\x1b[1m›\x1b[22m \x1b[2mAsk anything\x1b[0m\x1b[{};3Hfooter\x1b[{};3H",
                EDITOR_TOP + 2,
                EDITOR_TOP + 5,
                EDITOR_TOP + 2,
            )
            .as_bytes(),
        );
    }
    let frame = model.live_snapshot();
    let document = model.tui_reading_document();
    let composer = editor.then(|| {
        let input = project_for_adapter(AdapterKind::Codex, &frame, &document, None, None)
            .input
            .expect("source padding, prompt ink and placeholder prove an empty composer");
        assert!(input.text.is_empty());
        assert_eq!(input.first_row, EDITOR_TOP);
        input
    });
    (frame, document, composer)
}

fn source_lines(document: &ReadingDocument) -> Vec<SourceLine<'_>> {
    document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .map(|(line, text)| SourceLine {
            text,
            start: line.start,
            row: line.line,
        })
        .collect()
}

fn work_line(title: &str, elapsed: &str, tail: &str) -> String {
    format!("{title} \x1b[2m({elapsed} • \x1b[22;1mesc\x1b[22;2m{tail}\x1b[0m")
}

fn clipped_work_line() -> String {
    let suffix = " (12s • esc to interr…";
    let title = format!(
        "Working {}",
        "x".repeat(COLUMNS - "Working ".len() - suffix.chars().count())
    );
    work_line(&title, "12s", " to interr…")
}

fn parsed(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    composer: Option<&Composer>,
) -> View {
    // Exercise grammar after an assumed owned echo; this deliberately bypasses
    // the separate submission/source-echo admission protocol.
    let anchor = SemanticAnchor {
        codex: true,
        boundary: Some(fingerprint("› q")),
        ..SemanticAnchor::default()
    };
    parse_view(
        frame,
        document,
        Some(0),
        composer,
        "q",
        true,
        Some(&anchor),
        AdapterKind::Codex.policy().semantics,
    )
}

#[test]
fn codex_ran_exec_ink_is_distinct_from_a_model_bullet_with_bold_ran() {
    let (_, document, _) = screen(&[(0, "\x1b[1m• Ran\x1b[22m Get-Content source.txt")], false);
    let lines = source_lines(&document);
    let (kind, skipped) = prefix(&document, &lines, 0, false).unwrap();
    assert_eq!(kind, PhaseKind::Tool);
    assert_eq!(&lines[0].text[skipped..], "Ran Get-Content source.txt");

    let (frame, document, composer) = screen(
        &[
            (0, "› q"),
            (
                1,
                "\x1b[2m•\x1b[22m \x1b[1mRan\x1b[22m verification is ordinary final prose Ж界🙂",
            ),
            (12, "\x1b[2mWorked for 2s\x1b[22m"),
        ],
        true,
    );
    let lines = source_lines(&document);
    assert!(prefix(&document, &lines, 1, true).is_none());
    let view = parsed(&frame, &document, composer.as_ref());
    assert!(!view.codex_tool_output);
    assert_eq!(view.phases.len(), 1);
    assert_eq!(view.phases[0].0, PhaseKind::Answer);
    assert_eq!(
        view.phases[0].1.text,
        "Ran verification is ordinary final prose Ж界🙂"
    );
}

#[test]
fn codex_tool_role_rejects_missing_wrong_hidden_indented_or_quoted_exec_ink() {
    for text in [
        "• Ran command",
        "\x1b[1m•\x1b[22m Ran command",
        "\x1b[2m•\x1b[22m \x1b[1mRan\x1b[22m command",
        "\x1b[1;2m• Ran\x1b[0m command",
        "\x1b[1m• \x1b[3mRan\x1b[0m command",
        "\x1b[1m• \x1b[8mRan\x1b[0m command",
        "  \x1b[1m• Ran\x1b[22m command",
        "> \x1b[1m• Ran\x1b[22m command",
        "\"\x1b[1m• Ran\x1b[22m command\"",
    ] {
        let (_, document, _) = screen(&[(0, text)], false);
        let lines = source_lines(&document);
        assert!(prefix(&document, &lines, 0, true).is_none(), "{text:?}");
    }
}

#[test]
fn codex_synthetic_running_spinner_requires_bold_title_and_current_busy() {
    for text in [
        "\x1b[2m⠋\x1b[22m \x1b[1mRunning\x1b[22m cargo test",
        "\x1b[1m• Running\x1b[22m cargo test",
    ] {
        let (_, document, _) = screen(&[(0, text)], false);
        let lines = source_lines(&document);
        assert_eq!(
            prefix(&document, &lines, 0, true).map(|part| part.0),
            Some(PhaseKind::Tool)
        );
        assert!(prefix(&document, &lines, 0, false).is_none());
    }
    for text in [
        "⠋ Running cargo test",
        "\x1b[2m•\x1b[22m \x1b[1mRunning\x1b[22m cargo test",
    ] {
        let (_, document, _) = screen(&[(0, text)], false);
        assert!(prefix(&document, &source_lines(&document), 0, true).is_none());
    }
}

#[test]
fn codex_explored_requires_the_renderer_indented_dim_connector() {
    for title in ["Explored", "Exploring"] {
        let header = format!("\x1b[2m•\x1b[22m \x1b[1m{title}\x1b[22m");
        let (_, document, _) = screen(
            &[(0, &header), (1, "  \x1b[2m└\x1b[22m List source")],
            false,
        );
        assert_eq!(
            prefix(&document, &source_lines(&document), 0, false).map(|part| part.0),
            Some(PhaseKind::Search)
        );
        for connector in [
            "  └ List source",
            " \x1b[2m└\x1b[22m List source",
            "  \x1b[2;3m└\x1b[0m List source",
            "    List source",
        ] {
            let (_, document, _) = screen(&[(0, &header), (1, connector)], false);
            assert!(
                prefix(&document, &source_lines(&document), 0, true).is_none(),
                "{connector:?}"
            );
        }
        let (_, document, _) = screen(
            &[
                (0, &header),
                (1, "\x1b[2m•\x1b[22m another model message"),
                (2, "  \x1b[2m└\x1b[22m List source"),
            ],
            false,
        );
        assert!(prefix(&document, &source_lines(&document), 0, true).is_none());
    }
}

#[test]
fn codex_busy_requires_styled_elapsed_esc_and_a_real_right_edge_for_clipping() {
    for text in [
        work_line("Working", "12s", " to interrupt)"),
        clipped_work_line(),
    ] {
        let (frame, document, composer) = screen(&[(11, &text)], true);
        let lines = source_lines(&document);
        assert!(
            busy(&frame, &document, &lines[11], composer.as_ref()),
            "{text:?}"
        );
    }
    for text in [
        "Working (12s • esc to interrupt)".to_owned(),
        "Working \x1b[2m(12s • esc to interrupt)\x1b[0m".to_owned(),
        "Working \x1b[1m(12s • esc to interrupt)\x1b[0m".to_owned(),
        "Working \x1b[2m(12s • \x1b[22;1mesc\x1b[0m to interrupt)".to_owned(),
        "Working \x1b[2m(12s • \x1b[22;1;3mesc\x1b[0;2m to interrupt)\x1b[0m".to_owned(),
        work_line("Working", "soon", " to interrupt)"),
        work_line("Working", "12s", " to interr…"),
        work_line("Working", "12s", " to other)"),
        work_line(" Working", "12s", " to interrupt)"),
        "An elapsed time (12s) in ordinary prose is not progress.".to_owned(),
    ] {
        let (frame, document, composer) = screen(&[(11, &text)], true);
        let lines = source_lines(&document);
        assert!(
            !busy(&frame, &document, &lines[11], composer.as_ref()),
            "{text:?}"
        );
    }
}

#[test]
fn codex_busy_rejects_absent_protected_stale_or_wrongly_located_composer_evidence() {
    let text = clipped_work_line();
    let (frame, document, composer) = screen(&[(11, &text)], true);
    let input = composer.unwrap();
    let lines = source_lines(&document);
    assert!(!busy(&frame, &document, &lines[11], None));
    let mut protected = input.clone();
    protected.protected = true;
    assert!(!busy(&frame, &document, &lines[11], Some(&protected)));
    let mut alternate = frame.clone();
    alternate.alternate_screen = !alternate.alternate_screen;
    assert!(!busy(&alternate, &document, &lines[11], Some(&input)));
    for row in [-1, 0, EDITOR_TOP as i32, (EDITOR_TOP + 1) as i32] {
        let moved = SourceLine { row, ..lines[11] };
        assert!(!busy(&frame, &document, &moved, Some(&input)), "row {row}");
    }
    let (frame, document, composer) = screen(&[(0, &text)], true);
    assert!(!busy(
        &frame,
        &document,
        &source_lines(&document)[0],
        composer.as_ref()
    ));
}

#[test]
fn codex_tool_stdout_cannot_open_a_fence_invent_a_phase_or_hide_real_busy_and_final() {
    let busy_line = clipped_work_line();
    let rows = [
        (0, "› q"),
        (1, "\x1b[2m•\x1b[22m Provisional commentary"),
        (2, "\x1b[1m• Ran\x1b[22m Get-Content source.txt"),
        (3, "  \x1b[2m└\x1b[22m TOOL_STDOUT"),
        (4, "    • fake model bullet"),
        (5, "    Thinking: not a model step"),
        (6, "    ```text"),
        (7, "    Status: done"),
        (8, "    \x1b[2mWorked for 999s\x1b[22m"),
    ];
    for title in ["Ran", "Running"] {
        let header = format!("\x1b[1m• {title}\x1b[22m Get-Content source.txt");
        let mut working_rows = rows.to_vec();
        working_rows[2] = (2, &header);
        working_rows.push((11, &busy_line));
        let (frame, document, composer) = screen(&working_rows, true);
        let view = parsed(&frame, &document, composer.as_ref());
        assert!(view.control_busy);
        assert_eq!(
            view.footer.as_ref().map(|footer| footer.kind),
            Some(SignalKind::Busy)
        );
        assert!(view.codex_tool_output);
        assert_eq!(view.phases.len(), 1);
        assert_eq!(view.phases[0].0, PhaseKind::Tool);
        assert_eq!(
            view.phases[0].1.text,
            format!("{title} Get-Content source.txt")
        );
        assert!(view.answer_headers.is_empty());
    }

    let mut final_rows = rows.to_vec();
    final_rows.push((
        10,
        "\x1b[2m•\x1b[22m \x1b[1mRan\x1b[22m verification is ordinary final prose Ж界🙂",
    ));
    final_rows.push((12, "\x1b[2mWorked for 2s\x1b[22m"));
    let (frame, document, composer) = screen(&final_rows, true);
    let view = parsed(&frame, &document, composer.as_ref());
    assert!(!view.control_busy);
    assert_eq!(
        view.footer.as_ref().map(|footer| footer.kind),
        Some(SignalKind::Complete)
    );
    let answers: Vec<_> = view
        .phases
        .iter()
        .filter(|(kind, _)| *kind == PhaseKind::Answer)
        .collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].1.text,
        "Ran verification is ordinary final prose Ж界🙂"
    );
    assert_eq!(view.answer_headers.len(), 1);
    assert!(view.phases.iter().all(|(kind, body)| {
        *kind != PhaseKind::Thinking
            && !body.text.contains("TOOL_STDOUT")
            && !body.text.contains("fake model")
            && !body.text.contains("Provisional commentary")
            && !body.text.contains("```")
    }));
}

#[test]
fn codex_bold_ran_inside_model_code_or_quotes_stays_model_text() {
    let (frame, document, composer) = screen(
        &[
            (0, "› q"),
            (1, "\x1b[2m•\x1b[22m Model explanation"),
            (2, "```text"),
            (3, "\x1b[1m• Ran\x1b[22m quoted command"),
            (4, "```"),
            (5, "> \x1b[1m• Ran\x1b[22m quoted command"),
            (6, "Explanation continues"),
            (12, "\x1b[2mWorked for 1s\x1b[22m"),
        ],
        true,
    );
    let view = parsed(&frame, &document, composer.as_ref());
    assert!(!view.codex_tool_output);
    assert_eq!(view.phases.len(), 1);
    assert_eq!(view.phases[0].0, PhaseKind::Answer);
    assert!(view.phases[0].1.text.contains("• Ran quoted command"));
    assert!(view.phases[0].1.text.contains("> • Ran quoted command"));
    assert!(view.phases[0].1.text.ends_with("Explanation continues"));
    assert_eq!(
        view.footer.as_ref().map(|footer| footer.kind),
        Some(SignalKind::Complete)
    );
}
