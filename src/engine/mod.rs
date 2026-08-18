//! Internal runtime orchestration.
//!
//! The Engine owns runtime modules and creates a temporary developer API
//! context for each callback. Plugins never receive its reader directly.
//! Rendering is expressed only as constrained commands; native graphics
//! backends consume them outside the Engine.

mod plugin;

use crate::offsets::api::menu::{MenuInput, MenuPlacement, MenuQueue};
use crate::offsets::api::{
    BoolSetting, FloatSetting, Memory, MenuCommand, QueuedRenderCommand, SettingsStore,
};

pub use crate::offsets::api::DeveloperApi;
pub use crate::offsets::api::PluginId;
pub use plugin::{
    DeveloperPlugin, PluginError, PluginFailure, PluginHost, PluginPhase, PluginRegistrationError,
    PluginResult,
};

/// Coordinates the reader and first-party developer plugins.
///
/// Later the Engine will own writer and renderer modules too. They will be
/// exposed to plugins only through similarly narrow API capabilities.
pub struct Engine<M: Memory> {
    reader: M,
    plugins: PluginHost<M>,
    render_queue: crate::offsets::api::render::RenderQueue,
    settings: SettingsStore,
    menu_input: MenuInput,
    menu_queue: MenuQueue,
}

impl<M: Memory> Engine<M> {
    /// Creates an Engine that owns the supplied process-memory reader.
    pub fn new(reader: M) -> Self {
        Self {
            reader,
            plugins: PluginHost::new(),
            render_queue: crate::offsets::api::render::RenderQueue::default(),
            settings: SettingsStore::default(),
            menu_input: MenuInput::default(),
            menu_queue: MenuQueue::default(),
        }
    }

    /// Registers a first-party developer plugin before or during runtime.
    pub fn register_plugin<P>(&mut self, plugin: P) -> Result<(), PluginRegistrationError>
    where
        P: DeveloperPlugin<M> + 'static,
    {
        self.plugins.register(plugin)
    }

    /// Registers an already type-erased trusted developer plugin.
    ///
    /// This is used by the developer-plugin catalog. Public SDK plugins will
    /// use a separate, capability-restricted adapter rather than this method.
    pub fn register_boxed_plugin(
        &mut self,
        plugin: Box<dyn DeveloperPlugin<M>>,
    ) -> Result<(), PluginRegistrationError> {
        self.plugins.register_boxed(plugin)
    }

    /// Whether an ID is already owned by this Engine's plugin host.
    #[must_use]
    pub fn has_plugin(&self, id: PluginId) -> bool {
        self.plugins.contains(id)
    }

    /// Loads every registered plugin that has not been initialized yet.
    ///
    /// Failures disable only the plugin that produced them.
    pub fn initialize(&mut self) -> EngineReport<M::Error> {
        EngineReport::new(self.plugins.load_all(&self.reader, &self.settings))
    }

    /// Dispatches the normal game-update lifecycle event to active plugins.
    pub fn update(&mut self) -> EngineReport<M::Error> {
        let mut report = self.initialize();
        report.extend(self.plugins.update(&self.reader, &self.settings));
        report
    }

    /// Dispatches the render lifecycle event to active plugins.
    ///
    /// This does not expose a native graphics API. The callback receives only
    /// a constrained command buffer consumed by the injected backend.
    pub fn render(&mut self) -> EngineReport<M::Error> {
        let mut report = self.initialize();
        self.render_queue.clear();
        report.extend(
            self.plugins
                .render(&self.reader, &self.settings, &mut self.render_queue),
        );
        report
    }

    /// Drains commands produced by the most recent successful render dispatch.
    ///
    /// This is the handoff point for the selected injected graphics backend.
    pub fn take_render_commands(&mut self) -> Vec<QueuedRenderCommand> {
        self.render_queue.take()
    }

    /// Dispatches plugin menu declarations for the current UI frame.
    pub fn menu(&mut self) -> EngineReport<M::Error> {
        let mut report = self.initialize();
        self.menu_queue.clear();
        report.extend(self.plugins.menu(
            &mut self.settings,
            &mut self.menu_input,
            &mut self.menu_queue,
        ));
        report
    }

    /// Drains plugin-menu declarations for the current UI frame.
    pub fn take_menu_commands(&mut self) -> Vec<MenuCommand> {
        self.menu_queue.take()
    }

    /// Applies a boolean value supplied by the future menu backend.
    pub fn set_bool_setting(&mut self, plugin: PluginId, setting: BoolSetting, value: bool) {
        self.settings.set_bool(plugin, setting, value);
    }

    /// Applies a floating-point value supplied by the future menu backend.
    pub fn set_float_setting(&mut self, plugin: PluginId, setting: FloatSetting, value: f32) {
        self.settings.set_float(plugin, setting, value);
    }

    /// Queues a one-shot button interaction for the next menu callback.
    pub fn click_menu_button(
        &mut self,
        plugin: PluginId,
        page: crate::offsets::api::PageId,
        button: crate::offsets::api::ButtonId,
    ) {
        self.menu_input
            .press(plugin, MenuPlacement::PluginTab, page, button);
    }

    /// Queues a root-menu button interaction for a trusted developer plugin.
    pub fn click_root_menu_button(
        &mut self,
        plugin: PluginId,
        page: crate::offsets::api::PageId,
        button: crate::offsets::api::ButtonId,
    ) {
        self.menu_input
            .press(plugin, MenuPlacement::Root, page, button);
    }

    /// Number of registered plugins that are currently active.
    #[must_use]
    pub fn active_plugin_count(&self) -> usize {
        self.plugins.active_count()
    }
}

/// Outcome of one Engine lifecycle dispatch.
#[derive(Debug)]
pub struct EngineReport<E> {
    failures: Vec<PluginFailure<E>>,
}

impl<E> EngineReport<E> {
    fn new(failures: Vec<PluginFailure<E>>) -> Self {
        Self { failures }
    }

    fn extend(&mut self, failures: Vec<PluginFailure<E>>) {
        self.failures.extend(failures);
    }

    /// Returns failures without preventing other plugins from being run.
    #[must_use]
    pub fn failures(&self) -> &[PluginFailure<E>] {
        &self.failures
    }

    /// Whether every callback completed successfully.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }

    /// Consumes the report and returns its isolated plugin failures.
    #[must_use]
    pub fn into_failures(self) -> Vec<PluginFailure<E>> {
        self.failures
    }
}
