//! Codex's explicit keyboard identity and source-verified editor projection.
//! The CLI decides what a key does; screen text never supplies actions.

use crate::terminal_input::SpecialKey;
use crate::tui_adapters::{ComposerStrategy, Policy, Semantics};
use std::fmt::Write as _;
use std::io;

#[path = "codex_chrome.rs"]
mod chrome;

pub(crate) const POLICY: Policy = Policy {
    composer: ComposerStrategy::Padded,
    semantics: Semantics {
        panel_responses: false,
        recorded_duration: true,
        bullet_roles: true,
        chrome: Some(chrome::metadata),
        bounded_busy: None,
    },
};

/// Preserve Enter modifiers for Codex's own key bindings. Legacy CR cannot
/// distinguish a modified Enter from the ordinary submit key.
pub fn modified_enter(key: SpecialKey, shift: bool, alt: bool, control: bool) -> Option<Vec<u8>> {
    if key != SpecialKey::Enter || !(shift || alt || control) {
        return None;
    }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    Some(format!("\x1b[13;{modifier}u").into_bytes())
}

/// A candidate line break still needs an exact, newer source frame before
/// it becomes visible input. Custom bindings may give these keys other uses.
pub fn newline_key(bytes: &[u8]) -> bool {
    matches!(bytes, b"\x1b[13;2u" | b"\x1b[13;3u" | b"\x1b\r")
        || win32_enter_state(bytes).is_some_and(|state| state & 0x1b != 0 && state & 0x0c == 0)
}

/// Native Ctrl+Enter does not edit the default Codex draft. Keep only the
/// submission hint; a custom binding's subsequent source edit overrides it,
/// and a request still needs its complete fresh echo before archival.
pub fn keeps_submission_hint(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        b"\x1b[13;5u" | b"\x1b[13;6u" | b"\x1b[13;7u" | b"\x1b[13;8u"
    ) || win32_enter_state(bytes).is_some_and(|state| state & 0x0c != 0)
}

/// ConPTY requests DECSET 9001 for native console consumers. Unlike CSI-u,
/// these records preserve the modifiers seen by Codex's Windows event reader.
/// The caller uses this only after the current session requested that mode.
pub fn win32_key(
    virtual_key: u16,
    scan: u16,
    character: u16,
    shift: bool,
    alt: bool,
    control: bool,
    enhanced: bool,
) -> Vec<u8> {
    let state = (u32::from(shift) * 0x10)
        | (u32::from(alt) * 0x02)
        | (u32::from(control) * 0x08)
        | (u32::from(enhanced) * 0x100);
    format!(
        "\x1b[{virtual_key};{scan};{character};1;{state};1_\x1b[{virtual_key};{scan};{character};0;{state};1_"
    )
    .into_bytes()
}

/// Preserve literal BMP identity instead of reconstructing keys through the
/// active Windows layout. Native Codex otherwise loses punctuation and marks.
/// Supplementary scalars stay UTF-8: surrogate key records have different
/// repeat/release behavior in the Windows event reader.
pub fn literal_input(bytes: &[u8]) -> io::Result<Option<Vec<u8>>> {
    let pasted = bytes
        .strip_prefix(b"\x1b[200~")
        .and_then(|payload| payload.strip_suffix(b"\x1b[201~"));
    let Ok(text) = std::str::from_utf8(pasted.unwrap_or(bytes)) else {
        return Ok(None);
    };
    if text.is_empty() || (pasted.is_none() && text.chars().any(char::is_control)) {
        return Ok(None);
    }
    if pasted.is_some()
        && text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Вставка Codex содержит управляющие символы.",
        ));
    }
    if text.encode_utf16().take(32_769).count() > 32_768 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Вставка Codex превышает 32768 единиц UTF-16.",
        ));
    }
    let mut wire = String::with_capacity(text.len().saturating_mul(32).min(2 * 1024 * 1024));
    if pasted.is_some() {
        wire.push_str("\x1b[200~");
    }
    for ch in text.chars() {
        if u32::from(ch) <= u32::from(u16::MAX) && !ch.is_control() {
            let value = u32::from(ch);
            let _ = write!(wire, "\x1b[231;0;{value};1;0;1_\x1b[231;0;{value};0;0;1_");
        } else {
            wire.push(ch);
        }
    }
    if pasted.is_some() {
        wire.push_str("\x1b[201~");
    }
    Ok(Some(wire.into_bytes()))
}

fn win32_enter_state(bytes: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (down, up) = text.strip_prefix("\x1b[")?.split_once("_\x1b[")?;
    let up = up.strip_suffix('_')?;
    let down: Vec<u32> = down
        .split(';')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let up: Vec<u32> = up
        .split(';')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if down.len() != 6
        || up.len() != 6
        || down[0] != 13
        || down[2] != 13
        || down[3] != 1
        || up[3] != 0
        || down[5] != 1
        || up[5] != 1
        || [0, 1, 2, 4, 5]
            .iter()
            .any(|&index| down[index] != up[index])
    {
        return None;
    }
    Some(down[4])
}

/// Codex lays out textarea rows before writing VT. Join a physical wrap only
/// when every glyph and the source caret agree with the explicitly known
/// canonical text; actual line breaks and user whitespace remain meaningful.
pub fn editor_caret(rendered: &str, expected: &str, caret: u32) -> Option<u32> {
    let (consumed, mapped) = editor_prefix(rendered, expected, caret)?;
    (consumed == expected.encode_utf16().count() as u32).then_some(mapped)
}

/// A paste can arrive in several source frames. Verify its complete visible
/// prefix with the same wrap rules, without claiming the remainder was drawn.
pub fn editor_prefix(rendered: &str, expected: &str, caret: u32) -> Option<(u32, u32)> {
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
    Some((canonical as u32, mapped?))
}

/// Locate the visible suffix of a known literal draft. This confirms only
/// the displayed window; the caller must not publish the hidden draft text.
pub fn editor_suffix(rendered: &str, expected: &str, caret: u32) -> Option<u32> {
    if rendered.is_empty() {
        return None;
    }
    let expected: Vec<_> = expected.encode_utf16().collect();
    let visible = rendered.encode_utf16().count();
    let first = expected.len().saturating_sub(visible + 64).max(1);
    let last = expected.len().saturating_sub(visible.saturating_sub(64));
    let mut found = None;
    for start in first..=last {
        let Ok(suffix) = String::from_utf16(&expected[start..]) else {
            continue;
        };
        if let Some(mapped) = editor_caret(rendered, &suffix, caret) {
            let candidate = start as u32 + mapped;
            if found.is_some_and(|previous| previous != candidate) {
                return None;
            }
            found = Some(candidate);
        }
    }
    found
}

/// During a large paste the renderer can clip the beginning before it has
/// drawn the final suffix. Accept only one exact contiguous source window.
pub fn editor_window(rendered: &str, expected: &str, caret: u32) -> Option<u32> {
    let first_line = rendered
        .split("\r\n")
        .next()
        .filter(|line| !line.is_empty())?;
    let mut found = None;
    for (candidate_index, (start, _)) in expected
        .char_indices()
        .filter(|(start, _)| *start > 0 && expected[*start..].starts_with(first_line))
        .enumerate()
    {
        if candidate_index >= 128 {
            return None;
        }
        let Some((consumed, mapped)) = editor_prefix(rendered, &expected[start..], caret) else {
            continue;
        };
        if consumed == 0 || consumed != mapped {
            continue;
        }
        let candidate = expected[..start].encode_utf16().count() as u32 + mapped;
        if found.is_some_and(|previous| previous != candidate) {
            return None;
        }
        found = Some(candidate);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_enter_preserves_all_chords_and_leaves_plain_keys_to_the_shared_encoder() {
        for shift in [false, true] {
            for alt in [false, true] {
                for control in [false, true] {
                    let bytes = modified_enter(SpecialKey::Enter, shift, alt, control);
                    if shift || alt || control {
                        let modifier =
                            1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
                        assert_eq!(bytes.unwrap(), format!("\x1b[13;{modifier}u").as_bytes());
                    } else {
                        assert!(bytes.is_none());
                    }
                    assert!(modified_enter(SpecialKey::Tab, shift, alt, control).is_none());
                }
            }
        }
    }

    #[test]
    fn physical_wraps_require_the_complete_expected_text_and_a_valid_source_caret() {
        assert_eq!(editor_caret("alpha\r\nbeta", "alpha beta", 11), Some(10));
        assert_eq!(editor_caret("alpha\r\nbeta", "alphabeta", 11), Some(9));
        assert_eq!(editor_caret("alpha\r\nbeta", "alpha\r\nbeta", 11), Some(11));
        assert_eq!(editor_caret("Ж界🙂\r\ntail", "Ж界🙂tail", 10), Some(8));
        assert_eq!(editor_caret("alpha\r\nchanged", "alpha beta", 14), None);
        assert_eq!(editor_caret("alpha", "alpha beta", 5), None);
        assert_eq!(editor_caret("alpha", "alpha", 6), None);
        assert_eq!(editor_prefix("alpha\r\nbe", "alpha beta", 9), Some((8, 8)));
        assert_eq!(
            editor_window("beta\r\ngamma", "alpha beta gamma delta", 11),
            Some(16)
        );
        assert_eq!(
            editor_window("beta\r\nchanged", "alpha beta gamma delta", 13),
            None
        );
        assert_eq!(editor_window("repeat", "a repeat b repeat c", 6), None);
    }

    #[test]
    fn native_enter_records_keep_modifiers_and_only_recorded_newline_chords_are_candidates() {
        let shift = win32_key(13, 28, 13, true, false, false, false);
        assert_eq!(shift, b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_");
        assert!(newline_key(&shift));
        assert!(newline_key(&win32_key(
            13, 28, 13, false, true, false, false
        )));
        assert!(!newline_key(&win32_key(
            13, 28, 13, false, false, true, false
        )));
        assert!(!newline_key(&win32_key(
            13, 28, 13, false, false, false, false
        )));
        assert!(!newline_key(&win32_key(
            74, 36, 10, false, false, true, false
        )));
        assert!(!newline_key(b"\r"));
        assert!(!newline_key(b"\n"));
        assert!(!newline_key(b"\x1b[13;28;13;1;16;1_"));
        assert!(!newline_key(b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;0;1_"));
    }

    #[test]
    fn native_literal_transport_keeps_paste_framing_bmp_identity_and_adjacent_emoji() {
        let paste = crate::terminal_input::paste_bytes("[]e\u{301}🙂🙂\n", true);
        let wire = literal_input(&paste).unwrap().unwrap();
        let mut expected = b"\x1b[200~".to_vec();
        for value in [91, 93, 101, 769] {
            expected.extend(win32_key(231, 0, value, false, false, false, false));
        }
        expected.extend_from_slice("🙂🙂\r\x1b[201~".as_bytes());
        assert_eq!(wire, expected);
        assert!(literal_input(b"\x1b[13;2u").unwrap().is_none());
        assert!(literal_input(b"\x03").unwrap().is_none());
        assert!(literal_input(b"\r").unwrap().is_none());
        assert!(literal_input(b"\x1b[200~a\x1b[31mb\x1b[201~").is_err());
        assert!(literal_input(&crate::terminal_input::paste_bytes(
            &"x".repeat(32_769),
            true
        ))
        .is_err());
        let maximum = literal_input(&crate::terminal_input::paste_bytes(
            &"界".repeat(32_768),
            true,
        ))
        .unwrap()
        .unwrap();
        assert!(maximum.len() < bks_pty::INPUT_QUEUE_MAX_BYTES);
    }
}
