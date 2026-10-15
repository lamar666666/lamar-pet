pub fn can_clear_draft(ticket: u64, last_ack: u64, command_failed: bool) -> bool {
    ticket == last_ack && !command_failed
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_success_never_discards_an_earlier_failed_or_unconfirmed_draft() {
        assert!(can_clear_draft(8, 8, false));
        assert!(!can_clear_draft(8, 8, true));
        assert!(!can_clear_draft(8, 9, false));
        assert!(!can_clear_draft(8, 7, false));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftTarget {
    Add,
    Edit(u64),
}

pub fn acknowledged_draft(
    target: DraftTarget,
    ticket: u64,
    ack: u64,
    failed: bool,
) -> Option<DraftTarget> {
    can_clear_draft(ticket, ack, failed).then_some(target)
}
pub fn can_begin_edit(editing: Option<u64>, requested: u64) -> bool {
    let _ = requested;
    editing.is_none()
}
#[cfg(test)]
mod inline_edit_tests {
    use super::*;
    #[test]
    fn acknowledgement_clears_only_its_own_draft() {
        assert_eq!(
            acknowledged_draft(DraftTarget::Add, 3, 3, false),
            Some(DraftTarget::Add)
        );
        assert_eq!(
            acknowledged_draft(DraftTarget::Edit(7), 4, 4, false),
            Some(DraftTarget::Edit(7))
        );
        assert_eq!(acknowledged_draft(DraftTarget::Edit(7), 4, 4, true), None);
        assert_eq!(acknowledged_draft(DraftTarget::Edit(7), 4, 5, false), None);
    }
    #[test]
    fn another_edit_cannot_replace_an_unsaved_draft() {
        assert!(can_begin_edit(None, 2));
        assert!(!can_begin_edit(Some(1), 2));
        assert!(!can_begin_edit(Some(1), 1));
    }
}
