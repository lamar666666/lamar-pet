//! Local tasks are independent of focus rounds and never expire automatically.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub day: String,
    pub priority: TodoPriority,
    pub done: bool,
    pub due_minute: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoState {
    pub schema_version: u32,
    pub next_id: u64,
    pub tasks: Vec<Task>,
    pub selected_id: Option<u64>,
}
impl Default for TodoState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            next_id: 1,
            tasks: Vec::new(),
            selected_id: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoCommand {
    Add {
        title: String,
        priority: TodoPriority,
        due_minute: Option<u16>,
    },
    Toggle(u64),
    Select(Option<u64>),
    Update {
        id: u64,
        title: String,
        priority: TodoPriority,
        due_minute: Option<u16>,
    },
    Remove(u64),
}
impl TodoState {
    pub fn is_valid(&self) -> bool {
        self.schema_version == 1
            && self.next_id > 0
            && self.tasks.len() <= MAX_TASKS
            && self.tasks.iter().all(|task| {
                task.id > 0
                    && task.id < self.next_id
                    && valid_title(&task.title)
                    && valid_day(&task.day)
                    && task.due_minute.is_none_or(|minute| minute < 1440)
            })
            && {
                let ids: std::collections::HashSet<_> =
                    self.tasks.iter().map(|task| task.id).collect();
                ids.len() == self.tasks.len()
            }
            && self
                .selected_id
                .is_none_or(|id| self.tasks.iter().any(|task| task.id == id && !task.done))
    }
    pub fn command(&mut self, command: TodoCommand, day: &str) -> Result<(), String> {
        match command {
            TodoCommand::Add {
                title,
                priority,
                due_minute,
            } => {
                if !valid_day(day) {
                    return Err("Local calendar is unavailable".into());
                }
                let title = checked_title(title, due_minute)?;
                if self.tasks.len() >= MAX_TASKS {
                    return Err(
                        "Task limit reached (1000); remove a task before adding another".into(),
                    );
                }
                let next = self.next_id.checked_add(1).ok_or("Task IDs exhausted")?;
                self.tasks.push(Task {
                    id: self.next_id,
                    title,
                    day: day.into(),
                    priority,
                    done: false,
                    due_minute,
                });
                self.next_id = next;
            }
            TodoCommand::Toggle(id) => {
                let task = self
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == id)
                    .ok_or("Task no longer exists")?;
                task.done = !task.done;
                if task.done && self.selected_id == Some(id) {
                    self.selected_id = None;
                }
            }
            TodoCommand::Select(id) => {
                if id.is_some_and(|id| !self.tasks.iter().any(|task| task.id == id && !task.done)) {
                    return Err("Select an unfinished task".into());
                }
                self.selected_id = id;
            }
            TodoCommand::Update {
                id,
                title,
                priority,
                due_minute,
            } => {
                let title = checked_title(title, due_minute)?;
                let task = self
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == id)
                    .ok_or("Task no longer exists")?;
                task.title = title;
                task.priority = priority;
                task.due_minute = due_minute;
            }
            TodoCommand::Remove(id) => {
                let index = self
                    .tasks
                    .iter()
                    .position(|task| task.id == id)
                    .ok_or("Task no longer exists")?;
                self.tasks.remove(index);
                if self.selected_id == Some(id) {
                    self.selected_id = None;
                }
            }
        }
        Ok(())
    }
    /// All unfinished tasks, including past days. Consumers share this ordering.
    pub fn pending_tasks(&self, day: &str, minute: u16) -> Vec<&Task> {
        let mut tasks: Vec<_> = self.tasks.iter().filter(|task| !task.done).collect();
        tasks.sort_by(|a, b| {
            (self.selected_id != Some(a.id))
                .cmp(&(self.selected_id != Some(b.id)))
                .then_with(|| b.is_overdue(day, minute).cmp(&a.is_overdue(day, minute)))
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| {
                    match (
                        a.due_minute.map(|minute| (a.day.as_str(), minute)),
                        b.due_minute.map(|minute| (b.day.as_str(), minute)),
                    ) {
                        (Some(a), Some(b)) => a.cmp(&b),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                })
                .then_with(|| a.id.cmp(&b.id))
        });
        tasks
    }
    pub fn important_tasks(&self, day: &str, minute: u16) -> Vec<&Task> {
        self.pending_tasks(day, minute)
            .into_iter()
            .take(3)
            .collect()
    }
}
impl Task {
    pub fn is_overdue(&self, day: &str, minute: u16) -> bool {
        !self.done
            && (self.day.as_str() < day
                || (self.day == day && self.due_minute.is_some_and(|due| minute >= due)))
    }
}

pub const MAX_TASKS: usize = 1000;
pub const MAX_TITLE_CHARS: usize = 120;
fn valid_title(title: &str) -> bool {
    !title.trim().is_empty()
        && title.chars().count() <= MAX_TITLE_CHARS
        && !title.chars().any(char::is_control)
}
fn checked_title(title: String, due_minute: Option<u16>) -> Result<String, String> {
    let title = title.trim();
    if !valid_title(title) {
        return Err("Task title must be 1-120 characters on one line".into());
    }
    if due_minute.is_some_and(|minute| minute >= 1440) {
        return Err("Deadline must be a valid HH:mm time".into());
    }
    Ok(title.into())
}
fn valid_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| i != 4 && i != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let year = day[..4].parse::<i32>().unwrap_or(0);
    let month = day[5..7].parse::<u8>().unwrap_or(0);
    let date = day[8..].parse::<u8>().unwrap_or(0);
    year > 0
        && time::Month::try_from(month)
            .ok()
            .is_some_and(|month| time::Date::from_calendar_date(year, month, date).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add(state: &mut TodoState, title: &str, priority: TodoPriority, due: Option<u16>) {
        state
            .command(
                TodoCommand::Add {
                    title: title.into(),
                    priority,
                    due_minute: due,
                },
                "2026-10-15",
            )
            .unwrap();
    }
    #[test]
    fn ids_survive_deletion_and_roundtrip() {
        let mut state = TodoState::default();
        add(&mut state, " first ", TodoPriority::Normal, Some(600));
        assert_eq!(state.tasks[0].title, "first");
        state.command(TodoCommand::Remove(1), "2026-10-15").unwrap();
        let mut restored: TodoState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        add(&mut restored, "second", TodoPriority::High, None);
        assert_eq!(restored.tasks[0].id, 2);
        assert!(restored.is_valid());
    }
    #[test]
    fn unfinished_history_stays_and_selection_leads_shared_top_three() {
        let mut state = TodoState::default();
        add(&mut state, "overdue", TodoPriority::Low, Some(60));
        add(&mut state, "high", TodoPriority::High, None);
        add(&mut state, "selected", TodoPriority::Normal, None);
        add(&mut state, "fourth", TodoPriority::Low, None);
        state.tasks[0].day = "2026-10-14".into();
        state
            .command(TodoCommand::Select(Some(3)), "2026-10-15")
            .unwrap();
        let tasks = state.important_tasks("2026-10-15", 600);
        assert_eq!(
            tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
        assert!(tasks[1].is_overdue("2026-10-15", 600));
        state.command(TodoCommand::Toggle(3), "2026-10-15").unwrap();
        assert_eq!(state.selected_id, None);
        assert_eq!(state.pending_tasks("2026-10-16", 0).len(), 3);
        assert_eq!(state.tasks.len(), 4);
    }
    #[test]
    fn invalid_commands_are_transactional_and_limits_never_truncate() {
        let mut state = TodoState::default();
        add(&mut state, "keep", TodoPriority::Normal, None);
        let original = state.clone();
        for command in [
            TodoCommand::Add {
                title: " ".into(),
                priority: TodoPriority::Normal,
                due_minute: None,
            },
            TodoCommand::Add {
                title: "bad time".into(),
                priority: TodoPriority::High,
                due_minute: Some(1440),
            },
            TodoCommand::Select(Some(99)),
            TodoCommand::Toggle(99),
        ] {
            assert!(state.command(command, "2026-10-15").is_err());
            assert_eq!(state, original);
        }
        state.next_id = u64::MAX;
        assert!(
            state
                .command(
                    TodoCommand::Add {
                        title: "overflow".into(),
                        priority: TodoPriority::Normal,
                        due_minute: None
                    },
                    "2026-10-15"
                )
                .is_err()
        );
        assert_eq!(state.tasks.len(), 1);
    }

    #[test]
    fn quantity_and_title_limits_reject_without_losing_existing_tasks() {
        let mut state = TodoState::default();
        for _ in 0..MAX_TASKS {
            add(&mut state, "keep", TodoPriority::Normal, None);
        }
        let original = state.clone();
        assert!(
            state
                .command(
                    TodoCommand::Add {
                        title: "one too many".into(),
                        priority: TodoPriority::Normal,
                        due_minute: None
                    },
                    "2026-10-15"
                )
                .is_err()
        );
        assert_eq!(state, original);
        for title in ["x".repeat(MAX_TITLE_CHARS + 1), "first\nsecond".into()] {
            assert!(
                state
                    .command(
                        TodoCommand::Update {
                            id: 1,
                            title,
                            priority: TodoPriority::High,
                            due_minute: None
                        },
                        "2026-10-15"
                    )
                    .is_err()
            );
            assert_eq!(state, original);
        }
    }

    #[test]
    fn corrupt_ids_dates_selection_and_unknown_schemas_are_invalid() {
        let mut state = TodoState::default();
        add(&mut state, "keep", TodoPriority::Normal, Some(600));
        for corrupt in [
            TodoState {
                schema_version: 2,
                ..state.clone()
            },
            TodoState {
                next_id: 1,
                ..state.clone()
            },
            TodoState {
                tasks: vec![state.tasks[0].clone(), state.tasks[0].clone()],
                ..state.clone()
            },
            TodoState {
                selected_id: Some(50),
                ..state.clone()
            },
        ] {
            assert!(!corrupt.is_valid());
        }
        state.tasks[0].day = "2026-02-30".into();
        assert!(!state.is_valid());
        assert!(
            state
                .command(
                    TodoCommand::Add {
                        title: "invalid date".into(),
                        priority: TodoPriority::Normal,
                        due_minute: None
                    },
                    "2026-02-30"
                )
                .is_err()
        );
    }
}
