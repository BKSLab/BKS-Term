use std::time::{Duration, Instant};

const QUERY_PAUSE: Duration = Duration::from_millis(350);

#[derive(Default)]
pub struct SearchFeedback {
    pending: Option<Pending>,
}

struct Pending {
    session: u64,
    query: String,
    due: Instant,
}

impl SearchFeedback {
    pub fn schedule(&mut self, now: Instant, session: u64, query: &str) {
        self.pending = Some(Pending {
            session,
            query: query.to_owned(),
            due: now + QUERY_PAUSE,
        });
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// Background document revisions never schedule speech. A pending explicit
    /// edit is consumed once and cannot escape into a different tab or query.
    pub fn take_due(&mut self, now: Instant, session: u64, query: &str, open: bool) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        if !open || pending.session != session || pending.query != query {
            self.cancel();
            return false;
        }
        if now < pending.due {
            return false;
        }
        self.cancel();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_pending_message_and_speaks_only_after_a_pause() {
        let start = Instant::now();
        let mut feedback = SearchFeedback::default();
        assert!(!feedback.take_due(start + Duration::from_secs(1), 1, "", true));
        feedback.schedule(start, 1, "a");
        feedback.schedule(start + Duration::from_millis(200), 1, "ab");
        assert!(!feedback.take_due(start + Duration::from_millis(400), 1, "ab", true));
        assert!(feedback.take_due(start + Duration::from_millis(550), 1, "ab", true));
        assert!(!feedback.take_due(start + Duration::from_secs(4), 1, "ab", true));
    }

    #[test]
    fn tab_close_query_change_and_explicit_navigation_cancel_stale_speech() {
        let start = Instant::now();
        let later = start + Duration::from_secs(1);
        let mut feedback = SearchFeedback::default();
        for (session, query, open) in [(2, "a", true), (1, "b", true), (1, "a", false)] {
            feedback.schedule(start, 1, "a");
            assert!(!feedback.take_due(later, session, query, open));
            assert!(!feedback.take_due(later, 1, "a", true));
        }
        feedback.schedule(start, 1, "a");
        feedback.cancel();
        assert!(!feedback.take_due(later, 1, "a", true));
    }
}
