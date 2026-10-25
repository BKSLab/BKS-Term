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
        SpecialKey::CtrlC => b"\x03",
        SpecialKey::CtrlD => b"\x04",
        SpecialKey::CtrlZ => b"\x1a",
    }
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
}
