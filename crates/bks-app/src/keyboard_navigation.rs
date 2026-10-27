#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAction {
    MoveFocus,
    SendToTerminal,
    SwitchSession { previous: bool },
    System,
}

/// Shift navigation selects readable terminal text locally. Alt combinations
/// and ordinary navigation keep the source application's existing key path.
pub fn starts_text_selection(key: usize, shift: bool, alt: bool) -> bool {
    shift && !alt && matches!(key, 0x21..=0x28)
}

/// Terminal capture is explicit and never overrides shell or system chords.
pub fn tab_action(control: bool, shift: bool, alt: bool, captured: bool) -> TabAction {
    if alt {
        TabAction::System
    } else if control {
        TabAction::SwitchSession { previous: shift }
    } else if captured {
        TabAction::SendToTerminal
    } else {
        TabAction::MoveFocus
    }
}

/// `stops` contains visible, enabled controls in their visual reading order.
pub fn next_tab_stop<T: Copy + Eq>(stops: &[T], current: T, backwards: bool) -> Option<T> {
    if stops.is_empty() {
        return None;
    }
    let index = stops.iter().position(|stop| *stop == current);
    let next = match index {
        Some(index) if backwards => (index + stops.len() - 1) % stops.len(),
        Some(index) => (index + 1) % stops.len(),
        None if backwards => stops.len() - 1,
        None => 0,
    };
    Some(stops[next])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_navigation_selects_text_without_claiming_cli_or_system_keys() {
        // PageUp/PageDown, Home/End and the four arrows; Ctrl can extend by word.
        for key in 0x21..=0x28 {
            assert!(starts_text_selection(key, true, false));
            assert!(!starts_text_selection(key, false, false));
            assert!(!starts_text_selection(key, true, true));
        }
        for key in [0x09, 0x0D, 0x1B, 0x2D, 0x2E, b'A' as usize, b'L' as usize] {
            assert!(!starts_text_selection(key, true, false));
        }
    }

    #[test]
    fn ordinary_tab_is_ui_navigation_until_explicit_capture() {
        for shift in [false, true] {
            assert_eq!(tab_action(false, shift, false, false), TabAction::MoveFocus);
            assert_eq!(
                tab_action(false, shift, false, true),
                TabAction::SendToTerminal
            );
        }
    }

    #[test]
    fn capture_cannot_swallow_system_or_session_switching() {
        for captured in [false, true] {
            for shift in [false, true] {
                for control in [false, true] {
                    assert_eq!(
                        tab_action(control, shift, true, captured),
                        TabAction::System
                    );
                }
                assert_eq!(
                    tab_action(true, shift, false, captured),
                    TabAction::SwitchSession { previous: shift }
                );
            }
        }
    }

    #[test]
    fn tab_cycle_wraps_and_recovers_from_a_removed_or_hidden_control() {
        let stops = [10, 20, 30, 40];
        assert_eq!(next_tab_stop(&stops, 20, false), Some(30));
        assert_eq!(next_tab_stop(&stops, 20, true), Some(10));
        assert_eq!(next_tab_stop(&stops, 40, false), Some(10));
        assert_eq!(next_tab_stop(&stops, 10, true), Some(40));
        assert_eq!(next_tab_stop(&stops, 99, false), Some(10));
        assert_eq!(next_tab_stop(&stops, 99, true), Some(40));
        assert_eq!(next_tab_stop::<u8>(&[], 99, false), None);
        assert_eq!(next_tab_stop(&[10], 10, true), Some(10));
    }
}
