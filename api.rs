//! Internal developer API for interacting with the 3.3.5a client.
//!
//! This is deliberately not a public SDK. It centralizes pointer traversal,
//! descriptor access, and client-specific errors while keeping the raw map in
//! the parent `offsets` module.
//!
//! ```ignore
//! let manager = api.world().object_manager()?;
//! let player = manager.local_player()?;
//! let health = player.map(|player| player.health()).transpose()?;
//! ```

mod camera;
mod combat;
mod descriptor;
mod developer;
mod entity;
mod error;
mod memory;
pub(crate) mod menu;
mod plugin;
pub(crate) mod render;
mod settings;
mod terrain;
mod world;

pub use camera::{Camera, CameraState, Vector3};
pub use combat::{AdvancedCombat, CastInfo, CooldownList, SpellCooldown};
pub use descriptor::Descriptor;
pub use developer::DeveloperApi;
pub use entity::{Aura, GameObject, Object, Player, Position, Unit};
pub use error::{ApiError, ApiResult};
pub use memory::{Memory, RemotePtr, WritableMemory};
pub use menu::{
    ButtonId, DeveloperMenu, FloatRange, MenuCommand, MenuPage, MenuPlacement, PageId, PluginMenu,
};
pub use plugin::PluginId;
pub use render::{
    Color, HudDraw, PluginRenderFrame, QueuedRenderCommand, RenderCommand, ScreenPoint, Stroke,
    WorldCircleGlow, WorldCirclePlacement, WorldCircleStyle, WorldDraw,
};
pub use settings::{BoolSetting, FloatSetting, PluginSettings, Setting, SettingKey, SettingsStore};
pub use terrain::{
    TerrainCache, TerrainCacheError, TerrainError, TerrainSample, TerrainTile, TerrainTileKey,
    TerrainTileLoader,
};
pub use world::{ObjectList, ObjectManager, World};

/// Entry point for all developer-facing game interactions.
pub struct GameApi<M> {
    memory: M,
}

impl<M> GameApi<M> {
    /// Wraps a process-memory backend.
    pub const fn new(memory: M) -> Self {
        Self { memory }
    }

    /// Exposes the backend for diagnostics and deliberately raw operations.
    pub const fn memory(&self) -> &M {
        &self.memory
    }

    /// Returns the underlying backend when the API is no longer needed.
    pub fn into_memory(self) -> M {
        self.memory
    }
}

impl<M: Memory> GameApi<M> {
    /// Starts world/object-manager operations.
    pub const fn world(&self) -> World<'_, M> {
        World::new(self)
    }

    /// Starts combat, casting, aura, and cooldown operations.
    pub const fn combat(&self) -> AdvancedCombat<'_, M> {
        AdvancedCombat::new(self)
    }

    /// Starts camera and future world-projection operations.
    pub const fn camera(&self) -> Camera<'_, M> {
        Camera::new(self)
    }

    /// Creates an object handle for a known client address.
    pub const fn object(&self, address: super::RemoteAddress) -> Object<'_, M> {
        Object::new(self, address)
    }
}
