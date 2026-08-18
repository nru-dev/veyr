//! First-party visual smoke test for the injected graphics pipeline.
//!
//! This is deliberately a developer plugin, rather than an Engine special
//! case: it validates the same constrained render API that real plugins use.

use crate::engine::{DeveloperApi, DeveloperPlugin, PluginResult};
use crate::offsets::api::{Color, Memory, PluginId, PluginRenderFrame, ScreenPoint, Stroke};

/// Draws a fixed, high-contrast HUD marker without reading client memory.
///
/// Start it only through the explicit `veyr_runtime_start_visual_smoke` export.
/// Normal runtime startup remains visual-free.
#[derive(Debug, Default)]
pub struct RenderSmokePlugin;

impl RenderSmokePlugin {
    /// Stable identity of this first-party, opt-in developer plugin.
    pub const ID: PluginId = PluginId::new("dev.render-smoke");
}

impl<M: Memory> DeveloperPlugin<M> for RenderSmokePlugin {
    fn id(&self) -> PluginId {
        Self::ID
    }

    fn on_render(
        &mut self,
        _api: &DeveloperApi<'_, '_, M>,
        frame: &mut PluginRenderFrame<'_>,
    ) -> PluginResult<M::Error> {
        let mut hud = frame.hud();
        hud.circle(
            ScreenPoint::new(120.0, 120.0),
            48.0,
            Stroke::new(Color::CYAN, 3.0),
        );
        hud.line(
            ScreenPoint::new(58.0, 120.0),
            ScreenPoint::new(182.0, 120.0),
            Stroke::new(Color::RED, 2.0),
        );
        hud.line(
            ScreenPoint::new(120.0, 58.0),
            ScreenPoint::new(120.0, 182.0),
            Stroke::new(Color::RED, 2.0),
        );
        Ok(())
    }
}
