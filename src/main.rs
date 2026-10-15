#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod desktop_layout;
mod feedback;
mod metrics;
mod pet;
mod quick_entry;
mod reminder_policy;
mod tray;
mod ui;
mod ui_policy;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{
    App, AppContext, Application, Bounds, Global, QuitMode, TitlebarOptions, WeakEntity,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    assets::AllAssets, point, px, size,
};
use lamar_pet::{
    focus::{FocusCommand, FocusPhase, FocusReminder},
    service::{Client, Command, Worker},
    sprites::SpriteAtlas,
};
use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

struct Owner {
    metrics: Arc<metrics::Metrics>,
    _instance: File,
    root: PathBuf,
    worker: Option<Worker>,
    client: Client,
    atlas: Rc<SpriteAtlas>,
    panel: Option<WindowHandle<Root>>,
    panel_view: Option<WeakEntity<ui::MainView>>,
    pet: Option<WindowHandle<pet::PetView>>,
    reminder: Option<WindowHandle<Root>>,
    tray: tray::Tray,
}
impl Global for Owner {}

fn main() {
    if let Err(error) = run() {
        if std::env::args().any(|arg| arg == "--smoke-seconds") {
            eprintln!("{error}");
        } else {
            show_error(&error);
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let smoke_seconds = args
        .windows(2)
        .find(|pair| pair[0] == "--smoke-seconds")
        .and_then(|pair| pair[1].parse::<u64>().ok());
    let root = args
        .windows(2)
        .find(|pair| pair[0] == "--data-dir")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| dirs::data_local_dir().map(|base| base.join("cn.lamarlog.lamar-pet.sprite")))
        .ok_or("Unable to locate local application data")?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let instance = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance.lock"))
        .map_err(|error| error.to_string())?;
    match instance.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(error) => return Err(error.to_string()),
    }
    let report_root = root.clone();
    let owner_root = root.clone();
    let worker = Worker::start(root)?;
    let client = worker.client();
    let report_client = client.clone();
    let smoke_resume = args.iter().any(|arg| arg == "--smoke-resume");
    let smoke_reminder = args.iter().any(|arg| arg == "--smoke-reminder");
    let metrics = Arc::new(metrics::Metrics::default());
    let report_metrics = metrics.clone();
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let report_failures = failures.clone();
    let atlas = Rc::new(SpriteAtlas::load()?);
    Application::with_platform(gpui_kit::platform::current_platform(false))
        .with_assets(AllAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            let tray = match tray::Tray::new(&atlas) {
                Ok(tray) => tray,
                Err(error) => {
                    failures.lock().unwrap().push(error.clone());
                    if smoke_seconds.is_none() {
                        show_error(&error);
                    }
                    cx.quit();
                    return;
                }
            };
            cx.set_global(Owner {
                metrics: metrics.clone(),
                _instance: instance,
                root: owner_root,
                worker: Some(worker),
                client: client.clone(),
                atlas,
                panel: None,
                panel_view: None,
                pet: None,
                reminder: None,
                tray,
            });
            if let Err(error) = show_panel(cx).and_then(|_| show_pet(cx)) {
                failures.lock().unwrap().push(error.clone());
                if smoke_seconds.is_none() {
                    show_error(&error);
                }
                cx.quit();
            }
            // Smoke hook: renders the reminder window and plays the chime once
            // without waiting for a real focus round to finish.
            if smoke_reminder {
                let _ = show_reminder(FocusReminder::FocusCompleted, cx);
                play_reminder_sound(client.snapshot().preferences.sound_enabled);
            }
            let shutdown_failures = failures.clone();
            cx.on_app_quit(move |cx| {
                let worker = cx.global_mut::<Owner>().worker.take();
                let shutdown_failures = shutdown_failures.clone();
                async move {
                    if let Some(worker) = worker
                        && let Err(error) = worker.shutdown()
                    {
                        shutdown_failures.lock().unwrap().push(error.clone());
                        if smoke_seconds.is_none() {
                            show_error(&error);
                        }
                    }
                }
            })
            .detach();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                let mut last_reminder = None;
                let mut last_phase: Option<FocusPhase> = None;
                let mut last_standup: Option<u64> = None;
                let mut resumed = false;
                let mut panel_topmost_set = false;
                loop {
                    executor.timer(Duration::from_millis(100)).await;
                    if !cx.update(|cx| cx.has_global::<Owner>()) {
                        break;
                    }
                    let snapshot = client.snapshot();
                    if smoke_resume && !resumed && !snapshot.load_pending {
                        let _ = client.send(Command::Focus(FocusCommand::Resume));
                        resumed = true;
                    }
                    cx.update(|cx| {
                        let panel = cx.global::<Owner>().panel;
                        let panel_alive =
                            panel.is_some_and(|handle| handle.update(cx, |_, _, _| ()).is_ok());
                        if panel_alive {
                            // The pet is permanently topmost, so the panel
                            // joins the same band once to stay above it. The
                            // pet is non-activatable and can never rise back
                            // over the panel, while still floating above all
                            // normal app windows.
                            if !panel_topmost_set && let Some(panel) = panel {
                                let _ = panel.update(cx, |_, window, _| {
                                    crate::pet::set_topmost(window, true)
                                });
                                panel_topmost_set = true;
                            }
                        } else {
                            let owner = cx.global_mut::<Owner>();
                            owner.panel = None;
                            owner.panel_view = None;
                            panel_topmost_set = false;
                        }
                    });
                    let actions = cx.update(|cx| {
                        let owner = cx.global::<Owner>();
                        owner.tray.update(&snapshot);
                        owner.tray.actions()
                    });
                    for action in actions {
                        match action {
                            tray::Action::Panel => {
                                let _ = cx.update(show_panel);
                            }
                            tray::Action::Pet => {
                                let _ = cx.update(show_pet);
                            }
                            tray::Action::Quit => cx.update(|cx| cx.quit()),
                            tray::Action::Stop => {
                                let _ = client.send(Command::Focus(FocusCommand::Stop));
                            }
                            tray::Action::Toggle => {
                                let action = match snapshot.focus.phase {
                                    FocusPhase::Idle => FocusCommand::Start,
                                    FocusPhase::Focus | FocusPhase::Break => FocusCommand::Pause,
                                    _ => FocusCommand::Resume,
                                };
                                let _ = client.send(Command::Focus(action));
                            }
                        }
                    }
                    if !snapshot.load_pending {
                        if last_reminder
                            .is_some_and(|previous| previous != snapshot.focus.reminder_sequence)
                            && reminder_policy::reminder_applicable(&snapshot.focus)
                        {
                            let delivered =
                                cx.update(|cx| show_reminder(snapshot.focus.reminder, cx).is_ok());
                            if !delivered {
                                continue;
                            }
                            play_reminder_sound(snapshot.preferences.sound_enabled);
                        }
                        last_reminder = Some(snapshot.focus.reminder_sequence);
                        if last_standup.is_some_and(|seen| seen != snapshot.standup_sequence)
                            && snapshot.standup_sequence > 0
                        {
                            let _ = cx.update(|cx| {
                                show_reminder(lamar_pet::focus::FocusReminder::Standup, cx)
                            });
                            play_reminder_sound(snapshot.preferences.sound_enabled);
                        }
                        last_standup = Some(snapshot.standup_sequence);
                        if snapshot.focus.phase == FocusPhase::Focus
                            && last_phase == Some(FocusPhase::Idle)
                        {
                            let minutes = snapshot.focus.focus_minutes;
                            let _ = cx.update(|cx| {
                                show_hint(
                                    format!(
                                        "专注 {minutes} 分钟开始，猫猫陪你。右键桌宠可以暂停。"
                                    ),
                                    cx,
                                )
                            });
                        }
                        last_phase = Some(snapshot.focus.phase);
                    }
                }
            })
            .detach();
            if let Some(seconds) = smoke_seconds {
                let executor = cx.background_executor().clone();
                cx.spawn(async move |cx| {
                    executor.timer(Duration::from_secs(seconds)).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
    let errors = report_failures.lock().unwrap().clone();
    if smoke_seconds.is_some() {
        let snapshot = report_client.snapshot();
        let report = serde_json::json!({
            "panel_frames": report_metrics.panel_frames.load(Ordering::Relaxed),
            "pet_frames": report_metrics.pet_frames.load(Ordering::Relaxed),
            "reminder_frames": report_metrics.reminder_frames.load(Ordering::Relaxed),
            "phase": format!("{:?}", snapshot.focus.phase),
            "reminder_sequence": snapshot.focus.reminder_sequence,
            "rounds": snapshot.focus.today_completed_rounds,
            "storage_error": snapshot.storage_error,
            "load_pending": snapshot.load_pending,
            "command_error": snapshot.command_error,
            "errors": errors,
        });
        fs::write(
            report_root.join("smoke-report.json"),
            serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    }
    if let Some(error) = errors.first() {
        return Err(error.clone());
    }
    Ok(())
}

fn show_panel(cx: &mut App) -> Result<(), String> {
    if let Some(handle) = cx.global::<Owner>().panel
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(());
    }
    let owner = cx.global::<Owner>();
    let client = owner.client.clone();
    let atlas = owner.atlas.clone();
    let metrics = owner.metrics.clone();
    let data_dir: Rc<std::path::Path> = owner.root.as_path().into();
    let height = cx
        .primary_display()
        .map(|display| (f32::from(display.bounds().size.height) - 100.0).clamp(580.0, 750.0))
        .unwrap_or(750.0);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(820.0), px(height)),
                    cx,
                ))),
                window_min_size: Some(size(px(660.0), px(580.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lamar Pet".into()),
                    ..Default::default()
                }),
                show: true,
                focus: true,
                ..Default::default()
            },
            move |window, cx| {
                ui::sync_theme(client.snapshot().preferences.appearance, window, cx);
                let view =
                    cx.new(|cx| ui::MainView::new(client, atlas, metrics, data_dir, window, cx));
                cx.global_mut::<Owner>().panel_view = Some(view.downgrade());
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .map_err(|error| error.to_string())?;
    cx.global_mut::<Owner>().panel = Some(handle);
    Ok(())
}

fn show_task_panel(cx: &mut App) -> Result<(), String> {
    show_panel(cx)?;
    let owner = cx.global::<Owner>();
    let panel = owner.panel;
    let view = owner.panel_view.as_ref().and_then(|v| v.upgrade());
    if let (Some(panel), Some(view)) = (panel, view) {
        panel
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.open_tasks(window, cx))
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn show_pet(cx: &mut App) -> Result<(), String> {
    if let Some(handle) = cx.global::<Owner>().pet
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(());
    }
    let owner = cx.global::<Owner>();
    let client = owner.client.clone();
    let atlas = owner.atlas.clone();
    let metrics = owner.metrics.clone();
    let scale = client.snapshot().preferences.scale_percent;
    let geometry = desktop_layout::geometry(scale, client.snapshot().preferences.show_todos, false);
    let bounds = cx
        .primary_display()
        .map(|display| {
            let display = display.bounds();
            Bounds::new(
                point(
                    display.origin.x + display.size.width - px(geometry.width + 30.0),
                    display.origin.y + display.size.height - px(geometry.height + 70.0),
                ),
                size(px(geometry.width), px(geometry.height)),
            )
        })
        .unwrap_or_else(|| {
            Bounds::centered(None, size(px(geometry.width), px(geometry.height)), cx)
        });
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                window_background: WindowBackgroundAppearance::Transparent,
                // A normal host is useful for accessibility and screenshot
                // validation of this same component; daily use stays a popup.
                kind: if std::env::args().any(|arg| arg == "--preview-widget") {
                    WindowKind::Normal
                } else {
                    WindowKind::PopUp
                },
                show: true,
                focus: false,
                is_resizable: false,
                inactive_frame_interval: None,
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(&format!("{} · 桌宠", client.snapshot().preferences.name));
                let callback = Rc::new(|cx: &mut App| {
                    let _ = show_task_panel(cx);
                });
                cx.new(|cx| pet::PetView::new(client, atlas, callback, metrics, cx))
            },
        )
        .map_err(|error| error.to_string())?;
    cx.global_mut::<Owner>().pet = Some(handle);
    Ok(())
}

/// Plays the bundled chime from memory; no-op when the user disabled sound.
fn play_reminder_sound(enabled: bool) {
    if !enabled {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        #[repr(align(2))]
        struct Wav(&'static [u8]);
        static ALERT: Wav = Wav(include_bytes!("../assets/alert.wav"));
        let _ = PlaySoundW(
            windows::core::PCWSTR(ALERT.0.as_ptr() as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
    #[cfg(target_os = "macos")]
    {
        static ALERT: &[u8] = include_bytes!("../assets/alert.wav");
        let path = std::env::temp_dir().join("lamar-pet-alert.wav");
        if !path.exists() {
            let _ = std::fs::write(&path, ALERT);
        }
        let _ = std::process::Command::new("afplay").arg(&path).spawn();
    }
}

fn show_reminder(reminder: FocusReminder, cx: &mut App) -> Result<(), String> {
    let metrics = cx.global::<Owner>().metrics.clone();
    let image = cx.global::<Owner>().atlas.frames.last().cloned();
    if let Some(handle) = cx.global_mut::<Owner>().reminder.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(400.0), px(200.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lamar Pet · 提醒".into()),
                    ..Default::default()
                }),
                show: true,
                focus: false,
                is_resizable: false,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|_| ui::ReminderView::reminder(reminder, metrics, image));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .map_err(|error| error.to_string())?;
    cx.global_mut::<Owner>().reminder = Some(handle);
    Ok(())
}

/// Transient toast shown when a focus round begins; dismisses itself.
fn show_hint(message: String, cx: &mut App) -> Result<(), String> {
    let metrics = cx.global::<Owner>().metrics.clone();
    if let Some(handle) = cx.global_mut::<Owner>().reminder.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(360.0), px(150.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lamar Pet".into()),
                    ..Default::default()
                }),
                show: true,
                focus: false,
                is_resizable: false,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|_| ui::ReminderView {
                    message: message.into(),
                    metrics,
                    image: None,
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .map_err(|error| error.to_string())?;
    cx.global_mut::<Owner>().reminder = Some(handle);
    let closer = cx.global::<Owner>().reminder;
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        executor.timer(Duration::from_secs(5)).await;
        if let Some(closer) = closer {
            let _ = closer.update(cx, |_, window, _| window.remove_window());
        }
    })
    .detach();
    Ok(())
}

fn show_error(message: &str) {
    #[cfg(windows)]
    {
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
            core::HSTRING,
        };
        // SAFETY: strings and ownerless dialog live for the duration of this synchronous call.
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(message),
                &HSTRING::from("Lamar Pet"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}
