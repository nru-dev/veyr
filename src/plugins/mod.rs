//! Composition layer for trusted, first-party developer plugins.
//!
//! This module deliberately sits outside Engine: the Engine hosts lifecycle
//! callbacks but does not decide which features or character-specific plugins
//! exist. The future public SDK gets a different catalog/adapter with narrower
//! capabilities and a stable binary boundary.

mod player_circle;
mod render_smoke;

use crate::engine::{DeveloperPlugin, Engine, PluginRegistrationError};
use crate::offsets::api::{Memory, PluginId};

pub use player_circle::PlayerCirclePlugin;
pub use render_smoke::RenderSmokePlugin;

/// A batch of trusted developer plugins assembled by the application bootstrap.
pub struct DeveloperPluginCatalog<M: Memory> {
    plugins: Vec<Box<dyn DeveloperPlugin<M>>>,
}

impl<M: Memory> Default for DeveloperPluginCatalog<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Memory> DeveloperPluginCatalog<M> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Adds a plugin to the catalog without coupling it to a particular Engine.
    pub fn add<P>(&mut self, plugin: P) -> Result<(), CatalogRegistrationError>
    where
        P: DeveloperPlugin<M> + 'static,
    {
        self.add_boxed(Box::new(plugin))
    }

    /// Adds an erased trusted plugin, for use by the future DEV plugin loader.
    pub fn add_boxed(
        &mut self,
        plugin: Box<dyn DeveloperPlugin<M>>,
    ) -> Result<(), CatalogRegistrationError> {
        let id = plugin.id();
        if self.plugins.iter().any(|candidate| candidate.id() == id) {
            return Err(CatalogRegistrationError::DuplicateInCatalog(id));
        }

        self.plugins.push(plugin);
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Atomically validates and then transfers all plugins to an Engine.
    ///
    /// A conflicting ID is reported before any plugin is registered, so a
    /// failed catalog install never leaves a half-installed collection behind.
    pub fn install_into(self, engine: &mut Engine<M>) -> Result<(), CatalogInstallError> {
        for plugin in &self.plugins {
            let id = plugin.id();
            if engine.has_plugin(id) {
                return Err(CatalogInstallError::EngineConflict(id));
            }
        }

        for plugin in self.plugins {
            engine
                .register_boxed_plugin(plugin)
                .map_err(CatalogInstallError::Registration)?;
        }

        Ok(())
    }
}

/// A catalog rejected a duplicate ID before it could reach Engine.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CatalogRegistrationError {
    DuplicateInCatalog(PluginId),
}

/// A catalog could not be transferred to the selected Engine.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CatalogInstallError {
    EngineConflict(PluginId),
    Registration(PluginRegistrationError),
}
