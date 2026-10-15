use crate::feedback::{FeedbackTracker, Kind};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Animation, AnimationExt, App, Context, CursorStyle, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Render, Window, div, img, prelude::*, px,
};
use lamar_pet::{
    focus::{FocusCommand, FocusPhase},
    service::{Client, Command},
    sprites::SpriteAtlas,
    todo::{TodoCommand, TodoPriority},
};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// gpui's `Window::start_window_move` is a no-op on Windows, so the pet
/// drags itself. Dragging is tracked entirely in physical pixels — cursor
/// offset inside the window rect — because gpui's logical coordinate space
/// can go stale across `WM_DPICHANGED` (its stored scale factor may not
/// match the monitor the window is actually on), which would silently
/// misplace every move.
#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut cursor = POINT::default();
    // SAFETY: the point buffer is valid for the duration of this call.
    unsafe { GetCursorPos(&mut cursor) }.ok()?;
    Some((cursor.x, cursor.y))
}

#[cfg(windows)]
fn window_rect(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};
    let mut rect = RECT::default();
    // SAFETY: rect is a valid out buffer; hwnd belongs to this process.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    ))
}

/// Effective physical/logical metrics for the window, measured live instead
/// of trusting `window.scale_factor()` (stale after `WM_DPICHANGED`) or
/// `bounds()` (client area only — the outer frame is larger by the border).
/// Returns (eff_x, eff_y, border_w, border_h): multiply logical deltas by
/// eff for physical pixels; add border for client->window sizes.
#[cfg(windows)]
fn frame_metrics(
    hwnd: windows::Win32::Foundation::HWND,
    window: &Window,
) -> Option<(f32, f32, f32, f32)> {
    use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetClientRect};
    let (_, _, win_w, win_h) = window_rect(hwnd)?;
    let mut client = RECT::default();
    // SAFETY: client is a valid out buffer; hwnd belongs to this process.
    unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
    let client_w = (client.right - client.left) as f32;
    let client_h = (client.bottom - client.top) as f32;
    let bounds = window.bounds();
    let logical_w = f32::from(bounds.size.width);
    let logical_h = f32::from(bounds.size.height);
    if client_w <= 0.0 || client_h <= 0.0 || logical_w <= 0.0 || logical_h <= 0.0 {
        return None;
    }
    Some((
        client_w / logical_w,
        client_h / logical_h,
        win_w as f32 - client_w,
        win_h as f32 - client_h,
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

/// The pet keeps WS_EX_NOACTIVATE so grabbing it can never raise it above the
/// control panel: while the panel exists it is itself topmost, and a window
/// that cannot be activated stays below it in the topmost band. That lets the
/// pet float over every normal app window without ever eating panel clicks
/// through its transparent pixels.
#[cfg(windows)]
fn disable_activation(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE,
        },
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut core::ffi::c_void);
    // SAFETY: hwnd is this window's own handle; only the ex-style bit changes.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE.0 as isize);
    }
}

/// `set_topmost` toggles the WS_EX_TOPMOST band, used to float the control
/// panel above the pet while it is open.
#[cfg(windows)]
pub(crate) fn set_topmost(window: &Window, on_top: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        },
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut core::ffi::c_void);
    let insert_after = if on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
    // SAFETY: hwnd is this window's own handle; only the z-order band changes.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(insert_after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
pub(crate) fn set_topmost(_window: &Window, _on_top: bool) {}

/// Window geometry requests applied on a dedicated thread. Calling
/// `SetWindowPos` from a gpui callback synchronously re-enters the window
/// procedure, and when a move crosses a monitor boundary `WM_MOVE` fires
/// `display_changed` while `&mut App` is still borrowed — the nested update
/// aborts the process. A call issued from another thread is queued onto the
/// owning thread's message pump instead, so the window procedure runs only
/// after the borrow is released.
#[cfg(windows)]
mod native_move {
    use std::sync::mpsc::{Sender, channel};
    use std::thread;
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        },
    };

    /// One full geometry request in physical pixels, already resolved on the
    /// UI thread where gpui's coordinate state is available; `size: None`
    /// keeps the current size.
    pub struct Request {
        pub hwnd: usize,
        pub x: i32,
        pub y: i32,
        pub size: Option<(i32, i32)>,
    }

    pub struct WindowMover(Sender<Request>);

    impl WindowMover {
        pub fn spawn() -> Self {
            let (sender, receiver) = channel::<Request>();
            thread::spawn(move || {
                while let Ok(mut request) = receiver.recv() {
                    // Drags emit a burst of positions; only the newest matters.
                    while let Ok(newer) = receiver.try_recv() {
                        request = newer;
                    }
                    let flags = if request.size.is_some() {
                        SET_WINDOW_POS_FLAGS(0)
                    } else {
                        SWP_NOSIZE
                    } | SWP_NOACTIVATE
                        | SWP_NOZORDER;
                    let (width, height) = request.size.unwrap_or_default();
                    // SAFETY: hwnd was captured from this process's own live
                    // window; the thread only repositions or resizes it.
                    unsafe {
                        let _ = SetWindowPos(
                            HWND(request.hwnd as *mut core::ffi::c_void),
                            None,
                            request.x,
                            request.y,
                            width,
                            height,
                            flags,
                        );
                    }
                }
            });
            Self(sender)
        }

        pub fn enqueue(&self, request: Request) {
            let _ = self.0.send(request);
        }
    }
}

#[cfg(windows)]
fn native_hwnd(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // `Window::window_handle` is an inherent gpui method; disambiguate to the
    // raw-window-handle trait to reach the OS-level HWND.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return None;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return None;
    };
    Some(windows::Win32::Foundation::HWND(
        raw.hwnd.get() as *mut core::ffi::c_void
    ))
}

#[cfg(windows)]
fn hwnd_bits(window: &Window) -> Option<usize> {
    native_hwnd(window).map(|hwnd| hwnd.0 as usize)
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
    drag_grab: Option<(i32, i32)>,
    drag_moved: bool,
    native_tweaks_done: bool,
    last_name: String,
    last_layout: Option<(u16, bool, bool, u32)>,
    direction: i32,
    frame: usize,
    compact: bool,
    pending: Option<u64>,
    just_completed: Option<(u64, Instant)>,
    send_error: bool,
    feedback: FeedbackTracker,
    layout_generation: u64,
    #[cfg(windows)]
    mover: native_move::WindowMover,
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
                executor.timer(Duration::from_millis(40)).await;
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
        let last_name = client.snapshot().preferences.name.clone();
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
            native_tweaks_done: false,
            last_name,
            last_layout: None,
            direction: 0,
            frame: 0,
            compact: false,
            pending: None,
            just_completed: None,
            send_error: false,
            feedback: FeedbackTracker::default(),
            layout_generation: 0,
            #[cfg(windows)]
            mover: native_move::WindowMover::spawn(),
        }
    }

    fn advance(&mut self) {
        let snapshot = self.client.snapshot();
        if self
            .pending
            .is_some_and(|ticket| snapshot.last_ack >= ticket)
        {
            self.pending = None;
            if let Some(feedback) = self.feedback.observe(
                snapshot.last_ack,
                snapshot.command_error.is_none() && !snapshot.storage_error,
                Instant::now(),
            ) {
                match feedback.kind {
                    Kind::Complete => {
                        self.just_completed = Some((feedback.task_id, Instant::now()));
                        self.celebrate_until = Instant::now() + Duration::from_millis(900);
                    }
                    Kind::Undo => self.just_completed = None,
                    Kind::Focus => {}
                }
            }
        }
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

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let effect = match &command {
            Command::Todo(TodoCommand::Toggle(id)) => self
                .client
                .snapshot()
                .todo
                .tasks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| (*id, if t.done { Kind::Undo } else { Kind::Complete })),
            Command::StartTodoFocus(id) => Some((*id, Kind::Focus)),
            _ => None,
        };
        match self.client.send(command) {
            Ok(ticket) => {
                self.pending = Some(ticket);
                self.send_error = false;
                if let Some((id, kind)) = effect {
                    self.feedback.begin(ticket, id, kind);
                }
            }
            Err(_) => self.send_error = true,
        }
        cx.notify();
    }

    #[cfg(windows)]
    fn follow_drag(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if let Some((grab_x, grab_y)) = self.drag_grab
            && let Some((x, y)) = cursor_position()
            && let Some(hwnd) = hwnd_bits(window)
        {
            self.drag_moved = true;
            self.mover.enqueue(native_move::Request {
                hwnd,
                x: x - grab_x,
                y: y - grab_y,
                size: None,
            });
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
        if !self.native_tweaks_done {
            self.native_tweaks_done = true;
            #[cfg(windows)]
            {
                clear_window_shadow(window);
                disable_activation(window);
            }
        }
        let snapshot = self.client.snapshot();
        crate::ui::sync_theme(snapshot.preferences.appearance, window, cx);
        if snapshot.preferences.name != self.last_name {
            window.set_window_title(&format!("{} · 桌宠", snapshot.preferences.name));
            self.last_name = snapshot.preferences.name.clone();
        }
        let layout = crate::desktop_layout::geometry(
            snapshot.preferences.scale_percent,
            snapshot.preferences.show_todos,
            self.compact,
        );
        // The signature carries the display scale so a drag onto a monitor
        // with a different DPI re-applies the layout once, and a persistent
        // size mismatch (e.g. a stale WM_DPICHANGED suggestion) is corrected
        // the same way. Corrections anchor the top-left corner — the sprite
        // stays put and the note grows or shrinks beneath it — instead of
        // bottom-right anchoring, which visibly shifts the cat on every
        // expand/collapse toggle.
        let signature = (
            snapshot.preferences.scale_percent,
            snapshot.preferences.show_todos,
            self.compact,
            (window.scale_factor() * 256.0) as u32,
        );
        let signature_changed = self.last_layout.replace(signature) != Some(signature);
        #[cfg(windows)]
        {
            let bounds = window.bounds();
            let size_off = (f32::from(bounds.size.width) - layout.width).abs() > 2.0
                || (f32::from(bounds.size.height) - layout.height).abs() > 2.0;
            let display = window.display(cx);
            let d = display.map(|display| display.bounds());
            let visible = d.is_some_and(|d| {
                let b = bounds;
                let margin = 16.0_f32;
                f32::from(b.origin.x) < f32::from(d.origin.x + d.size.width) - margin
                    && f32::from(b.origin.x + b.size.width) > f32::from(d.origin.x) + margin
                    && f32::from(b.origin.y) < f32::from(d.origin.y + d.size.height) - margin
                    && f32::from(b.origin.y + b.size.height) > f32::from(d.origin.y) + margin
            });
            let mut logical_target: Option<(f32, f32)> = None;
            if visible && (signature_changed || size_off) {
                let mut x = f32::from(bounds.origin.x);
                let mut y = f32::from(bounds.origin.y);
                if let Some(d) = d {
                    let min_x = f32::from(d.origin.x) + 8.0;
                    let min_y = f32::from(d.origin.y) + 8.0;
                    x = x.clamp(
                        min_x,
                        (f32::from(d.origin.x + d.size.width) - layout.width - 8.0).max(min_x),
                    );
                    y = y.clamp(
                        min_y,
                        (f32::from(d.origin.y + d.size.height) - layout.height - 54.0).max(min_y),
                    );
                }
                logical_target = Some((x, y));
            }
            if (!visible || logical_target.is_some())
                && let Some(hwnd) = native_hwnd(window)
                && let Some((eff_x, eff_y, border_w, border_h)) = frame_metrics(hwnd, window)
                && let Some((win_x, win_y, win_w, win_h)) = window_rect(hwnd)
            {
                let size = Some((
                    (layout.width * eff_x + border_w) as i32,
                    (layout.height * eff_y + border_h) as i32,
                ));
                // Same re-entrancy constraint as follow_drag: resize/move go
                // through SetWindowPos, so both run on the mover thread.
                if !visible {
                    // Off-screen recovery: compute the home spot from the
                    // monitor's physical work area — gpui's logical space may
                    // not match this display's scale right now.
                    use windows::Win32::Graphics::Gdi::{
                        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
                    };
                    let mut info = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    // SAFETY: info is a correctly sized out buffer.
                    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
                    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
                        let area = info.rcWork;
                        self.mover.enqueue(native_move::Request {
                            hwnd: hwnd.0 as usize,
                            x: area.right - win_w - (30.0 * eff_x) as i32,
                            y: area.bottom - win_h - (70.0 * eff_y) as i32,
                            size,
                        });
                    }
                } else if let Some((x, y)) = logical_target {
                    self.mover.enqueue(native_move::Request {
                        hwnd: hwnd.0 as usize,
                        x: win_x + ((x - f32::from(bounds.origin.x)) * eff_x) as i32,
                        y: win_y + ((y - f32::from(bounds.origin.y)) * eff_y) as i32,
                        size,
                    });
                }
            }
        }
        #[cfg(not(windows))]
        if signature_changed {
            window.resize(gpui_kit::size(px(layout.width), px(layout.height)));
        }
        let side = layout.pet_side;
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
            FocusPhase::PausedFocus | FocusPhase::PausedBreak => "专注已暂停".into(),
            FocusPhase::Idle => "右键打开工作面板".into(),
        };
        let open_panel = self.open_panel.clone();
        let mut root = div()
            .id("desktop-pet")
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .on_mouse_move(
                cx.listener(|view, _: &MouseMoveEvent, window, cx| view.follow_drag(window, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, cx| view.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, cx| view.end_drag(cx)),
            )
            .on_mouse_down(MouseButton::Right, move |_, _, cx| open_panel(cx))
            .child(
                div()
                    .id("pet-drag-handle")
                    .cursor(if self.drag_grab.is_some() {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .w(px(side))
                    .h(px(side))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseDownEvent, window, _| {
                            view.drag_moved = false;
                            #[cfg(windows)]
                            {
                                view.drag_grab = native_hwnd(window)
                                    .and_then(window_rect)
                                    .zip(cursor_position())
                                    .map(|((wx, wy, _, _), (cx, cy))| (cx - wx, cy - wy));
                            }
                            #[cfg(not(windows))]
                            window.start_window_move();
                        }),
                    )
                    .child(
                        img(self.atlas.frames[self.frame].clone())
                            .w(px(side))
                            .h(px(side)),
                    ),
            );
        if !snapshot.preferences.show_todos {
            return root.child(
                div()
                    .rounded_full()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(label),
            );
        }
        let today = snapshot.focus.checkpoint.calendar_day.clone();
        let minute = time::OffsetDateTime::now_local()
            .map(|t| u16::from(t.hour()) * 60 + u16::from(t.minute()))
            .unwrap_or(0);
        let pending = snapshot.todo.pending_tasks(&today, minute);
        let count = pending.len();
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let surface = cx.theme().popover;
        let green = if cx.theme().mode.is_dark() {
            gpui_kit::rgb(0xa9c898)
        } else {
            gpui_kit::rgb(0x56764d)
        };
        let selected_bg = if cx.theme().mode.is_dark() {
            cx.theme().muted
        } else {
            gpui_kit::rgb(0xf0f4e9).into()
        };
        // Commands that arrive while one is in flight are dropped by `send`
        // anyway; disabling every button for the ~50ms ack window just makes
        // the whole note flash grey on each click.
        let blocked = snapshot.load_pending;
        let mut card = div()
            .id("desktop-work-note")
            .w(px(layout.width - 16.0))
            .p_3()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(surface)
            .text_color(foreground)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("今日待办"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{}项未完成", count)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("compact-note")
                            .ghost()
                            .cursor_pointer()
                            .outline()
                            .small()
                            .label(if self.compact { "展开" } else { "收起" })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.compact = !view.compact;
                                view.layout_generation = view.layout_generation.saturating_add(1);
                                cx.notify();
                            })),
                    ),
            );
        let limit = if self.compact { 1 } else { 3 };
        for (index, task) in pending.iter().take(limit).enumerate() {
            let id = task.id;
            let selected = snapshot.todo.selected_id == Some(id);
            let mut hint = if selected {
                if snapshot.focus.phase == FocusPhase::Idle {
                    "当前任务 · 待开始".into()
                } else {
                    format!("现在做 · {}", label)
                }
            } else if task.day < today {
                format!("{} 未完成", &task.day[5..])
            } else {
                match task.priority {
                    TodoPriority::High => "优先处理".into(),
                    TodoPriority::Low => "稍后处理".into(),
                    TodoPriority::Normal => "待处理".into(),
                }
            };
            if index == 0 && snapshot.todo.selected_id.is_none() {
                hint = format!("下一件 · {}", hint);
            }
            if let Some(due) = task.due_minute {
                hint = format!(
                    "{} · {:02}:{:02}{}",
                    hint,
                    due / 60,
                    due % 60,
                    if task.is_overdue(&today, minute) {
                        " 已逾期"
                    } else {
                        " 截止"
                    }
                );
            }
            let row = div()
                .id(("hud-task-row", id))
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .p_2()
                .rounded_lg()
                .hover(|style| style.bg(selected_bg))
                .when(selected, |row| row.bg(selected_bg))
                .child(
                    Button::new(("todo-done", id))
                        .ghost()
                        .outline()
                        .cursor_pointer()
                        .small()
                        .label("完成")
                        .disabled(blocked)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::Todo(TodoCommand::Toggle(id)), cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .line_height(px(18.0))
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(task.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .line_height(px(14.0))
                                .text_color(if selected { green.into() } else { muted })
                                .child(hint),
                        ),
                )
                .child(
                    Button::new(("todo-focus", id))
                        .success()
                        .outline()
                        .cursor_pointer()
                        .small()
                        .label(
                            if matches!(
                                snapshot.focus.phase,
                                FocusPhase::Break | FocusPhase::PausedBreak
                            ) {
                                "休息中"
                            } else if selected && snapshot.focus.phase == FocusPhase::Focus {
                                "暂停"
                            } else if selected && snapshot.focus.phase == FocusPhase::PausedFocus {
                                "继续"
                            } else if selected {
                                "开始"
                            } else {
                                "现在做"
                            },
                        )
                        .disabled(
                            blocked
                                || matches!(
                                    snapshot.focus.phase,
                                    FocusPhase::Break | FocusPhase::PausedBreak
                                ),
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            let phase = view.client.snapshot().focus.phase;
                            if selected && phase == FocusPhase::Focus {
                                view.send(Command::Focus(FocusCommand::Pause), cx)
                            } else {
                                view.send(Command::StartTodoFocus(id), cx)
                            }
                        })),
                );
            if let Some(feedback) = self
                .feedback
                .active(Instant::now())
                .filter(|f| f.task_id == id && f.kind == Kind::Focus)
            {
                card = card.child(
                    row.with_animation(
                        ("task-click", feedback.generation),
                        Animation::new(Duration::from_millis(200)),
                        |row, progress| row.opacity(0.65 + 0.35 * progress),
                    )
                    .into_any_element(),
                );
            } else {
                card = card.child(row.into_any_element());
            }
        }
        if count == 0 {
            card = card
                .child(div().py_3().text_sm().child("今天要做什么？"))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("添加第一项待办，它会一直留在这里。"),
                );
        }
        if !self.compact {
            if let Some((id, when)) = self.just_completed.filter(|(id, at)| {
                at.elapsed() < Duration::from_secs(6)
                    && snapshot.todo.tasks.iter().any(|t| t.id == *id && t.done)
            }) {
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().text_xs().text_color(green).child("已完成一项"))
                        .child(div().flex_1())
                        .child(
                            Button::new("undo-task")
                                .ghost()
                                .outline()
                                .cursor_pointer()
                                .small()
                                .label("撤销")
                                .disabled(blocked)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.send(Command::Todo(TodoCommand::Toggle(id)), cx);
                                })),
                        )
                        .with_animation(
                            (
                                "complete-confirmed",
                                when.duration_since(self.started).as_millis() as u64,
                            ),
                            Animation::new(Duration::from_millis(180)),
                            |notice, progress| notice.opacity(0.25 + 0.75 * progress),
                        ),
                )
            } else {
                let open = self.open_panel.clone();
                let manage = self.open_panel.clone();
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            Button::new("add-task")
                                .ghost()
                                .outline()
                                .cursor_pointer()
                                .small()
                                .label("＋ 添加待办")
                                .on_click(move |_, _, cx| open(cx)),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("manage-tasks")
                                .ghost()
                                .small()
                                .cursor_pointer()
                                .label(if count > 3 {
                                    format!("全部{count}项")
                                } else {
                                    "管理待办".into()
                                })
                                .on_click(move |_, _, cx| manage(cx)),
                        ),
                );
            }
        }
        if self.send_error || snapshot.storage_error || snapshot.command_error.is_some() {
            card = card.child(
                div()
                    .text_size(px(10.0))
                    .text_color(gpui_kit::rgb(0xad6448))
                    .child(if snapshot.storage_error {
                        "保存失败，请打开面板查看"
                    } else {
                        "操作未完成，请打开面板查看"
                    }),
            );
        }
        root = root.child(card.with_animation(
            ("note-layout", self.layout_generation),
            Animation::new(Duration::from_millis(180)),
            |card, progress| card.opacity(0.88 + 0.12 * progress),
        ));
        root
    }
}
