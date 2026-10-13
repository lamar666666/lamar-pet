//! Focus sessions are owned by the runtime; persisted data contains no clock origin.
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusPhase {
    #[default]
    Idle,
    Focus,
    Break,
    PausedFocus,
    PausedBreak,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusReminder {
    #[default]
    None,
    FocusCompleted,
    BreakCompleted,
    /// Stand-up nudge from the machine-level idle timer, not a focus event.
    Standup,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusDailyTotal {
    pub day: String,
    pub focus_millis: u64,
    pub completed_rounds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusCheckpoint {
    pub focus_minutes: u16,
    pub break_minutes: u16,
    pub phase: FocusPhase,
    pub remaining_millis: u64,
    pub calendar_day: String,
    pub history: Vec<FocusDailyTotal>,
    pub reminder_sequence: u64,
    pub reminder: FocusReminder,
}

impl Default for FocusCheckpoint {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            break_minutes: 5,
            phase: FocusPhase::Idle,
            remaining_millis: 0,
            calendar_day: String::new(),
            history: Vec::new(),
            reminder_sequence: 0,
            reminder: FocusReminder::None,
        }
    }
}

impl FocusCheckpoint {
    /// Validate settings, session bounds and the bounded daily history before
    /// accepting a persisted document. Restore applies the same validation.
    pub fn is_valid(&self) -> bool {
        let maximum = match self.phase {
            FocusPhase::Idle => 0,
            FocusPhase::Focus | FocusPhase::PausedFocus => u64::from(self.focus_minutes) * 60_000,
            FocusPhase::Break | FocusPhase::PausedBreak => u64::from(self.break_minutes) * 60_000,
        };
        valid_settings(self.focus_minutes, self.break_minutes)
            && self.remaining_millis <= maximum
            && (self.phase == FocusPhase::Idle
                || (self.remaining_millis > 0 && !self.calendar_day.is_empty()))
            && (self.calendar_day.is_empty() || valid_day(&self.calendar_day))
            && self.history.len() <= 90
            && self.history.iter().all(|day| {
                valid_day(&day.day)
                    && day.focus_millis <= 86_400_000
                    && day.completed_rounds <= 1440
            })
            && self
                .history
                .iter()
                .enumerate()
                .all(|(i, day)| self.history[..i].iter().all(|other| other.day != day.day))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FocusCommand {
    Start,
    Pause,
    Resume,
    Stop,
    Configure {
        focus_minutes: u16,
        break_minutes: u16,
    },
    Restore(FocusCheckpoint),
    /// Switch the day at receipt, accounting elapsed time to the previous day.
    SetCalendarDay(String),
    /// Split elapsed time precisely at the supplied local midnight boundary.
    SetCalendarBoundary {
        day: String,
        elapsed_since_boundary: Duration,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusSnapshot {
    pub phase: FocusPhase,
    pub remaining_seconds: u64,
    pub focus_minutes: u16,
    pub break_minutes: u16,
    pub today_focus_seconds: u64,
    pub today_completed_rounds: u32,
    pub reminder_sequence: u64,
    pub reminder: FocusReminder,
    pub checkpoint: FocusCheckpoint,
}

impl Default for FocusSnapshot {
    fn default() -> Self {
        FocusState::default().snapshot()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusCommandError;

#[derive(Default)]
pub(crate) struct FocusState {
    checkpoint: FocusCheckpoint,
    last_tick: Option<Duration>,
    recent_focus: Vec<(Duration, Duration)>,
    recent_completion: Option<Duration>,
}

fn valid_settings(focus: u16, rest: u16) -> bool {
    (1..=180).contains(&focus) && (1..=60).contains(&rest)
}

fn valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year = day[..4].parse::<u32>().unwrap_or(0);
    let month = day[5..7].parse::<u32>().unwrap_or(0);
    let date = day[8..].parse::<u32>().unwrap_or(0);
    let max_day = match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0 && date > 0 && date <= max_day
}

impl FocusState {
    pub(crate) fn snapshot(&self) -> FocusSnapshot {
        let c = &self.checkpoint;
        let today = c.history.iter().find(|entry| entry.day == c.calendar_day);
        FocusSnapshot {
            phase: c.phase,
            remaining_seconds: c.remaining_millis.div_ceil(1000),
            focus_minutes: c.focus_minutes,
            break_minutes: c.break_minutes,
            today_focus_seconds: today.map_or(0, |day| day.focus_millis / 1000),
            today_completed_rounds: today.map_or(0, |day| day.completed_rounds),
            reminder_sequence: c.reminder_sequence,
            reminder: c.reminder,
            checkpoint: c.clone(),
        }
    }

    fn day(&mut self) -> &mut FocusDailyTotal {
        let day = self.checkpoint.calendar_day.clone();
        let index = self
            .checkpoint
            .history
            .iter()
            .position(|entry| entry.day == day);
        let index = index.unwrap_or_else(|| {
            self.checkpoint.history.push(FocusDailyTotal {
                day,
                ..Default::default()
            });
            if self.checkpoint.history.len() > 90 {
                self.checkpoint.history.remove(0);
            }
            self.checkpoint.history.len() - 1
        });
        &mut self.checkpoint.history[index]
    }

    pub(crate) fn tick(&mut self, now: Duration) {
        // Quantize the absolute clock, not each delta: discarding each frame's
        // fractional millisecond would make a 60Hz timer lose seconds.
        let now = Duration::from_millis(u64::try_from(now.as_millis()).unwrap_or(u64::MAX));
        let previous = self.last_tick.unwrap_or(now);
        let elapsed = self
            .last_tick
            .map_or(Duration::ZERO, |last| now.saturating_sub(last));
        self.last_tick = Some(now);
        if !matches!(self.checkpoint.phase, FocusPhase::Focus | FocusPhase::Break) {
            return;
        }
        let elapsed = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let used = elapsed.min(self.checkpoint.remaining_millis);
        if self.checkpoint.phase == FocusPhase::Focus {
            let end = previous + Duration::from_millis(used);
            if let Some((_, last_end)) = self.recent_focus.last_mut()
                && *last_end == previous
            {
                *last_end = end;
            } else {
                self.recent_focus.push((previous, end));
            }
            // Calendar updates arrive each second. Keep a bounded ledger even
            // if the application stops publishing local dates.
            if self.recent_focus.len() > 128 {
                self.recent_focus.remove(0);
            }
            let day = self.day();
            day.focus_millis = day.focus_millis.saturating_add(used).min(86_400_000);
        }
        self.checkpoint.remaining_millis -= used;
        if self.checkpoint.remaining_millis == 0 {
            self.checkpoint.reminder_sequence = self.checkpoint.reminder_sequence.saturating_add(1);
            if self.checkpoint.phase == FocusPhase::Focus {
                self.recent_completion = Some(previous + Duration::from_millis(used));
                let day = self.day();
                day.completed_rounds = day.completed_rounds.saturating_add(1).min(1440);
                self.checkpoint.phase = FocusPhase::Break;
                self.checkpoint.remaining_millis =
                    u64::from(self.checkpoint.break_minutes) * 60_000;
                self.checkpoint.reminder = FocusReminder::FocusCompleted;
            } else {
                self.checkpoint.phase = FocusPhase::Idle;
                self.checkpoint.reminder = FocusReminder::BreakCompleted;
            }
        }
    }

    /// Sleep/reset discards time since the last observed active tick.
    pub(crate) fn suspend(&mut self, now: Duration) {
        self.last_tick = Some(Duration::from_millis(
            u64::try_from(now.as_millis()).unwrap_or(u64::MAX),
        ));
        self.checkpoint.phase = match self.checkpoint.phase {
            FocusPhase::Focus => FocusPhase::PausedFocus,
            FocusPhase::Break => FocusPhase::PausedBreak,
            phase => phase,
        };
    }

    pub(crate) fn command(
        &mut self,
        command: FocusCommand,
        now: Duration,
    ) -> Result<(), FocusCommandError> {
        match &command {
            FocusCommand::Start if self.checkpoint.calendar_day.is_empty() => {
                return Err(FocusCommandError);
            }
            FocusCommand::Configure {
                focus_minutes,
                break_minutes,
            } if !valid_settings(*focus_minutes, *break_minutes)
                || self.checkpoint.phase != FocusPhase::Idle =>
            {
                return Err(FocusCommandError);
            }
            FocusCommand::SetCalendarDay(day) | FocusCommand::SetCalendarBoundary { day, .. }
                if !valid_day(day) =>
            {
                return Err(FocusCommandError);
            }
            FocusCommand::Restore(c) if !c.is_valid() => {
                return Err(FocusCommandError);
            }
            _ => {}
        }
        if let FocusCommand::Restore(checkpoint) = command {
            self.checkpoint = checkpoint;
            self.recent_focus.clear();
            self.recent_completion = None;
            self.suspend(now);
            return Ok(());
        }
        if let FocusCommand::SetCalendarBoundary {
            day,
            elapsed_since_boundary,
        } = &command
            && *day != self.checkpoint.calendar_day
        {
            let boundary = now.saturating_sub(*elapsed_since_boundary);
            let reassigned = self
                .recent_focus
                .iter()
                .map(|(start, end)| {
                    u64::try_from(end.saturating_sub((*start).max(boundary)).as_millis())
                        .unwrap_or(u64::MAX)
                })
                .fold(0u64, u64::saturating_add);
            self.tick(boundary.max(self.last_tick.unwrap_or(boundary)));
            let completed = self.recent_completion.is_some_and(|end| end >= boundary);
            let reassigned = if self.checkpoint.calendar_day.is_empty() {
                // Fresh/idle checkpoints have no prior local day. Initialize
                // directly rather than archiving a synthetic empty date.
                0
            } else {
                let old = self.day();
                let reassigned = reassigned.min(old.focus_millis);
                old.focus_millis -= reassigned;
                if completed {
                    old.completed_rounds = old.completed_rounds.saturating_sub(1);
                }
                reassigned
            };
            self.checkpoint.calendar_day = day.clone();
            let new = self.day();
            new.focus_millis = new.focus_millis.saturating_add(reassigned).min(86_400_000);
            if completed {
                new.completed_rounds = new.completed_rounds.saturating_add(1).min(1440);
            }
            self.recent_focus.clear();
            self.recent_completion = None;
        }
        self.tick(now);
        match command {
            FocusCommand::Start if self.checkpoint.phase == FocusPhase::Idle => {
                self.checkpoint.phase = FocusPhase::Focus;
                self.checkpoint.remaining_millis =
                    u64::from(self.checkpoint.focus_minutes) * 60_000;
            }
            FocusCommand::Pause => self.suspend(now),
            FocusCommand::Resume => {
                self.checkpoint.phase = match self.checkpoint.phase {
                    FocusPhase::PausedFocus => FocusPhase::Focus,
                    FocusPhase::PausedBreak => FocusPhase::Break,
                    phase => phase,
                }
            }
            FocusCommand::Stop => {
                self.checkpoint.phase = FocusPhase::Idle;
                self.checkpoint.remaining_millis = 0;
            }
            FocusCommand::Configure {
                focus_minutes,
                break_minutes,
            } => {
                self.checkpoint.focus_minutes = focus_minutes;
                self.checkpoint.break_minutes = break_minutes;
            }
            FocusCommand::SetCalendarDay(day) => {
                self.checkpoint.calendar_day = day;
                self.day();
                self.recent_focus.clear();
                self.recent_completion = None;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "focus/tests.rs"]
mod tests;
