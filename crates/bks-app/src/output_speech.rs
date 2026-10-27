//! Conservative automatic speech for newly appended, stable visible rows.
//!
//! Speech is derived from the live VT screen. The journal chunk confirms a
//! linear append when an entire screen scrolls away between observations.
//! Callers suppress that evidence for cursor/erase/CR rewrites, so redraws
//! cannot replay the journal. This does not interpret arbitrary TUI screens.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const STABLE_FOR: Duration = Duration::from_millis(350);
const EVENT_INTERVAL: Duration = Duration::from_secs(1);
const MAX_AGE: Duration = Duration::from_secs(3);
const DEDUPE_FOR: Duration = Duration::from_secs(5);
const MAX_SCREEN_ROWS: usize = 128;
const MAX_SCREEN_CHARS: usize = 16_384;
const MAX_ROW_CHARS: usize = 1_024;
const MAX_PENDING_ROWS: usize = 12;
const MAX_PENDING_CHARS: usize = 2_048;
const MAX_EVENT_CHARS: usize = 480;
const MAX_RECENT_ROWS: usize = 24;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechMode {
    Manual,
    #[default]
    Important,
    All,
}

#[derive(Default)]
pub struct OutputSpeech {
    screen: Option<Screen>,
    eligible: bool,
    pending: VecDeque<PendingRow>,
    recent: VecDeque<SpokenRow>,
    last_event: Option<Instant>,
}

struct Screen {
    rows: Vec<String>,
    alternate: bool,
}

struct PendingRow {
    text: String,
    stable_since: Instant,
}

struct SpokenRow {
    text: String,
    at: Instant,
}

impl OutputSpeech {
    pub fn new() -> Self {
        Self::default()
    }

    /// `screen` contains live visible VT rows, never user-scrolled history.
    /// `appended` is the normalized journal chunk, never raw VT bytes. Pass an
    /// empty chunk when VT metadata reports cursor/erase/CR screen rewrites.
    /// `eligible` requires the active foreground tab and no protected input.
    /// The first observation, return from ineligibility, and main/alternate
    /// transitions establish a silent baseline instead of replaying content.
    pub fn observe(
        &mut self,
        now: Instant,
        screen: &str,
        appended: &str,
        alternate: bool,
        eligible: bool,
    ) {
        let current = Screen::new(screen, alternate);
        self.expire(now);
        if !eligible {
            self.cancel();
            self.screen = Some(current);
            self.eligible = false;
            return;
        }
        let Some(previous) = self.screen.take() else {
            self.screen = Some(current);
            self.eligible = true;
            return;
        };
        if !self.eligible || previous.alternate != alternate {
            self.cancel();
            self.screen = Some(current);
            self.eligible = true;
            return;
        }

        let has_text = appended
            .chars()
            .any(|character| !character.is_whitespace() && !character.is_control());
        let additions = has_text
            .then(|| appended_rows(&previous.rows, &current.rows))
            .flatten();
        let completed_scroll = (has_text && additions.is_none())
            .then(|| completed_scroll_rows(&current.rows, appended))
            .flatten();
        let extend_pending = additions
            .and_then(|(_, extension)| extension)
            .is_some_and(|(old, _)| self.pending.iter().any(|row| row.text == old));

        // A redraw invalidates rows that vanished. An established append/scroll
        // overlap keeps completed rows even when fast logs scroll them away;
        // otherwise a stream faster than 350 ms per screen would stay silent.
        if previous.rows != current.rows && additions.is_none() && completed_scroll.is_none() {
            self.pending.retain(|row| current.rows.contains(&row.text));
        }
        if let Some((old, _)) = additions.and_then(|(_, extension)| extension) {
            self.pending.retain(|row| row.text != old);
        }

        if let Some((added_at, extension)) = additions {
            if let Some((_, new)) = extension.filter(|_| extend_pending) {
                // A partial line can span PTY chunks. Replace its unspoken
                // candidate, but never announce a spoken row's suffix or turn
                // an old shell prompt into a command echo.
                self.enqueue(now, new);
            }
            for row in &current.rows[added_at..] {
                if !previous.rows.contains(row) {
                    self.enqueue(now, row);
                }
            }
        } else if let Some(completed) = completed_scroll {
            for row in &current.rows[..completed] {
                if !previous.rows.contains(row) {
                    self.enqueue(now, row);
                }
            }
        }
        self.screen = Some(current);
    }

    /// Returns a bounded batch of rows that have individually been stable for
    /// 350 ms. New rows do not postpone earlier rows, so a continuous log still
    /// produces speech. Explicit statuses and requested reading bypass this
    /// policy and are the caller's responsibility.
    pub fn take_due(&mut self, now: Instant, mode: SpeechMode) -> Option<String> {
        self.expire(now);
        if mode != SpeechMode::All || !self.eligible {
            self.cancel();
            return None;
        }
        if self
            .last_event
            .is_some_and(|last| now.saturating_duration_since(last) < EVENT_INTERVAL)
        {
            return None;
        }

        let mut text = String::new();
        let mut characters = 0;
        while let Some(row) = self.pending.front() {
            if now.saturating_duration_since(row.stable_since) < STABLE_FOR {
                break;
            }
            let row_characters = row.text.chars().count();
            let separator = usize::from(!text.is_empty());
            if !text.is_empty() && characters + separator + row_characters > MAX_EVENT_CHARS {
                break;
            }
            let row = self.pending.pop_front().expect("front row exists");
            if separator > 0 {
                text.push('\n');
            }
            let spoken = clipped(&row.text, MAX_EVENT_CHARS - characters - separator);
            characters += separator + spoken.chars().count();
            text.push_str(&spoken);
            self.recent.push_back(SpokenRow {
                text: row.text,
                at: now,
            });
            while self.recent.len() > MAX_RECENT_ROWS {
                self.recent.pop_front();
            }
        }
        if text.is_empty() {
            return None;
        }
        self.last_event = Some(now);
        Some(text)
    }

    /// Silently synchronize after changing tab, mode, foreground, or input
    /// protection. The caller supplies the destination's current live screen.
    /// Preserve the rate limit across these transitions.
    pub fn baseline(&mut self, screen: &str, alternate: bool) {
        self.cancel();
        self.recent.clear();
        self.screen = Some(Screen::new(screen, alternate));
        self.eligible = true;
    }

    /// Cancel queued speech while retaining the current screen baseline.
    pub fn cancel(&mut self) {
        self.pending.clear();
    }

    /// Drop the previous context. The next observation is a silent baseline.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn enqueue(&mut self, now: Instant, text: &str) {
        if text.is_empty()
            || looks_like_animation(text)
            || self.pending.iter().any(|row| row.text == text)
            || self.recent.iter().any(|row| row.text == text)
        {
            return;
        }
        self.pending.push_back(PendingRow {
            text: text.to_owned(),
            stable_since: now,
        });
        while self.pending.len() > MAX_PENDING_ROWS
            || self
                .pending
                .iter()
                .map(|row| row.text.chars().count())
                .sum::<usize>()
                > MAX_PENDING_CHARS
        {
            // Retain one oldest candidate until it can pass the debounce. The
            // remaining slots keep a recent tail; evicting the oldest on every
            // addition would starve speech during a very fast continuous log.
            self.pending.remove(usize::from(self.pending.len() > 1));
        }
    }

    fn expire(&mut self, now: Instant) {
        self.pending
            .retain(|row| now.saturating_duration_since(row.stable_since) <= MAX_AGE);
        self.recent
            .retain(|row| now.saturating_duration_since(row.at) < DEDUPE_FOR);
    }
}

impl Screen {
    fn new(text: &str, alternate: bool) -> Self {
        let mut rows = VecDeque::new();
        let mut remaining = MAX_SCREEN_CHARS;
        for line in text
            .lines()
            .rev()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let row: String = line.chars().take(remaining.min(MAX_ROW_CHARS)).collect();
            remaining -= row.chars().count();
            rows.push_front(row);
            if rows.len() == MAX_SCREEN_ROWS || remaining == 0 {
                break;
            }
        }
        Self {
            rows: rows.into(),
            alternate,
        }
    }
}

/// Find append/scroll overlap, allowing the previous final row to grow. A
/// replacement without overlap is a redraw and establishes a new baseline.
fn appended_rows<'a>(
    previous: &'a [String],
    current: &'a [String],
) -> Option<(usize, Option<(&'a str, &'a str)>)> {
    if previous.is_empty() {
        return Some((0, None));
    }
    for overlap in (1..=previous.len().min(current.len())).rev() {
        let old = &previous[previous.len() - overlap..];
        let new = &current[..overlap];
        if old[..overlap - 1] != new[..overlap - 1] {
            continue;
        }
        if old[overlap - 1] == new[overlap - 1] {
            return Some((overlap, None));
        }
        if new[overlap - 1].starts_with(&old[overlap - 1]) {
            return Some((overlap, Some((&old[overlap - 1], &new[overlap - 1]))));
        }
    }
    None
}

/// A sufficiently large linear append may replace every old visible row. Its
/// entire current screen must equal the journal suffix; a matching last line
/// alone is not evidence. Only newline-completed rows become candidates.
fn completed_scroll_rows(current: &[String], appended: &str) -> Option<usize> {
    if current.is_empty() {
        return None;
    }
    let appended_rows = Screen::new(appended, false).rows;
    if !appended_rows.ends_with(current) {
        return None;
    }
    let partial_last_row = appended
        .rsplit('\n')
        .next()
        .is_some_and(|last| !last.trim().is_empty());
    let completed = current.len().saturating_sub(usize::from(partial_last_row));
    (completed > 0).then_some(completed)
}

fn looks_like_animation(text: &str) -> bool {
    let last = text.split_whitespace().last().unwrap_or_default();
    if matches!(last, "|" | "/" | "-" | "\\")
        || text
            .chars()
            .any(|character| matches!(character, '\u{2801}'..='\u{28ff}' | '◐' | '◓' | '◑' | '◒'))
    {
        return true;
    }
    let mut previous = '\0';
    for character in text.chars() {
        if character == '%' && previous.is_ascii_digit() {
            return true;
        }
        previous = character;
    }
    false
}

fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars()
        .take(limit.saturating_sub(1))
        .chain(std::iter::once('…').take(usize::from(limit > 0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(now: Instant, milliseconds: u64) -> Instant {
        now + Duration::from_millis(milliseconds)
    }

    fn ready() -> (OutputSpeech, Instant) {
        let mut speech = OutputSpeech::new();
        speech.baseline("prompt", false);
        (speech, Instant::now())
    }

    #[test]
    fn defaults_to_important_and_serializes_stable_mode_names() {
        assert_eq!(SpeechMode::default(), SpeechMode::Important);
        for (mode, name) in [
            (SpeechMode::Manual, "manual"),
            (SpeechMode::Important, "important"),
            (SpeechMode::All, "all"),
        ] {
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&mode).unwrap(), json);
            assert_eq!(serde_json::from_str::<SpeechMode>(&json).unwrap(), mode);
        }
        assert!(serde_json::from_str::<SpeechMode>("\"guess\"").is_err());
    }

    #[test]
    fn new_log_waits_for_stability_and_identical_redraw_does_not_postpone_it() {
        let (mut speech, now) = ready();
        speech.observe(
            now,
            "prompt\nfirst\nsecond",
            "first\r\nsecond\r\n",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 349), SpeechMode::All), None);
        speech.observe(
            after(now, 300),
            "prompt\nfirst\nsecond",
            "first\r\nsecond",
            false,
            true,
        );
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("first\nsecond".into())
        );
        speech.observe(
            after(now, 1_400),
            "prompt\nfirst\nsecond",
            "first\r\nsecond",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 1_800), SpeechMode::All), None);
    }

    #[test]
    fn continual_log_emits_without_waiting_for_the_stream_to_stop() {
        let (mut speech, now) = ready();
        let mut screen = "prompt".to_owned();
        for index in 0..8 {
            let line = format!("line {index}");
            screen.push('\n');
            screen.push_str(&line);
            speech.observe(after(now, index * 50), &screen, &line, false, true);
        }
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("line 0".into())
        );
        assert_eq!(speech.take_due(after(now, 1_349), SpeechMode::All), None);
        assert_eq!(
            speech.take_due(after(now, 1_350), SpeechMode::All),
            Some("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7".into())
        );
    }

    #[test]
    fn rapid_scroll_and_queue_overflow_do_not_starve_automatic_speech() {
        let (mut speech, now) = ready();
        let mut rows = VecDeque::from(["prompt".to_owned()]);
        for index in 0..=35 {
            let line = format!("line {index}");
            rows.push_back(line.clone());
            if rows.len() > 10 {
                rows.pop_front();
            }
            speech.observe(
                after(now, index * 10),
                &rows.iter().cloned().collect::<Vec<_>>().join("\n"),
                &line,
                false,
                true,
            );
        }
        assert!(speech.pending.len() <= MAX_PENDING_ROWS);
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("line 0".into())
        );
    }

    #[test]
    fn entirely_new_log_screens_every_fifty_ms_keep_emitting_bounded_batches() {
        let mut speech = OutputSpeech::new();
        let now = Instant::now();
        speech.baseline("old 1\nold 2\nold 3\nold 4", false);
        let mut events = Vec::new();
        for batch in 0..60 {
            let appended = (0..4)
                .map(|row| format!("log {}\r\n", batch * 4 + row))
                .collect::<String>();
            let instant = after(now, batch * 50);
            speech.observe(instant, &appended, &appended, false, true);
            if let Some(text) = speech.take_due(instant, SpeechMode::All) {
                assert!(text.chars().count() <= MAX_EVENT_CHARS);
                events.push((instant, text));
            }
            assert!(speech.pending.len() <= MAX_PENDING_ROWS);
        }
        assert!(events.len() >= 3);
        assert!(events[0].1.starts_with("log 0"));
        for pair in events.windows(2) {
            assert!(pair[1].0.duration_since(pair[0].0) >= EVENT_INTERVAL);
            assert_ne!(pair[1].1, pair[0].1);
        }
    }

    #[test]
    fn whole_screen_scroll_requires_an_exact_journal_suffix_and_completed_rows() {
        let (mut speech, now) = ready();
        speech.observe(
            now,
            "новая 😀\nsecond\nunfinished",
            "scrolled away\r\nновая 😀\r\nsecond\r\nunfinished",
            false,
            true,
        );
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("новая 😀\nsecond".into())
        );

        speech.baseline("previous screen", false);
        speech.observe(
            after(now, 2_000),
            "unrelated header\nonly matching suffix",
            "different header\r\nonly matching suffix\r\n",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 2_350), SpeechMode::All), None);

        speech.baseline("previous screen", false);
        speech.observe(after(now, 3_000), "partial", "partial", false, true);
        assert_eq!(speech.take_due(after(now, 3_350), SpeechMode::All), None);
    }

    #[test]
    fn filtered_redraw_metadata_cannot_enable_whole_screen_scroll_fallback() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\npending", "pending\r\n", false, true);
        // The decoder reports screen_rewrite, so the caller passes no linear
        // journal evidence even when a redraw contains complete text lines.
        speech.observe(
            after(now, 50),
            "repaint first\nrepaint second",
            "",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 400), SpeechMode::All), None);
    }

    #[test]
    fn manual_and_important_discard_raw_output_without_replay() {
        for mode in [SpeechMode::Manual, SpeechMode::Important] {
            let (mut speech, now) = ready();
            speech.observe(
                now,
                "prompt\nerror: failed",
                "error: failed\r\n",
                false,
                true,
            );
            assert_eq!(speech.take_due(after(now, 350), mode), None);
            assert_eq!(speech.take_due(after(now, 2_000), SpeechMode::All), None);
        }
    }

    #[test]
    fn rewrites_cancel_pending_rows_and_progress_never_becomes_speech() {
        let (mut speech, now) = ready();
        for (index, line) in ["Loading 10%", "Loading 50%", "Loading 100%"]
            .iter()
            .enumerate()
        {
            speech.observe(
                after(now, index as u64 * 500),
                &format!("prompt\n{line}"),
                &format!("\r{line}"),
                false,
                true,
            );
            assert_eq!(
                speech.take_due(after(now, index as u64 * 500 + 350), SpeechMode::All),
                None
            );
        }
        speech.observe(after(now, 2_000), "prompt\nresult", "\rresult", false, true);
        assert_eq!(speech.take_due(after(now, 2_400), SpeechMode::All), None);
        speech.observe(
            after(now, 3_000),
            "prompt\nresult\nerror: failed",
            "error: failed\r\n",
            false,
            true,
        );
        assert_eq!(
            speech.take_due(after(now, 3_350), SpeechMode::All),
            Some("error: failed".into())
        );
    }

    #[test]
    fn spinner_frames_and_arbitrary_in_place_replacements_are_silent() {
        let (mut speech, now) = ready();
        for (index, line) in [
            "Working ⠋",
            "Working ⠙",
            "Working |",
            "Working /",
            "Working -",
            "Working \\",
        ]
        .iter()
        .enumerate()
        {
            speech.observe(
                after(now, index as u64 * 1_000),
                &format!("prompt\n{line}"),
                line,
                false,
                true,
            );
            assert_eq!(
                speech.take_due(after(now, index as u64 * 1_000 + 350), SpeechMode::All),
                None
            );
        }
        speech.baseline("old menu\nold choice", false);
        speech.observe(
            after(now, 7_000),
            "new menu\nnew choice",
            "", // Cursor/erase metadata suppresses journal evidence for redraws.
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 7_350), SpeechMode::All), None);
    }

    #[test]
    fn hidden_output_and_return_to_foreground_do_not_replay() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\nqueued", "queued", false, true);
        speech.observe(
            after(now, 100),
            "prompt\nqueued\nbackground",
            "background",
            false,
            false,
        );
        assert_eq!(speech.take_due(after(now, 400), SpeechMode::All), None);
        speech.observe(
            after(now, 500),
            "prompt\nqueued\nbackground\nreturn",
            "return",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 900), SpeechMode::All), None);
        speech.observe(
            after(now, 1_000),
            "background\nreturn\nfresh",
            "fresh",
            false,
            true,
        );
        assert_eq!(
            speech.take_due(after(now, 1_350), SpeechMode::All),
            Some("fresh".into())
        );
    }

    #[test]
    fn baselines_tab_mode_and_alternate_transitions_are_silent() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\nqueued", "queued", false, true);
        speech.baseline("other tab\nold text", false);
        assert_eq!(speech.take_due(after(now, 350), SpeechMode::All), None);
        speech.observe(
            after(now, 500),
            "dialog\nYes\nNo",
            "dialog Yes No",
            true,
            true,
        );
        assert_eq!(speech.take_due(after(now, 850), SpeechMode::All), None);
        speech.observe(
            after(now, 1_000),
            "prompt\nrestored",
            "restored",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 1_350), SpeechMode::All), None);
        speech.reset();
        speech.observe(
            after(now, 2_000),
            "entire old screen",
            "entire old screen",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 2_350), SpeechMode::All), None);
    }

    #[test]
    fn stale_or_explicitly_cancelled_output_is_never_spoken() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\nold", "old", false, true);
        assert_eq!(speech.take_due(after(now, 3_001), SpeechMode::All), None);
        speech.observe(
            after(now, 4_000),
            "prompt\nold\nfresh",
            "fresh",
            false,
            true,
        );
        speech.cancel();
        assert_eq!(speech.take_due(after(now, 4_350), SpeechMode::All), None);
    }

    #[test]
    fn scroll_overlap_emits_new_rows_and_deduplicates_recent_text() {
        let mut speech = OutputSpeech::new();
        let now = Instant::now();
        speech.baseline("first\nsecond\nthird", false);
        speech.observe(now, "second\nthird\nfourth", "fourth\r\n", false, true);
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("fourth".into())
        );
        speech.observe(after(now, 500), "unrelated", "unrelated", false, true);
        speech.observe(
            after(now, 1_000),
            "unrelated\nfourth",
            "fourth",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 1_350), SpeechMode::All), None);
    }

    #[test]
    fn blank_only_chunks_and_screen_only_resize_changes_are_silent() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\n\n", "\r\n\r\n", false, true);
        speech.observe(after(now, 100), "prompt\nwrapped text", "", false, true);
        assert_eq!(speech.take_due(after(now, 500), SpeechMode::All), None);
    }

    #[test]
    fn command_echo_does_not_hide_the_following_output() {
        let (mut speech, now) = ready();
        speech.observe(
            now,
            "prompt command\noutput\nprompt",
            "command\r\noutput\r\nprompt",
            false,
            true,
        );
        assert_eq!(
            speech.take_due(after(now, 350), SpeechMode::All),
            Some("output".into())
        );
    }

    #[test]
    fn split_partial_line_is_replaced_before_speech_and_never_replayed_after_speech() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\nПривет", "Привет", false, true);
        speech.observe(
            after(now, 100),
            "prompt\nПривет 😀 мир",
            " 😀 мир",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 350), SpeechMode::All), None);
        assert_eq!(
            speech.take_due(after(now, 450), SpeechMode::All),
            Some("Привет 😀 мир".into())
        );
        speech.observe(
            after(now, 1_000),
            "prompt\nПривет 😀 мир!",
            "!",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 1_450), SpeechMode::All), None);
    }

    #[test]
    fn rewriting_a_candidate_before_debounce_cancels_the_old_text() {
        let (mut speech, now) = ready();
        speech.observe(now, "prompt\ncandidate", "candidate", false, true);
        speech.observe(
            after(now, 200),
            "prompt\nreplacement",
            "\rreplacement",
            false,
            true,
        );
        assert_eq!(speech.take_due(after(now, 600), SpeechMode::All), None);
    }

    #[test]
    fn output_budget_keeps_a_recent_tail_and_never_splits_unicode() {
        let (mut speech, now) = ready();
        let mut screen = "prompt".to_owned();
        for index in 0..40 {
            screen.push_str(&format!("\nстрока {index}: 😀中e\u{301}"));
        }
        speech.observe(now, &screen, &screen, false, true);
        assert!(speech.pending.len() <= MAX_PENDING_ROWS);
        assert!(speech.pending.front().unwrap().text.contains("строка 0:"));
        assert!(speech.pending.back().unwrap().text.contains("строка 39:"));
        let text = speech.take_due(after(now, 350), SpeechMode::All).unwrap();
        assert!(text.chars().count() <= MAX_EVENT_CHARS);
        assert!(text.contains("😀中e\u{301}"));

        speech.baseline("prompt", false);
        let huge = "😀中e\u{301}".repeat(MAX_ROW_CHARS);
        speech.observe(
            after(now, 2_000),
            &format!("prompt\n{huge}"),
            &huge,
            false,
            true,
        );
        let text = speech.take_due(after(now, 2_350), SpeechMode::All).unwrap();
        assert_eq!(text.chars().count(), MAX_EVENT_CHARS);
        assert!(text.ends_with('…'));
        assert!(!text.contains('\u{fffd}'));
        assert!(
            speech
                .screen
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .map(|row| row.chars().count())
                .sum::<usize>()
                <= MAX_SCREEN_CHARS
        );
    }
}
