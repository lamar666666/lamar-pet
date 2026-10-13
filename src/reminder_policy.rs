use lamar_pet::focus::{FocusPhase, FocusReminder, FocusSnapshot};

pub(crate) fn reminder_applicable(focus: &FocusSnapshot) -> bool {
    matches!(
        (focus.reminder, focus.phase),
        (
            FocusReminder::FocusCompleted,
            FocusPhase::Break | FocusPhase::PausedBreak
        ) | (FocusReminder::BreakCompleted, FocusPhase::Idle)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_completion_popup_is_not_retried_in_a_new_focus_session() {
        let mut focus = FocusSnapshot {
            phase: FocusPhase::Break,
            reminder: FocusReminder::FocusCompleted,
            reminder_sequence: 1,
            ..Default::default()
        };
        assert!(reminder_applicable(&focus));
        focus.phase = FocusPhase::Focus;
        assert!(!reminder_applicable(&focus));
        focus.phase = FocusPhase::Idle;
        assert!(!reminder_applicable(&focus));
        focus.reminder = FocusReminder::BreakCompleted;
        assert!(reminder_applicable(&focus));
        focus.phase = FocusPhase::PausedFocus;
        assert!(!reminder_applicable(&focus));
    }
}
