use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
};
use gpui_kit::{App, Context, Render, Window, div, img, prelude::*, px};
use lamar_pet::{
    focus::{FocusCommand, FocusPhase, FocusReminder},
    service::{AppSnapshot, Appearance, Client, Command},
    sprites::SpriteAtlas,
};
use std::sync::{Arc, atomic::Ordering};
use std::{rc::Rc, time::Duration};

pub struct MainView {
    metrics: Arc<crate::metrics::Metrics>,
    client: Client,
    atlas: Rc<SpriteAtlas>,
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
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = client.snapshot();
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
            snapshot,
            send_error: false,
            pending: None,
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
                                    .child("Lamar Pet"),
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
            );
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
                    .child(format!("{} 分钟", value))
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
        content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("左键拖动桌宠，右键打开面板。数据仅保存在本机。"),
        )
    }
}

pub struct ReminderView {
    pub reminder: FocusReminder,
    pub metrics: Arc<crate::metrics::Metrics>,
}
impl Render for ReminderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics.reminder_frames.fetch_add(1, Ordering::Relaxed);
        let message = if self.reminder == FocusReminder::FocusCompleted {
            "本轮专注已完成，休息一下。"
        } else {
            "休息结束，准备好再开始下一轮。"
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_lg().child(message))
            .child(
                Button::new("dismiss")
                    .label("知道了")
                    .on_click(|_, window, _| window.remove_window()),
            )
    }
}
