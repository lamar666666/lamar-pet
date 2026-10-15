use std::time::{Duration, Instant};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Complete,
    Undo,
    Focus,
}
#[derive(Clone, Copy)]
pub struct Feedback {
    pub generation: u64,
    pub task_id: u64,
    pub kind: Kind,
    started: Instant,
}
#[derive(Default)]
pub struct FeedbackTracker {
    pending: Option<(u64, u64, Kind)>,
    active: Option<Feedback>,
    generation: u64,
}
impl FeedbackTracker {
    pub fn begin(&mut self, ticket: u64, task_id: u64, kind: Kind) {
        self.pending = Some((ticket, task_id, kind));
        self.active = None;
    }
    pub fn observe(&mut self, ack: u64, success: bool, now: Instant) -> Option<Feedback> {
        let (ticket, task_id, kind) = self.pending?;
        if ack < ticket {
            return None;
        }
        self.pending = None;
        if ack != ticket || !success {
            self.active = None;
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        let feedback = Feedback {
            generation: self.generation,
            task_id,
            kind,
            started: now,
        };
        self.active = Some(feedback);
        Some(feedback)
    }
    pub fn active(&self, now: Instant) -> Option<Feedback> {
        self.active
            .filter(|f| now.saturating_duration_since(f.started) < Duration::from_millis(500))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn acknowledged_click_animates_once_and_periodic_snapshots_do_not_restart_it() {
        let now = Instant::now();
        let mut tracker = FeedbackTracker::default();
        tracker.begin(12, 3, Kind::Focus);
        assert!(tracker.observe(11, true, now).is_none());
        let shown = tracker.observe(12, true, now).unwrap();
        assert_eq!(shown.task_id, 3);
        assert!(
            tracker
                .observe(12, true, now + Duration::from_millis(100))
                .is_none()
        );
        assert!(tracker.active(now + Duration::from_millis(600)).is_none());
        tracker.begin(13, 4, Kind::Complete);
        assert!(tracker.observe(13, true, now).unwrap().generation > shown.generation);
    }
    #[test]
    fn failed_and_unrelated_acknowledgements_never_show_success() {
        let now = Instant::now();
        let mut tracker = FeedbackTracker::default();
        tracker.begin(4, 1, Kind::Complete);
        assert!(tracker.observe(4, false, now).is_none());
        tracker.begin(5, 1, Kind::Undo);
        assert!(tracker.observe(6, true, now).is_none());
        assert!(tracker.active(now).is_none());
    }
}
