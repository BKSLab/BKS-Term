//! Source formatting for a linear journal, using the same VT parser as the grid.
//! This handler ignores printing and IO; concealment remains the journal's job.

use std::time::Duration;

use alacritty_terminal::vte::ansi::{
    Attr, Handler, NamedPrivateMode, PrivateMode, Processor, Timeout,
};

use crate::{Flags, TextStyle};

/// The journal is a linear transcript, not a renderer. CSI 2026 must not hold
/// its style changes in a synchronized-screen buffer after text was emitted.
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
struct StyleState {
    current: TextStyle,
    saved: [TextStyle; 2],
    main: TextStyle,
    alternate: bool,
}

impl Handler for StyleState {
    fn terminal_attribute(&mut self, attr: Attr) {
        let underline = match attr {
            Attr::Underline => Some(Flags::UNDERLINE),
            Attr::DoubleUnderline => Some(Flags::DOUBLE_UNDERLINE),
            Attr::Undercurl => Some(Flags::UNDERCURL),
            Attr::DottedUnderline => Some(Flags::DOTTED_UNDERLINE),
            Attr::DashedUnderline => Some(Flags::DASHED_UNDERLINE),
            _ => None,
        };
        if let Some(underline) = underline {
            self.current.flags.remove(Flags::ALL_UNDERLINES);
            self.current.flags.insert(underline);
            return;
        }
        match attr {
            Attr::Reset => self.current = TextStyle::default(),
            Attr::Foreground(color) => self.current.foreground = color,
            Attr::Background(color) => self.current.background = color,
            Attr::Bold => self.current.flags.insert(Flags::BOLD),
            Attr::Dim => self.current.flags.insert(Flags::DIM),
            Attr::Italic => self.current.flags.insert(Flags::ITALIC),
            Attr::Reverse => self.current.flags.insert(Flags::INVERSE),
            Attr::Strike => self.current.flags.insert(Flags::STRIKEOUT),
            Attr::CancelBold => self.current.flags.remove(Flags::BOLD),
            Attr::CancelBoldDim => self.current.flags.remove(Flags::BOLD | Flags::DIM),
            Attr::CancelItalic => self.current.flags.remove(Flags::ITALIC),
            Attr::CancelUnderline => self.current.flags.remove(Flags::ALL_UNDERLINES),
            Attr::CancelReverse => self.current.flags.remove(Flags::INVERSE),
            Attr::CancelStrike => self.current.flags.remove(Flags::STRIKEOUT),
            // Hidden text is redacted independently by TerminalOutput. Blink,
            // underline color and hyperlinks have no TextStyle representation.
            _ => {}
        }
    }

    fn save_cursor_position(&mut self) {
        self.saved[self.alternate as usize] = self.current;
    }

    fn restore_cursor_position(&mut self) {
        self.current = self.saved[self.alternate as usize];
    }

    fn reset_state(&mut self) {
        *self = Self::default();
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
            && !self.alternate
        {
            self.main = self.current;
            self.saved[0] = self.current;
            self.alternate = true;
        }
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
            && self.alternate
        {
            self.current = self.main;
            self.alternate = false;
        }
    }
}

#[derive(Default)]
pub struct StyleTracker {
    parser: Processor<NoSync>,
    state: StyleState,
    filter: Filter,
}

#[derive(Default)]
enum Filter {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

impl StyleTracker {
    pub fn style(&self) -> TextStyle {
        self.state.current
    }

    /// Consume exactly the byte already handled by the normalizer. Read
    /// `style()` before a byte to style text that byte emits; control bytes
    /// affect subsequent output, including across journal chunk boundaries.
    pub fn push_byte(&mut self, byte: u8) {
        // OSC payload cannot change SGR/cursor attributes. Do not duplicate
        // the VT parser's potentially growing std OSC buffer for a journal.
        match self.filter {
            Filter::Osc => {
                self.filter = match byte {
                    0x07 => Filter::Ground,
                    0x1b => Filter::OscEscape,
                    _ => Filter::Osc,
                };
                return;
            }
            Filter::OscEscape => {
                self.filter = match byte {
                    b'\\' | 0x07 => Filter::Ground,
                    0x1b => Filter::OscEscape,
                    _ => Filter::Osc,
                };
                return;
            }
            Filter::Escape if byte == b']' => {
                // Complete the already forwarded ESC with a harmless ST.
                self.parser.advance(&mut self.state, b"\\");
                self.filter = Filter::Osc;
                return;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, NamedColor, Rgb};

    fn push(tracker: &mut StyleTracker, bytes: &[u8]) {
        for &byte in bytes {
            tracker.push_byte(byte);
        }
    }

    #[test]
    fn sgr_uses_the_grid_parser_for_colon_and_semicolon_colors_and_attributes() {
        let mut tracker = StyleTracker::default();
        push(&mut tracker, b"\x1b[1;2;3;4:3;7;9;38:2::8:28:255;48;5;28m");
        assert_eq!(
            tracker.style(),
            TextStyle::new(
                Color::Spec(Rgb {
                    r: 8,
                    g: 28,
                    b: 255
                }),
                Color::Indexed(28),
                Flags::BOLD
                    | Flags::DIM
                    | Flags::ITALIC
                    | Flags::UNDERCURL
                    | Flags::INVERSE
                    | Flags::STRIKEOUT,
            )
        );
        push(&mut tracker, b"\x1b[22;23;24;27;29;39;49m");
        assert_eq!(tracker.style(), TextStyle::default());
        push(&mut tracker, b"\x1b[91;104m");
        assert_eq!(
            tracker.style().foreground,
            Color::Named(NamedColor::BrightRed)
        );
        assert_eq!(
            tracker.style().background,
            Color::Named(NamedColor::BrightBlue)
        );
    }

    #[test]
    fn save_restore_alternate_and_reset_match_source_cursor_attributes() {
        let mut tracker = StyleTracker::default();
        push(&mut tracker, b"\x1b[31m\x1b7\x1b[32m\x1b8");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Red));
        push(&mut tracker, b"\x1b[?1049h\x1b[34m\x1b[s\x1b[35m\x1b[u");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Blue));
        push(&mut tracker, b"\x1b[?1049l");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Red));
        push(&mut tracker, b"\x1bc\x1b8");
        assert_eq!(tracker.style(), TextStyle::default());
    }

    #[test]
    fn synchronized_screen_updates_do_not_delay_linear_journal_styles() {
        let mut tracker = StyleTracker::default();
        push(&mut tracker, b"\x1b[?2026h\x1b[32m");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Green));
        push(&mut tracker, b"\x1b[?2026l\x1b[0m");
        assert_eq!(tracker.style(), TextStyle::default());
    }

    #[test]
    fn osc_payload_is_discarded_without_changing_or_buffering_styles() {
        let mut tracker = StyleTracker::default();
        push(&mut tracker, b"\x1b[31m\x1b]0;");
        for _ in 0..100_000 {
            tracker.push_byte(b'x');
        }
        push(&mut tracker, b"\x1b[32m\x07");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Red));
        push(&mut tracker, b"\x1b]0;ignored\x1b\\\x1b[34m");
        assert_eq!(tracker.style().foreground, Color::Named(NamedColor::Blue));
    }

    #[test]
    fn source_attributes_match_the_real_grid_for_formats_and_cancellations() {
        for sgr in [
            "0",
            "1",
            "2",
            "3",
            "4",
            "4:2",
            "4:3",
            "4:4",
            "4:5",
            "7",
            "9",
            "22",
            "23",
            "24",
            "27",
            "29",
            "39",
            "49",
            "90;107",
            "38;5;8;48;5;28",
            "38;2;8;28;255;48;2;28;8;0",
            "38:2::8:28:255;48:2::28:8:0",
        ] {
            let input = format!("\x1b[1;2;3;4;7;9;31;44m\x1b[{sgr}mX");
            let mut tracker = StyleTracker::default();
            push(&mut tracker, input.as_bytes());
            let mut grid = crate::TerminalModel::new(12, 2);
            grid.push_bytes(input.as_bytes());
            let snapshot = grid.live_snapshot();
            let cell = &snapshot.cells[0];
            assert_eq!(
                tracker.style(),
                TextStyle::new(cell.fg, cell.bg, cell.flags),
                "SGR {sgr}"
            );
        }
    }
}
