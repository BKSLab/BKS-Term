#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Insert,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    CtrlC,
    CtrlD,
    CtrlZ,
}

pub fn special_key_bytes(key: SpecialKey) -> &'static [u8] {
    match key {
        SpecialKey::Enter => b"\r",
        SpecialKey::Backspace => b"\x7f",
        SpecialKey::Tab => b"\t",
        SpecialKey::Escape => b"\x1b",
        SpecialKey::Up => b"\x1b[A",
        SpecialKey::Down => b"\x1b[B",
        SpecialKey::Right => b"\x1b[C",
        SpecialKey::Left => b"\x1b[D",
        SpecialKey::Home => b"\x1b[H",
        SpecialKey::End => b"\x1b[F",
        SpecialKey::Delete => b"\x1b[3~",
        SpecialKey::Insert => b"\x1b[2~",
        SpecialKey::PageUp => b"\x1b[5~",
        SpecialKey::PageDown => b"\x1b[6~",
        SpecialKey::F1 => b"\x1bOP",
        SpecialKey::F2 => b"\x1bOQ",
        SpecialKey::F3 => b"\x1bOR",
        SpecialKey::F4 => b"\x1bOS",
        SpecialKey::F5 => b"\x1b[15~",
        SpecialKey::F6 => b"\x1b[17~",
        SpecialKey::F7 => b"\x1b[18~",
        SpecialKey::F8 => b"\x1b[19~",
        SpecialKey::F9 => b"\x1b[20~",
        SpecialKey::F10 => b"\x1b[21~",
        SpecialKey::F11 => b"\x1b[23~",
        SpecialKey::F12 => b"\x1b[24~",
        SpecialKey::CtrlC => b"\x03",
        SpecialKey::CtrlD => b"\x04",
        SpecialKey::CtrlZ => b"\x1a",
    }
}

/// Encode cursor and function keys with the xterm modifier parameter.
/// Application cursor mode affects unmodified cursor keys only.
pub fn encode_key(
    key: SpecialKey,
    shift: bool,
    alt: bool,
    control: bool,
    application_cursor: bool,
) -> Vec<u8> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let cursor = match key {
        SpecialKey::Up => Some('A'),
        SpecialKey::Down => Some('B'),
        SpecialKey::Right => Some('C'),
        SpecialKey::Left => Some('D'),
        SpecialKey::Home => Some('H'),
        SpecialKey::End => Some('F'),
        _ => None,
    };
    if let Some(final_byte) = cursor {
        return if modifier != 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            special_key_bytes(key).to_vec()
        };
    }
    let function = match key {
        SpecialKey::F1 => Some('P'),
        SpecialKey::F2 => Some('Q'),
        SpecialKey::F3 => Some('R'),
        SpecialKey::F4 => Some('S'),
        _ => None,
    };
    if let Some(final_byte) = function {
        return if modifier == 1 {
            special_key_bytes(key).to_vec()
        } else {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        };
    }
    let number = match key {
        SpecialKey::Insert => Some(2),
        SpecialKey::Delete => Some(3),
        SpecialKey::PageUp => Some(5),
        SpecialKey::PageDown => Some(6),
        SpecialKey::F5 => Some(15),
        SpecialKey::F6 => Some(17),
        SpecialKey::F7 => Some(18),
        SpecialKey::F8 => Some(19),
        SpecialKey::F9 => Some(20),
        SpecialKey::F10 => Some(21),
        SpecialKey::F11 => Some(23),
        SpecialKey::F12 => Some(24),
        _ => None,
    };
    if let Some(number) = number {
        return if modifier == 1 {
            special_key_bytes(key).to_vec()
        } else {
            format!("\x1b[{number};{modifier}~").into_bytes()
        };
    }

    let mut output = Vec::new();
    if alt {
        output.push(0x1b);
    }
    if key == SpecialKey::Tab && shift {
        output.extend_from_slice(b"\x1b[Z");
    } else if key == SpecialKey::Backspace && control {
        output.push(0x08);
    } else {
        output.extend_from_slice(special_key_bytes(key));
    }
    output
}

/// Map Windows virtual keys to the traditional ASCII control characters.
/// The OEM punctuation keys use the Windows US-keyboard virtual-key names.
pub fn control_char(vkey: usize, _shift: bool) -> Option<u8> {
    match vkey {
        0x43 => Some(special_key_bytes(SpecialKey::CtrlC)[0]),
        0x44 => Some(special_key_bytes(SpecialKey::CtrlD)[0]),
        0x5a => Some(special_key_bytes(SpecialKey::CtrlZ)[0]),
        0x41..=0x5a => Some(vkey as u8 - b'A' + 1),
        0x20 | 0x32 => Some(0x00), // Space / 2 (@).
        0xdb => Some(0x1b),        // [ or {.
        0xdc => Some(0x1c),        // \ or |.
        0xdd => Some(0x1d),        // ] or }.
        0x36 => Some(0x1e),        // 6 (^).
        0xbd => Some(0x1f),        // - (_).
        _ => None,
    }
}

/// Encode unmodified Windows keypad keys in VT application keypad mode.
/// The extended flag distinguishes keypad Enter from the main Enter key.
pub fn application_keypad_bytes(vkey: usize, extended: bool) -> Option<&'static [u8]> {
    match vkey {
        0x60 => Some(b"\x1bOp"),
        0x61 => Some(b"\x1bOq"),
        0x62 => Some(b"\x1bOr"),
        0x63 => Some(b"\x1bOs"),
        0x64 => Some(b"\x1bOt"),
        0x65 => Some(b"\x1bOu"),
        0x66 => Some(b"\x1bOv"),
        0x67 => Some(b"\x1bOw"),
        0x68 => Some(b"\x1bOx"),
        0x69 => Some(b"\x1bOy"),
        0x6a => Some(b"\x1bOj"), // Multiply.
        0x6b => Some(b"\x1bOk"), // Add.
        0x6c => Some(b"\x1bOl"), // Separator.
        0x6d => Some(b"\x1bOm"), // Subtract.
        0x6e => Some(b"\x1bOn"), // Decimal.
        0x6f => Some(b"\x1bOo"), // Divide.
        0x0d if extended => Some(b"\x1bOM"),
        _ => None,
    }
}

/// Paste uses terminal line endings, independently of the clipboard's platform.
/// A bracketed payload cannot contain its own framing delimiters.
pub fn paste_bytes(text: &str, bracketed_paste: bool) -> Vec<u8> {
    let payload = text.replace("\r\n", "\r").replace('\n', "\r");
    if !bracketed_paste {
        return payload.into_bytes();
    }
    let mut sanitized = Vec::with_capacity(payload.len());
    for byte in payload.bytes() {
        sanitized.push(byte);
        for delimiter in [
            b"\x1b[200~".as_slice(),
            b"\x1b[201~".as_slice(),
            "\u{009b}200~".as_bytes(),
            "\u{009b}201~".as_bytes(),
        ] {
            if sanitized.ends_with(delimiter) {
                sanitized.truncate(sanitized.len() - delimiter.len());
                break;
            }
        }
    }
    let mut output = Vec::with_capacity(sanitized.len() + 12);
    output.extend_from_slice(b"\x1b[200~");
    output.extend_from_slice(&sanitized);
    output.extend_from_slice(b"\x1b[201~");
    output
}

#[derive(Default)]
pub struct Utf16Encoder {
    pending_high: Option<u16>,
}

impl Utf16Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, unit: u16) -> Option<Vec<u8>> {
        if let Some(high) = self.pending_high.take() {
            if (0xDC00..=0xDFFF).contains(&unit) {
                let code = 0x10000 + (((high as u32 - 0xD800) << 10) | (unit as u32 - 0xDC00));
                let character = char::from_u32(code).unwrap_or('\u{FFFD}');
                return Some(character.to_string().into_bytes());
            }
            let mut output = '\u{FFFD}'.to_string().into_bytes();
            if (0xD800..=0xDBFF).contains(&unit) {
                self.pending_high = Some(unit);
            } else {
                let character = char::from_u32(unit as u32).unwrap_or('\u{FFFD}');
                output.extend_from_slice(character.to_string().as_bytes());
            }
            return Some(output);
        }
        if (0xD800..=0xDBFF).contains(&unit) {
            self.pending_high = Some(unit);
            None
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            Some('\u{FFFD}'.to_string().into_bytes())
        } else {
            let character = char::from_u32(unit as u32).unwrap_or('\u{FFFD}');
            Some(character.to_string().into_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_keys_bytes() {
        assert_eq!(special_key_bytes(SpecialKey::Enter), b"\r");
        assert_eq!(special_key_bytes(SpecialKey::Backspace), b"\x7f");
        assert_eq!(special_key_bytes(SpecialKey::Tab), b"\t");
        assert_eq!(special_key_bytes(SpecialKey::Escape), b"\x1b");
        assert_eq!(special_key_bytes(SpecialKey::Up), b"\x1b[A");
        assert_eq!(special_key_bytes(SpecialKey::Down), b"\x1b[B");
        assert_eq!(special_key_bytes(SpecialKey::Right), b"\x1b[C");
        assert_eq!(special_key_bytes(SpecialKey::Left), b"\x1b[D");
        assert_eq!(special_key_bytes(SpecialKey::Home), b"\x1b[H");
        assert_eq!(special_key_bytes(SpecialKey::End), b"\x1b[F");
        assert_eq!(special_key_bytes(SpecialKey::Delete), b"\x1b[3~");
        assert_eq!(special_key_bytes(SpecialKey::Insert), b"\x1b[2~");
        assert_eq!(special_key_bytes(SpecialKey::PageUp), b"\x1b[5~");
        assert_eq!(special_key_bytes(SpecialKey::PageDown), b"\x1b[6~");
        assert_eq!(special_key_bytes(SpecialKey::CtrlC), b"\x03");
        assert_eq!(special_key_bytes(SpecialKey::CtrlD), b"\x04");
        assert_eq!(special_key_bytes(SpecialKey::CtrlZ), b"\x1a");
    }

    #[test]
    fn unicode_input() {
        let mut encoder = Utf16Encoder::new();
        assert_eq!(encoder.push(0x0041), Some(b"A".to_vec()));
        assert_eq!(encoder.push(0x0416), Some(vec![0xD0, 0x96]));
        assert_eq!(encoder.push(0xD83D), None);
        assert_eq!(encoder.push(0xDE00), Some("😀".as_bytes().to_vec()));
        assert_eq!(encoder.push(0xDC00), Some(vec![0xEF, 0xBF, 0xBD]));
        assert_eq!(encoder.push(0xD800), None);
        assert_eq!(encoder.push(0x0041), Some(vec![0xEF, 0xBF, 0xBD, b'A']));
        assert_eq!(encoder.push(0xD83D), None);
        assert_eq!(encoder.push(0xD83D), Some(vec![0xEF, 0xBF, 0xBD]));
        assert_eq!(encoder.push(0xDE00), Some("😀".as_bytes().to_vec()));
    }

    #[test]
    fn application_cursor_mode_does_not_override_modifiers() {
        assert_eq!(
            encode_key(SpecialKey::Up, false, false, false, true),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(SpecialKey::Home, false, false, false, true),
            b"\x1bOH"
        );
        assert_eq!(
            encode_key(SpecialKey::End, false, false, false, false),
            b"\x1b[F"
        );
        assert_eq!(
            encode_key(SpecialKey::Up, false, false, true, true),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encode_key(SpecialKey::Left, true, true, true, true),
            b"\x1b[1;8D"
        );
    }

    #[test]
    fn cursor_key_modifiers_cover_each_xterm_combination() {
        let modifiers = [
            (true, false, false, 2),
            (false, true, false, 3),
            (true, true, false, 4),
            (false, false, true, 5),
            (true, false, true, 6),
            (false, true, true, 7),
            (true, true, true, 8),
        ];
        for (shift, alt, control, expected) in modifiers {
            assert_eq!(
                encode_key(SpecialKey::Down, shift, alt, control, false),
                format!("\x1b[1;{expected}B").as_bytes(),
            );
        }
    }

    #[test]
    fn function_and_navigation_keys_keep_their_protocol_families() {
        assert_eq!(
            encode_key(SpecialKey::F1, false, false, false, true),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(SpecialKey::F4, true, false, false, true),
            b"\x1b[1;2S"
        );
        assert_eq!(
            encode_key(SpecialKey::F5, false, true, false, false),
            b"\x1b[15;3~"
        );
        assert_eq!(
            encode_key(SpecialKey::F12, false, false, false, false),
            b"\x1b[24~"
        );
        assert_eq!(
            encode_key(SpecialKey::Delete, true, false, true, false),
            b"\x1b[3;6~"
        );
        assert_eq!(
            encode_key(SpecialKey::PageDown, false, false, true, false),
            b"\x1b[6;5~"
        );
    }

    #[test]
    fn reverse_tab_and_alt_control_keys_remain_distinct() {
        assert_eq!(
            encode_key(SpecialKey::Tab, true, false, false, false),
            b"\x1b[Z"
        );
        assert_eq!(
            encode_key(SpecialKey::Tab, false, true, false, false),
            b"\x1b\t"
        );
        assert_eq!(
            encode_key(SpecialKey::Enter, false, true, false, false),
            b"\x1b\r"
        );
        assert_eq!(
            encode_key(SpecialKey::Backspace, false, false, true, false),
            b"\x08"
        );
        assert_eq!(
            encode_key(SpecialKey::Backspace, false, true, true, false),
            b"\x1b\x08"
        );
    }

    #[test]
    fn control_keys_cover_the_ascii_control_alphabet() {
        for shift in [false, true] {
            assert_eq!(control_char(0x41, shift), Some(0x01));
            assert_eq!(control_char(0x43, shift), Some(0x03));
            assert_eq!(control_char(0x56, shift), Some(0x16));
            assert_eq!(control_char(0x5a, shift), Some(0x1a));
            assert_eq!(control_char(0x20, shift), Some(0x00));
            assert_eq!(control_char(0xdb, shift), Some(0x1b));
            assert_eq!(control_char(0xdc, shift), Some(0x1c));
            assert_eq!(control_char(0xdd, shift), Some(0x1d));
            assert_eq!(control_char(0x36, shift), Some(0x1e));
            assert_eq!(control_char(0xbd, shift), Some(0x1f));
        }
        assert_eq!(control_char(0x70, false), None); // F1.
        assert_eq!(control_char(0x25, false), None); // Left.
    }

    #[test]
    fn paste_preserves_unicode_tabs_and_spaces_with_terminal_line_endings() {
        assert_eq!(
            paste_bytes("  Ж😀\t\r\nnext\nlast\r", false),
            "  Ж😀\t\rnext\rlast\r".as_bytes(),
        );
        assert_eq!(paste_bytes("", false), b"");
        assert_eq!(paste_bytes("", true), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn application_keypad_distinguishes_keypad_and_ordinary_keys() {
        assert_eq!(
            application_keypad_bytes(0x60, false),
            Some(b"\x1bOp".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x69, false),
            Some(b"\x1bOy".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x0d, true),
            Some(b"\x1bOM".as_slice())
        );
        assert_eq!(application_keypad_bytes(0x0d, false), None);
        assert_eq!(application_keypad_bytes(0x30, false), None);
        assert_eq!(application_keypad_bytes(0x39, true), None);
    }

    #[test]
    fn application_keypad_encodes_decimal_and_arithmetic_operators() {
        assert_eq!(
            application_keypad_bytes(0x6a, false),
            Some(b"\x1bOj".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x6b, false),
            Some(b"\x1bOk".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x6c, false),
            Some(b"\x1bOl".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x6d, false),
            Some(b"\x1bOm".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x6e, false),
            Some(b"\x1bOn".as_slice())
        );
        assert_eq!(
            application_keypad_bytes(0x6f, true),
            Some(b"\x1bOo".as_slice())
        );
        assert_eq!(application_keypad_bytes(0xbd, false), None); // Main minus.
        assert_eq!(application_keypad_bytes(0xbb, false), None); // Main plus.
    }

    #[test]
    fn bracketed_paste_cannot_end_inside_the_clipboard_text() {
        let clipboard = "a\x1b[201~\nb\x1b[200~c\u{009b}201~d\u{009b}200~";
        assert_eq!(paste_bytes(clipboard, true), b"\x1b[200~a\rbcd\x1b[201~");
        assert_eq!(
            paste_bytes("\x1b[31mred\x1b[0m", true),
            b"\x1b[200~\x1b[31mred\x1b[0m\x1b[201~",
        );
        assert_eq!(paste_bytes("\x1b[201~", false), b"\x1b[201~");
        assert_eq!(
            paste_bytes("\x1b[20\x1b[201~1~", true),
            b"\x1b[200~\x1b[201~",
        );
    }
}
