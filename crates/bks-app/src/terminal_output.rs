use bks_terminal::{StyleSpan, StyleTracker, TextStyle};

pub struct TerminalChunk {
    pub text: String,
    /// Source formatting in chunk-local canonical UTF-16 coordinates.
    pub styles: Vec<StyleSpan>,
    pub events: Vec<TerminalEvent>,
    /// Cursor/erase operations or a bare CR followed by text. These chunks
    /// cannot establish that journal lines represent a linear screen append.
    pub screen_rewrite: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    PromptStart(usize),
    CommandStart(usize),
    OwnedPromptStart(usize, u64),
    OwnedCommandStart(usize, u64),
    Clear(usize),
    /// Empty-row traversal following Clear, completed by a return to home.
    ClearLayout(usize),
}

#[derive(Default)]
pub struct TerminalOutput {
    state: State,
    pending: [u8; 4],
    pending_len: usize,
    last_cr: bool,
    osc: Vec<u8>,
    csi: Vec<u8>,
    csi_overflow: bool,
    concealed: bool,
    saved_concealed: [bool; 2],
    main_concealed: bool,
    alternate_screen: bool,
    clear_alternate_screen: bool,
    initial_layout: bool,
    journal_started: bool,
    shell_lines: bool,
    line_has_text: bool,
    clear_layout: Option<ClearLayout>,
    styles: StyleTracker,
}

#[derive(Default)]
struct ClearLayout {
    row: ClearRow,
    rows: u16,
}

#[derive(Default)]
enum ClearRow {
    #[default]
    Start,
    Erased,
    CarriageReturn,
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

    pub fn set_shell_lines(&mut self, enabled: bool) {
        self.shell_lines = enabled;
    }

    #[cfg(test)]
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.push_chunk(bytes).text
    }

    pub fn push_chunk(&mut self, bytes: &[u8]) -> TerminalChunk {
        let mut out = String::new();
        let mut events = Vec::new();
        let mut styles = Vec::new();
        let mut offset = 0u32;
        let mut screen_rewrite = false;
        for &b in bytes {
            if matches!(self.state, State::Ground) {
                self.clear_layout_ground(b);
            } else if matches!(self.state, State::Esc) && b != b'[' {
                self.clear_layout = None;
            }
            let before = out.len();
            let style = if self.concealed {
                TextStyle::default()
            } else {
                self.styles.style()
            };
            match self.state {
                State::Ground => {
                    // Defer classification until the byte after CR is known.
                    // last_cr survives chunk boundaries and nonprinting SGR/
                    // OSC sequences; CRLF remains a normal line ending.
                    if (self.last_cr && (b >= 0x20 || matches!(b, b'\r' | b'\t'))) || b == 0x08 {
                        screen_rewrite = true;
                    }
                    self.ground(b, &mut out);
                }
                State::Esc => match b {
                    b'[' => {
                        self.csi.clear();
                        self.csi_overflow = false;
                        self.state = State::Csi;
                    }
                    b']' => {
                        self.osc.clear();
                        self.state = State::Osc;
                    }
                    0x1B => {}
                    b'7' | b'8' | b'D' | b'E' | b'M' | b'c' => {
                        match b {
                            b'7' => {
                                self.saved_concealed[self.alternate_screen as usize] =
                                    self.concealed
                            }
                            b'8' => {
                                self.concealed =
                                    self.saved_concealed[self.alternate_screen as usize]
                            }
                            b'c' => {
                                self.concealed = false;
                                self.saved_concealed = [false; 2];
                                self.main_concealed = false;
                                self.alternate_screen = false;
                                self.clear_alternate_screen = false;
                            }
                            _ => {}
                        }
                        screen_rewrite = true;
                        self.state = State::Ground;
                    }
                    _ => self.state = State::Ground,
                },
                State::Csi => {
                    if b == 0x1B {
                        self.clear_layout = None;
                        self.state = State::Esc;
                    } else if (0x40..=0x7E).contains(&b) {
                        if !self.journal_started && b == b'J' && self.csi == b"2" {
                            // ConPTY serializes a resize before the first
                            // output as ED 2/home, blank rows, then home again.
                            // Those initial rows describe layout, not a log.
                            self.initial_layout = true;
                        }
                        self.csi_journal_events(b, offset, &mut events);
                        self.csi_shell_line_break(b, &mut out);
                        self.csi_visibility(b);
                        screen_rewrite |= rewrites_screen(b);
                        self.state = State::Ground;
                    } else if matches!(b, 0x18 | 0x1A) {
                        self.clear_layout = None;
                        self.state = State::Ground;
                    } else if b >= 0x20 {
                        if self.csi.len() < 256 {
                            self.csi.push(b);
                        } else {
                            self.csi_overflow = true;
                        }
                    }
                }
                State::Osc => match b {
                    0x07 => self.finish_osc(offset, &mut events),
                    0x1B => self.state = State::OscEsc,
                    _ => {
                        if self.osc.len() < 4096 {
                            self.osc.push(b);
                        }
                    }
                },
                State::OscEsc => match b {
                    b'\\' => self.finish_osc(offset, &mut events),
                    0x1B => {}
                    _ => self.state = State::Osc,
                },
            }
            // Only newly emitted characters are scanned. In particular, SGR
            // and OSC-heavy chunks never repeatedly scan the growing journal.
            let emitted = out[before..].encode_utf16().count() as u32;
            StyleSpan::append(&mut styles, offset, offset + emitted, style);
            offset += emitted;
            self.styles.push_byte(b);
        }
        TerminalChunk {
            text: out,
            styles,
            events,
            screen_rewrite,
        }
    }

    fn clear_layout_ground(&mut self, byte: u8) {
        if byte == 0x1b {
            return;
        }
        let Some(layout) = &mut self.clear_layout else {
            return;
        };
        match (&layout.row, byte) {
            (ClearRow::Erased, b'\r') => layout.row = ClearRow::CarriageReturn,
            (ClearRow::CarriageReturn, b'\n') if layout.rows < 4_096 => {
                layout.rows += 1;
                layout.row = ClearRow::Start;
            }
            _ => self.clear_layout = None,
        }
    }

    fn csi_journal_events(&mut self, final_byte: u8, offset: u32, events: &mut Vec<TerminalEvent>) {
        if self.csi_overflow {
            self.clear_layout = None;
            return;
        }
        if matches!(final_byte, b'h' | b'l')
            && self.csi.first() == Some(&b'?')
            && self.csi[1..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || *byte == b';')
            && self.csi[1..]
                .split(|byte| *byte == b';')
                .any(|parameter| matches!(sgr_number(parameter), Some(47 | 1047 | 1049)))
        {
            self.clear_alternate_screen = final_byte == b'h';
            self.clear_layout = None;
            return;
        }
        if final_byte == b'J'
            && matches!(self.csi.as_slice(), b"2" | b"3")
            && !self.clear_alternate_screen
        {
            // The first ED2 is ConPTY's initial layout, not an application
            // command clearing a journal that may already track accepted input.
            if self.csi == b"2" && !self.journal_started {
                self.clear_layout = None;
                return;
            }
            events.push(TerminalEvent::Clear(offset as usize));
            self.clear_layout = Some(ClearLayout::default());
            return;
        }
        let Some(mut layout) = self.clear_layout.take() else {
            return;
        };
        match (final_byte, self.csi.as_slice()) {
            (b'K', b"" | b"0" | b"2")
                if matches!(layout.row, ClearRow::Start | ClearRow::Erased) =>
            {
                layout.row = ClearRow::Erased;
                self.clear_layout = Some(layout);
            }
            (b'H', b"" | b"1" | b"1;1")
                if layout.rows > 0 && !matches!(layout.row, ClearRow::CarriageReturn) =>
            {
                events.push(TerminalEvent::ClearLayout(offset as usize));
            }
            (b'h' | b'l', b"?25") => self.clear_layout = Some(layout),
            (b'm', parameters)
                if parameters
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':')) =>
            {
                self.clear_layout = Some(layout);
            }
            _ => {}
        }
    }

    fn csi_shell_line_break(&mut self, final_byte: u8, out: &mut String) {
        if !self.shell_lines
            || !self.line_has_text
            || self.alternate_screen
            || self.csi_overflow
            || !matches!(final_byte, b'B' | b'E' | b'd' | b'H' | b'f')
        {
            return;
        }
        // ConPTY may replace a shell's line feed with a cursor move to blank
        // rows. Keep a paragraph boundary in the reading journal instead of
        // joining the echoed command and the first result. This is a journal
        // separator, not an attempt to replay cursor rewrites as a VT screen.
        let parameters: Vec<_> = self.csi.split(|byte| *byte == b';').collect();
        let number = |index: usize| {
            parameters.get(index).and_then(|value| {
                if value.is_empty() {
                    Some(1)
                } else {
                    sgr_number(value)
                }
            })
        };
        let moves_line = match final_byte {
            b'B' | b'E' => parameters.len() == 1 && number(0).is_some(),
            b'd' => parameters.len() == 1 && number(0).is_some_and(|row| row > 1),
            b'H' | b'f' => {
                parameters.len() <= 2
                    && number(0).is_some_and(|row| row > 1)
                    && (parameters.len() == 1 || number(1).is_some_and(|column| column <= 1))
            }
            _ => false,
        };
        if moves_line {
            out.push_str("\r\n");
            self.line_has_text = false;
            self.last_cr = false;
        }
    }

    fn finish_osc(&mut self, offset: u32, events: &mut Vec<TerminalEvent>) {
        let event = match self.osc.as_slice() {
            b"133;A" => Some(TerminalEvent::PromptStart(offset as usize)),
            b"133;B" => Some(TerminalEvent::CommandStart(offset as usize)),
            marker => owned_prompt_marker(marker, offset as usize),
        };
        if let Some(event) = event {
            events.push(event);
        }
        self.state = State::Ground;
    }

    fn ground(&mut self, b: u8, out: &mut String) {
        if b < 0x80 {
            if self.pending_len > 0 {
                self.character('\u{FFFD}', out);
                self.pending_len = 0;
                self.last_cr = false;
            }
            match b {
                0x1B => self.state = State::Esc,
                b'\n' => {
                    self.line_has_text = false;
                    if !self.initial_layout || self.journal_started {
                        if !self.last_cr {
                            out.push('\r');
                        }
                        out.push('\n');
                        self.journal_started = true;
                    }
                    self.last_cr = false;
                }
                b'\r' => {
                    self.line_has_text = false;
                    if !self.initial_layout || self.journal_started {
                        out.push('\r');
                        self.journal_started = true;
                    }
                    self.last_cr = true;
                }
                b'\t' => {
                    self.character('\t', out);
                    self.last_cr = false;
                }
                0x00..=0x1F | 0x7F => {}
                _ => {
                    self.character(b as char, out);
                    self.last_cr = false;
                }
            }
        } else {
            if self.pending_len >= 4 {
                self.character('\u{FFFD}', out);
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
                    self.journal_started = true;
                    if self.concealed {
                        out.extend(std::iter::repeat_n(' ', s.encode_utf16().count()));
                    } else {
                        out.push_str(s);
                    }
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
                    self.character('\u{FFFD}', out);
                    self.pending.copy_within(valid + n..len, 0);
                    self.pending_len = len - valid - n;
                }
            }
        }
    }

    fn character(&mut self, character: char, out: &mut String) {
        // Leading spaces/tabs are content too: preserve first-line indentation
        // and every subsequent blank line instead of trimming all whitespace.
        self.journal_started = true;
        self.line_has_text = true;
        if self.concealed {
            out.extend(std::iter::repeat_n(' ', character.len_utf16()));
        } else {
            out.push(character);
        }
    }

    fn csi_visibility(&mut self, final_byte: u8) {
        if final_byte == b'm' {
            self.sgr();
        } else if !self.csi_overflow
            && self
                .csi
                .iter()
                .all(|b| b.is_ascii_digit() || matches!(b, b';' | b':'))
            && self.csi.iter().filter(|b| matches!(b, b';' | b':')).count() < 32
        {
            match final_byte {
                b's' => self.saved_concealed[self.alternate_screen as usize] = self.concealed,
                b'u' => self.concealed = self.saved_concealed[self.alternate_screen as usize],
                _ => {}
            }
        } else if !self.csi_overflow
            && matches!(final_byte, b'h' | b'l')
            && self.csi.first() == Some(&b'?')
            && self.csi[1..]
                .iter()
                .all(|b| b.is_ascii_digit() || *b == b';')
            && self.csi.iter().filter(|b| **b == b';').count() < 32
            && self.csi[1..]
                .split(|b| *b == b';')
                .any(|p| sgr_number(p) == Some(1049))
        {
            // The VT engine copies current attributes to the alternate cursor
            // and preserves the main cursor on entry; leaving restores main.
            let alternate = final_byte == b'h';
            if alternate && !self.alternate_screen {
                self.main_concealed = self.concealed;
                self.saved_concealed[0] = self.concealed;
            } else if !alternate && self.alternate_screen {
                self.concealed = self.main_concealed;
            }
            self.alternate_screen = alternate;
        }
    }

    fn sgr(&mut self) {
        // A truncated or unsupported encoding must never turn conceal off.
        // Only an unambiguous SGR 0/28 restores journal text after that case.
        if self.csi_overflow
            || self.csi.iter().filter(|b| matches!(b, b';' | b':')).count() >= 32
            || self
                .csi
                .iter()
                .any(|b| !b.is_ascii_digit() && !matches!(b, b';' | b':'))
        {
            self.concealed = true;
            return;
        }
        let mut parameters = self.csi.split(|b| *b == b';');
        while let Some(parameter) = parameters.next() {
            // Colon subparameters belong to one SGR attribute, so e.g. a
            // color component 8 or 28 cannot affect visibility.
            if parameter.contains(&b':') {
                continue;
            }
            match sgr_number(parameter) {
                Some(0 | 28) => self.concealed = false,
                Some(8) => self.concealed = true,
                Some(38 | 48 | 58) => {
                    // Consume color payload exactly as the VT parser does;
                    // invalid channel values stop consuming that color.
                    let mode = parameters.next().and_then(|p| {
                        sgr_number(p.split(|b| *b == b':').next().unwrap_or_default())
                    });
                    let channels = match mode {
                        Some(2) => 3,
                        Some(5) => 1,
                        _ => 0,
                    };
                    for _ in 0..channels {
                        let Some(channel) = parameters.next() else {
                            break;
                        };
                        let value =
                            sgr_number(channel.split(|b| *b == b':').next().unwrap_or_default());
                        if value.is_none_or(|n| n > 255) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn owned_prompt_marker(marker: &[u8], offset: usize) -> Option<TerminalEvent> {
    let (start, token) = if let Some(token) = marker.strip_prefix(b"133;A;BKS=") {
        (true, token)
    } else {
        (false, marker.strip_prefix(b"133;B;BKS=")?)
    };
    if token.len() != 16 || !token.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let token = u64::from_str_radix(std::str::from_utf8(token).ok()?, 16).ok()?;
    Some(if start {
        TerminalEvent::OwnedPromptStart(offset, token)
    } else {
        TerminalEvent::OwnedCommandStart(offset, token)
    })
}

fn sgr_number(parameter: &[u8]) -> Option<u16> {
    if parameter.is_empty() {
        Some(0)
    } else {
        std::str::from_utf8(parameter).ok()?.parse().ok()
    }
}

fn rewrites_screen(final_byte: u8) -> bool {
    // Cursor movement, insertion/deletion/erasure, scrolling, scroll regions,
    // saved cursor and modes (including main/alternate-screen transitions).
    // SGR 'm' and status/device queries do not rewrite text.
    matches!(
        final_byte,
        b'@'..=b'T'
            | b'X'
            | b'Z'
            | b'`'
            | b'a'
            | b'b'
            | b'd'..=b'i'
            | b'l'
            | b'p'
            | b'r'
            | b's'
            | b'u'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::{Color, Flags, NamedColor, Rgb};

    #[test]
    fn shell_vertical_cursor_moves_separate_journal_lines_across_reads() {
        let source = b"PS> ls\x1b[7;1H    result\x1b[2Btail\x1b[1;1Hsame";
        for bytewise in [false, true] {
            let mut output = TerminalOutput::new();
            output.set_shell_lines(true);
            let mut text = String::new();
            for part in source.chunks(if bytewise { 1 } else { source.len() }) {
                text.push_str(&output.push(part));
            }
            assert_eq!(text, "PS> ls\r\n    result\r\ntailsame");
        }
        let mut raw = TerminalOutput::new();
        assert_eq!(raw.push(source), "PS> ls    resulttailsame");
        raw.set_shell_lines(true);
        assert_eq!(
            raw.push(b"\x1b[4;?Hinvalid\x1b[4;2Hcolumn"),
            "invalidcolumn"
        );
        assert_eq!(raw.push(b"\x1b[?1049hTUI\x1b[4;1Hnext"), "TUInext");
    }

    #[test]
    fn shell_line_separators_keep_unicode_styles_and_prompt_event_offsets() {
        let mut output = TerminalOutput::new();
        output.set_shell_lines(true);
        let chunk = output
            .push_chunk("🙂 $ ls\x1b[6d    result\r\n\x1b]133;A\x07PS> \x1b]133;B\x07".as_bytes());
        let start = "🙂 $ ls\r\n    result\r\n".encode_utf16().count();
        assert_eq!(
            chunk.events,
            [
                TerminalEvent::PromptStart(start),
                TerminalEvent::CommandStart(start + 4)
            ]
        );
        assert_eq!(
            chunk.styles.last().unwrap().end,
            chunk.text.encode_utf16().count() as u32
        );
        assert!(chunk.screen_rewrite);
    }

    fn collected_events(parts: &[&[u8]]) -> Vec<TerminalEvent> {
        let mut output = TerminalOutput::new();
        let mut events = Vec::new();
        let mut base = 0;
        for part in parts {
            let chunk = output.push_chunk(part);
            events.extend(chunk.events.into_iter().map(|event| match event {
                TerminalEvent::PromptStart(at) => TerminalEvent::PromptStart(base + at),
                TerminalEvent::CommandStart(at) => TerminalEvent::CommandStart(base + at),
                TerminalEvent::OwnedPromptStart(at, token) => {
                    TerminalEvent::OwnedPromptStart(base + at, token)
                }
                TerminalEvent::OwnedCommandStart(at, token) => {
                    TerminalEvent::OwnedCommandStart(base + at, token)
                }
                TerminalEvent::Clear(at) => TerminalEvent::Clear(base + at),
                TerminalEvent::ClearLayout(at) => TerminalEvent::ClearLayout(base + at),
            }));
            base += chunk.text.encode_utf16().count();
        }
        events
    }

    #[test]
    fn owned_prompt_markers_keep_exact_identity_and_utf16_offsets_across_splits() {
        let bytes =
            "Ж🙂\x1b]133;A;BKS=0123456789abcdef\x1b\\PS> \x1b]133;B;BKS=0123456789abcdef\x07"
                .as_bytes();
        for split in 0..=bytes.len() {
            assert_eq!(
                collected_events(&[&bytes[..split], &bytes[split..]]),
                [
                    TerminalEvent::OwnedPromptStart(3, 0x0123_4567_89ab_cdef),
                    TerminalEvent::OwnedCommandStart(7, 0x0123_4567_89ab_cdef),
                ],
                "split={split}"
            );
        }
    }

    #[test]
    fn invalid_owned_prompt_markers_are_not_legacy_prompt_events() {
        for marker in [
            "133;A;BKS=",
            "133;A;BKS=0123456789abcde",
            "133;B;BKS=0123456789abcdef0",
            "133;A;BKS=0123456789abcdeg",
            "133;B;BKS=0123456789abcdef;extra",
            "133;A;other=0123456789abcdef",
        ] {
            let mut parser = TerminalOutput::new();
            let chunk = parser.push_chunk(format!("before\x1b]{marker}\x07after").as_bytes());
            assert_eq!(chunk.text, "beforeafter", "{marker}");
            assert!(chunk.events.is_empty(), "{marker}");
        }
    }

    #[test]
    fn startup_clear_is_layout_and_cannot_erase_already_accepted_shell_input() {
        let mut output = TerminalOutput::new();
        let startup = output.push_chunk(b"\x1b[?25l\x1b[2J\x1b[H\r\n\x1b[H");
        assert!(startup.text.is_empty() && startup.events.is_empty());
        output.push(b"ready");
        assert_eq!(
            output.push_chunk(b"\x1b[2J").events,
            [TerminalEvent::Clear(0)]
        );
    }

    #[test]
    fn full_clear_events_have_utf16_offsets_across_every_csi_split() {
        let bytes = "Ж🙂\x1b[2Jmiddle\x1b[3Jlast".as_bytes();
        let expected = [TerminalEvent::Clear(3), TerminalEvent::Clear(9)];
        for split in 0..=bytes.len() {
            assert_eq!(
                collected_events(&[&bytes[..split], &bytes[split..]]),
                expected,
                "split {split}"
            );
        }
    }

    #[test]
    fn partial_private_overflow_and_osc_erase_text_are_not_clear_directives() {
        let mut bytes = b"old\x1b[J\x1b[0J\x1b[1J\x1b[?2J\x1b[2;J\x1b]0;\x1b[3J\x07".to_vec();
        bytes.extend_from_slice(b"\x1b[");
        bytes.extend_from_slice(&[b'2'; 257]);
        bytes.extend_from_slice(b"Jtail");
        assert!(collected_events(&[&bytes]).is_empty());
        bytes.extend_from_slice(b"\x1b[3J");
        assert_eq!(collected_events(&[&bytes]), [TerminalEvent::Clear(7)]);
        for mode in [47, 1047, 1049] {
            let bytes = format!("old\x1b[?{mode}h\x1b[2Jalt\x1b[3J\x1b[?{mode}l\x1b[2Jnew");
            assert_eq!(
                collected_events(&[bytes.as_bytes()]),
                [TerminalEvent::Clear(6)]
            );
        }
    }

    #[test]
    fn clear_layout_requires_bounded_erased_rows_and_home_instead_of_plain_newlines() {
        let bytes = "Ж🙂\x1b[3J\x1b[?25l\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[H\x1b[?25h\x1b]133;A\x07PS> \x1b]133;B\x07".as_bytes();
        let expected = [
            TerminalEvent::Clear(3),
            TerminalEvent::ClearLayout(7),
            TerminalEvent::PromptStart(7),
            TerminalEvent::CommandStart(11),
        ];
        for split in 0..=bytes.len() {
            assert_eq!(
                collected_events(&[&bytes[..split], &bytes[split..]]),
                expected,
                "split {split}"
            );
        }
        for tail in [
            "\r\n\r\ntext",
            "\x1b[K\r\ntext\x1b[H",
            "\x1b[K\r\n\x1b]133;A\x07\x1b[H",
            "\x1b[K\r\n\x1b[2;1H",
        ] {
            let bytes = format!("old\x1b[3J{tail}");
            assert!(!collected_events(&[bytes.as_bytes()])
                .iter()
                .any(|event| matches!(event, TerminalEvent::ClearLayout(_))));
        }
        let bytes = format!("old\x1b[3J{}\x1b[K\x1b[H", "\x1b[K\r\n".repeat(4_097));
        assert_eq!(
            collected_events(&[bytes.as_bytes()]),
            [TerminalEvent::Clear(3)]
        );
    }

    fn append_chunk(text: &mut String, styles: &mut Vec<StyleSpan>, chunk: TerminalChunk) {
        let base = text.encode_utf16().count() as u32;
        for span in chunk.styles {
            StyleSpan::append(styles, base + span.start, base + span.end, span.style);
        }
        text.push_str(&chunk.text);
    }

    #[test]
    fn journal_styles_preserve_utf16_and_sgr_across_every_chunk_boundary() {
        let bytes = concat!(
            "plain\x1b[31mЖ🙂e\u{301}",
            "\x1b[38:2::8:28:255;48;5;28;1;4mX\x1b[0mY"
        );
        let plain = TextStyle::default();
        let red = TextStyle::new(
            Color::Named(NamedColor::Red),
            plain.background,
            Flags::empty(),
        );
        let decorated = TextStyle::new(
            Color::Spec(Rgb {
                r: 8,
                g: 28,
                b: 255,
            }),
            Color::Indexed(28),
            Flags::BOLD | Flags::UNDERLINE,
        );
        let expected = vec![
            StyleSpan {
                start: 0,
                end: 5,
                style: plain,
            },
            StyleSpan {
                start: 5,
                end: 10,
                style: red,
            },
            StyleSpan {
                start: 10,
                end: 11,
                style: decorated,
            },
            StyleSpan {
                start: 11,
                end: 12,
                style: plain,
            },
        ];
        for split in 0..=bytes.len() {
            let mut output = TerminalOutput::new();
            let mut text = String::new();
            let mut styles = Vec::new();
            append_chunk(
                &mut text,
                &mut styles,
                output.push_chunk(&bytes.as_bytes()[..split]),
            );
            append_chunk(
                &mut text,
                &mut styles,
                output.push_chunk(&bytes.as_bytes()[split..]),
            );
            assert_eq!(text, "plainЖ🙂e\u{301}XY", "split {split}");
            assert_eq!(styles, expected, "split {split}");
        }
    }

    #[test]
    fn concealed_journal_ranges_have_default_style_and_color_payloads_do_not_hide() {
        let mut output = TerminalOutput::new();
        let chunk =
            output.push_chunk("\x1b[31mA\x1b[8mS🙂\x1b[38;2;8;28;255mH\x1b[28mB".as_bytes());
        assert_eq!(chunk.text, "A    B");
        assert_eq!((chunk.styles[1].start, chunk.styles[1].end), (1, 5));
        assert_eq!(chunk.styles[1].style, TextStyle::default());
        assert_eq!(
            chunk.styles[2].style.foreground,
            Color::Spec(Rgb {
                r: 8,
                g: 28,
                b: 255
            })
        );
        assert_eq!(chunk.styles[2].start, 5);

        let chunk = output.push_chunk(b"\x1b[48:2::28:8:0mC");
        assert_eq!(chunk.text, "C");
        assert_eq!(
            chunk.styles[0].style.background,
            Color::Spec(Rgb { r: 28, g: 8, b: 0 })
        );
    }

    #[test]
    fn journal_saved_cursor_alternate_screen_and_reset_restore_styles() {
        let mut output = TerminalOutput::new();
        let chunk = output.push_chunk(
            concat!(
                "\x1b[31mA\x1b7\x1b[32mB\x1b8C",
                "\x1b[?1049h\x1b[34mD\x1b[s\x1b[35mE\x1b[uF",
                "\x1b[?1049lG\x1bcH"
            )
            .as_bytes(),
        );
        assert_eq!(chunk.text, "ABCDEFGH");
        let colors: Vec<_> = chunk
            .styles
            .iter()
            .map(|span| span.style.foreground)
            .collect();
        assert_eq!(
            colors,
            vec![
                Color::Named(NamedColor::Red),
                Color::Named(NamedColor::Green),
                Color::Named(NamedColor::Red),
                Color::Named(NamedColor::Blue),
                Color::Named(NamedColor::Magenta),
                Color::Named(NamedColor::Blue),
                Color::Named(NamedColor::Red),
                Color::Named(NamedColor::Foreground),
            ]
        );
    }

    #[test]
    fn color_only_chunks_update_future_styles_and_sync_does_not_buffer_journal() {
        let mut output = TerminalOutput::new();
        assert!(output.push_chunk(b"\x1b[31m").styles.is_empty());
        let first = output.push_chunk(b"\x1b[?2026hA\x1b[32m");
        assert_eq!(first.text, "A");
        assert_eq!(
            first.styles[0].style.foreground,
            Color::Named(NamedColor::Red)
        );
        let second = output.push_chunk(b"B\x1b[?2026l\x1b[0mC");
        assert_eq!(second.text, "BC");
        assert_eq!(
            second.styles[0].style.foreground,
            Color::Named(NamedColor::Green)
        );
        assert_eq!(second.styles[1].style, TextStyle::default());

        let plain = output.push_chunk(b"a\x1b[31m\x1b[0mb");
        assert_eq!(plain.styles.len(), 1);
        assert_eq!((plain.styles[0].start, plain.styles[0].end), (0, 2));
    }

    #[test]
    fn initial_conpty_blank_layout_is_ignored_across_every_chunk_boundary() {
        let bytes = format!("\x1b[?9001h\x1b[?1004h\x1b[?25l\x1b[2J\x1b[m\x1b[H{}\x1b[H\x1b]0;private probe\x07\x1b[?25h  first Ж🙂e\u{301}\r\n\r\n    second\r\n", "\r\n".repeat(39));
        let expected = "  first Ж🙂e\u{301}\r\n\r\n    second\r\n";
        for split in 0..=bytes.len() {
            let mut output = TerminalOutput::new();
            let mut text = output.push(&bytes.as_bytes()[..split]);
            text.push_str(&output.push(&bytes.as_bytes()[split..]));
            assert_eq!(text, expected, "split at byte {split}");
        }
        let mut output = TerminalOutput::new();
        let text: String = bytes
            .as_bytes()
            .iter()
            .map(|byte| output.push(&[*byte]))
            .collect();
        assert_eq!(text, expected);
    }

    #[test]
    fn startup_filter_preserves_indentation_plain_blank_lines_and_later_clears() {
        for text in ["\r\n\r\n  first\r\n\r\nlast", "\t  first\r\n\r\nlast"] {
            assert_eq!(TerminalOutput::new().push(text.as_bytes()), text);
        }
        let mut output = TerminalOutput::new();
        assert_eq!(output.push(b"\x1b[2J\x1b[H\r\n"), "");
        assert_eq!(output.push(b"\t  "), "\t  ");
        assert_eq!(output.push(b"first\r\n\r\n"), "first\r\n\r\n");
        assert_eq!(
            output.push(b"\x1b[2J\x1b[H\r\n\r\n  later"),
            "\r\n\r\n  later"
        );
    }

    #[test]
    fn initial_layout_filter_keeps_prompt_event_utf16_offsets() {
        let mut output = TerminalOutput::new();
        assert!(output.push(b"\x1b[2J\x1b[H\r\n\r").is_empty());
        let chunk = output.push_chunk("\n\x1b[H\x1b]133;A\x07  Ж🙂> \x1b]133;B\x07".as_bytes());
        assert_eq!(chunk.text, "  Ж🙂> ");
        assert_eq!(
            chunk.events,
            [
                TerminalEvent::PromptStart(0),
                TerminalEvent::CommandStart(7)
            ]
        );
    }

    #[test]
    fn owned_powershell_early_resize_has_no_initial_journal_gap() {
        use bks_pty::{PtySession, PtySize};
        use std::time::{Duration, Instant};

        let powershell =
            std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let command = format!("\"{}\" -NoLogo -NoProfile -NonInteractive -Command \"Start-Sleep -Milliseconds 150; [Console]::Write('  BKS_INITIAL_PADDING_MARKER')\"", powershell.display());
        let session =
            PtySession::spawn(&command, PtySize::new(80, 4)).expect("own PowerShell fixture");
        session.resize(PtySize::new(100, 30)).expect("early resize");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut output = TerminalOutput::new();
        let mut text = String::new();
        while Instant::now() < deadline {
            text.push_str(&output.push(&session.drain_output_limited(16_384)));
            if text.contains("BKS_INITIAL_PADDING_MARKER") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            text.starts_with("  BKS_INITIAL_PADDING_MARKER"),
            "initial layout leaked to journal: {text:?}"
        );
        // The finite fixture exits immediately after printing its marker.
        // Give that normal exit time to signal before requesting termination:
        // Windows can reject TerminateProcess during the exit transition.
        let finish_by = Instant::now() + Duration::from_secs(2);
        while session.is_alive() && Instant::now() < finish_by {
            session.drain_output_limited(16_384);
            std::thread::sleep(Duration::from_millis(10));
        }
        if session.is_alive() {
            session.request_terminate().expect("stop own fixture");
        }
    }

    #[test]
    fn sgr_conceal_never_journals_ascii_unicode_or_combining_text() {
        let mut output = TerminalOutput::new();
        let secret = "Secret界🙂e\u{301}";
        let expected = format!(
            "before|{}|after| |reset",
            " ".repeat(secret.encode_utf16().count())
        );
        let chunk = output.push_chunk(
            format!("before|\x1b[8m{secret}\x1b[28m|after|\x1b[8mX\x1b[0m|reset").as_bytes(),
        );
        assert_eq!(chunk.text, expected);
        assert!(!chunk.text.contains(secret));
        assert!(!chunk.screen_rewrite);
    }

    #[test]
    fn conceal_survives_every_csi_and_utf8_chunk_boundary() {
        let bytes =
            "visible|\x1b[8mSecret界🙂e\u{301}\x1b[28m|after\x1b[8mhidden\x1b[m|reset".as_bytes();
        let mut whole = TerminalOutput::new();
        let expected = whole.push(bytes);
        assert_eq!(expected, "visible|           |after      |reset");
        for split in 0..=bytes.len() {
            let mut output = TerminalOutput::new();
            let mut text = output.push(&bytes[..split]);
            text.push_str(&output.push(&bytes[split..]));
            assert_eq!(text, expected, "split at byte {split}");
        }
        let mut output = TerminalOutput::new();
        let text: String = bytes.iter().map(|byte| output.push(&[*byte])).collect();
        assert_eq!(text, expected);
    }

    #[test]
    fn rgb_and_indexed_operands_are_not_conceal_or_reset_attributes() {
        let mut output = TerminalOutput::new();
        assert_eq!(
            output.push(b"\x1b[38;2;0;0;8mRGB\x1b[48;5;8mINDEX\x1b[58;2;28;8;0mUNDERLINE"),
            "RGBINDEXUNDERLINE"
        );
        assert_eq!(
            output.push(b"\x1b[8mX\x1b[38;2;0;8;28mSECRET\x1b[48;5;28mHIDDEN\x1b[28mvisible"),
            "             visible"
        );
    }

    #[test]
    fn colon_color_subparameters_never_change_conceal() {
        let mut output = TerminalOutput::new();
        assert_eq!(
            output.push(b"\x1b[38:2::0:8:28mRGB\x1b[48:5:8mINDEX\x1b[4:0mUNDERLINE"),
            "RGBINDEXUNDERLINE"
        );
        assert_eq!(
            output.push(b"\x1b[8;38:2::0:8:28mSECRET\x1b[28:0mHIDDEN\x1b[0:0mX\x1b[28mvisible"),
            "             visible"
        );
    }

    #[test]
    fn invalid_color_channels_do_not_swallow_following_real_conceal_attributes() {
        let mut output = TerminalOutput::new();
        assert_eq!(
            output.push(b"\x1b[38;2;999;8mSECRET\x1b[38;2;999;28mvisible"),
            "      visible"
        );
        assert_eq!(
            output.push(b"\x1b[38;999;8mHIDDEN\x1b[28mvisible"),
            "      visible"
        );
    }

    #[test]
    fn unknown_or_truncated_sgr_cannot_expose_concealed_text() {
        let mut output = TerminalOutput::new();
        assert_eq!(output.push(b"\x1b[8mA\x1b[999mB\x1b[?28mC"), "   ");
        let excess = format!("\x1b[0;{}mD", "1;".repeat(33));
        assert_eq!(output.push(excess.as_bytes()), " ");
        let excess_restore = format!("\x1b[{}uE", "1;".repeat(33));
        assert_eq!(output.push(excess_restore.as_bytes()), " ");
        let oversized = format!("\x1b[38;2;{};8mSECRET\x1b[28mvisible", "9".repeat(300));
        assert_eq!(output.push(oversized.as_bytes()), "      visible");
    }

    #[test]
    fn concealed_text_is_absent_from_actual_search_copy_and_journal_sources() {
        use crate::search::SearchState;
        use bks_terminal::TerminalModel;

        let secret = "Secret界🙂e\u{301}";
        let bytes = format!("before|\x1b[8m{secret}\x1b[28m|visible");
        let mut terminal = TerminalModel::new(80, 3);
        terminal.push_bytes(bytes.as_bytes());
        let document = terminal.reading_document();
        let mut search = SearchState::default();
        for query in [secret, "Secret", "界", "🙂", "\u{301}"] {
            search.set_query(query.to_owned(), &document);
            assert!(search.matches().is_empty());
        }
        search.set_query("visible".to_owned(), &document);
        assert_eq!(search.matches().len(), 1);
        let copy = String::from_utf16(&document.text.encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(copy, document.text);
        assert!(!copy.contains(secret));
        let mut output = TerminalOutput::new();
        assert_eq!(output.push(bytes.as_bytes()), document.text);
    }

    #[test]
    fn restored_cursor_and_main_screen_restore_known_conceal_attributes() {
        use bks_terminal::TerminalModel;

        let secret = "Secret界🙂e\u{301}";
        for (save, restore) in [
            ("\x1b7", "\x1b8"),
            ("\x1b[s", "\x1b[u"),
            ("\x1b[0:1s", "\x1b[28:0u"),
        ] {
            let bytes = format!("\x1b[8m{save}\x1b[28mvisible{restore}{secret}\x1b[0mreset");
            let mut terminal = TerminalModel::new(80, 3);
            terminal.push_bytes(bytes.as_bytes());
            assert!(!terminal.reading_document().text.contains(secret));
            let mut output = TerminalOutput::new();
            assert_eq!(
                output.push(bytes.as_bytes()),
                format!("visible{}reset", " ".repeat(secret.encode_utf16().count()))
            );
        }
        for (bytes, expected) in [
            (
                format!(
                    "\x1b[8m\x1b[?1049h\x1b[28maltvisible\x1b[?1049l{secret}\x1b[28mmainvisible"
                ),
                format!(
                    "altvisible{}mainvisible",
                    " ".repeat(secret.encode_utf16().count())
                ),
            ),
            (
                format!("mainvisible\x1b[?1049h\x1b[8m{secret}\x1b[?1049lmainvisible"),
                format!(
                    "mainvisible{}mainvisible",
                    " ".repeat(secret.encode_utf16().count())
                ),
            ),
        ] {
            let mut terminal = TerminalModel::new(80, 3);
            terminal.push_bytes(bytes.as_bytes());
            assert!(!terminal.reading_document().text.contains(secret));
            assert!(terminal.reading_document().text.contains("mainvisible"));
            let mut output = TerminalOutput::new();
            let text: String = bytes
                .as_bytes()
                .iter()
                .map(|b| output.push(&[*b]))
                .collect();
            assert_eq!(text, expected);
        }
    }

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

    #[test]
    fn csi_cursor_and_erase_sequences_mark_redraw_but_sgr_and_newlines_do_not() {
        let mut t = TerminalOutput::new();
        let linear = t.push_chunk(b"\x1b[31mred\x1b[0m\r\nnext\n");
        assert_eq!(linear.text, "red\r\nnext\r\n");
        assert!(!linear.screen_rewrite);

        for sequence in [
            "\x1b[H",
            "\x1b[2;3f",
            "\x1b[2A",
            "\x1b[B",
            "\x1b[C",
            "\x1b[D",
            "\x1b[E",
            "\x1b[F",
            "\x1b[10G",
            "\x1b[2J",
            "\x1b[K",
            "\x1b[3P",
            "\x1b[4X",
            "\x1b[S",
            "\x1b[T",
            "\x1b[?1049h",
            "\x1b[?1049l",
            "\x1b7",
            "\x1b8",
            "\x1bM",
            "\x1bc",
        ] {
            let chunk = t.push_chunk(format!("{sequence}redraw\r\n").as_bytes());
            assert!(chunk.screen_rewrite, "{sequence:?}");
            assert_eq!(chunk.text, "redraw\r\n");
        }
        assert!(!t.push_chunk(b"ordinary\r\n").screen_rewrite);
    }

    #[test]
    fn split_csi_metadata_follows_the_final_byte_without_leaking_to_later_chunks() {
        let mut t = TerminalOutput::new();
        assert!(!t.push_chunk(b"before\x1b[").screen_rewrite);
        let repaint = t.push_chunk(b"2Jafter");
        assert!(repaint.screen_rewrite);
        assert_eq!(repaint.text, "after");
        assert!(!t.push_chunk(b"\x1b[38;2;").screen_rewrite);
        let color = t.push_chunk(b"1;2;3mcolored");
        assert!(!color.screen_rewrite);
        assert_eq!(color.text, "colored");
    }

    #[test]
    fn crlf_metadata_is_linear_across_chunks_and_sgr_but_bare_cr_is_a_rewrite() {
        let mut t = TerminalOutput::new();
        assert!(!t.push_chunk(b"first\r").screen_rewrite);
        assert!(!t.push_chunk(b"").screen_rewrite);
        assert!(!t.push_chunk(b"\nsecond\r").screen_rewrite);
        assert!(!t.push_chunk(b"\x1b[32m").screen_rewrite);
        assert!(!t.push_chunk(b"\n").screen_rewrite);

        assert!(!t.push_chunk(b"progress\r").screen_rewrite);
        let progress = t.push_chunk("готово".as_bytes());
        assert!(progress.screen_rewrite);
        assert_eq!(progress.text, "готово");
        assert!(!t.push_chunk(b"\r\nnext\r\n").screen_rewrite);
        assert!(t.push_chunk(b"old\rnew").screen_rewrite);
        assert!(t.push_chunk(b"back\x08space").screen_rewrite);
    }
}
