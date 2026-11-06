//! Session keyboard protocols absent from the upstream terminal mode flags.
//! Use VT parser events, never substring matching against application text.

use std::time::Duration;

use alacritty_terminal::vte::ansi::{Handler, PrivateMode, Processor, Timeout};

#[derive(Default)]
struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _: Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct State {
    win32: bool,
}

impl Handler for State {
    fn set_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(9001) {
            self.win32 = true;
        }
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(9001) {
            self.win32 = false;
        }
    }

    fn reset_state(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
enum Filter {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

#[derive(Default)]
pub(crate) struct InputModeTracker {
    parser: Processor<NoSync>,
    state: State,
    filter: Filter,
}

impl InputModeTracker {
    pub(crate) fn win32(&self) -> bool {
        self.state.win32
    }

    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            // OSC text cannot set input modes. Discard its payload instead of
            // allocating an unbounded duplicate of the upstream OSC buffer.
            match self.filter {
                Filter::Osc => {
                    self.filter = match byte {
                        0x07 => Filter::Ground,
                        0x1b => Filter::OscEscape,
                        _ => Filter::Osc,
                    };
                    continue;
                }
                Filter::OscEscape => {
                    self.filter = match byte {
                        b'\\' | 0x07 => Filter::Ground,
                        0x1b => Filter::OscEscape,
                        _ => Filter::Osc,
                    };
                    continue;
                }
                Filter::Escape if byte == b']' => {
                    self.parser.advance(&mut self.state, b"\\");
                    self.filter = Filter::Osc;
                    continue;
                }
                _ => {
                    self.filter = if byte == 0x1b {
                        Filter::Escape
                    } else {
                        Filter::Ground
                    };
                }
            }
            self.parser
                .advance(&mut self.state, std::slice::from_ref(&byte));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::TerminalModel;

    #[test]
    fn win32_mode_tracks_split_private_sequences_and_terminal_reset() {
        let mut model = TerminalModel::new(40, 12);
        assert!(!model.modes().win32_input);
        for &byte in b"\x1b[?2004;9001h" {
            model.push_bytes(&[byte]);
        }
        assert!(model.modes().win32_input);
        assert!(model.modes().bracketed_paste);
        model.resize(80, 24);
        assert!(model.modes().win32_input);
        model.push_bytes(b"\x1b[?9001l");
        assert!(!model.modes().win32_input);
        model.push_bytes(b"\x1b[?9001h\x1bc");
        assert!(!model.modes().win32_input);
    }

    #[test]
    fn string_payload_and_oversized_osc_cannot_enable_win32_input() {
        for (begin, end) in [
            (b"\x1b]0;".as_slice(), b"\x07".as_slice()),
            (b"\x1b]0;".as_slice(), b"\x1b\\".as_slice()),
        ] {
            let mut model = TerminalModel::new(40, 12);
            model.push_bytes(begin);
            model.push_bytes(&vec![b'x'; 65_536]);
            model.push_bytes(b"\x1b[?9001h");
            model.push_bytes(end);
            assert!(!model.modes().win32_input, "string prefix {begin:?}");
            model.push_bytes(b"\x1b[?9001h");
            assert!(model.modes().win32_input);
        }
        let mut model = TerminalModel::new(40, 12);
        model.push_bytes(b"\x1bP1q[?9001h\x1b\\\x1b_[?9001h\x1b\\");
        assert!(!model.modes().win32_input);
        // In VT, ESC [ cancels an unfinished DCS and begins a real CSI;
        // follow the parser's transition rather than treating that as text.
        model.push_bytes(b"\x1bP1qpayload\x1b[?9001h");
        assert!(model.modes().win32_input);
    }

    #[test]
    fn frame_consumption_does_not_apply_input_modes_from_the_unconsumed_tail() {
        let mut model = TerminalModel::new(40, 12);
        let bytes = b"\x1b[?2026h\x1b[?9001htext\x1b[?2026l\x1b[?9001l";
        let (used, complete) = model.push_bytes_until_frame(bytes);
        assert!(complete && used < bytes.len());
        assert!(model.modes().win32_input);
        model.push_bytes(&bytes[used..]);
        assert!(!model.modes().win32_input);
    }
}
