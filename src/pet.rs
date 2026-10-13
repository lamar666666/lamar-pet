use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, Context, Image, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, Window,
    div, img, prelude::*, px, size,
};
use lamar_pet::{focus::FocusPhase, service::Client, sprites::SpriteAtlas};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// gpui's `Window::start_window_move` is a no-op on Windows, so the pet
/// drags itself: remember where the cursor grabbed the window, then apply
/// `SetWindowPos` while the button stays down.
#[cfg(windows)]
fn cursor_position(window: &Window) -> Option<(f32, f32)> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut cursor = POINT::default();
    // SAFETY: the point buffer is valid for the duration of this call.
    unsafe { GetCursorPos(&mut cursor) }.ok()?;
    let scale = f64::from(window.scale_factor());
    Some((
        (f64::from(cursor.x) / scale) as f32,
        (f64::from(cursor.y) / scale) as f32,
    ))
}

/// Tool/popup windows still pick up a DWM drop shadow on Windows 11; the pet
/// needs to sit flat on the wallpaper, so non-client rendering is disabled.
#[cfg(windows)]
fn clear_window_shadow(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY, DwmSetWindowAttribute},
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut core::ffi::c_void);
    let policy = DWMNCRP_DISABLED;
    // SAFETY: hwnd is this window's own handle and the policy pointer is a
    // valid u32 for the duration of the call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const _,
            std::mem::size_of_val(&policy) as u32,
        );
    }
}

#[cfg(windows)]
fn move_window(window: &Window, x: f32, y: f32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos},
    };
    // `Window::window_handle` is an inherent gpui method; disambiguate to the
    // raw-window-handle trait to reach the OS-level HWND.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut core::ffi::c_void);
    let scale = window.scale_factor();
    // SAFETY: hwnd comes from this process's own live window; the call only
    // repositions it without touching size, z-order or activation.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            (x * scale) as i32,
            (y * scale) as i32,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

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
    drag_grab: Option<(f32, f32)>,
    drag_moved: bool,
    shadow_cleared: bool,
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
            drag_grab: None,
            drag_moved: false,
            shadow_cleared: false,
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

    /// Released without moving counts as a pat: the cat celebrates briefly.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_grab.is_some() && !self.drag_moved {
            self.celebrate_until = Instant::now() + Duration::from_millis(800);
        }
        self.drag_grab = None;
        self.drag_moved = false;
        cx.notify();
    }

    #[cfg(windows)]
    fn follow_drag(&mut self, window: &mut Window) {
        if let Some((grab_x, grab_y)) = self.drag_grab
            && let Some((x, y)) = cursor_position(window)
        {
            self.drag_moved = true;
            move_window(window, x - grab_x, y - grab_y);
        }
    }

    #[cfg(not(windows))]
    fn follow_drag(&mut self, _window: &mut Window) {}
}

impl Render for PetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.metrics
            .pet_frames
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !self.shadow_cleared {
            self.shadow_cleared = true;
            #[cfg(windows)]
            clear_window_shadow(window);
        }
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, window, _| {
                    view.drag_moved = false;
                    #[cfg(windows)]
                    {
                        view.drag_grab = cursor_position(window).map(|(x, y)| {
                            let origin = window.bounds().origin;
                            (x - f32::from(origin.x), y - f32::from(origin.y))
                        });
                    }
                    #[cfg(not(windows))]
                    window.start_window_move();
                }),
            )
            .on_mouse_move(cx.listener(|view, _: &MouseMoveEvent, window, _| {
                view.follow_drag(window);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, cx| view.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, cx| view.end_drag(cx)),
            )
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
