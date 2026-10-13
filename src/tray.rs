use lamar_pet::{focus::FocusPhase, service::AppSnapshot, sprites::SpriteAtlas};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum Action {
    Panel,
    Pet,
    Toggle,
    Stop,
    Quit,
}
pub struct Tray {
    _icon: TrayIcon,
    _menu: Menu,
    toggle: MenuItem,
    stop: MenuItem,
}

impl Tray {
    pub fn new(atlas: &SpriteAtlas) -> Result<Self, String> {
        let menu = Menu::new();
        let panel = MenuItem::with_id("lamar.panel", "打开面板", true, None);
        let pet = MenuItem::with_id("lamar.pet", "显示桌宠", true, None);
        let toggle = MenuItem::with_id("lamar.toggle", "开始专注", true, None);
        let stop = MenuItem::with_id("lamar.stop", "结束本轮", false, None);
        let quit = MenuItem::with_id("lamar.quit", "退出", true, None);
        menu.append_items(&[
            &panel,
            &pet,
            &PredefinedMenuItem::separator(),
            &toggle,
            &stop,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|error| error.to_string())?;
        let icon =
            Icon::from_rgba(atlas.icon_rgba.clone(), 32, 32).map_err(|error| error.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_id("lamar-pet")
            .with_icon(icon)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .with_tooltip("Lamar Pet")
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _icon: tray,
            _menu: menu,
            toggle,
            stop,
        })
    }

    pub fn update(&self, snapshot: &AppSnapshot) {
        let _ = self._icon.set_tooltip(Some(&snapshot.preferences.name));
        self.toggle.set_text(match snapshot.focus.phase {
            FocusPhase::Idle => "开始专注",
            FocusPhase::Focus => "暂停专注",
            FocusPhase::Break => "暂停休息",
            _ => "继续计时",
        });
        self.toggle.set_enabled(!snapshot.load_pending);
        self.stop
            .set_enabled(!snapshot.load_pending && snapshot.focus.phase != FocusPhase::Idle);
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = match event.id.0.as_str() {
                "lamar.panel" => Some(Action::Panel),
                "lamar.pet" => Some(Action::Pet),
                "lamar.toggle" => Some(Action::Toggle),
                "lamar.stop" => Some(Action::Stop),
                "lamar.quit" => Some(Action::Quit),
                _ => None,
            } {
                actions.push(action);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                actions.push(Action::Panel);
            }
        }
        actions
    }
}
