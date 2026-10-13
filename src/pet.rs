use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Context, Image, MouseButton, Render, Window, div, img, prelude::*, px, size};
use lamar_pet::{focus::FocusPhase, service::Client, sprites::SpriteAtlas};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct PetView {
    metrics: Arc<crate::metrics::Metrics>,
    client: Client,
    atlas: Rc<SpriteAtlas>,
    open_panel: Rc<dyn Fn(&mut App)>,
    started: Instant,
    active_until: Instant,
    celebrate_until: Instant,
    last_activity: u64,
    last_reminder: Option<u64>,
    last_mouse: Option<(i32, i32)>,
    direction: i32,
    frame: usize,
    scale: u16,
}

impl PetView {
    pub fn new(
        client: Client,
        atlas: Rc<SpriteAtlas>,
        open_panel: Rc<dyn Fn(&mut App)>,
        metrics: Arc<crate::metrics::Metrics>,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = Instant::now();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(80)).await;
                if this
                    .update(cx, |view, cx| {
                        view.advance();
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
            open_panel,
            started: now,
            active_until: now,
            celebrate_until: now,
            last_activity: 0,
            last_reminder: None,
            last_mouse: None,
            direction: 0,
            frame: 0,
            scale: 100,
        }
    }

    fn advance(&mut self) {
        let snapshot = self.client.snapshot();
        let now = Instant::now();
        if snapshot.activity_pulse != self.last_activity {
            self.active_until = now + Duration::from_millis(500);
            self.last_activity = snapshot.activity_pulse;
            self.direction = 0;
        } else if let (Some(previous), Some(current)) = (self.last_mouse, snapshot.mouse) {
            let delta = current.0.saturating_sub(previous.0);
            if delta.abs() > 4 {
                self.direction = delta.signum();
                self.active_until = now + Duration::from_millis(300);
            }
        }
        self.last_mouse = snapshot.mouse;
        if !snapshot.load_pending {
            if self
                .last_reminder
                .is_some_and(|sequence| sequence != snapshot.focus.reminder_sequence)
            {
                self.celebrate_until = now + Duration::from_secs(3);
            }
            self.last_reminder = Some(snapshot.focus.reminder_sequence);
        }
        let elapsed = self.started.elapsed().as_millis();
        self.frame = if now < self.celebrate_until {
            8
        } else if matches!(
            snapshot.focus.phase,
            FocusPhase::Break | FocusPhase::PausedBreak
        ) {
            7
        } else if now < self.active_until {
            match self.direction {
                -1 => 5,
                1 => 6,
                _ => 3 + (snapshot.activity_pulse % 2) as usize,
            }
        } else if elapsed % 6000 < 200 {
            1
        } else if elapsed % 1500 < 700 {
            0
        } else {
            2
        };
    }
}

impl Render for PetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics
            .pet_frames
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::ui::sync_theme(self.client.snapshot().preferences.appearance, window, cx);
        let snapshot = self.client.snapshot();
        let scale = snapshot.preferences.scale_percent;
        let side = 180.0 * f32::from(scale) / 100.0;
        if scale != self.scale {
            window.resize(size(px(side), px(side + 28.0)));
            self.scale = scale;
        }
        let label = match snapshot.focus.phase {
            FocusPhase::Focus => format!(
                "专注 {:02}:{:02}",
                snapshot.focus.remaining_seconds / 60,
                snapshot.focus.remaining_seconds % 60
            ),
            FocusPhase::Break => format!(
                "休息 {:02}:{:02}",
                snapshot.focus.remaining_seconds / 60,
                snapshot.focus.remaining_seconds % 60
            ),
            FocusPhase::PausedFocus | FocusPhase::PausedBreak => "已暂停".into(),
            FocusPhase::Idle => "右键打开面板".into(),
        };
        let image: Arc<Image> = self.atlas.frames[self.frame].clone();
        let open_panel = self.open_panel.clone();
        div()
            .id("desktop-pet")
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .on_mouse_down(MouseButton::Right, move |_, _, cx| open_panel(cx))
            .child(img(image).w(px(side)).h(px(side)))
            .child(
                div()
                    .rounded_full()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(label),
            )
    }
}
