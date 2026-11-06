/// One accepted key may focus its source-confirmed menu. The first new frame
/// consumes that intent even if it has no menu; unrelated later output cannot
/// reuse it. This state is independent of HWND activation and is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuFocusIntent {
    session: u64,
    revision: u64,
    source: (usize, usize, bool),
    started: std::time::Instant,
}

impl MenuFocusIntent {
    pub fn new(
        session: u64,
        revision: u64,
        source: (usize, usize, bool),
        now: std::time::Instant,
    ) -> Self {
        Self {
            session,
            revision,
            source,
            started: now,
        }
    }

    pub fn source_frame(
        self,
        session: u64,
        revision: u64,
        source: (usize, usize, bool),
        menu: bool,
        now: std::time::Instant,
    ) -> (Option<Self>, bool) {
        if self.session != session
            || self.source != source
            || now.saturating_duration_since(self.started) > std::time::Duration::from_millis(800)
        {
            return (None, false);
        }
        if revision == self.revision {
            return (Some(self), false);
        }
        (None, menu)
    }
}

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

/// Win32 message timestamps and GetTickCount share the same wrapping clock.
/// Equal ticks are conservatively kept on the old side of an input handoff.
pub fn predates_handoff(message_time: u32, handoff_time: u32) -> bool {
    message_time.wrapping_sub(handoff_time) as i32 <= 0
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
    fn menu_focus_intent_expires_and_is_consumed_by_an_acknowledged_frame_without_a_menu() {
        let now = std::time::Instant::now();
        let geometry = (80, 24, false);
        let intent = MenuFocusIntent::new(1, 4, geometry, now);
        assert_eq!(
            intent.source_frame(1, 4, geometry, false, now),
            (Some(intent), false)
        );
        assert_eq!(
            intent.source_frame(1, 5, geometry, false, now),
            (None, false)
        );
        assert_eq!(
            intent.source_frame(
                1,
                5,
                geometry,
                true,
                now + std::time::Duration::from_millis(801)
            ),
            (None, false)
        );
        assert_eq!(intent.source_frame(1, 5, geometry, true, now), (None, true));
    }

    #[test]
    fn changing_session_geometry_or_screen_cancels_menu_focus_intent() {
        let now = std::time::Instant::now();
        let geometry = (80, 24, false);
        let intent = MenuFocusIntent::new(1, 4, geometry, now);
        for (session, source) in [
            (2, geometry),
            (1, (81, 24, false)),
            (1, (80, 25, false)),
            (1, (80, 24, true)),
        ] {
            assert_eq!(
                intent.source_frame(session, 5, source, true, now),
                (None, false)
            );
        }
    }

    #[test]
    fn queued_enter_cannot_cross_the_shared_field_handoff_even_at_tick_wrap() {
        assert!(predates_handoff(99, 100));
        assert!(predates_handoff(100, 100));
        assert!(!predates_handoff(101, 100));
        assert!(predates_handoff(u32::MAX - 2, 3));
        assert!(!predates_handoff(3, u32::MAX - 2));
    }

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
