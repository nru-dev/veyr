use super::super::RemoteAddress;
use super::{
    AdvancedCombat, Camera, GameApi, Memory, Object, PluginId, PluginSettings, SettingsStore, World,
};

/// Full internal API context passed to a trusted developer plugin callback.
///
/// It combines semantic game access with a read-only settings view scoped to
/// the current plugin. The Engine owns the reader and settings store; neither
/// can be retained by the plugin after its callback returns.
pub struct DeveloperApi<'reader, 'settings, M> {
    game: GameApi<&'reader M>,
    settings: PluginSettings<'settings>,
}

impl<'reader, 'settings, M> DeveloperApi<'reader, 'settings, M> {
    pub(crate) fn new(
        reader: &'reader M,
        plugin: PluginId,
        settings: &'settings SettingsStore,
    ) -> Self {
        Self {
            game: GameApi::new(reader),
            settings: PluginSettings::new(plugin, settings),
        }
    }

    /// Gives trusted developer plugins access to the complete underlying game
    /// facade. The future SDK API will not expose this escape hatch.
    #[must_use]
    pub const fn game(&self) -> &GameApi<&'reader M> {
        &self.game
    }

    /// Returns this plugin's read-only settings namespace.
    #[must_use]
    pub const fn settings(&self) -> &PluginSettings<'settings> {
        &self.settings
    }
}

impl<'reader, 'settings, M: Memory> DeveloperApi<'reader, 'settings, M> {
    /// Starts semantic world/object operations.
    pub const fn world(&self) -> World<'_, &M> {
        self.game.world()
    }

    /// Starts semantic combat and cast operations.
    pub const fn combat(&self) -> AdvancedCombat<'_, &M> {
        self.game.combat()
    }

    /// Starts camera and future world-projection operations.
    pub const fn camera(&self) -> Camera<'_, &M> {
        self.game.camera()
    }

    /// Creates a handle for a known x86 client object address.
    pub const fn object(&self, address: RemoteAddress) -> Object<'_, &M> {
        self.game.object(address)
    }
}
