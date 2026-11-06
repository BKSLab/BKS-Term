//! OpenCode's native editor, selected semantics and explicit launch defaults.
//! No screen text authorizes a key or a command.

use crate::profile::Profile;
use crate::terminal_input::SpecialKey;
use crate::tui_adapters::{ComposerStrategy, Policy, Semantics};

pub(crate) const POLICY: Policy = Policy {
    composer: ComposerStrategy::Gutter,
    semantics: Semantics {
        panel_responses: true,
        recorded_duration: false,
        bullet_roles: false,
        chrome: None,
        bounded_busy: None,
    },
};

/// OpenCode 2's editor accepts CSI-u modifier identities. Legacy CR loses
/// Shift/Ctrl and submits instead of inserting a newline. Preserve the actual
/// chord so the application's own bindings still decide what it means.
pub fn modified_enter(key: SpecialKey, shift: bool, alt: bool, control: bool) -> Option<Vec<u8>> {
    if key != SpecialKey::Enter || !(shift || alt || control) {
        return None;
    }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    Some(format!("\x1b[13;{modifier}u").into_bytes())
}

pub fn newline_key(bytes: &[u8]) -> bool {
    matches!(bytes, b"\x1b[13;2u" | b"\x1b[13;3u" | b"\x1b[13;5u" | b"\n")
}

/// Match every visible input glyph against explicitly accepted text. OpenTUI
/// wraps words itself, sometimes replacing one space with a physical row break.
/// Only that layout difference is allowed; changed/missing glyphs reject proof.
pub fn editor_caret(rendered: &str, expected: &str, caret: u32) -> Option<u32> {
    let rendered: Vec<_> = rendered.encode_utf16().collect();
    let expected: Vec<_> = expected.encode_utf16().collect();
    let (mut source, mut canonical) = (0, 0);
    let mut mapped = None;
    loop {
        if source == caret as usize {
            mapped = Some(canonical as u32);
        }
        if source == rendered.len() {
            break;
        }
        if rendered[source..].starts_with(&[13, 10])
            && !expected[canonical..].starts_with(&[13, 10])
        {
            source += 2;
            if expected.get(canonical) == Some(&32) {
                canonical += 1;
            }
        } else if rendered.get(source) == expected.get(canonical) {
            source += 1;
            canonical += 1;
        } else {
            return None;
        }
    }
    (canonical == expected.len()).then_some(mapped).flatten()
}

/// Infer the source editor's deletion from the visible result, rather than
/// assuming one UTF-16 unit, scalar or grapheme per key. Physical wrapping can
/// change length by at most two units per row; only a unique exact match wins.
pub fn deleted_input(
    before: &str,
    before_caret: u32,
    rendered: &str,
    caret: u32,
    backspace: bool,
    rows: usize,
) -> Option<(String, u32)> {
    let old: Vec<_> = before.encode_utf16().collect();
    let origin = before_caret as usize;
    if origin > old.len() {
        return None;
    }
    let maximum = if backspace {
        origin
    } else {
        old.len() - origin
    };
    let difference = old.len() as isize - rendered.encode_utf16().count() as isize;
    let allowance = 2 * rows.clamp(1, 32) as isize;
    let first = (difference - allowance).max(1) as usize;
    let last = ((difference + allowance).max(0) as usize).min(maximum);
    let mut found = None;
    for removed in first..=last {
        let start = if backspace { origin - removed } else { origin };
        let finish = start + removed;
        let units: Vec<_> = old[..start].iter().chain(&old[finish..]).copied().collect();
        let Ok(text) = String::from_utf16(&units) else {
            continue;
        };
        if editor_caret(rendered, &text, caret) == Some(start as u32) {
            if found.is_some() {
                return None;
            }
            found = Some((text, start as u32));
        }
    }
    found
}

/// Only a discovered, help-confirmed --standalone installation gets these
/// built-in defaults. Saved/custom profiles are never rewritten. Inline user
/// settings take priority; a global cli.json is merged by OpenCode itself.
pub fn configure_profile(profile: &mut Profile, inherited_inline_settings: bool) {
    profile.args.push("--standalone".into());
    if !inherited_inline_settings {
        profile.launch.environment.insert(
            "OPENCODE_CLI_CONFIG_CONTENT".into(),
            r#"{"prompt":{"paste":"full"}}"#.into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_adapters::AdapterKind;

    #[test]
    fn word_wrapped_input_matches_only_the_full_accepted_glyph_sequence() {
        let expected = "Ж🙂 word wrap\r\n\r\nsecond line";
        let rendered = "Ж🙂 word\r\nwrap\r\n\r\nsecond line";
        assert_eq!(
            editor_caret(rendered, expected, rendered.encode_utf16().count() as u32),
            Some(expected.encode_utf16().count() as u32)
        );
        assert_eq!(
            editor_caret(
                rendered,
                expected,
                "Ж🙂 word\r\n".encode_utf16().count() as u32
            ),
            Some("Ж🙂 word ".encode_utf16().count() as u32)
        );
        for changed in [
            "Ж🙂 work\r\nwrap\r\n\r\nsecond line",
            "Ж🙂 word\r\nwrap\r\n\r\nsecond",
            "Ж🙂 word wrap changed\r\n\r\nsecond line",
        ] {
            assert!(editor_caret(changed, expected, 0).is_none());
        }
        assert!(newline_key(b"\x1b[13;2u") && newline_key(b"\n"));
        assert!(!newline_key(b"\r") && !newline_key(b"\x1b[13;6u"));
    }

    #[test]
    fn wrapped_deletion_requires_a_unique_visible_result_at_the_reported_caret() {
        let before = "Ж🙂 word wrap\r\nlast.";
        let rendered = "Ж🙂 word\r\nwrap\r\nlast";
        let caret = rendered.encode_utf16().count() as u32;
        let end = before.encode_utf16().count() as u32;
        assert_eq!(
            deleted_input(before, end, rendered, caret, true, 3),
            Some(("Ж🙂 word wrap\r\nlast".into(), end - 1))
        );
        assert_eq!(
            deleted_input(before, end - 1, rendered, caret, false, 3),
            Some(("Ж🙂 word wrap\r\nlast".into(), end - 1))
        );
        assert!(deleted_input(before, end, "Ж🙂 changed\r\nlast", caret, true, 3).is_none());
        assert!(deleted_input(before, end, before, end, true, 3).is_none());
    }

    #[test]
    fn larger_paste_evidence_requires_opencode_source_and_keeps_existing_expiry_and_limits() {
        let capture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/tui-opencode-profile28/editor.json"
        ))
        .unwrap();
        let events = capture["events"].as_array().unwrap();
        let first_input = events
            .iter()
            .find(|event| event["kind"] == "input")
            .unwrap()["at_ms"]
            .as_u64()
            .unwrap();
        let end = events
            .iter()
            .rev()
            .find(|event| {
                event["kind"] == "poll" && event["at_ms"].as_u64().unwrap() <= first_input
            })
            .unwrap()["end_byte"]
            .as_u64()
            .unwrap() as usize;
        let mut terminal = bks_terminal::TerminalModel::new(110, 36);
        terminal.push_bytes(
            &include_bytes!("../tests/fixtures/tui-opencode-profile28/editor.vt")[..end],
        );
        let frame = terminal.live_snapshot();
        let document = terminal.tui_reading_document();
        let input = crate::tui_projection::project_for_adapter(
            AdapterKind::OpenCode,
            &frame,
            &document,
            None,
            None,
        )
        .input
        .unwrap();
        let now = std::time::Instant::now();
        let revision = terminal.reading_revision();
        let text = "x".repeat(128);
        let paste = format!("\x1b[200~{text}\x1b[201~");
        let mut source = crate::source_input::SourceInput::default();
        source.accepted(&frame, revision, paste.as_bytes(), Some(&input), now);
        assert_eq!(
            source
                .confirmation(&frame, revision + 1, now)
                .unwrap()
                .0
                .text,
            text
        );
        assert!(source
            .confirmation(
                &frame,
                revision + 1,
                now + std::time::Duration::from_millis(751)
            )
            .is_none());
        for (data, composer) in [
            (text.as_bytes().to_vec(), Some(input.clone())),
            (
                format!("\x1b[200~{}\x1b[201~", "x".repeat(32_769)).into_bytes(),
                Some(input.clone()),
            ),
            (paste.as_bytes().to_vec(), None),
            (
                paste.as_bytes().to_vec(),
                crate::tui_projection::project(&frame, &document, None).input,
            ),
        ] {
            source.accepted(&frame, revision, &data, composer.as_ref(), now);
            assert!(source.confirmation(&frame, revision + 1, now).is_none());
        }
    }

    #[test]
    fn modified_enters_keep_their_identity_and_never_become_submission_cr() {
        for (shift, alt, control, modifier) in [
            (true, false, false, 2),
            (false, true, false, 3),
            (true, true, false, 4),
            (false, false, true, 5),
            (true, false, true, 6),
            (false, true, true, 7),
            (true, true, true, 8),
        ] {
            assert_eq!(
                AdapterKind::OpenCode.encode_key(SpecialKey::Enter, shift, alt, control, false),
                format!("\x1b[13;{modifier}u").as_bytes()
            );
        }
        assert_eq!(
            AdapterKind::OpenCode.encode_key(SpecialKey::Enter, false, false, false, false),
            b"\r"
        );
    }

    #[test]
    fn other_adapters_and_non_enter_keys_keep_their_existing_encodings() {
        for adapter in AdapterKind::ALL {
            for key in [SpecialKey::Enter, SpecialKey::Up, SpecialKey::Tab] {
                for (shift, alt, control) in [(true, false, false), (false, false, true)] {
                    if matches!(adapter, AdapterKind::OpenCode | AdapterKind::Codex)
                        && key == SpecialKey::Enter
                    {
                        continue;
                    }
                    assert_eq!(
                        adapter.encode_key(key, shift, alt, control, true),
                        crate::terminal_input::encode_key(key, shift, alt, control, true)
                    );
                }
            }
        }
    }

    #[test]
    fn profile_display_defaults_preserve_explicit_inline_settings_and_other_fields() {
        let base = Profile {
            id: "cli-opencode".into(),
            name: "OpenCode".into(),
            kind: crate::profile::ProfileKind::Local,
            executable: r"C:\CLI Tools\opencode.exe".into(),
            args: Vec::new(),
            working_directory: None,
            launch: Default::default(),
        };
        let mut full = base.clone();
        configure_profile(&mut full, false);
        let inline: serde_json::Value =
            serde_json::from_str(&full.launch.environment["OPENCODE_CLI_CONFIG_CONTENT"]).unwrap();
        assert_eq!(inline, serde_json::json!({"prompt":{"paste":"full"}}));
        let mut inherited = base.clone();
        configure_profile(&mut inherited, true);
        assert!(inherited.launch.environment.is_empty());
        for profile in [&full, &inherited] {
            assert_eq!(profile.args, ["--standalone"]);
            assert_eq!(profile.executable, base.executable);
            assert_eq!(profile.working_directory, base.working_directory);
        }
        assert!(base.args.is_empty() && base.launch.environment.is_empty());
    }
}
