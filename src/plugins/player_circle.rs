//! First world-space rendering milestone: a plain circle around the player.
//!
//! The plugin intentionally treats all world reads as transient. Login,
//! character-select, loading screens, and object-manager rebuilds simply leave
//! it without a position for that update; they never become plugin failures.

use crate::engine::{DeveloperApi, DeveloperPlugin, PluginResult};
use crate::offsets::api::{
    Color, Memory, PluginId, PluginRenderFrame, Position, Stroke, WorldCircleStyle,
};

/// Draws a smooth radius-20 circle around the current local player.
///
/// The renderer promotes this request to terrain/collision placement only
/// after its native client profile has been explicitly validated; otherwise
/// the circle remains a safe camera-projected static ring.
#[derive(Debug, Default)]
pub struct PlayerCirclePlugin {
    player_position: Option<Position>,
}

impl PlayerCirclePlugin {
    /// Stable identity of this first-party world-render test.
    pub const ID: PluginId = PluginId::new("dev.player-circle");
    const RADIUS: f32 = 20.0;
    // Deliberately visible while dynamic terrain/collision placement is being
    // validated in the supported live client.
    const STROKE: Stroke = Stroke::new(Color::CYAN, 6.0);
    /// Keep the diagnostic line visibly above z-fighting while preserving the
    /// actual terrain shape.
    const TERRAIN_CLEARANCE: f32 = 0.25;

    fn refresh_position<M: Memory>(&mut self, api: &DeveloperApi<'_, '_, M>) {
        self.player_position = local_player_position(api);
    }
}

impl<M: Memory> DeveloperPlugin<M> for PlayerCirclePlugin {
    fn id(&self) -> PluginId {
        Self::ID
    }

    fn on_update(&mut self, api: &DeveloperApi<'_, '_, M>) -> PluginResult<M::Error> {
        self.refresh_position(api);
        Ok(())
    }

    fn on_render(
        &mut self,
        api: &DeveloperApi<'_, '_, M>,
        frame: &mut PluginRenderFrame<'_>,
    ) -> PluginResult<M::Error> {
        // Rendering runs once per graphics callback. Read this deliberately
        // lightweight value again here instead of waiting for the 16 ms plugin
        // update cadence, so the visual follows the client at render speed.
        let position = local_player_position(api).or(self.player_position);
        let Some(position) = position else {
            return Ok(());
        };
        self.player_position = Some(position);
        frame.world().circle_with_style(
            position,
            Self::RADIUS,
            WorldCircleStyle::terrain_obstacle_outline(Self::STROKE, Self::TERRAIN_CLEARANCE),
        );
        Ok(())
    }
}

fn local_player_position<M: Memory>(api: &DeveloperApi<'_, '_, M>) -> Option<Position> {
    if !api.world().is_in_game().ok()? {
        return None;
    }
    let player = api.world().local_player().ok()??;
    let position = player.position().ok()?;
    (position.x.is_finite()
        && position.y.is_finite()
        && position.z.is_finite()
        && position.rotation.is_finite())
    .then_some(position)
}
