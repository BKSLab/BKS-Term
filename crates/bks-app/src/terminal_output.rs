pub struct TerminalChunk {
    pub text: String,
    pub events: Vec<TerminalEvent>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    PromptStart(usize),
    CommandStart(usize),
}

#[derive(Default)]
pub struct TerminalOutput {
    state: State,
    pending: [u8; 4],
    pending_len: usize,
    last_cr: bool,
    osc: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Ground,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

impl TerminalOutput {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.push_chunk(bytes).text
    }

    pub fn push_chunk(&mut self, bytes: &[u8]) -> TerminalChunk {
        let mut out = String::new();
        let mut events = Vec::new();
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b, &mut out),
                State::Esc => match b {
                    b'[' => self.state = State::Csi,
                    b']' => {
                        self.osc.clear();
                        self.state = State::Osc;
                    }
                    0x1B => {}
                    _ => self.state = State::Ground,
                },
                State::Csi => {
                    if b == 0x1B {
                        self.state = State::Esc;
                    } else if (0x40..=0x7E).contains(&b) {
                        self.state = State::Ground;
                    }
                }
                State::Osc => match b {
                    0x07 => self.finish_osc(&out, &mut events),
                    0x1B => self.state = State::OscEsc,
                    _ => {
                        if self.osc.len() < 4096 {
                            self.osc.push(b);
                        }
                    }
                },
                State::OscEsc => match b {
                    b'\\' => self.finish_osc(&out, &mut events),
                    0x1B => {}
                    _ => self.state = State::Osc,
                },
            }
        }
        TerminalChunk { text: out, events }
    }

    fn finish_osc(&mut self, out: &str, events: &mut Vec<TerminalEvent>) {
        let event = match self.osc.as_slice() {
            b"133;A" => Some(TerminalEvent::PromptStart(out.encode_utf16().count())),
            b"133;B" => Some(TerminalEvent::CommandStart(out.encode_utf16().count())),
            _ => None,
        };
        if let Some(event) = event {
            events.push(event);
        }
        self.state = State::Ground;
    }

    fn ground(&mut self, b: u8, out: &mut String) {
        if b < 0x80 {
            if self.pending_len > 0 {
                out.push('\u{FFFD}');
                self.pending_len = 0;
                self.last_cr = false;
            }
            match b {
                0x1B => self.state = State::Esc,
                b'\n' => {
                    if !self.last_cr {
                        out.push('\r');
                    }
                    out.push('\n');
                    self.last_cr = false;
                }
                b'\r' => {
                    out.push('\r');
                    self.last_cr = true;
                }
                b'\t' => {
                    out.push('\t');
                    self.last_cr = false;
                }
                0x00..=0x1F | 0x7F => {}
                _ => {
                    out.push(b as char);
                    self.last_cr = false;
                }
            }
        } else {
            if self.pending_len >= 4 {
                out.push('\u{FFFD}');
                self.pending_len = 0;
                self.last_cr = false;
            }
            self.pending[self.pending_len] = b;
            self.pending_len += 1;
            let before = out.len();
            self.flush_utf8(out);
            if out.len() > before {
                self.last_cr = false;
            }
        }
    }

    fn flush_utf8(&mut self, out: &mut String) {
        loop {
            let len = self.pending_len;
            let (valid, err_len) = match std::str::from_utf8(&self.pending[..len]) {
                Ok(_) => (len, None),
                Err(e) => (e.valid_up_to(), e.error_len()),
            };
            if valid > 0 {
                if let Ok(s) = std::str::from_utf8(&self.pending[..valid]) {
                    out.push_str(s);
                }
            }
            match err_len {
                None => {
                    if valid > 0 {
                        self.pending.copy_within(valid..len, 0);
                        self.pending_len = len - valid;
                    }
                    return;
                }
                Some(n) => {
                    out.push('\u{FFFD}');
                    self.pending.copy_within(valid + n..len, 0);
                    self.pending_len = len - valid - n;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_split_between_calls() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(&[0xD0]), "");
        assert_eq!(t.push(&[0x9F, b'!']), "П!");
    }

    #[test]
    fn csi_split_between_calls() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"ab\x1b["), "ab");
        assert_eq!(t.push(b"31mcd"), "cd");
    }

    #[test]
    fn osc_split_between_calls() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"x\x1b]0;ti"), "x");
        assert_eq!(t.push(b"tle\x07y"), "y");

        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"\x1b]0;a\x1b"), "");
        assert_eq!(t.push(b"\\b"), "b");
    }

    #[test]
    fn osc_command_block_events() {
        let mut t = TerminalOutput::new();
        let first = t.push_chunk("П\x1b]133;A\x1b\\PS \x1b]133;B\x1b".as_bytes());
        assert_eq!(first.text, "ПPS ");
        assert_eq!(first.events, vec![TerminalEvent::PromptStart(1)]);

        let second = t.push_chunk("\\результат".as_bytes());
        assert_eq!(second.text, "результат");
        assert_eq!(second.events, vec![TerminalEvent::CommandStart(0)]);

        let bel = t.push_chunk(b"x\x1b]133;B\x07y");
        assert_eq!(bel.text, "xy");
        assert_eq!(bel.events, vec![TerminalEvent::CommandStart(1)]);
    }

    #[test]
    fn crlf_not_doubled() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"a\r\nb\nc"), "a\r\nb\r\nc");

        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"x\r"), "x\r");
        assert_eq!(t.push(b"\ny"), "\ny");
    }

    #[test]
    fn plain_text_and_bel() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(b"hi\x07there\x1b[0m"), "hithere");
        assert_eq!(t.push(b"\t!\n"), "\t!\r\n");
    }

    #[test]
    fn invalid_utf8() {
        let mut t = TerminalOutput::new();
        assert_eq!(t.push(&[b'A', 0xFF, b'B']), "A\u{FFFD}B");

        let mut t = TerminalOutput::new();
        assert_eq!(t.push(&[0xE2, 0x82]), "");
        assert_eq!(t.push(b"A"), "\u{FFFD}A");
    }
}
