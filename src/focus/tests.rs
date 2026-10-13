use super::*;

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
fn start() -> FocusState {
    let mut state = FocusState::default();
    state
        .command(FocusCommand::SetCalendarDay("2026-10-13".into()), at(0))
        .unwrap();
    state.command(FocusCommand::Start, at(0)).unwrap();
    state
}

#[test]
fn defaults_and_pause_resume_exclude_paused_time() {
    let mut state = start();
    assert_eq!(state.snapshot().remaining_seconds, 1500);
    assert_eq!(state.snapshot().break_minutes, 5);
    state.command(FocusCommand::Pause, at(10)).unwrap();
    state.tick(at(100));
    assert_eq!(state.snapshot().today_focus_seconds, 10);
    state.command(FocusCommand::Resume, at(100)).unwrap();
    state.command(FocusCommand::Stop, at(120)).unwrap();
    assert_eq!(state.snapshot().today_focus_seconds, 30);
    assert_eq!(state.snapshot().today_completed_rounds, 0);
    assert_eq!(state.snapshot().phase, FocusPhase::Idle);
}

#[test]
fn completion_is_clamped_and_reminds_once_then_break_returns_idle() {
    let mut state = start();
    state.tick(at(1600));
    assert_eq!(state.snapshot().phase, FocusPhase::Break);
    assert_eq!(state.snapshot().today_focus_seconds, 1500);
    assert_eq!(state.snapshot().today_completed_rounds, 1);
    assert_eq!(state.snapshot().reminder_sequence, 1);
    state.tick(at(1600));
    assert_eq!(state.snapshot().reminder_sequence, 1);
    state.tick(at(1900));
    assert_eq!(state.snapshot().phase, FocusPhase::Idle);
    assert_eq!(state.snapshot().reminder, FocusReminder::BreakCompleted);
    assert_eq!(state.snapshot().reminder_sequence, 2);
    assert_eq!(state.snapshot().today_focus_seconds, 1500);
}

#[test]
fn suspend_and_restart_never_count_downtime() {
    let mut state = start();
    state.tick(at(10));
    state.suspend(at(5000));
    assert_eq!(state.snapshot().phase, FocusPhase::PausedFocus);
    assert_eq!(state.snapshot().today_focus_seconds, 10);
    let checkpoint = state.snapshot().checkpoint;
    let mut restored = FocusState::default();
    restored
        .command(FocusCommand::Restore(checkpoint), at(9000))
        .unwrap();
    restored.tick(at(9999));
    assert_eq!(restored.snapshot().today_focus_seconds, 10);
    assert_eq!(restored.snapshot().phase, FocusPhase::PausedFocus);
}

#[test]
fn explicit_calendar_boundary_allocates_both_days() {
    let mut state = start();
    state.tick(at(10));
    state
        .command(
            FocusCommand::SetCalendarBoundary {
                day: "2026-10-14".into(),
                elapsed_since_boundary: at(1),
            },
            at(12),
        )
        .unwrap();
    assert_eq!(state.snapshot().today_focus_seconds, 1);
    assert_eq!(state.snapshot().checkpoint.history[0].focus_millis, 11000);
}

#[test]
fn calendar_boundary_reassigns_ticks_already_observed_after_midnight() {
    let mut state = start();
    state.tick(at(12));
    state
        .command(
            FocusCommand::SetCalendarBoundary {
                day: "2026-10-14".into(),
                elapsed_since_boundary: at(1),
            },
            at(12),
        )
        .unwrap();
    assert_eq!(state.snapshot().today_focus_seconds, 1);
    assert_eq!(state.snapshot().checkpoint.history[0].focus_millis, 11000);
}

#[test]
fn invalid_configuration_and_restore_are_atomic() {
    let mut state = FocusState::default();
    let before = state.snapshot();
    for minutes in [0, 181, u16::MAX] {
        assert!(
            state
                .command(
                    FocusCommand::Configure {
                        focus_minutes: minutes,
                        break_minutes: 5
                    },
                    at(0)
                )
                .is_err()
        );
        assert_eq!(state.snapshot(), before);
    }
    let mut checkpoint = before.checkpoint.clone();
    checkpoint.remaining_millis = u64::MAX;
    assert!(
        state
            .command(FocusCommand::Restore(checkpoint), at(0))
            .is_err()
    );
    assert_eq!(state.snapshot(), before);
}

#[test]
fn history_is_bounded_and_checkpoint_round_trips() {
    let mut state = FocusState::default();
    for index in 0..100 {
        let day = format!("2026-{:02}-{:02}", index / 28 + 1, index % 28 + 1);
        state
            .command(FocusCommand::SetCalendarDay(day), at(index))
            .unwrap();
    }
    assert!(state.snapshot().checkpoint.history.len() <= 90);
    let json = serde_json::to_string(&state.snapshot().checkpoint).unwrap();
    let checkpoint: FocusCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(checkpoint, state.snapshot().checkpoint);
}

#[test]
fn running_focus_and_break_restore_as_paused() {
    let mut state = start();
    state.tick(at(10));
    let mut restored = FocusState::default();
    restored
        .command(FocusCommand::Restore(state.snapshot().checkpoint), at(2000))
        .unwrap();
    assert_eq!(restored.snapshot().phase, FocusPhase::PausedFocus);
    state.tick(at(1500));
    restored
        .command(FocusCommand::Restore(state.snapshot().checkpoint), at(3000))
        .unwrap();
    assert_eq!(restored.snapshot().phase, FocusPhase::PausedBreak);
    assert_eq!(restored.snapshot().remaining_seconds, 300);
}

#[test]
fn completion_at_midnight_belongs_to_new_day() {
    let mut state = start();
    state.tick(at(1499));
    state
        .command(
            FocusCommand::SetCalendarBoundary {
                day: "2026-10-14".into(),
                elapsed_since_boundary: at(1),
            },
            at(1501),
        )
        .unwrap();
    assert_eq!(state.snapshot().today_completed_rounds, 1);
    assert_eq!(state.snapshot().checkpoint.history[0].completed_rounds, 0);
}

#[test]
fn frequent_fractional_ticks_do_not_lose_focus_time() {
    let mut state = start();
    for tick in 1..=300 {
        state.tick(Duration::from_nanos(tick * 16_666_667));
    }
    assert_eq!(state.snapshot().today_focus_seconds, 5);
    assert_eq!(state.snapshot().remaining_seconds, 1495);
}

#[test]
fn start_requires_calendar_day_so_checkpoints_always_validate() {
    let mut state = FocusState::default();
    assert!(state.command(FocusCommand::Start, at(0)).is_err());
    assert!(state.snapshot().checkpoint.is_valid());
}

#[test]
fn active_or_paused_checkpoints_require_calendar_day() {
    for phase in [
        FocusPhase::Focus,
        FocusPhase::Break,
        FocusPhase::PausedFocus,
        FocusPhase::PausedBreak,
    ] {
        let checkpoint = FocusCheckpoint {
            phase,
            remaining_millis: 60_000,
            ..Default::default()
        };
        assert!(!checkpoint.is_valid(), "{phase:?} requires a calendar day");
        let mut state = start();
        let before = state.snapshot();
        assert!(
            state
                .command(FocusCommand::Restore(checkpoint), at(10))
                .is_err()
        );
        assert_eq!(state.snapshot(), before, "invalid restore is atomic");
    }
    assert!(FocusCheckpoint::default().is_valid());
}

#[test]
fn initial_calendar_boundary_never_inserts_empty_daily_record() {
    for restore in [false, true] {
        let mut state = FocusState::default();
        if restore {
            state
                .command(
                    FocusCommand::Restore(FocusCheckpoint {
                        focus_minutes: 40,
                        ..Default::default()
                    }),
                    at(0),
                )
                .unwrap();
        }
        state
            .command(
                FocusCommand::SetCalendarBoundary {
                    day: "2026-10-13".into(),
                    elapsed_since_boundary: at(3600),
                },
                at(1),
            )
            .unwrap();
        let checkpoint = state.snapshot().checkpoint;
        assert!(checkpoint.is_valid());
        assert_eq!(checkpoint.history.len(), 1);
        assert_eq!(checkpoint.history[0].day, "2026-10-13");
        if restore {
            assert_eq!(checkpoint.focus_minutes, 40);
        }
    }
}
