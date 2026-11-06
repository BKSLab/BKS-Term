//! Bounded, cancellable speech for visible TUI activity and completed answers.
//! The semantic observer supplies facts; this queue never infers completion.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::output_speech::SpeechMode;

const MAX_NOTICES: usize = 8;
const MAX_PHASE_CHARS: usize = 480;
const MAX_ANSWER_CHARS: usize = 131_072;
const NOTICE_AGE: Duration = Duration::from_secs(8);
const INTERVAL: Duration = Duration::from_millis(900);
const WORKING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct TurnSpeech {
    pending: VecDeque<Notice>,
    last: Option<Instant>,
    phase_text: VecDeque<(String, String)>,
    output_text: Option<String>,
    output_notified: bool,
    working: Option<Working>,
}

struct Working {
    turn: u64,
    started: Instant,
}

struct Notice {
    text: String,
    at: Instant,
    ready: bool,
}

impl TurnSpeech {
    pub fn clear(&mut self) {
        self.pending.clear();
        self.working = None;
    }
    pub fn reset(&mut self) {
        self.clear();
        self.phase_text.clear();
        self.output_text = None;
        self.output_notified = false;
    }
    pub fn baseline_phase(&mut self, label: &str, text: &str) {
        self.clear();
        self.remember_phase(label, text.trim());
    }

    pub fn typing(&mut self) {
        self.working = None;
        self.pending.retain(|notice| notice.ready);
        for notice in &mut self.pending {
            notice.text = "Ответ готов".into();
        }
    }

    /// The caller supplies a currently source-confirmed busy turn. Time only
    /// controls the reminder interval; it cannot establish work or completion.
    pub fn working(&mut self, turn: Option<u64>, now: Instant) {
        match turn {
            Some(turn)
                if self
                    .working
                    .as_ref()
                    .is_some_and(|working| working.turn == turn) => {}
            Some(turn) => self.working = Some(Working { turn, started: now }),
            None => self.working = None,
        }
    }

    pub fn phase(&mut self, label: &str, text: &str, now: Instant) {
        let text = text.trim();
        let delta = self
            .phase_text
            .iter()
            .find(|(old_label, _)| old_label == label)
            .map_or(Some(text), |(_, old)| text_delta(old, text));
        self.remember_phase(label, text);
        let Some(delta) = delta else {
            return;
        };
        let message = if delta.is_empty() || delta.eq_ignore_ascii_case(label) {
            label.to_owned()
        } else {
            format!("{label}: {delta}")
        };
        let message: String = message.chars().take(MAX_PHASE_CHARS).collect();
        if self.pending.back().is_some_and(|old| old.text == message) {
            return;
        }
        self.push(message, now, false);
    }

    fn remember_phase(&mut self, label: &str, text: &str) {
        self.phase_text.retain(|(old_label, _)| old_label != label);
        if self.phase_text.len() == MAX_NOTICES {
            self.phase_text.pop_front();
        }
        self.phase_text
            .push_back((label.into(), text.chars().take(MAX_ANSWER_CHARS).collect()));
    }

    pub fn ready(&mut self, answer: Option<&str>, now: Instant) {
        // A finished result supersedes unspoken intermediate progress.
        self.clear();
        self.output_text = None;
        self.output_notified = false;
        let mut text = "Ответ готов".to_owned();
        if let Some(answer) = answer.filter(|answer| !answer.trim().is_empty()) {
            text.push_str(".\n");
            text.extend(answer.chars().take(MAX_ANSWER_CHARS));
            if answer.chars().count() > MAX_ANSWER_CHARS {
                text.push_str("\nПродолжение доступно в журнале ответов.");
            }
        }
        self.push(text, now, true);
    }

    pub fn baseline_output(&mut self, text: &str) {
        self.clear();
        self.output_text = Some(text.trim().chars().take(MAX_ANSWER_CHARS).collect());
        self.output_notified = false;
    }

    pub fn new_output(&mut self, text: &str, mode: SpeechMode, known_turn: bool, now: Instant) {
        // Unclassified text must not reset the recognized phase. Otherwise a
        // footer redraw makes the same Answer/Thinking label sound new again.
        let text = text.trim();
        if text.is_empty() || self.output_text.as_deref() == Some(text) {
            return;
        }
        let delta = self
            .output_text
            .as_deref()
            .and_then(|old| text.strip_prefix(old))
            .unwrap_or(text)
            .trim();
        let message = if mode == SpeechMode::All {
            if known_turn {
                delta.to_owned()
            } else {
                format!("Новый вывод: {delta}")
            }
        } else if known_turn || self.output_notified || mode == SpeechMode::Manual {
            String::new()
        } else {
            "Новый вывод".into()
        };
        self.output_text = Some(text.chars().take(MAX_ANSWER_CHARS).collect());
        if mode == SpeechMode::Important && !known_turn && !message.is_empty() {
            self.output_notified = true;
        }
        self.push(message.chars().take(MAX_PHASE_CHARS).collect(), now, false);
    }

    pub fn important(&mut self, text: &str, now: Instant) {
        self.clear();
        self.push(text.chars().take(MAX_PHASE_CHARS).collect(), now, false);
    }

    fn push(&mut self, text: String, now: Instant, ready: bool) {
        if text.is_empty() {
            return;
        }
        if self.pending.len() == MAX_NOTICES {
            self.pending.pop_front();
        }
        self.pending.push_back(Notice {
            text,
            at: now,
            ready,
        });
    }

    pub fn take_due(
        &mut self,
        now: Instant,
        mode: SpeechMode,
        foreground: bool,
        paused: bool,
    ) -> Option<String> {
        if mode == SpeechMode::Manual || !foreground {
            self.clear();
            return None;
        }
        self.pending.retain(|notice| {
            now.saturating_duration_since(notice.at)
                <= if notice.ready {
                    Duration::from_secs(60)
                } else {
                    NOTICE_AGE
                }
        });
        if paused {
            self.working = None;
            return None;
        }
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < INTERVAL)
        {
            return None;
        }
        if let Some(notice) = self.pending.pop_front() {
            self.last = Some(now);
            return Some(notice.text);
        }
        let working = self.working.as_ref()?;
        let quiet_since = self
            .last
            .map_or(working.started, |last| last.max(working.started));
        if now.saturating_duration_since(quiet_since) < WORKING_INTERVAL {
            return None;
        }
        // Generate at most one current reminder, without retaining missed
        // intervals through background work, typing or a speech pause.
        self.last = Some(now);
        Some("Работа продолжается".into())
    }
}

/// Native/VT reflow changes whitespace, not the words already spoken. Return
/// the original visible suffix so this comparison never invents output text.
fn text_delta<'a>(old: &str, text: &'a str) -> Option<&'a str> {
    if old == text {
        return None;
    }
    if let Some(suffix) = text.strip_prefix(old) {
        return Some(suffix.trim());
    }
    let before: Vec<_> = old.split_whitespace().collect();
    let after: Vec<_> = text.split_whitespace().collect();
    if before == after {
        return None;
    }
    if !before.is_empty() && after.starts_with(&before) {
        let mut word = 0;
        let mut inside = false;
        for (index, ch) in text.char_indices() {
            if ch.is_whitespace() {
                inside = false;
            } else if !inside {
                if word == before.len() {
                    return Some(&text[index..]);
                }
                word += 1;
                inside = true;
            }
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take_automatic(queue: &mut TurnSpeech, now: Instant) -> Option<String> {
        queue.take_due(now, SpeechMode::Important, true, false)
    }

    #[test]
    fn completion_supersedes_progress_without_inventing_a_result() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Поиск", "Ищу определение функции", now);
        assert_eq!(
            queue
                .take_due(now, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Поиск: Ищу определение функции")
        );
        queue.phase("Выполнение", "Проверяю тесты", now);
        queue.ready(Some("Первый абзац.\nВторой абзац Ж🙂."), now);
        assert!(queue.take_due(now, SpeechMode::All, true, false).is_none());
        assert_eq!(
            queue
                .take_due(now + INTERVAL, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Ответ готов.\nПервый абзац.\nВторой абзац Ж🙂.")
        );
        assert!(queue
            .take_due(now + INTERVAL * 2, SpeechMode::Important, true, false)
            .is_none());
    }

    #[test]
    fn manual_background_and_typing_cancellation_do_not_replay_old_answers() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.ready(None, now);
        assert!(queue
            .take_due(now, SpeechMode::Manual, true, false)
            .is_none());
        queue.ready(Some("old"), now);
        assert!(queue.take_due(now, SpeechMode::All, false, false).is_none());
        assert!(queue
            .take_due(now + INTERVAL, SpeechMode::All, true, false)
            .is_none());
        queue.ready(None, now);
        assert!(queue
            .take_due(now, SpeechMode::Important, true, true)
            .is_none());
        queue.clear();
        assert!(queue
            .take_due(now + INTERVAL, SpeechMode::Important, true, false)
            .is_none());
    }

    #[test]
    fn paused_notices_expire_and_long_unicode_is_bounded_on_characters() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Размышление", &"🙂".repeat(1000), now);
        let text = queue.take_due(now, SpeechMode::All, true, false).unwrap();
        assert_eq!(text.chars().count(), MAX_PHASE_CHARS);
        queue.important("Требуется действие", now);
        assert!(queue
            .take_due(now + NOTICE_AGE + INTERVAL, SpeechMode::All, true, false)
            .is_none());
    }

    #[test]
    fn growing_visible_steps_speak_only_new_text_and_typing_keeps_a_short_ready_notice() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Поиск", "Первый файл", now);
        queue
            .take_due(now, SpeechMode::Important, true, false)
            .unwrap();
        queue.phase("Поиск", "Первый файл\nВторой файл", now + INTERVAL);
        assert_eq!(
            queue
                .take_due(now + INTERVAL, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Поиск: Второй файл")
        );
        queue.ready(
            Some("Длинный ответ, который не должен прерывать набор"),
            now + INTERVAL,
        );
        queue.typing();
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 2, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Ответ готов")
        );
    }

    #[test]
    fn returning_to_a_background_turn_does_not_read_its_accumulated_progress() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Размышление", "first", now);
        queue
            .take_due(now, SpeechMode::Important, true, false)
            .unwrap();
        queue.baseline_phase("Размышление", "first\nbackground");
        queue.phase("Размышление", "first\nbackground\nnew", now + INTERVAL);
        assert_eq!(
            queue
                .take_due(now + INTERVAL, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Размышление: new")
        );
    }

    #[test]
    fn unknown_output_after_a_known_answer_uses_the_current_speech_mode() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.ready(None, now);
        assert_eq!(
            queue.take_due(now, SpeechMode::All, true, false).as_deref(),
            Some("Ответ готов")
        );
        queue.new_output("new tool output", SpeechMode::All, false, now + INTERVAL);
        assert_eq!(
            queue
                .take_due(now + INTERVAL, SpeechMode::All, true, false)
                .as_deref(),
            Some("Новый вывод: new tool output")
        );
        queue.new_output(
            "another tool output",
            SpeechMode::Important,
            false,
            now + INTERVAL * 2,
        );
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 2, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Новый вывод")
        );
    }

    #[test]
    fn source_redraw_does_not_interrupt_a_known_answer_or_repeat_its_phase() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Формируется ответ", "", now);
        assert_eq!(
            queue
                .take_due(now, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Формируется ответ")
        );
        queue.new_output(
            "unclassified redraw",
            SpeechMode::Important,
            true,
            now + INTERVAL,
        );
        queue.phase("Формируется ответ", "", now + INTERVAL);
        assert!(queue
            .take_due(now + INTERVAL, SpeechMode::Important, true, false)
            .is_none());
        queue.ready(None, now + INTERVAL * 2);
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 2, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Ответ готов")
        );
        queue.new_output(
            "new standalone output",
            SpeechMode::Important,
            false,
            now + INTERVAL * 3,
        );
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 3, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Новый вывод")
        );
    }

    #[test]
    fn reading_unclassified_text_in_all_mode_preserves_the_thinking_delta() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Размышление", "first part", now);
        queue.take_due(now, SpeechMode::All, true, false).unwrap();
        queue.new_output(
            "visible unclassified step",
            SpeechMode::All,
            true,
            now + INTERVAL,
        );
        assert_eq!(
            queue
                .take_due(now + INTERVAL, SpeechMode::All, true, false)
                .as_deref(),
            Some("visible unclassified step")
        );
        queue.phase("Размышление", "first part\nnew part", now + INTERVAL * 2);
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 2, SpeechMode::All, true, false)
                .as_deref(),
            Some("Размышление: new part")
        );
        queue.baseline_output("background text");
        queue.phase(
            "Размышление",
            "first part\nnew part\nlatest part",
            now + INTERVAL * 3,
        );
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 3, SpeechMode::All, true, false)
                .as_deref(),
            Some("Размышление: latest part")
        );
    }

    #[test]
    fn retained_phase_reflow_and_inline_steps_do_not_repeat_a_turns_answer_label() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Размышление", "first line second line", now);
        queue
            .take_due(now, SpeechMode::Important, true, false)
            .unwrap();
        queue.phase("Формируется ответ", "", now + INTERVAL);
        queue
            .take_due(now + INTERVAL, SpeechMode::Important, true, false)
            .unwrap();
        queue.phase(
            "Размышление",
            "first line\r\nsecond line",
            now + INTERVAL * 2,
        );
        queue.phase("Формируется ответ", "", now + INTERVAL * 2);
        assert!(queue
            .take_due(now + INTERVAL * 2, SpeechMode::Important, true, false)
            .is_none());
        queue.phase(
            "Размышление",
            "first line\r\nsecond line\nновый Ж🙂 фрагмент",
            now + INTERVAL * 3,
        );
        assert_eq!(
            queue
                .take_due(now + INTERVAL * 3, SpeechMode::Important, true, false)
                .as_deref(),
            Some("Размышление: новый Ж🙂 фрагмент")
        );
    }

    #[test]
    fn working_feedback_requires_current_source_admission() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Выполнение", "visible tool step", now);
        assert_eq!(
            take_automatic(&mut queue, now).as_deref(),
            Some("Выполнение: visible tool step")
        );
        assert!(take_automatic(&mut queue, now + Duration::from_secs(60)).is_none());
        queue.working(Some(0), now + Duration::from_secs(60));
        queue.working(None, now + Duration::from_secs(74));
        assert!(take_automatic(&mut queue, now + Duration::from_secs(120)).is_none());
    }

    #[test]
    fn working_feedback_repeats_after_fifteen_seconds_without_catching_up() {
        let now = Instant::now();
        for mode in [SpeechMode::Important, SpeechMode::All] {
            let mut queue = TurnSpeech::default();
            queue.working(Some(0), now);
            for millis in [1_000, 5_000, 10_000, 14_999] {
                let at = now + Duration::from_millis(millis);
                queue.working(Some(0), at);
                assert!(queue.take_due(at, mode, true, false).is_none());
            }
            for seconds in [15, 30, 120] {
                let at = now + Duration::from_secs(seconds);
                assert_eq!(
                    queue.take_due(at, mode, true, false).as_deref(),
                    Some("Работа продолжается")
                );
                assert!(queue.take_due(at + INTERVAL, mode, true, false).is_none());
                assert!(queue
                    .take_due(
                        at + WORKING_INTERVAL - Duration::from_millis(1),
                        mode,
                        true,
                        false
                    )
                    .is_none());
            }
        }
    }

    #[test]
    fn working_feedback_starts_a_full_interval_after_an_older_spoken_notice() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.phase("Выполнение", "first step", now);
        take_automatic(&mut queue, now).unwrap();
        queue.working(Some(1), now + Duration::from_secs(10));
        assert!(take_automatic(&mut queue, now + Duration::from_secs(15)).is_none());
        assert!(take_automatic(&mut queue, now + Duration::from_millis(24_999)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(25)).as_deref(),
            Some("Работа продолжается")
        );
    }

    #[test]
    fn working_feedback_yields_to_queued_progress_and_times_silence_from_delivery() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(1), now);
        queue.phase("Поиск", "second file", now + Duration::from_secs(14));
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(17)).as_deref(),
            Some("Поиск: second file")
        );
        assert!(take_automatic(&mut queue, now + Duration::from_secs(29)).is_none());
        assert!(take_automatic(&mut queue, now + Duration::from_millis(31_999)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(32)).as_deref(),
            Some("Работа продолжается")
        );
    }

    #[test]
    fn working_feedback_keeps_phase_dedup_and_does_not_count_redraws_as_speech() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(1), now);
        queue.phase("Выполнение", "same command", now);
        take_automatic(&mut queue, now).unwrap();
        queue.phase("Выполнение", "same command", now + Duration::from_secs(14));
        assert!(take_automatic(&mut queue, now + Duration::from_secs(14)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(15)).as_deref(),
            Some("Работа продолжается")
        );
        queue.phase("Выполнение", "same command", now + Duration::from_secs(16));
        assert!(take_automatic(&mut queue, now + Duration::from_secs(16)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(30)).as_deref(),
            Some("Работа продолжается")
        );
    }

    #[test]
    fn working_feedback_waits_after_output_that_was_actually_spoken() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(1), now);
        queue.new_output(
            "new visible line",
            SpeechMode::All,
            true,
            now + Duration::from_secs(14),
        );
        assert_eq!(
            queue
                .take_due(now + Duration::from_secs(14), SpeechMode::All, true, false)
                .as_deref(),
            Some("new visible line")
        );
        assert!(queue
            .take_due(now + Duration::from_secs(28), SpeechMode::All, true, false)
            .is_none());
        assert_eq!(
            queue
                .take_due(now + Duration::from_secs(29), SpeechMode::All, true, false)
                .as_deref(),
            Some("Работа продолжается")
        );
    }

    #[test]
    fn working_feedback_is_not_deferred_by_output_suppressed_in_important_mode() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(1), now);
        queue.new_output(
            "unclassified redraw",
            SpeechMode::Important,
            true,
            now + Duration::from_secs(14),
        );
        assert!(take_automatic(&mut queue, now + Duration::from_secs(14)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(15)).as_deref(),
            Some("Работа продолжается")
        );
    }

    #[test]
    fn working_feedback_stops_for_ready_failure_interruption_or_required_input() {
        let now = Instant::now();
        for message in [
            "Ответ готов",
            "Ошибка: tool failed",
            "Прервано: cancelled",
            "Требуется действие: approve",
        ] {
            let mut queue = TurnSpeech::default();
            queue.working(Some(1), now);
            if message == "Ответ готов" {
                queue.ready(None, now + Duration::from_secs(15));
            } else {
                queue.important(message, now + Duration::from_secs(15));
            }
            assert_eq!(
                take_automatic(&mut queue, now + Duration::from_secs(15)).as_deref(),
                Some(message)
            );
            assert!(take_automatic(&mut queue, now + Duration::from_secs(60)).is_none());
        }
    }

    #[test]
    fn working_feedback_manual_background_and_pause_require_a_fresh_interval() {
        let now = Instant::now();
        for (mode, foreground, paused) in [
            (SpeechMode::Manual, true, false),
            (SpeechMode::Important, false, false),
            (SpeechMode::All, true, true),
        ] {
            let mut queue = TurnSpeech::default();
            queue.working(Some(1), now);
            assert!(queue
                .take_due(now + Duration::from_secs(120), mode, foreground, paused)
                .is_none());
            assert!(take_automatic(&mut queue, now + Duration::from_secs(121)).is_none());
            queue.working(Some(1), now + Duration::from_secs(121));
            assert!(take_automatic(&mut queue, now + Duration::from_millis(135_999)).is_none());
            assert_eq!(
                take_automatic(&mut queue, now + Duration::from_secs(136)).as_deref(),
                Some("Работа продолжается")
            );
        }
    }

    #[test]
    fn working_feedback_typing_cancels_busy_feedback_but_keeps_a_short_ready_notice() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(1), now);
        queue.typing();
        assert!(take_automatic(&mut queue, now + Duration::from_secs(60)).is_none());
        queue.working(Some(1), now + Duration::from_secs(60));
        assert!(take_automatic(&mut queue, now + Duration::from_millis(74_999)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(75)).as_deref(),
            Some("Работа продолжается")
        );
        queue.ready(Some("long automatic answer"), now + Duration::from_secs(76));
        queue.typing();
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(76)).as_deref(),
            Some("Ответ готов")
        );
        assert!(take_automatic(&mut queue, now + Duration::from_secs(120)).is_none());
    }

    #[test]
    fn working_feedback_clear_reset_and_background_baselines_cancel_old_work() {
        let now = Instant::now();
        for cancellation in ["clear", "reset", "phase baseline", "output baseline"] {
            let mut queue = TurnSpeech::default();
            queue.working(Some(1), now);
            match cancellation {
                "clear" => queue.clear(),
                "reset" => queue.reset(),
                "phase baseline" => queue.baseline_phase("Выполнение", "background command"),
                "output baseline" => queue.baseline_output("background output"),
                _ => unreachable!(),
            }
            assert!(
                take_automatic(&mut queue, now + Duration::from_secs(60)).is_none(),
                "{cancellation}"
            );
        }
    }

    #[test]
    fn working_feedback_new_or_readmitted_turn_cannot_reuse_an_old_deadline() {
        let now = Instant::now();
        let mut queue = TurnSpeech::default();
        queue.working(Some(0), now);
        queue.working(Some(1), now + Duration::from_secs(14));
        assert!(take_automatic(&mut queue, now + Duration::from_secs(15)).is_none());
        assert!(take_automatic(&mut queue, now + Duration::from_millis(28_999)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(29)).as_deref(),
            Some("Работа продолжается")
        );
        queue.working(None, now + Duration::from_secs(30));
        queue.working(Some(0), now + Duration::from_secs(60));
        assert!(take_automatic(&mut queue, now + Duration::from_millis(74_999)).is_none());
        assert_eq!(
            take_automatic(&mut queue, now + Duration::from_secs(75)).as_deref(),
            Some("Работа продолжается")
        );
    }
}
