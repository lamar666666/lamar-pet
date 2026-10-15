use crate::ui_policy::DraftTarget;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    Animation, AnimationExt, App, Context, Entity, Image, Render, Window, div, img, prelude::*, px,
};
use lamar_pet::{
    focus::{FocusCheckpoint, FocusCommand, FocusPhase, FocusReminder},
    service::{AppSnapshot, Appearance, Client, Command},
    sprites::SpriteAtlas,
    todo::{TodoCommand, TodoPriority},
};
use std::sync::{Arc, atomic::Ordering};
use std::{rc::Rc, time::Duration};

pub struct MainView {
    metrics: Arc<crate::metrics::Metrics>,
    client: Client,
    atlas: Rc<SpriteAtlas>,
    data_dir: Rc<std::path::Path>,
    name_input: Entity<InputState>,
    focus_input: Entity<InputState>,
    break_input: Entity<InputState>,
    standup_input: Entity<InputState>,
    editing_minutes: bool,
    task_input: Entity<InputState>,
    due_input: Entity<InputState>,
    draft_priority: TodoPriority,
    draft_error: Option<String>,
    editing_task: Option<u64>,
    edit_title_input: Entity<InputState>,
    edit_due_input: Entity<InputState>,
    edit_priority: TodoPriority,
    edit_error: Option<String>,
    edit_generation: u64,
    saved_feedback: Option<(u64, std::time::Instant)>,
    pending_save: Option<DraftTarget>,
    clear_draft: Option<DraftTarget>,
    page: u8,
    show_completed: bool,
    main_scroll: gpui_kit::ScrollHandle,
    last_name: String,
    snapshot: AppSnapshot,
    send_error: bool,
    pending: Option<u64>,
}

pub fn sync_theme(appearance: Appearance, window: &mut Window, cx: &mut App) {
    let mode = match appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
        Appearance::System => {
            Theme::sync_system_appearance(Some(window), cx);
            return;
        }
    };
    if cx.theme().mode != mode {
        Theme::change(mode, Some(window), cx);
    }
}

impl MainView {
    pub fn open_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = 0;
        self.main_scroll.scroll_to_top_of_item(0);
        self.task_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub fn new(
        client: Client,
        atlas: Rc<SpriteAtlas>,
        metrics: Arc<crate::metrics::Metrics>,
        data_dir: Rc<std::path::Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = client.snapshot();
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("宠物名")
                .default_value(snapshot.preferences.name.clone())
        });
        let focus_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(snapshot.focus.focus_minutes.to_string())
        });
        let break_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(snapshot.focus.break_minutes.to_string())
        });
        let standup_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(snapshot.preferences.standup_minutes.to_string())
        });
        cx.subscribe(
            &standup_input,
            |view, _, event: &InputEvent, cx| match event {
                InputEvent::Focus => view.editing_minutes = true,
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    view.editing_minutes = false;
                    view.apply_standup(cx);
                }
                _ => {}
            },
        )
        .detach();
        cx.subscribe(&name_input, |view, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let name = state.read(cx).value().trim().to_string();
                if !name.is_empty() && name != view.snapshot.preferences.name {
                    view.send(Command::SetName(name), cx);
                }
            }
        })
        .detach();
        for input in [&focus_input, &break_input] {
            cx.subscribe(input, |view, _, event: &InputEvent, cx| match event {
                InputEvent::Focus => view.editing_minutes = true,
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    view.editing_minutes = false;
                    view.apply_minutes(cx);
                }
                _ => {}
            })
            .detach();
        }
        let task_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("添加任务，例如 15:30 发送周报"));
        let due_input = cx.new(|cx| InputState::new(window, cx).placeholder("HH:mm"));
        for input in [&task_input, &due_input] {
            cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.save_task(DraftTarget::Add, cx);
                }
            })
            .detach();
        }
        let edit_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("任务名称"));
        let edit_due_input = cx.new(|cx| InputState::new(window, cx).placeholder("HH:mm"));
        for input in [&edit_title_input, &edit_due_input] {
            cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && let Some(id) = view.editing_task
                {
                    view.save_task(DraftTarget::Edit(id), cx);
                }
            })
            .detach();
        }
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(150)).await;
                if this
                    .update(cx, |view, cx| {
                        view.snapshot = view.client.snapshot();
                        if view
                            .pending
                            .is_some_and(|ticket| view.snapshot.last_ack >= ticket)
                        {
                            let ticket = view.pending.take().expect("acknowledged command");
                            if let Some(target) = view.pending_save.take() {
                                view.clear_draft = crate::ui_policy::acknowledged_draft(
                                    target,
                                    ticket,
                                    view.snapshot.last_ack,
                                    view.snapshot.command_error.is_some(),
                                );
                                if view.clear_draft.is_none() {
                                    view.set_draft_error(
                                        target,
                                        Some("保存未确认，草稿已保留，请检查任务后重试。".into()),
                                    );
                                }
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let main_scroll = gpui_kit::ScrollHandle::new();
        main_scroll.scroll_to_top_of_item(0);
        Self {
            metrics,
            client,
            atlas,
            data_dir,
            name_input,
            focus_input,
            break_input,
            standup_input,
            editing_minutes: false,
            task_input,
            due_input,
            draft_priority: TodoPriority::Normal,
            draft_error: None,
            editing_task: None,
            edit_title_input,
            edit_due_input,
            edit_priority: TodoPriority::Normal,
            edit_error: None,
            edit_generation: 0,
            saved_feedback: None,
            pending_save: None,
            clear_draft: None,
            page: 0,
            show_completed: false,
            main_scroll,
            last_name: snapshot.preferences.name.clone(),
            snapshot,
            send_error: false,
            pending: None,
        }
    }

    /// Minutes typed directly apply on Enter/blur; the focused box is left
    /// alone until then so keystrokes are never clobbered by the 150ms sync.
    fn apply_minutes(&mut self, cx: &mut Context<Self>) {
        let focus = &self.snapshot.focus;
        let parse = |input: &Entity<InputState>, fallback: u16, cx: &App| {
            input
                .read(cx)
                .value()
                .trim()
                .parse::<u16>()
                .unwrap_or(fallback)
        };
        let focus_minutes = parse(&self.focus_input, focus.focus_minutes, cx).clamp(1, 180);
        let break_minutes = parse(&self.break_input, focus.break_minutes, cx).clamp(1, 60);
        if focus_minutes != focus.focus_minutes || break_minutes != focus.break_minutes {
            self.send(
                Command::Focus(FocusCommand::Configure {
                    focus_minutes,
                    break_minutes,
                }),
                cx,
            );
        }
    }

    fn apply_standup(&mut self, cx: &mut Context<Self>) {
        let minutes = self
            .standup_input
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(self.snapshot.preferences.standup_minutes)
            .min(180);
        if minutes != self.snapshot.preferences.standup_minutes {
            self.send(Command::SetStandup(minutes), cx);
        }
    }

    fn save_task(&mut self, target: DraftTarget, cx: &mut Context<Self>) {
        if self.pending.is_some() || self.snapshot.load_pending {
            return;
        }
        let (title_input, due_input, priority) = match target {
            DraftTarget::Add => (&self.task_input, &self.due_input, self.draft_priority),
            DraftTarget::Edit(id) if self.editing_task == Some(id) => (
                &self.edit_title_input,
                &self.edit_due_input,
                self.edit_priority,
            ),
            _ => return,
        };
        let title = title_input.read(cx).value().trim().to_string();
        let raw = due_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.set_draft_error(target, Some("先写下任务名称".into()));
            cx.notify();
            return;
        }
        let due_minute = if raw.is_empty() {
            None
        } else {
            let parsed = raw.split_once(':').and_then(|(h, m)| {
                if h.len() != 2
                    || m.len() != 2
                    || !h.bytes().all(|c| c.is_ascii_digit())
                    || !m.bytes().all(|c| c.is_ascii_digit())
                {
                    return None;
                }
                let h = h.parse::<u16>().ok()?;
                let m = m.parse::<u16>().ok()?;
                (h < 24 && m < 60).then_some(h * 60 + m)
            });
            if parsed.is_none() {
                self.set_draft_error(target, Some("截止时间请写为 HH:mm，例如 17:30".into()));
                cx.notify();
                return;
            }
            parsed
        };
        let (title, due_minute) = if target == DraftTarget::Add {
            match crate::quick_entry::parse(&title, due_minute) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.set_draft_error(target, Some(error));
                    cx.notify();
                    return;
                }
            }
        } else {
            (title, due_minute)
        };
        self.set_draft_error(target, None);
        let command = match target {
            DraftTarget::Edit(id) => TodoCommand::Update {
                id,
                title,
                priority,
                due_minute,
            },
            DraftTarget::Add => TodoCommand::Add {
                title,
                priority,
                due_minute,
            },
        };
        self.send(Command::Todo(command), cx);
        self.pending_save = self.pending.map(|_| target);
        if self.send_error {
            self.set_draft_error(target, Some("保存未发送，草稿已保留，请重试。".into()));
        }
    }

    fn set_draft_error(&mut self, target: DraftTarget, error: Option<String>) {
        match target {
            DraftTarget::Add => self.draft_error = error,
            DraftTarget::Edit(_) => self.edit_error = error,
        }
    }
    fn edit_task(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some()
            || self.snapshot.load_pending
            || !crate::ui_policy::can_begin_edit(self.editing_task, id)
        {
            return;
        }
        if let Some(task) = self
            .snapshot
            .todo
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
        {
            self.editing_task = Some(id);
            self.edit_generation = self.edit_generation.wrapping_add(1);
            self.edit_priority = task.priority;
            self.edit_error = None;
            self.edit_title_input
                .update(cx, |state, cx| state.set_value(task.title, window, cx));
            let due = task
                .due_minute
                .map(|minute| format!("{:02}:{:02}", minute / 60, minute % 60))
                .unwrap_or_default();
            self.edit_due_input
                .update(cx, |state, cx| state.set_value(due, window, cx));
            self.edit_title_input
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    fn reset_draft(&mut self, target: DraftTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            DraftTarget::Add => {
                self.draft_error = None;
                self.draft_priority = TodoPriority::Normal;
                self.task_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.due_input
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
            DraftTarget::Edit(id) => {
                self.editing_task = None;
                self.edit_error = None;
                if let Some(task) = self.snapshot.todo.tasks.iter().find(|task| task.id == id) {
                    self.edit_priority = task.priority;
                    self.edit_title_input.update(cx, |state, cx| {
                        state.set_value(task.title.clone(), window, cx)
                    });
                    let due = task
                        .due_minute
                        .map(|m| format!("{:02}:{:02}", m / 60, m % 60))
                        .unwrap_or_default();
                    self.edit_due_input
                        .update(cx, |state, cx| state.set_value(due, window, cx));
                }
            }
        }
        cx.notify();
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        match self.client.send(command) {
            Ok(ticket) => {
                self.pending = Some(ticket);
                self.send_error = false;
            }
            Err(_) => self.send_error = true,
        }
        cx.notify();
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics.panel_frames.fetch_add(1, Ordering::Relaxed);
        sync_theme(self.snapshot.preferences.appearance, window, cx);
        let name = self.snapshot.preferences.name.clone();
        if name != self.last_name {
            window.set_window_title(&name);
            self.last_name = name;
        }
        if !self.editing_minutes {
            let want = [
                (
                    self.focus_input.clone(),
                    self.snapshot.focus.focus_minutes.to_string(),
                ),
                (
                    self.break_input.clone(),
                    self.snapshot.focus.break_minutes.to_string(),
                ),
                (
                    self.standup_input.clone(),
                    self.snapshot.preferences.standup_minutes.to_string(),
                ),
            ];
            for (input, value) in want {
                input.update(cx, |state, cx| {
                    if state.value() != value {
                        state.set_value(value, window, cx);
                    }
                });
            }
        }
        if let Some(target) = self.clear_draft.take() {
            if let DraftTarget::Edit(id) = target {
                self.saved_feedback = Some((id, std::time::Instant::now()));
            }
            self.reset_draft(target, window, cx);
        }
        let focus = self.snapshot.focus.clone();
        let dark = cx.theme().is_dark();
        let background = if dark {
            cx.theme().background
        } else {
            gpui_kit::rgb(0xf7f6f2).into()
        };
        let muted_fg = cx.theme().muted_foreground;
        let primary: gpui_kit::Hsla = if dark {
            gpui_kit::rgb(0x86bd9c).into()
        } else {
            gpui_kit::rgb(0x56764d).into()
        };
        let day = focus.checkpoint.calendar_day.clone();
        let mut content = div()
            .id("main-scroll")
            .track_scroll(&self.main_scroll)
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .bg(background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(self.atlas.frames[0].clone()).w(px(42.0)).h(px(42.0)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(self.snapshot.preferences.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("办公助手 · {day}")),
                            ),
                    )
                    .child(div().text_xs().text_color(muted_fg).child(format!(
                        "今日专注 {} 分钟 · {} 轮",
                        focus.today_focus_seconds / 60,
                        focus.today_completed_rounds
                    ))),
            )
            .child(
                div().flex().gap_2().children(
                    [
                        (0u8, "page-todos", "今日待办"),
                        (1, "page-settings", "偏好设置"),
                        (2, "page-data", "专注统计"),
                    ]
                    .into_iter()
                    .map(|(page, id, label)| {
                        Button::new(id)
                            .ghost()
                            .label(label)
                            .selected(self.page == page)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.page = page;
                                view.main_scroll.scroll_to_top_of_item(0);
                                cx.notify();
                            }))
                    }),
                ),
            );
        content = content.child(match self.page {
            0 => self.todo_page(cx),
            1 => self.settings_page(cx),
            _ => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(week_strip(&focus.checkpoint, cx))
                .child(data_card(&focus.checkpoint, self.data_dir.clone(), cx))
                .into_any_element(),
        });
        if self.snapshot.storage_error {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child("本地数据无法读取或保存，正在重试。"),
            );
        } else if self.send_error || self.snapshot.command_error.is_some() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child("操作暂未完成，请重试。"),
            );
        }
        let reminder = match (focus.phase, focus.reminder) {
            (FocusPhase::Break | FocusPhase::PausedBreak, FocusReminder::FocusCompleted) => {
                Some("本轮已完成，休息一下。")
            }
            (FocusPhase::Idle, FocusReminder::BreakCompleted) => Some("休息结束，可以开始下一轮。"),
            _ => None,
        };
        if let Some(message) = reminder {
            content = content.child(div().text_sm().text_color(primary).child(message));
        }
        content
    }
}

impl MainView {
    #[inline(never)]
    fn todo_page(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let day = self.snapshot.focus.checkpoint.calendar_day.clone();
        let muted_bg = if cx.theme().is_dark() {
            cx.theme().muted
        } else {
            gpui_kit::rgb(0xefeee8).into()
        };
        let muted_fg = cx.theme().muted_foreground;
        let selected_id = self.snapshot.todo.selected_id;
        let mut content = div().flex().flex_col().gap_4();
        content = content.child(self.focus_card(cx));
        let unfinished = self
            .snapshot
            .todo
            .tasks
            .iter()
            .filter(|task| !task.done)
            .count();
        let completed = self
            .snapshot
            .todo
            .tasks
            .iter()
            .filter(|task| task.done)
            .count();
        content = content.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_lg()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child("今天要做的事"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(format!("{unfinished} 待办 · {completed} 已完成")),
                ),
        );
        content = content.child(self.task_composer(cx));
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let minute = u16::from(now.hour()) * 60 + u16::from(now.minute());
        let mut tasks = self
            .snapshot
            .todo
            .pending_tasks(&day, minute)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if self.show_completed {
            tasks.extend(
                self.snapshot
                    .todo
                    .tasks
                    .iter()
                    .filter(|task| task.done)
                    .cloned(),
            );
        }
        if let Some(id) = self.editing_task
            && !tasks.iter().any(|task| task.id == id)
            && let Some(task) = self.snapshot.todo.tasks.iter().find(|task| task.id == id)
        {
            tasks.push(task.clone());
        }
        if tasks.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_5()
                    .rounded_xl()
                    .bg(muted_bg)
                    .child(div().text_sm().child("待办清单是空的"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("在上方添加任务，或直接开始一轮专注。"),
                    ),
            );
        }
        for task in tasks {
            content = content.child(self.task_row(task, &day, minute, selected_id, cx));
        }
        content = content.child(
            Button::new("show-completed")
                .ghost()
                .label(if self.show_completed {
                    "收起已完成"
                } else {
                    "查看已完成"
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    view.show_completed = !view.show_completed;
                    cx.notify();
                })),
        );
        content.into_any_element()
    }
    #[inline(never)]
    fn focus_card(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let focus = self.snapshot.focus.clone();
        let idle = focus.phase == FocusPhase::Idle;
        let blocked = self.snapshot.load_pending;
        let surface = if cx.theme().is_dark() {
            cx.theme().popover
        } else {
            gpui_kit::rgb(0xffffff).into()
        };
        let muted_fg = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let primary: gpui_kit::Hsla = if cx.theme().is_dark() {
            gpui_kit::rgb(0x86bd9c).into()
        } else {
            gpui_kit::rgb(0x56764d).into()
        };
        let (state, action, button) = match focus.phase {
            FocusPhase::Idle => ("准备开始", FocusCommand::Start, "开始专注"),
            FocusPhase::Focus => ("正在专注", FocusCommand::Pause, "暂停"),
            FocusPhase::Break => ("休息时间", FocusCommand::Pause, "暂停"),
            _ => ("已暂停", FocusCommand::Resume, "继续"),
        };
        let seconds = if idle {
            u64::from(focus.focus_minutes) * 60
        } else {
            focus.remaining_seconds
        };
        let selected = self
            .snapshot
            .todo
            .tasks
            .iter()
            .find(|task| Some(task.id) == self.snapshot.todo.selected_id && !task.done);
        let selected_id = selected.map(|task| task.id);
        let selected_title = selected
            .map(|task| task.title.clone())
            .unwrap_or_else(|| "选择一件事，开始专注".into());
        let focus_command = if idle {
            selected_id
                .map(Command::StartTodoFocus)
                .unwrap_or(Command::Focus(FocusCommand::Start))
        } else {
            Command::Focus(action.clone())
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(border)
            .bg(surface)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_xs().text_color(primary).child(state))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(selected_title),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(38.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(format!("{:02}:{:02}", seconds / 60, seconds % 60)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("primary")
                            .success()
                            .label(button)
                            .disabled(blocked)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(focus_command.clone(), cx)
                            })),
                    )
                    .child(
                        Button::new("end")
                            .ghost()
                            .label("结束本轮")
                            .disabled(blocked || idle)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send(Command::Focus(FocusCommand::Stop), cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted_fg).child(format!(
                        "专注 {} 分钟 / 休息 {} 分钟",
                        focus.focus_minutes, focus.break_minutes
                    ))),
            )
            .into_any_element()
    }
    #[inline(never)]
    fn task_composer(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let blocked = self.snapshot.load_pending;
        let surface = if cx.theme().is_dark() {
            cx.theme().popover
        } else {
            gpui_kit::rgb(0xffffff).into()
        };
        let muted_fg = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut composer = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_xl()
            .border_1()
            .border_color(border)
            .bg(surface)
            .child(
                div().flex().items_center().gap_2().child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("快速新增"),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Input::new(&self.task_input)
                            .id("task-title-input")
                            .flex_1()
                            .disabled(blocked),
                    )
                    .child(
                        Button::new("add-task")
                            .success()
                            .label("添加")
                            .disabled(blocked)
                            .on_click(
                                cx.listener(|view, _, _, cx| view.save_task(DraftTarget::Add, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().text_color(muted_fg).child("优先级"))
                    .children(
                        [
                            (TodoPriority::Low, "priority-low", "低"),
                            (TodoPriority::Normal, "priority-normal", "普通"),
                            (TodoPriority::High, "priority-high", "高"),
                        ]
                        .into_iter()
                        .map(|(priority, id, label)| {
                            Button::new(id)
                                .ghost()
                                .label(label)
                                .selected(self.draft_priority == priority)
                                .disabled(blocked)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.draft_priority = priority;
                                    cx.notify();
                                }))
                        }),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted_fg).child("截止（可选）"))
                    .child(
                        Input::new(&self.due_input)
                            .id("task-due-input")
                            .w(px(90.0))
                            .disabled(blocked),
                    ),
            );
        if let Some(error) = &self.draft_error {
            composer = composer.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        composer.into_any_element()
    }
    #[inline(never)]
    fn task_row(
        &mut self,
        task: lamar_pet::todo::Task,
        day: &str,
        minute: u16,
        selected_id: Option<u64>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let blocked = self.snapshot.load_pending;
        let surface = cx.theme().popover;
        let muted = cx.theme().muted;
        let muted_fg = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let primary = cx.theme().primary;
        let id = task.id;
        let selected = Some(id) == selected_id;
        let editing = self.editing_task == Some(id);
        let edit_disabled = blocked || self.editing_task.is_some();
        let phase = self.snapshot.focus.phase;
        let resting = matches!(phase, FocusPhase::Break | FocusPhase::PausedBreak);
        let in_progress = selected && phase == FocusPhase::Focus;
        let focus_label = if resting {
            "休息中"
        } else if in_progress {
            "进行中"
        } else if phase == FocusPhase::PausedFocus {
            "继续专注"
        } else {
            "开始专注"
        };
        let mut metadata = task
            .due_minute
            .map(|m| format!("{:02}:{:02} 截止", m / 60, m % 60))
            .unwrap_or_else(|| "无截止时间".into());
        if task.day != day && !task.done {
            metadata.push_str(&format!(" · {} 未完成", task.day));
        }
        if task.is_overdue(day, minute) {
            metadata.push_str(" · 已逾期");
        }
        let priority_label = match task.priority {
            TodoPriority::Low => "低 ↻",
            TodoPriority::Normal => "普通 ↻",
            TodoPriority::High => "高 ↻",
        };
        let next_priority = match task.priority {
            TodoPriority::Low => TodoPriority::Normal,
            TodoPriority::Normal => TodoPriority::High,
            TodoPriority::High => TodoPriority::Low,
        };
        let update = TodoCommand::Update {
            id,
            title: task.title.clone(),
            priority: next_priority,
            due_minute: task.due_minute,
        };
        let mut row = div()
            .id(format!("task-row-{id}"))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected || editing { primary } else { border })
            .bg(surface)
            .hover(move |style| style.border_color(primary))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(format!("toggle-{id}"))
                            .ghost()
                            .label(if task.done {
                                "撤销完成"
                            } else {
                                "✓ 完成"
                            })
                            .disabled(blocked || editing)
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Command::Todo(TodoCommand::Toggle(id)), cx)
                            })),
                    )
                    .child(
                        div()
                            .id(format!("task-title-{id}"))
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(if task.done {
                                muted_fg
                            } else {
                                cx.theme().foreground
                            })
                            .rounded_sm()
                            .px_2()
                            .py_1()
                            .when(!edit_disabled, |title| {
                                title
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(muted))
                                    .active(|style| style.opacity(0.7))
                            })
                            .child(task.title.clone())
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.edit_task(id, window, cx)
                            })),
                    )
                    .child(
                        Button::new(format!("edit-{id}"))
                            .ghost()
                            .label(if editing { "编辑中" } else { "✎ 编辑" })
                            .disabled(edit_disabled)
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.edit_task(id, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(if task.is_overdue(day, minute) {
                                cx.theme().danger
                            } else {
                                muted_fg
                            })
                            .child(metadata),
                    )
                    .child(
                        Button::new(format!("change-priority-{id}"))
                            .ghost()
                            .label(priority_label)
                            .rounded_full()
                            .border_1()
                            .border_color(border)
                            .bg(muted)
                            .cursor_pointer()
                            .disabled(blocked || task.done || editing)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Command::Todo(update.clone()), cx)
                            })),
                    )
                    .child(
                        Button::new(format!("focus-{id}"))
                            .ghost()
                            .label(focus_label)
                            .cursor_pointer()
                            .disabled(blocked || task.done || resting || in_progress || editing)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Command::StartTodoFocus(id), cx)
                            })),
                    ),
            );
        if editing {
            let mut editor = div()
                .id(format!("inline-editor-{id}"))
                .flex()
                .flex_col()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(border)
                .capture_key_down(cx.listener(
                    move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && view.pending.is_none() {
                            view.reset_draft(DraftTarget::Edit(id), window, cx);
                            cx.stop_propagation();
                        }
                    },
                ))
                .child(
                    Input::new(&self.edit_title_input)
                        .id("edit-title-input")
                        .disabled(blocked),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_xs().text_color(muted_fg).child("截止（可选）"))
                        .child(
                            Input::new(&self.edit_due_input)
                                .id("edit-due-input")
                                .w(px(90.0))
                                .disabled(blocked),
                        )
                        .children(
                            [
                                (TodoPriority::Low, "低"),
                                (TodoPriority::Normal, "普通"),
                                (TodoPriority::High, "高"),
                            ]
                            .into_iter()
                            .map(|(priority, label)| {
                                Button::new(format!("edit-priority-{id}-{label}"))
                                    .ghost()
                                    .label(label)
                                    .selected(self.edit_priority == priority)
                                    .disabled(blocked)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.edit_priority = priority;
                                        cx.notify();
                                    }))
                            }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(format!("save-edit-{id}"))
                                .success()
                                .label(if self.pending_save == Some(DraftTarget::Edit(id)) {
                                    "保存中…"
                                } else {
                                    "保存修改"
                                })
                                .disabled(blocked)
                                .cursor_pointer()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.save_task(DraftTarget::Edit(id), cx)
                                })),
                        )
                        .child(
                            Button::new(format!("cancel-edit-{id}"))
                                .ghost()
                                .label("取消")
                                .disabled(blocked)
                                .cursor_pointer()
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.reset_draft(DraftTarget::Edit(id), window, cx)
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child("Enter 保存 · Esc 取消"),
                        ),
                );
            if let Some(error) = &self.edit_error {
                editor = editor.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                );
            }
            row = row.child(editor.with_animation(
                format!("row-edit-{id}-{}", self.edit_generation),
                Animation::new(Duration::from_millis(180)),
                |el, value| {
                    el.opacity(0.4 + value * 0.6)
                        .pt(px(8.0 * value))
                        .max_h(px(240.0 * value))
                        .overflow_hidden()
                },
            ));
        }
        if let Some((saved_id, since)) = self.saved_feedback
            && saved_id == id
            && since.elapsed() < Duration::from_secs(3)
        {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(primary)
                    .child("✓ 修改已保存")
                    .with_animation(
                        format!("row-saved-{id}-{}", self.edit_generation),
                        Animation::new(Duration::from_millis(220)),
                        |el, value| el.opacity(value),
                    ),
            );
        }
        row.into_any_element()
    }
    #[inline(never)]
    fn settings_page(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let focus = self.snapshot.focus.clone();
        let idle = focus.phase == FocusPhase::Idle;
        let blocked = self.snapshot.load_pending;
        let surface = if cx.theme().is_dark() {
            cx.theme().popover
        } else {
            gpui_kit::rgb(0xffffff).into()
        };
        let muted_fg = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let focus_minutes = focus.focus_minutes;
        let break_minutes = focus.break_minutes;
        let mut settings_rows = Vec::new();
        for (is_focus, value, label, maximum) in [
            (true, focus_minutes, "专注时长", 180),
            (false, break_minutes, "休息时长", 60),
        ] {
            let configure = |increase: bool| {
                let next = if increase {
                    value.saturating_add(1).min(maximum)
                } else {
                    value.saturating_sub(1).max(1)
                };
                Command::Focus(FocusCommand::Configure {
                    focus_minutes: if is_focus { next } else { focus_minutes },
                    break_minutes: if is_focus { break_minutes } else { next },
                })
            };
            let less = configure(false);
            let more = configure(true);
            let input = if is_focus {
                self.focus_input.clone()
            } else {
                self.break_input.clone()
            };
            settings_rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .child(div().flex_1().text_sm().child(label))
                    .child(
                        Button::new(if is_focus { "focus-less" } else { "break-less" })
                            .label("−")
                            .disabled(blocked || !idle || value == 1)
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.send(less.clone(), cx)),
                            ),
                    )
                    .child(Input::new(&input).w(px(56.0)).disabled(blocked || !idle))
                    .child(div().text_xs().text_color(muted_fg).child("分钟"))
                    .child(
                        Button::new(if is_focus { "focus-more" } else { "break-more" })
                            .label("+")
                            .disabled(blocked || !idle || value == maximum)
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.send(more.clone(), cx)),
                            ),
                    ),
            );
        }
        let scale = self.snapshot.preferences.scale_percent;
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("桌宠大小"))
                .child(
                    Button::new("smaller")
                        .label("−")
                        .disabled(blocked || scale <= 50)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::SetScale(scale.saturating_sub(10).max(50)), cx)
                        })),
                )
                .child(
                    div()
                        .w(px(56.0))
                        .text_sm()
                        .text_center()
                        .child(format!("{scale}%")),
                )
                .child(
                    Button::new("larger")
                        .label("+")
                        .disabled(blocked || scale >= 200)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::SetScale(scale.saturating_add(10).min(200)), cx)
                        })),
                ),
        );
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("久坐提醒"))
                .child(
                    Input::new(&self.standup_input)
                        .w(px(56.0))
                        .disabled(blocked),
                )
                .child(div().text_xs().text_color(muted_fg).child("分钟，0 关闭")),
        );
        let auto_continue = self.snapshot.preferences.auto_continue;
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("自动连转"))
                .child(
                    Button::new("auto-on")
                        .label("开启")
                        .selected(auto_continue)
                        .disabled(blocked)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send(Command::SetAutoContinue(true), cx)
                        })),
                )
                .child(
                    Button::new("auto-off")
                        .label("关闭")
                        .selected(!auto_continue)
                        .disabled(blocked)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send(Command::SetAutoContinue(false), cx)
                        })),
                ),
        );
        let appearance = self.snapshot.preferences.appearance;
        let mut theme_row = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .child(div().flex_1().text_sm().child("外观"));
        for (mode, id, label) in [
            (Appearance::Light, "light", "浅色"),
            (Appearance::Dark, "dark", "深色"),
            (Appearance::System, "system", "系统"),
        ] {
            theme_row = theme_row.child(
                Button::new(id)
                    .label(label)
                    .selected(mode == appearance)
                    .disabled(blocked)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(Command::SetAppearance(mode), cx)
                    })),
            );
        }
        settings_rows.push(theme_row);
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("宠物名"))
                .child(Input::new(&self.name_input).w(px(160.0)).disabled(blocked)),
        );
        let show_todos = self.snapshot.preferences.show_todos;
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("桌宠下方显示待办"))
                .child(
                    Button::new("hud-todos-on")
                        .label("显示")
                        .selected(show_todos)
                        .disabled(blocked)
                        .on_click(
                            cx.listener(|view, _, _, cx| {
                                view.send(Command::SetShowTodos(true), cx)
                            }),
                        ),
                )
                .child(
                    Button::new("hud-todos-off")
                        .label("隐藏")
                        .selected(!show_todos)
                        .disabled(blocked)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send(Command::SetShowTodos(false), cx)
                        })),
                ),
        );
        let sound_enabled = self.snapshot.preferences.sound_enabled;
        settings_rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .child(div().flex_1().text_sm().child("提醒声音"))
                .child(
                    Button::new("sound-on")
                        .label("开启")
                        .selected(sound_enabled)
                        .disabled(blocked)
                        .on_click(
                            cx.listener(|view, _, _, cx| view.send(Command::SetSound(true), cx)),
                        ),
                )
                .child(
                    Button::new("sound-off")
                        .label("关闭")
                        .selected(!sound_enabled)
                        .disabled(blocked)
                        .on_click(
                            cx.listener(|view, _, _, cx| view.send(Command::SetSound(false), cx)),
                        ),
                ),
        );
        let mut settings = div()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(border)
            .bg(surface);
        for (index, row) in settings_rows.into_iter().enumerate() {
            if index > 0 {
                settings = settings.child(div().h(px(1.0)).bg(border));
            }
            settings = settings.child(row);
        }
        settings.into_any_element()
    }
}

#[cfg(windows)]
fn open_data_dir(dir: &std::path::Path, _: &mut App) {
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

#[cfg(not(windows))]
fn open_data_dir(dir: &std::path::Path, cx: &mut App) {
    if let Some(text) = dir.to_str() {
        cx.open_url(&format!("file://{text}"));
    }
}

/// Seven-day strip of focus minutes ending today; today's bar is highlighted.
fn week_strip(checkpoint: &FocusCheckpoint, cx: &mut Context<MainView>) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let surface = cx.theme().popover;
    let primary = cx.theme().primary;
    let mut past = primary;
    past.a = 0.35;
    if checkpoint.calendar_day.len() < 10 {
        return div();
    }
    let mut days = Vec::with_capacity(7);
    let mut day = checkpoint.calendar_day.clone();
    for _ in 0..7 {
        days.push(day.clone());
        day = prev_day(&day);
    }
    days.reverse();
    let maximum = days
        .iter()
        .map(|day| {
            checkpoint
                .history
                .iter()
                .find(|entry| entry.day == *day)
                .map_or(0, |entry| entry.focus_millis / 60_000)
        })
        .max()
        .unwrap_or(0)
        .max(1);
    let mut strip = div().flex().items_end().gap_2().h(px(92.0));
    for (index, day) in days.iter().enumerate() {
        let minutes = checkpoint
            .history
            .iter()
            .find(|entry| entry.day == *day)
            .map_or(0, |entry| entry.focus_millis / 60_000);
        let today = index == 6;
        let height = if minutes == 0 {
            3.0
        } else {
            8.0 + (minutes as f32 / maximum as f32) * 52.0
        };
        strip = strip.child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .h_full()
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .h(px(16.0))
                        .child(if minutes == 0 {
                            "".into()
                        } else {
                            minutes.to_string()
                        }),
                )
                .child(div().flex_1())
                .child(div().w_full().h(px(height)).rounded_sm().bg(if today {
                    primary
                } else {
                    past
                }))
                .child(div().text_xs().text_color(muted).child(if today {
                    "今天".to_string()
                } else {
                    day[5..].to_string()
                })),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_xl()
        .border_1()
        .border_color(border)
        .bg(surface)
        .child(div().text_sm().text_color(muted).child("本周专注（分钟）"))
        .child(strip)
}

/// Friendly data summary: lifetime totals plus a backup button — no raw
/// paths or file names that non-developers would not recognise.
fn data_card(
    checkpoint: &FocusCheckpoint,
    data_dir: Rc<std::path::Path>,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let surface = cx.theme().popover;
    let total_minutes: u64 = checkpoint
        .history
        .iter()
        .map(|day| day.focus_millis / 60_000)
        .sum();
    let total_rounds: u32 = checkpoint
        .history
        .iter()
        .map(|day| day.completed_rounds)
        .sum();
    let streak = focus_streak(checkpoint);
    let duration = if total_minutes >= 60 {
        format!("{:.1} 小时", total_minutes as f32 / 60.0)
    } else {
        format!("{total_minutes} 分钟")
    };
    let cell = |value: String, label: &'static str| {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(div().text_lg().child(value))
            .child(div().text_xs().text_color(muted).child(label))
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_xl()
        .border_1()
        .border_color(border)
        .bg(surface)
        .child(div().text_sm().text_color(muted).child("我的数据"))
        .child(
            div()
                .flex()
                .items_center()
                .child(cell(format!("{} 天", checkpoint.history.len()), "陪伴天数"))
                .child(cell(duration, "累计专注"))
                .child(cell(format!("{total_rounds} 轮"), "完成轮数"))
                .child(cell(format!("{streak} 天"), "连续专注")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(muted)
                        .child("数据只保存在这台电脑上"),
                )
                .child(
                    Button::new("open-data-dir")
                        .label("备份数据")
                        .on_click(move |_, _, cx| open_data_dir(&data_dir, cx)),
                ),
        )
}

/// Consecutive days with recorded focus, ending today (or yesterday when
/// today has none yet).
fn focus_streak(checkpoint: &FocusCheckpoint) -> u32 {
    if checkpoint.calendar_day.len() < 10 {
        return 0;
    }
    let mut streak = 0;
    let mut day = checkpoint.calendar_day.clone();
    loop {
        let focused = checkpoint
            .history
            .iter()
            .any(|entry| entry.day == day && entry.focus_millis > 0);
        if focused {
            streak += 1;
        } else if streak > 0 || day != checkpoint.calendar_day {
            break;
        }
        day = prev_day(&day);
    }
    streak
}

fn prev_day(day: &str) -> String {
    let year: i32 = day[..4].parse().unwrap_or(0);
    let month: u32 = day[5..7].parse().unwrap_or(1);
    let date: u32 = day[8..].parse().unwrap_or(1);
    if date > 1 {
        return format!("{year:04}-{month:02}-{:02}", date - 1);
    }
    let (year, month) = if month > 1 {
        (year, month - 1)
    } else {
        (year - 1, 12)
    };
    let days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    format!("{year:04}-{month:02}-{days:02}")
}

pub struct ReminderView {
    pub message: std::borrow::Cow<'static, str>,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub image: Option<Arc<Image>>,
}
impl ReminderView {
    pub fn reminder(
        reminder: FocusReminder,
        metrics: Arc<crate::metrics::Metrics>,
        image: Option<Arc<Image>>,
    ) -> Self {
        let message = match reminder {
            FocusReminder::FocusCompleted => "本轮专注已完成，休息一下。",
            FocusReminder::Standup => "坐太久啦，起来活动几分钟、喝点水。",
            _ => "休息结束，准备好再开始下一轮。",
        };
        Self {
            message: message.into(),
            metrics,
            image,
        }
    }
}
impl Render for ReminderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics.reminder_frames.fetch_add(1, Ordering::Relaxed);
        let message = self.message.clone();
        let mut card = div()
            .flex()
            .items_center()
            .gap_5()
            .p_6()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if let Some(image) = self.image.clone() {
            card = card.child(
                div()
                    .with_animation(
                        "reminder-pet",
                        Animation::new(Duration::from_millis(1400)).repeat(),
                        |el, progress| el.mt(px(-5.0 * (std::f32::consts::TAU * progress).sin())),
                    )
                    .child(img(image).w(px(64.0)).h(px(64.0))),
            );
        }
        card.child(
            div()
                .flex()
                .flex_1()
                .flex_col()
                .gap_4()
                .child(div().text_lg().child(message.into_owned()))
                .child(
                    Button::new("dismiss")
                        .label("知道了")
                        .on_click(|_, window, _| window.remove_window()),
                ),
        )
        .with_animation(
            "reminder-in",
            Animation::new(Duration::from_millis(280)),
            |el, progress| el.opacity(progress).mt(px(10.0 * (1.0 - progress))),
        )
    }
}
