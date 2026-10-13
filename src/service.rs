use crate::focus::{FocusCommand, FocusSnapshot, FocusState};
use crate::storage::{Document, Storage};
use crate::windows_input::Sampler;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Light,
    Dark,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub scale_percent: u16,
    pub appearance: Appearance,
    pub name: String,
    /// Stand-up reminder interval in minutes; 0 disables it.
    pub standup_minutes: u16,
    /// Start the next focus round automatically when a break ends.
    pub auto_continue: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            scale_percent: 100,
            appearance: Appearance::Light,
            name: "Lamar Pet".into(),
            standup_minutes: 45,
            auto_continue: false,
        }
    }
}

impl Preferences {
    pub fn is_valid(&self) -> bool {
        (50..=200).contains(&self.scale_percent)
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 24
            && self.standup_minutes <= 180
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Focus(FocusCommand),
    SetScale(u16),
    SetAppearance(Appearance),
    SetName(String),
    SetStandup(u16),
    SetAutoContinue(bool),
    Shutdown,
}

#[derive(Clone, Debug)]
pub struct AppSnapshot {
    pub focus: FocusSnapshot,
    pub preferences: Preferences,
    pub storage_error: bool,
    pub load_pending: bool,
    pub command_error: Option<String>,
    pub activity_pulse: u64,
    pub mouse: Option<(i32, i32)>,
    /// Bumped each time the stand-up reminder fires; session-scoped.
    pub standup_sequence: u64,
    pub revision: u64,
    pub last_ack: u64,
}

struct Machine {
    focus: FocusState,
    preferences: Preferences,
    storage_error: bool,
    load_pending: bool,
    command_error: Option<String>,
    last_observed: Option<Duration>,
    last_step: Option<Duration>,
    last_phase: Option<crate::focus::FocusPhase>,
    last_reminder_sequence: u64,
    standup_elapsed: Duration,
    standup_sequence: u64,
    desktop_available: bool,
    activity_pulse: u64,
    mouse: Option<(i32, i32)>,
    revision: u64,
    last_ack: u64,
}
impl Default for Machine {
    fn default() -> Self {
        Self {
            focus: FocusState::default(),
            preferences: Preferences::default(),
            storage_error: false,
            load_pending: true,
            command_error: None,
            last_observed: None,
            last_step: None,
            last_phase: None,
            last_reminder_sequence: 0,
            standup_elapsed: Duration::ZERO,
            standup_sequence: 0,
            desktop_available: false,
            activity_pulse: 0,
            mouse: None,
            revision: 0,
            last_ack: 0,
        }
    }
}
impl Machine {
    fn restore(&mut self, document: Document, now: Duration) -> Result<(), String> {
        if !document.is_valid() {
            return Err("Persisted focus data is invalid".into());
        }
        self.focus
            .command(FocusCommand::Restore(document.checkpoint), now)
            .map_err(|_| "Unable to restore focus data")?;
        self.preferences = document.preferences;
        self.load_pending = false;
        self.storage_error = false;
        self.last_observed = Some(now);
        Ok(())
    }
    fn step(&mut self, now: Duration, calendar: Option<(String, Duration)>, desktop: bool) {
        self.desktop_available = desktop;
        let step_delta = self
            .last_step
            .map_or(Duration::ZERO, |last| now.saturating_sub(last));
        self.last_step = Some(now);
        if self.load_pending {
            return;
        }
        if !desktop
            || self
                .last_observed
                .is_some_and(|last| now.saturating_sub(last) > Duration::from_secs(5))
            || calendar.is_none()
        {
            self.focus.suspend(now);
        }
        self.last_observed = Some(now);
        if let Some((day, elapsed_since_boundary)) = calendar {
            let _ = self.focus.command(
                FocusCommand::SetCalendarBoundary {
                    day,
                    elapsed_since_boundary,
                },
                now,
            );
        }
        self.focus.tick(now);
        // Sitting time only accrues while the desktop is usable; locking or
        // sleeping the machine counts as a break already.
        if desktop && self.preferences.standup_minutes > 0 {
            // Cap the delta so waking from sleep does not dump a whole
            // suspended interval into the sitting timer at once.
            self.standup_elapsed += step_delta.min(Duration::from_secs(10));
            if self.standup_elapsed
                >= Duration::from_secs(u64::from(self.preferences.standup_minutes) * 60)
            {
                self.standup_elapsed = Duration::ZERO;
                self.standup_sequence = self.standup_sequence.saturating_add(1);
            }
        } else if !desktop {
            // Do not penalise time spent away; resume the interval instead
            // of firing immediately after a long lock.
            self.standup_elapsed = self
                .standup_elapsed
                .min(Duration::from_secs(30));
        }
        let phase = self.focus.snapshot().phase;
        // Auto-continue only on a naturally completed break — an explicit
        // stop (Break -> Idle without a new reminder) is a real exit.
        if self.preferences.auto_continue
            && self.last_phase == Some(crate::focus::FocusPhase::Break)
            && phase == crate::focus::FocusPhase::Idle
            && self.focus.snapshot().reminder == crate::focus::FocusReminder::BreakCompleted
            && self.focus.snapshot().reminder_sequence != self.last_reminder_sequence
        {
            let _ = self.focus.command(FocusCommand::Start, now);
        }
        self.last_phase = Some(phase);
        self.last_reminder_sequence = self.focus.snapshot().reminder_sequence;
    }
    fn command(&mut self, command: Command, now: Duration) -> Result<(), String> {
        if self.load_pending {
            return Err("Focus data is unavailable; retrying load".into());
        }
        match command {
            Command::Focus(FocusCommand::Start | FocusCommand::Resume)
                if !self.desktop_available =>
            {
                Err("Focus is paused while the desktop is unavailable".into())
            }
            Command::Focus(command) => self
                .focus
                .command(command, now)
                .map_err(|_| "Invalid focus command or duration".to_string()),
            Command::SetScale(scale) if (50..=200).contains(&scale) => {
                self.preferences.scale_percent = scale;
                Ok(())
            }
            Command::SetScale(_) => Err("Pet scale must be between 50 and 200 percent".into()),
            Command::SetAppearance(appearance) => {
                self.preferences.appearance = appearance;
                Ok(())
            }
            Command::SetName(name) => {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > 24 {
                    return Err("Pet name must be 1-24 characters".into());
                }
                self.preferences.name = name.into();
                Ok(())
            }
            Command::SetStandup(minutes) if minutes <= 180 => {
                self.preferences.standup_minutes = minutes;
                self.standup_elapsed = Duration::ZERO;
                Ok(())
            }
            Command::SetStandup(_) => Err("Stand-up interval must be at most 180 minutes".into()),
            Command::SetAutoContinue(enabled) => {
                self.preferences.auto_continue = enabled;
                Ok(())
            }
            Command::Shutdown => Ok(()),
        }
    }
    fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            focus: self.focus.snapshot(),
            preferences: self.preferences.clone(),
            storage_error: self.storage_error,
            load_pending: self.load_pending,
            command_error: self.command_error.clone(),
            activity_pulse: self.activity_pulse,
            mouse: self.mouse,
            standup_sequence: self.standup_sequence,
            revision: self.revision,
            last_ack: self.last_ack,
        }
    }
    fn document(&self) -> Document {
        Document {
            schema_version: 1,
            preferences: self.preferences.clone(),
            checkpoint: self.focus.snapshot().checkpoint,
        }
    }
}

#[derive(Clone)]
pub struct Client {
    sender: Arc<Mutex<Producer>>,
    snapshot: Arc<Mutex<AppSnapshot>>,
    stopping: Arc<AtomicBool>,
}

struct Envelope {
    sequence: u64,
    command: Command,
}

struct Producer {
    sender: SyncSender<Envelope>,
    next_sequence: u64,
}

impl Client {
    pub fn send(&self, command: Command) -> Result<u64, String> {
        // Sequence allocation and enqueue must share this short lock. An
        // atomic counter alone would allow cloned senders to enqueue out of order.
        let mut producer = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.stopping.load(Ordering::Acquire) {
            return Err("Pet service is stopping".into());
        }
        let sequence = producer.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or("Pet command sequence exhausted")?;
        producer
            .sender
            .try_send(Envelope { sequence, command })
            .map_err(|error| -> String {
                match error {
                    TrySendError::Full(_) => "Pet command queue is full".into(),
                    TrySendError::Disconnected(_) => "Pet service has stopped".into(),
                }
            })?;
        producer.next_sequence = next_sequence;
        Ok(sequence)
    }
    pub fn snapshot(&self) -> AppSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

pub struct Worker {
    client: Client,
    finished: Receiver<Result<(), String>>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(root: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(64);
        let (done_sender, finished) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Machine::default().snapshot()));
        let client = Client {
            sender: Arc::new(Mutex::new(Producer {
                sender,
                next_sequence: 1,
            })),
            snapshot: snapshot.clone(),
            stopping: stopping.clone(),
        };
        let handle = thread::Builder::new()
            .name("lamar-focus".into())
            .spawn(move || {
                let result = run(Storage::new(root), receiver, snapshot, stopping);
                let _ = done_sender.send(result);
            })
            .map_err(|_| "Unable to start pet worker")?;
        Ok(Self {
            client,
            finished,
            handle: Some(handle),
        })
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
    pub fn shutdown(mut self) -> Result<(), String> {
        self.client.stopping.store(true, Ordering::Release);
        let result = self
            .finished
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Pet worker did not stop within two seconds".to_string())?;
        if let Some(handle) = self.handle.take() {
            handle.join().map_err(|_| "Pet worker panicked")?;
        }
        result
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.client.stopping.store(true, Ordering::Release);
    }
}

fn local_calendar() -> Option<(String, Duration)> {
    let now = time::OffsetDateTime::now_local().ok()?;
    let day = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    );
    let seconds =
        u64::from(now.hour()) * 3600 + u64::from(now.minute()) * 60 + u64::from(now.second());
    Some((day, Duration::new(seconds, now.nanosecond())))
}

fn publish(machine: &mut Machine, snapshot: &Mutex<AppSnapshot>) {
    machine.revision = machine.revision.saturating_add(1);
    *snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = machine.snapshot();
}

fn run(
    storage: Storage,
    receiver: Receiver<Envelope>,
    snapshot: Arc<Mutex<AppSnapshot>>,
    stopping: Arc<AtomicBool>,
) -> Result<(), String> {
    let origin = Instant::now();
    let mut machine = Machine::default();
    let mut sampler = Sampler::default();
    let mut last_load_attempt = None;
    let mut last_save = Duration::ZERO;
    let mut last_save_attempt = None;
    let mut dirty = false;
    loop {
        let received = receiver.recv_timeout(Duration::from_millis(50));
        let now = origin.elapsed();
        if machine.load_pending
            && last_load_attempt
                .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1))
        {
            last_load_attempt = Some(now);
            match storage
                .load()
                .and_then(|document| machine.restore(document, now))
            {
                Ok(()) => dirty = true,
                Err(_) => machine.storage_error = true,
            }
        }
        let sample = sampler.sample(now);
        machine.activity_pulse = machine
            .activity_pulse
            .saturating_add(sample.activity_pulses);
        machine.mouse = sample.mouse;
        let previous_phase = machine.focus.snapshot().phase;
        machine.step(now, local_calendar(), sample.desktop_available);
        let mut urgent_save = previous_phase != machine.focus.snapshot().phase;
        // Stop accepting new work, then drain the bounded queue before the
        // final checkpoint so successfully queued preferences are not lost.
        if (stopping.load(Ordering::Acquire) && matches!(received, Err(RecvTimeoutError::Timeout)))
            || matches!(
                received,
                Err(RecvTimeoutError::Disconnected)
                    | Ok(Envelope {
                        command: Command::Shutdown,
                        ..
                    })
            )
        {
            stopping.store(true, Ordering::Release);
            if let Ok(envelope) = &received {
                machine.last_ack = envelope.sequence;
                machine.command_error = None;
            }
            machine.focus.suspend(now);
            if machine.load_pending {
                machine.storage_error = true;
                publish(&mut machine, &snapshot);
                return Err("Focus data unavailable; original file preserved".into());
            }
            let result = storage.save(&machine.document());
            machine.storage_error = result.is_err();
            publish(&mut machine, &snapshot);
            return result;
        }
        if let Ok(envelope) = received {
            let result = machine.command(envelope.command, now);
            machine.last_ack = envelope.sequence;
            urgent_save |= result.is_ok();
            machine.command_error = result.err();
        }
        dirty |= urgent_save
            || matches!(
                machine.focus.snapshot().phase,
                crate::focus::FocusPhase::Focus | crate::focus::FocusPhase::Break
            );
        let periodic_due = dirty && now.saturating_sub(last_save) >= Duration::from_secs(15);
        let retry_due = machine.storage_error
            && last_save_attempt
                .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1));
        let save_due = if machine.storage_error {
            retry_due
        } else {
            urgent_save || periodic_due
        };
        if !machine.load_pending && save_due {
            last_save_attempt = Some(now);
            match storage.save(&machine.document()) {
                Ok(()) => {
                    machine.storage_error = false;
                    dirty = false;
                    last_save = now;
                }
                Err(_) => machine.storage_error = true,
            }
        }
        publish(&mut machine, &snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn calendar() -> (String, Duration) {
        ("2026-10-13".into(), Duration::from_secs(3600))
    }

    #[test]
    fn injected_worker_ticks_pause_on_lock_and_never_count_sleep() {
        let mut machine = Machine::default();
        machine
            .restore(crate::storage::Document::default(), Duration::ZERO)
            .unwrap();
        machine.step(Duration::ZERO, Some(calendar()), true);
        machine
            .command(Command::Focus(FocusCommand::Start), Duration::ZERO)
            .unwrap();
        machine.step(Duration::from_secs(2), Some(calendar()), true);
        assert_eq!(machine.snapshot().focus.today_focus_seconds, 2);
        machine.step(Duration::from_secs(3), Some(calendar()), false);
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::PausedFocus
        );
        assert!(
            machine
                .command(Command::Focus(FocusCommand::Resume), Duration::from_secs(3))
                .is_err()
        );
        machine.step(Duration::from_secs(3), Some(calendar()), true);
        machine
            .command(Command::Focus(FocusCommand::Resume), Duration::from_secs(3))
            .unwrap();
        machine.step(Duration::from_secs(100), Some(calendar()), true);
        assert_eq!(machine.snapshot().focus.today_focus_seconds, 2);
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::PausedFocus
        );
    }

    #[test]
    fn load_barrier_rejects_mutations_and_restore_pauses_running_session() {
        let mut machine = Machine::default();
        assert!(
            machine
                .command(Command::SetScale(125), Duration::ZERO)
                .is_err()
        );
        let document = crate::storage::Document {
            checkpoint: crate::focus::FocusCheckpoint {
                phase: crate::focus::FocusPhase::Focus,
                calendar_day: "2026-10-13".into(),
                remaining_millis: 1000,
                ..Default::default()
            },
            ..Default::default()
        };
        machine.restore(document, Duration::from_secs(100)).unwrap();
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::PausedFocus
        );
        assert!(
            machine
                .command(Command::SetScale(0), Duration::from_secs(100))
                .is_err()
        );
        machine
            .command(Command::SetScale(125), Duration::from_secs(100))
            .unwrap();
        assert_eq!(machine.document().preferences.scale_percent, 125);
    }

    fn wait_until(client: &Client, condition: impl Fn(&AppSnapshot) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition(&client.snapshot()) {
            assert!(Instant::now() < deadline, "worker condition timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn worker_preserves_corrupt_original_until_retry_load_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("focus.json");
        std::fs::write(&path, b"broken").unwrap();
        let worker = Worker::start(root.path().into()).unwrap();
        let client = worker.client();
        wait_until(&client, |snapshot| {
            snapshot.storage_error && snapshot.load_pending
        });
        client.send(Command::SetScale(125)).unwrap();
        wait_until(&client, |snapshot| snapshot.command_error.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        std::fs::write(&path, serde_json::to_vec(&Document::default()).unwrap()).unwrap();
        wait_until(&client, |snapshot| !snapshot.load_pending);
        client.send(Command::SetScale(125)).unwrap();
        wait_until(&client, |snapshot| {
            snapshot.preferences.scale_percent == 125
        });
        worker.shutdown().unwrap();
        assert_eq!(
            Storage::new(root.path().into())
                .load()
                .unwrap()
                .preferences
                .scale_percent,
            125
        );
    }

    #[test]
    fn shutdown_drains_already_accepted_commands_before_final_save() {
        let root = tempfile::tempdir().unwrap();
        let worker = Worker::start(root.path().into()).unwrap();
        let client = worker.client();
        wait_until(&client, |snapshot| !snapshot.load_pending);
        client.send(Command::SetScale(125)).unwrap();
        client
            .send(Command::SetAppearance(Appearance::Dark))
            .unwrap();
        worker.shutdown().unwrap();
        let document = Storage::new(root.path().into()).load().unwrap();
        assert_eq!(document.preferences.scale_percent, 125);
        assert_eq!(document.preferences.appearance, Appearance::Dark);
        assert!(client.send(Command::SetScale(100)).is_err());
    }

    #[test]
    fn inaccessible_desktop_rejects_start_and_resume() {
        let mut machine = Machine::default();
        machine
            .restore(Document::default(), Duration::ZERO)
            .unwrap();
        machine.step(Duration::ZERO, Some(calendar()), false);
        assert!(
            machine
                .command(Command::Focus(FocusCommand::Start), Duration::ZERO)
                .is_err()
        );
        machine.step(Duration::ZERO, Some(calendar()), true);
        machine
            .command(Command::Focus(FocusCommand::Start), Duration::ZERO)
            .unwrap();
        machine.step(Duration::from_secs(1), Some(calendar()), false);
        assert!(
            machine
                .command(Command::Focus(FocusCommand::Resume), Duration::from_secs(1))
                .is_err()
        );
    }

    #[test]
    fn standup_timer_fires_once_per_interval_and_ignores_locks() {
        let mut machine = Machine::default();
        machine
            .restore(Document::default(), Duration::ZERO)
            .unwrap();
        machine.step(Duration::ZERO, Some(calendar()), true);
        machine
            .command(Command::SetStandup(1), Duration::ZERO)
            .unwrap();
        // The per-step delta is capped, so accrue a minute in 10s steps.
        for seconds in (10..=60).step_by(10) {
            machine.step(Duration::from_secs(seconds), Some(calendar()), true);
        }
        assert_eq!(machine.snapshot().standup_sequence, 1);
        // A locked desktop must not dump its whole interval on unlock.
        machine.step(Duration::from_secs(70), Some(calendar()), false);
        machine.step(Duration::from_secs(75), Some(calendar()), true);
        assert_eq!(machine.snapshot().standup_sequence, 1);
        machine
            .command(Command::SetStandup(0), Duration::from_secs(75))
            .unwrap();
        machine.step(Duration::from_secs(400), Some(calendar()), true);
        assert_eq!(machine.snapshot().standup_sequence, 1);
        assert!(
            machine
                .command(Command::SetStandup(181), Duration::from_secs(400))
                .is_err()
        );
    }

    #[test]
    fn auto_continue_starts_after_completed_break_but_not_after_manual_stop() {
        let mut machine = Machine::default();
        machine
            .restore(Document::default(), Duration::ZERO)
            .unwrap();
        machine.step(Duration::ZERO, Some(calendar()), true);
        machine
            .command(
                Command::Focus(FocusCommand::Configure {
                    focus_minutes: 1,
                    break_minutes: 1,
                }),
                Duration::ZERO,
            )
            .unwrap();
        machine
            .command(Command::SetAutoContinue(true), Duration::ZERO)
            .unwrap();
        machine
            .command(Command::Focus(FocusCommand::Start), Duration::ZERO)
            .unwrap();
        // Gaps above five seconds suspend the session, so tick in 5s steps.
        for seconds in (5..=65).step_by(5) {
            machine.step(Duration::from_secs(seconds), Some(calendar()), true);
        }
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::Break
        );
        for seconds in (70..=120).step_by(5) {
            machine.step(Duration::from_secs(seconds), Some(calendar()), true);
        }
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::Focus
        );
        // Manual stop during the next break must stay idle.
        for seconds in (125..=180).step_by(5) {
            machine.step(Duration::from_secs(seconds), Some(calendar()), true);
        }
        machine
            .command(
                Command::Focus(FocusCommand::Stop),
                Duration::from_secs(185),
            )
            .unwrap();
        machine.step(Duration::from_secs(190), Some(calendar()), true);
        assert_eq!(
            machine.snapshot().focus.phase,
            crate::focus::FocusPhase::Idle
        );
    }

    #[test]
    fn failed_save_reports_error_and_retries_without_losing_preferences() {
        let root = tempfile::tempdir().unwrap();
        let store = Storage::new(root.path().into());
        store.save(&Document::default()).unwrap();
        std::fs::create_dir(root.path().join("focus.json.bak")).unwrap();
        let worker = Worker::start(root.path().into()).unwrap();
        let client = worker.client();
        wait_until(&client, |snapshot| !snapshot.load_pending);
        client.send(Command::SetScale(125)).unwrap();
        wait_until(&client, |snapshot| snapshot.storage_error);
        assert_eq!(store.load().unwrap().preferences.scale_percent, 100);
        std::fs::remove_dir(root.path().join("focus.json.bak")).unwrap();
        wait_until(&client, |snapshot| !snapshot.storage_error);
        assert_eq!(store.load().unwrap().preferences.scale_percent, 125);
        worker.shutdown().unwrap();
    }

    #[test]
    fn worker_acknowledges_success_and_failure_and_drains_settings_tickets() {
        let root = tempfile::tempdir().unwrap();
        let worker = Worker::start(root.path().into()).unwrap();
        let client = worker.client();
        wait_until(&client, |snapshot| !snapshot.load_pending);
        assert_eq!(client.snapshot().last_ack, 0);
        let first = client.send(Command::SetScale(125)).unwrap();
        wait_until(&client, |snapshot| snapshot.last_ack >= first);
        assert_eq!(client.snapshot().preferences.scale_percent, 125);
        let failure = client.send(Command::SetScale(0)).unwrap();
        assert!(failure > first);
        wait_until(&client, |snapshot| snapshot.last_ack >= failure);
        assert!(client.snapshot().command_error.is_some());
        assert_eq!(client.snapshot().preferences.scale_percent, 125);
        let scale = client.send(Command::SetScale(150)).unwrap();
        let appearance = client
            .send(Command::SetAppearance(Appearance::Dark))
            .unwrap();
        assert!(appearance > scale);
        worker.shutdown().unwrap();
        assert_eq!(client.snapshot().last_ack, appearance);
        let document = Storage::new(root.path().into()).load().unwrap();
        assert_eq!(document.preferences.scale_percent, 150);
        assert_eq!(document.preferences.appearance, Appearance::Dark);
    }

    #[test]
    fn cloned_senders_enqueue_tickets_in_sequence_order() {
        let (sender, receiver) = mpsc::sync_channel(64);
        let client = Client {
            sender: Arc::new(Mutex::new(Producer {
                sender,
                next_sequence: 1,
            })),
            snapshot: Arc::new(Mutex::new(Machine::default().snapshot())),
            stopping: Arc::new(AtomicBool::new(false)),
        };
        let threads: Vec<_> = (0..2)
            .map(|index| {
                let client = client.clone();
                thread::spawn(move || {
                    (0..32)
                        .map(|offset| {
                            let scale = 100 + index * 32 + offset;
                            let ticket = client.send(Command::SetScale(scale)).unwrap();
                            (ticket, scale)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let tickets: std::collections::BTreeMap<_, _> = threads
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        assert!(
            client.send(Command::SetScale(100)).is_err(),
            "full queue rejected"
        );
        for expected in 1..=64 {
            let envelope = receiver.try_recv().unwrap();
            assert_eq!(envelope.sequence, expected);
            let Command::SetScale(scale) = envelope.command else {
                panic!("scale command expected");
            };
            assert_eq!(scale, tickets[&expected]);
        }
        assert_eq!(
            client.send(Command::SetScale(100)).unwrap(),
            65,
            "rejected enqueue consumes no ticket"
        );
    }
}
