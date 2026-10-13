use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{App, Context, Entity, Render, Window, div, img, prelude::*, px};
use lamar_pet::{
    focus::{FocusCheckpoint, FocusCommand, FocusPhase, FocusReminder},
    service::{AppSnapshot, Appearance, Client, Command},
    sprites::SpriteAtlas,
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
        cx.subscribe(&standup_input, |view, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Focus => view.editing_minutes = true,
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    view.editing_minutes = false;
                    view.apply_standup(cx);
                }
                _ => {}
            }
        })
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
            cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                match event {
                    InputEvent::Focus => view.editing_minutes = true,
                    InputEvent::PressEnter { .. } | InputEvent::Blur => {
                        view.editing_minutes = false;
                        view.apply_minutes(cx);
                    }
                    _ => {}
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
                            view.pending = None;
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
        let focus = &self.snapshot.focus;
        let idle = focus.phase == FocusPhase::Idle;
        let blocked = self.snapshot.load_pending || self.pending.is_some();
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
        let mut content = div()
            .id("main-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(img(self.atlas.frames[0].clone()).w(px(72.0)).h(px(72.0)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(self.snapshot.preferences.name.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("桌面专注伴侣"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .p_5()
                    .rounded_xl()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(state),
                    )
                    .child(
                        div()
                            .text_size(px(60.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(format!("{:02}:{:02}", seconds / 60, seconds % 60)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("primary")
                                    .primary()
                                    .label(button)
                                    .disabled(blocked)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.send(Command::Focus(action.clone()), cx)
                                    })),
                            )
                            .child(
                                Button::new("end")
                                    .label("结束本轮")
                                    .disabled(blocked || idle)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.send(Command::Focus(FocusCommand::Stop), cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("今日专注"),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .child(format!("{} 分钟", focus.today_focus_seconds / 60)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("完成轮数"),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .child(focus.today_completed_rounds.to_string()),
                            ),
                    ),
            )
            .child(week_strip(&focus.checkpoint, cx));
        let focus_minutes = focus.focus_minutes;
        let break_minutes = focus.break_minutes;
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
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child(label))
                    .child(
                        Button::new(if is_focus { "focus-less" } else { "break-less" })
                            .label("−")
                            .disabled(blocked || !idle || value == 1)
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.send(less.clone(), cx)),
                            ),
                    )
                    .child(
                        Input::new(&input)
                            .w(px(64.0))
                            .disabled(blocked || !idle),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("分钟"),
                    )
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
        let appearance = self.snapshot.preferences.appearance;
        let auto_continue = self.snapshot.preferences.auto_continue;
        let mut themes = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().child("外观"));
        for (mode, id, label) in [
            (Appearance::Light, "light", "浅色"),
            (Appearance::Dark, "dark", "深色"),
            (Appearance::System, "system", "系统"),
        ] {
            themes = themes.child(
                Button::new(id)
                    .label(label)
                    .selected(mode == appearance)
                    .disabled(blocked)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(Command::SetAppearance(mode), cx)
                    })),
            );
        }
        let scale = self.snapshot.preferences.scale_percent;
        content = content.child(themes).child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().flex_1().child("桌宠大小"))
                .child(
                    Button::new("smaller")
                        .label("−")
                        .disabled(blocked || scale <= 50)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::SetScale(scale.saturating_sub(10).max(50)), cx)
                        })),
                )
                .child(format!("{scale}%"))
                .child(
                    Button::new("larger")
                        .label("+")
                        .disabled(blocked || scale >= 200)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::SetScale(scale.saturating_add(10).min(200)), cx)
                        })),
                ),
        );
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
            (FocusPhase::Idle, FocusReminder::BreakCompleted) => {
                Some("休息结束，准备好再开始下一轮。")
            }
            _ => None,
        };
        if let Some(message) = reminder {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().primary)
                    .child(message),
            );
        }
        content = content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child("久坐提醒"))
                    .child(Input::new(&self.standup_input).w(px(64.0)).disabled(blocked))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("分钟，0 关闭"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child("自动连转"))
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
        let data_dir = self.data_dir.clone();
        content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child("宠物名"))
                    .child(
                        Input::new(&self.name_input)
                            .w(px(180.0))
                            .disabled(blocked),
                    ),
            )
            .child(data_card(&focus.checkpoint, data_dir, cx))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("左键拖动桌宠，点一下它会高兴，右键打开面板。"),
            )
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
fn week_strip(
    checkpoint: &FocusCheckpoint,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let primary = cx.theme().primary;
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
                .child(
                    div()
                        .w_full()
                        .h(px(height))
                        .rounded_sm()
                        .bg(if today { primary } else { muted }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(if today { "今天".to_string() } else { day[5..].to_string() }),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(border)
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
    let total_minutes: u64 = checkpoint
        .history
        .iter()
        .map(|day| day.focus_millis / 60_000)
        .sum();
    let total_rounds: u32 = checkpoint.history.iter().map(|day| day.completed_rounds).sum();
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
        .rounded_lg()
        .border_1()
        .border_color(border)
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
                .child(Button::new("open-data-dir").label("备份数据").on_click(
                    move |_, _, cx| open_data_dir(&data_dir, cx),
                )),
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
    let (year, month) = if month > 1 { (year, month - 1) } else { (year - 1, 12) };
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
}
impl ReminderView {
    pub fn reminder(reminder: FocusReminder, metrics: Arc<crate::metrics::Metrics>) -> Self {
        let message = match reminder {
            FocusReminder::FocusCompleted => "本轮专注已完成，休息一下。",
            FocusReminder::Standup => "坐太久啦，起来活动几分钟、喝点水。",
            _ => "休息结束，准备好再开始下一轮。",
        };
        Self {
            message: message.into(),
            metrics,
        }
    }
}
impl Render for ReminderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics.reminder_frames.fetch_add(1, Ordering::Relaxed);
        let message = self.message.clone();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_lg().child(message.into_owned()))
            .child(
                Button::new("dismiss")
                    .label("知道了")
                    .on_click(|_, window, _| window.remove_window()),
            )
    }
}
