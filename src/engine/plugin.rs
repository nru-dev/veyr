use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::offsets::api::menu::{MenuInput, MenuQueue};
use crate::offsets::api::render::RenderQueue;
use crate::offsets::api::{
    ApiError, DeveloperApi, DeveloperMenu, Memory, PluginId, PluginRenderFrame, SettingsStore,
};

/// Lifecycle phase in which a plugin failed.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginPhase {
    Load,
    Update,
    Render,
    Menu,
}

/// An error a plugin intentionally returned, or an error isolated by the host.
#[derive(Debug)]
pub enum PluginError<E> {
    /// A read or semantic API operation failed.
    Api(ApiError<E>),
    /// The plugin rejected the callback with a static diagnostic.
    Rejected(&'static str),
    /// The plugin panicked; the host caught it and disabled the plugin.
    Panicked,
}

impl<E> From<ApiError<E>> for PluginError<E> {
    fn from(error: ApiError<E>) -> Self {
        Self::Api(error)
    }
}

/// Result returned by a developer-plugin callback.
pub type PluginResult<E> = Result<(), PluginError<E>>;

/// One failure isolated by the plugin host.
#[derive(Debug)]
pub struct PluginFailure<E> {
    pub plugin: PluginId,
    pub phase: PluginPhase,
    pub error: PluginError<E>,
}

/// Contract implemented by built-in, trusted developer plugins.
///
/// This is intentionally a Rust trait, not the future public SDK ABI. The SDK
/// adapter will be a separate host implementation with narrower capabilities.
pub trait DeveloperPlugin<M: Memory>: Send {
    fn id(&self) -> PluginId;

    fn on_load(&mut self, _api: &DeveloperApi<'_, '_, M>) -> PluginResult<M::Error> {
        Ok(())
    }

    fn on_update(&mut self, _api: &DeveloperApi<'_, '_, M>) -> PluginResult<M::Error> {
        Ok(())
    }

    /// Runs in render event order with a plugin-scoped command buffer.
    fn on_render(
        &mut self,
        _api: &DeveloperApi<'_, '_, M>,
        _frame: &mut PluginRenderFrame<'_>,
    ) -> PluginResult<M::Error> {
        Ok(())
    }

    /// Declares DEV controls in either a plugin tab or a privileged root page.
    fn on_menu(&mut self, _menu: &mut DeveloperMenu<'_, '_, '_>) -> PluginResult<M::Error> {
        Ok(())
    }
}

/// Registration was rejected before the plugin entered the lifecycle.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginRegistrationError {
    DuplicateId(PluginId),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum PluginState {
    Registered,
    Active,
    Disabled,
}

struct PluginSlot<M: Memory> {
    plugin: Box<dyn DeveloperPlugin<M>>,
    state: PluginState,
}

/// Owns developer plugins and isolates their lifecycle failures.
pub struct PluginHost<M: Memory> {
    slots: Vec<PluginSlot<M>>,
}

impl<M: Memory> Default for PluginHost<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Memory> PluginHost<M> {
    #[must_use]
    pub const fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Registers a plugin. IDs must be unique for stable settings and logging.
    pub fn register<P>(&mut self, plugin: P) -> Result<(), PluginRegistrationError>
    where
        P: DeveloperPlugin<M> + 'static,
    {
        self.register_boxed(Box::new(plugin))
    }

    /// Registers an already type-erased trusted developer plugin.
    pub fn register_boxed(
        &mut self,
        plugin: Box<dyn DeveloperPlugin<M>>,
    ) -> Result<(), PluginRegistrationError> {
        let id = plugin.id();
        if self.slots.iter().any(|slot| slot.plugin.id() == id) {
            return Err(PluginRegistrationError::DuplicateId(id));
        }

        self.slots.push(PluginSlot {
            plugin,
            state: PluginState::Registered,
        });
        Ok(())
    }

    #[must_use]
    pub fn contains(&self, id: PluginId) -> bool {
        self.slots.iter().any(|slot| slot.plugin.id() == id)
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state == PluginState::Active)
            .count()
    }

    pub(crate) fn load_all(
        &mut self,
        reader: &M,
        settings: &SettingsStore,
    ) -> Vec<PluginFailure<M::Error>> {
        let mut failures = Vec::new();

        for slot in &mut self.slots {
            if slot.state != PluginState::Registered {
                continue;
            }

            let id = slot.plugin.id();
            let api = DeveloperApi::new(reader, id, settings);
            match invoke(|| slot.plugin.on_load(&api)) {
                Ok(()) => slot.state = PluginState::Active,
                Err(error) => {
                    slot.state = PluginState::Disabled;
                    failures.push(PluginFailure {
                        plugin: id,
                        phase: PluginPhase::Load,
                        error,
                    });
                }
            }
        }

        failures
    }

    pub(crate) fn update(
        &mut self,
        reader: &M,
        settings: &SettingsStore,
    ) -> Vec<PluginFailure<M::Error>> {
        let mut failures = Vec::new();

        for slot in &mut self.slots {
            if slot.state != PluginState::Active {
                continue;
            }

            let id = slot.plugin.id();
            let api = DeveloperApi::new(reader, id, settings);
            if let Err(error) = invoke(|| slot.plugin.on_update(&api)) {
                slot.state = PluginState::Disabled;
                failures.push(PluginFailure {
                    plugin: id,
                    phase: PluginPhase::Update,
                    error,
                });
            }
        }

        failures
    }

    pub(crate) fn render(
        &mut self,
        reader: &M,
        settings: &SettingsStore,
        queue: &mut RenderQueue,
    ) -> Vec<PluginFailure<M::Error>> {
        let mut failures = Vec::new();

        for slot in &mut self.slots {
            if slot.state != PluginState::Active {
                continue;
            }

            let id = slot.plugin.id();
            let mut plugin_queue = RenderQueue::default();
            let api = DeveloperApi::new(reader, id, settings);
            let result = {
                let mut frame = plugin_queue.frame_for(id);
                invoke(|| slot.plugin.on_render(&api, &mut frame))
            };

            match result {
                Ok(()) => queue.append(plugin_queue),
                Err(error) => {
                    slot.state = PluginState::Disabled;
                    failures.push(PluginFailure {
                        plugin: id,
                        phase: PluginPhase::Render,
                        error,
                    });
                }
            }
        }

        failures
    }

    pub(crate) fn menu(
        &mut self,
        settings: &mut SettingsStore,
        input: &mut MenuInput,
        queue: &mut MenuQueue,
    ) -> Vec<PluginFailure<M::Error>> {
        let mut failures = Vec::new();

        for slot in &mut self.slots {
            if slot.state != PluginState::Active {
                continue;
            }

            let id = slot.plugin.id();
            let mut plugin_queue = MenuQueue::default();
            let result = {
                let mut menu = plugin_queue.developer_frame_for(id, settings, input);
                invoke(|| slot.plugin.on_menu(&mut menu))
            };

            match result {
                Ok(()) => queue.append(plugin_queue),
                Err(error) => {
                    slot.state = PluginState::Disabled;
                    failures.push(PluginFailure {
                        plugin: id,
                        phase: PluginPhase::Menu,
                        error,
                    });
                }
            }
        }

        failures
    }
}

fn invoke<E>(callback: impl FnOnce() -> PluginResult<E>) -> PluginResult<E> {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(result) => result,
        Err(_) => Err(PluginError::Panicked),
    }
}
