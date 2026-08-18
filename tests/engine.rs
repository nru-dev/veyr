use std::sync::{Arc, Mutex};

use veyr::engine::{
    DeveloperApi, DeveloperPlugin, Engine, PluginError, PluginId, PluginPhase, PluginResult,
};
use veyr::offsets::{
    api::{
        BoolSetting, ButtonId, Color, DeveloperMenu, FloatRange, FloatSetting, Memory, MenuCommand,
        MenuPlacement, PageId, PluginRenderFrame, Position, RenderCommand, ScreenPoint, Stroke,
        WorldCircleStyle,
    },
    RemoteAddress,
};

struct NoMemory;

impl Memory for NoMemory {
    type Error = ();

    fn read_u8(&self, _address: RemoteAddress) -> Result<u8, Self::Error> {
        Err(())
    }

    fn read_u32(&self, _address: RemoteAddress) -> Result<u32, Self::Error> {
        Err(())
    }

    fn read_u64(&self, _address: RemoteAddress) -> Result<u64, Self::Error> {
        Err(())
    }

    fn read_f32(&self, _address: RemoteAddress) -> Result<f32, Self::Error> {
        Err(())
    }
}

struct RecordingPlugin {
    id: PluginId,
    calls: Arc<Mutex<Vec<&'static str>>>,
    fails_on_update: bool,
    fails_on_render: bool,
}

impl DeveloperPlugin<NoMemory> for RecordingPlugin {
    fn id(&self) -> PluginId {
        self.id
    }

    fn on_load(&mut self, _api: &DeveloperApi<'_, '_, NoMemory>) -> PluginResult<()> {
        self.calls
            .lock()
            .expect("test calls lock")
            .push(self.id.as_str());
        Ok(())
    }

    fn on_update(&mut self, _api: &DeveloperApi<'_, '_, NoMemory>) -> PluginResult<()> {
        self.calls
            .lock()
            .expect("test calls lock")
            .push(self.id.as_str());
        if self.fails_on_update {
            return Err(PluginError::Rejected("expected test failure"));
        }
        Ok(())
    }

    fn on_render(
        &mut self,
        _api: &DeveloperApi<'_, '_, NoMemory>,
        frame: &mut PluginRenderFrame<'_>,
    ) -> PluginResult<()> {
        self.calls
            .lock()
            .expect("test calls lock")
            .push(self.id.as_str());
        frame.world().glow_circle(
            Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                rotation: 0.0,
            },
            30.0,
            Color::CYAN,
        );
        frame.hud().circle(
            ScreenPoint::new(40.0, 50.0),
            20.0,
            Stroke::new(Color::CYAN, 2.0),
        );
        if self.fails_on_render {
            return Err(PluginError::Rejected("expected render failure"));
        }
        Ok(())
    }
}

#[test]
fn engine_runs_plugins_in_lifecycle_order_and_isolates_a_failure() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(NoMemory);

    engine
        .register_plugin(RecordingPlugin {
            id: PluginId::new("dev.good"),
            calls: Arc::clone(&calls),
            fails_on_update: false,
            fails_on_render: false,
        })
        .expect("first plugin registration");
    engine
        .register_plugin(RecordingPlugin {
            id: PluginId::new("dev.failing"),
            calls: Arc::clone(&calls),
            fails_on_update: true,
            fails_on_render: false,
        })
        .expect("second plugin registration");

    let update = engine.update();
    assert_eq!(update.failures().len(), 1);
    assert_eq!(update.failures()[0].plugin, PluginId::new("dev.failing"));
    assert_eq!(update.failures()[0].phase, PluginPhase::Update);
    assert_eq!(engine.active_plugin_count(), 1);
    assert_eq!(
        calls.lock().expect("test calls lock").as_slice(),
        ["dev.good", "dev.failing", "dev.good", "dev.failing"]
    );

    let render = engine.render();
    assert!(render.is_clean());
    assert_eq!(
        calls.lock().expect("test calls lock").as_slice(),
        [
            "dev.good",
            "dev.failing",
            "dev.good",
            "dev.failing",
            "dev.good"
        ]
    );
    let commands = engine.take_render_commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].plugin, PluginId::new("dev.good"));
    assert_eq!(
        commands[0].command,
        RenderCommand::WorldCircle {
            center: Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                rotation: 0.0,
            },
            radius: 30.0,
            style: WorldCircleStyle::static_full_glow(Stroke::new(Color::CYAN, 1.0), 4.0),
        }
    );
    assert_eq!(commands[1].plugin, PluginId::new("dev.good"));
    assert_eq!(
        commands[1].command,
        RenderCommand::HudCircle {
            center: ScreenPoint::new(40.0, 50.0),
            radius: 20.0,
            stroke: Stroke::new(Color::CYAN, 2.0),
        }
    );

    let next_update = engine.update();
    assert!(next_update.is_clean());
    assert_eq!(
        calls.lock().expect("test calls lock").as_slice(),
        [
            "dev.good",
            "dev.failing",
            "dev.good",
            "dev.failing",
            "dev.good",
            "dev.good",
        ]
    );
}

#[test]
fn engine_loads_a_plugin_registered_after_runtime_started() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(NoMemory);

    engine
        .register_plugin(RecordingPlugin {
            id: PluginId::new("dev.first"),
            calls: Arc::clone(&calls),
            fails_on_update: false,
            fails_on_render: false,
        })
        .expect("first plugin registration");
    assert!(engine.update().is_clean());

    engine
        .register_plugin(RecordingPlugin {
            id: PluginId::new("dev.late"),
            calls: Arc::clone(&calls),
            fails_on_update: false,
            fails_on_render: false,
        })
        .expect("late plugin registration");
    assert!(engine.render().is_clean());

    assert_eq!(
        calls.lock().expect("test calls lock").as_slice(),
        [
            "dev.first",
            "dev.first",
            "dev.late",
            "dev.first",
            "dev.late",
        ]
    );
}

#[test]
fn engine_discards_partial_render_commands_from_a_failed_plugin() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(NoMemory);

    engine
        .register_plugin(RecordingPlugin {
            id: PluginId::new("dev.render-failure"),
            calls,
            fails_on_update: false,
            fails_on_render: true,
        })
        .expect("plugin registration");

    let report = engine.render();
    assert_eq!(report.failures().len(), 1);
    assert_eq!(report.failures()[0].phase, PluginPhase::Render);
    assert_eq!(engine.active_plugin_count(), 0);
    assert!(engine.take_render_commands().is_empty());
}

const SETTINGS_PLUGIN: PluginId = PluginId::new("dev.settings");
const DRAW_RANGE: BoolSetting = BoolSetting::new("draw_range", true);
const RANGE: FloatSetting = FloatSetting::new("range", 30.0);
const DRAW_PAGE: PageId = PageId::new("drawings");
const RESET_BUTTON: ButtonId = ButtonId::new("reset");

struct SettingsPlugin {
    observed_values: Arc<Mutex<Vec<(bool, f32)>>>,
    button_clicks: Arc<Mutex<u32>>,
}

impl DeveloperPlugin<NoMemory> for SettingsPlugin {
    fn id(&self) -> PluginId {
        SETTINGS_PLUGIN
    }

    fn on_update(&mut self, api: &DeveloperApi<'_, '_, NoMemory>) -> PluginResult<()> {
        self.observed_values
            .lock()
            .expect("test settings lock")
            .push((api.settings().get(DRAW_RANGE), api.settings().get(RANGE)));
        Ok(())
    }

    fn on_menu(&mut self, menu: &mut DeveloperMenu<'_, '_, '_>) -> PluginResult<()> {
        menu.plugin_page(DRAW_PAGE, "Drawings", |ui| {
            ui.toggle(DRAW_RANGE, "Draw range");
            ui.slider(RANGE, "Range", FloatRange::new(5.0, 60.0, 1.0));
            if ui.button(RESET_BUTTON, "Reset") {
                ui.set(DRAW_RANGE, true);
                ui.set(RANGE, 30.0);
                *self.button_clicks.lock().expect("test button lock") += 1;
            }
        });
        Ok(())
    }
}

#[test]
fn engine_scopes_typed_settings_and_plugin_menu_commands() {
    let observed_values = Arc::new(Mutex::new(Vec::new()));
    let button_clicks = Arc::new(Mutex::new(0));
    let mut engine = Engine::new(NoMemory);
    engine
        .register_plugin(SettingsPlugin {
            observed_values: Arc::clone(&observed_values),
            button_clicks: Arc::clone(&button_clicks),
        })
        .expect("settings plugin registration");

    assert!(engine.update().is_clean());
    assert_eq!(
        observed_values
            .lock()
            .expect("test settings lock")
            .as_slice(),
        [(true, 30.0)]
    );

    assert!(engine.menu().is_clean());
    assert_eq!(
        engine.take_menu_commands(),
        vec![
            MenuCommand::Page {
                plugin: SETTINGS_PLUGIN,
                placement: MenuPlacement::PluginTab,
                page: DRAW_PAGE,
                label: "Drawings",
            },
            MenuCommand::Toggle {
                plugin: SETTINGS_PLUGIN,
                placement: MenuPlacement::PluginTab,
                page: DRAW_PAGE,
                setting: veyr::offsets::api::SettingKey::new("draw_range"),
                label: "Draw range",
            },
            MenuCommand::Slider {
                plugin: SETTINGS_PLUGIN,
                placement: MenuPlacement::PluginTab,
                page: DRAW_PAGE,
                setting: veyr::offsets::api::SettingKey::new("range"),
                label: "Range",
                range: FloatRange::new(5.0, 60.0, 1.0),
            },
            MenuCommand::Button {
                plugin: SETTINGS_PLUGIN,
                placement: MenuPlacement::PluginTab,
                page: DRAW_PAGE,
                button: RESET_BUTTON,
                label: "Reset",
            },
        ]
    );

    engine.set_bool_setting(SETTINGS_PLUGIN, DRAW_RANGE, false);
    engine.set_float_setting(SETTINGS_PLUGIN, RANGE, 45.0);
    assert!(engine.update().is_clean());
    assert_eq!(
        observed_values
            .lock()
            .expect("test settings lock")
            .as_slice(),
        [(true, 30.0), (false, 45.0)]
    );

    engine.click_menu_button(SETTINGS_PLUGIN, DRAW_PAGE, RESET_BUTTON);
    assert!(engine.menu().is_clean());
    assert_eq!(*button_clicks.lock().expect("test button lock"), 1);
    assert!(engine.update().is_clean());
    assert_eq!(
        observed_values
            .lock()
            .expect("test settings lock")
            .as_slice(),
        [(true, 30.0), (false, 45.0), (true, 30.0)]
    );
}

const ROOT_PLUGIN: PluginId = PluginId::new("dev.root-menu");
const ROOT_PAGE: PageId = PageId::new("diagnostics");
const ROOT_BUTTON: ButtonId = ButtonId::new("clear-cache");

struct RootMenuPlugin {
    clicks: Arc<Mutex<u32>>,
}

impl DeveloperPlugin<NoMemory> for RootMenuPlugin {
    fn id(&self) -> PluginId {
        ROOT_PLUGIN
    }

    fn on_menu(&mut self, menu: &mut DeveloperMenu<'_, '_, '_>) -> PluginResult<()> {
        menu.root_page(ROOT_PAGE, "Diagnostics", |ui| {
            if ui.button(ROOT_BUTTON, "Clear cache") {
                *self.clicks.lock().expect("root button lock") += 1;
            }
        });
        Ok(())
    }
}

#[test]
fn trusted_developer_plugins_can_use_root_menu_pages() {
    let clicks = Arc::new(Mutex::new(0));
    let mut engine = Engine::new(NoMemory);
    engine
        .register_plugin(RootMenuPlugin {
            clicks: Arc::clone(&clicks),
        })
        .expect("root plugin registration");

    assert!(engine.menu().is_clean());
    assert_eq!(
        engine.take_menu_commands(),
        vec![
            MenuCommand::Page {
                plugin: ROOT_PLUGIN,
                placement: MenuPlacement::Root,
                page: ROOT_PAGE,
                label: "Diagnostics",
            },
            MenuCommand::Button {
                plugin: ROOT_PLUGIN,
                placement: MenuPlacement::Root,
                page: ROOT_PAGE,
                button: ROOT_BUTTON,
                label: "Clear cache",
            },
        ]
    );

    engine.click_root_menu_button(ROOT_PLUGIN, ROOT_PAGE, ROOT_BUTTON);
    assert!(engine.menu().is_clean());
    assert_eq!(*clicks.lock().expect("root button lock"), 1);
}
