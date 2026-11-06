use super::*;
use crate::tui_adapters::AdapterKind;
use bks_terminal::TerminalModel;

fn claude_archive(injection: &str) -> String {
    let mut model = TerminalModel::new(110, 36);
    model.push_bytes(include_bytes!(
        "../tests/fixtures/tui-cli-audit26/claude.vt"
    ));
    model.push_bytes(injection.as_bytes());
    model.push_bytes(b"\x1b[34;3H");
    let mut frame = model.live_snapshot();
    frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
    let raw = model.tui_reading_document();
    let projection =
        crate::tui_projection::project_for_adapter(AdapterKind::Claude, &frame, &raw, None, None);
    assert!(projection.input.is_some());
    let view = parse_view(
        &frame,
        &projection.output,
        Some(0),
        projection.input.as_ref(),
        "BKS_AUDIT_SECOND",
        true,
        None,
        AdapterKind::Claude.policy().semantics,
    );
    view.phases
        .into_iter()
        .filter(|(kind, _)| *kind == PhaseKind::Answer)
        .map(|(_, text)| text.text)
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[test]
fn claude_recorded_chrome_does_not_become_answer_or_completion_evidence() {
    let answer = claude_archive("");
    assert!(answer.contains("BKS_AUDIT_ANSWER_ONE"));
    assert!(answer.contains("BKS_AUDIT_ANSWER_TWO"));
    assert!(!answer.contains("/effort"), "{answer}");
    assert!(!answer.contains(" for 0s"), "{answer}");
    for adapter in [
        AdapterKind::Auto,
        AdapterKind::Shell,
        AdapterKind::Universal,
        AdapterKind::OpenCode,
        AdapterKind::Kimi,
        AdapterKind::Gemini,
    ] {
        assert!(adapter.policy().semantics.chrome.is_none());
    }
}

#[test]
fn claude_keeps_quoted_fenced_ordinary_ink_and_unknown_elapsed_text() {
    for (injection,expected) in [
        ("\x1b[31;1H\x1b[2K\x1b[38;2;153;153;153m> ✻ Cooked for 0s\x1b[0m","> ✻ Cooked for 0s"),
        ("\x1b[29;1H\x1b[2K```text\x1b[30;1H\x1b[2K\x1b[38;2;153;153;153m✻ Cooked for 0s\x1b[0m\x1b[31;1H\x1b[2K```","✻ Cooked for 0s"),
        ("\x1b[31;1H\x1b[2K\x1b[0m✻ Cooked for 0s","✻ Cooked for 0s"),
        ("\x1b[31;1H\x1b[2K\x1b[38;2;153;153;153m✻ Cooked for briefly\x1b[0m","✻ Cooked for briefly"),
        ("\x1b[31;1H\x1b[2K\x1b[38;2;153;153;153m✻ Cook\x1b[0med for 0s","✻ Cooked for 0s"),
    ] {
        let answer=claude_archive(injection);
        assert!(answer.contains(expected),"{injection:?}: {answer}");
    }
}

#[test]
fn claude_same_effort_words_outside_the_recorded_source_slot_are_preserved() {
    for injection in [
        "\x1b[26;1H\x1b[2K\x1b[38;2;153;153;153m● high · /effort\x1b[0m",
        "\x1b[26;1H\x1b[2K\x1b[26;93H\x1b[38;2;153;153;153m● high · /effort\x1b[0m",
        "\x1b[32;1H\x1b[2K\x1b[38;2;153;153;153m● high · /effort\x1b[0m",
    ] {
        let answer = claude_archive(injection);
        assert!(answer.contains("high · /effort"), "{answer}");
    }
}
