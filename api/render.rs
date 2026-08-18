use super::{PluginId, Position};

/// RGBA colour used by render commands.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Self = Self::rgba(255, 255, 255, 255);
    pub const BLACK: Self = Self::rgba(0, 0, 0, 255);
    pub const RED: Self = Self::rgba(255, 80, 80, 255);
    pub const GREEN: Self = Self::rgba(80, 255, 140, 255);
    pub const CYAN: Self = Self::rgba(80, 220, 255, 255);

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

/// A point in screen pixels, measured from the top-left corner.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ScreenPoint {
    pub x: f32,
    pub y: f32,
}

impl ScreenPoint {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Visual parameters for a line or circle outline.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
}

/// How a world circle is attached to the game world.
///
/// `Static` keeps every sampled point at the supplied world height. `Dynamic`
/// requests terrain-following placement: on a backend whose native collision
/// profile has been validated, samples are placed `terrain_clearance` units
/// above terrain. With `avoid_obstacles`, that validated backend may also make
/// radial notches before static model/WMO collision and suppress segments
/// hidden behind world geometry.
///
/// Collision profiles are deliberately fail-closed: if native collision is
/// unavailable or not validated for the running client, the command falls back
/// to the safe camera-projected static ring rather than invoking guessed game
/// functions. Sampling density still scales with radius inside the backend, so
/// callers never need to guess an appropriate segment count.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum WorldCirclePlacement {
    Static,
    Dynamic {
        terrain_clearance: f32,
        avoid_obstacles: bool,
    },
}

/// Glow treatment requested for a world circle.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum WorldCircleGlow {
    None,
    Outer,
    Inner,
    Full,
}

/// Backend-neutral appearance and placement contract for one world circle.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WorldCircleStyle {
    pub stroke: Stroke,
    pub glow: WorldCircleGlow,
    pub glow_width: f32,
    pub placement: WorldCirclePlacement,
}

impl WorldCircleStyle {
    /// A plain static outline at the supplied world height.
    #[must_use]
    pub const fn static_outline(stroke: Stroke) -> Self {
        Self {
            stroke,
            glow: WorldCircleGlow::None,
            glow_width: 0.0,
            placement: WorldCirclePlacement::Static,
        }
    }

    /// A static circle with a complete glow around its outline.
    #[must_use]
    pub const fn static_full_glow(stroke: Stroke, glow_width: f32) -> Self {
        Self {
            stroke,
            glow: WorldCircleGlow::Full,
            glow_width,
            placement: WorldCirclePlacement::Static,
        }
    }

    /// Requests a terrain-following outline. On an unsupported or unvalidated
    /// native backend it safely falls back to a static outline.
    #[must_use]
    pub const fn terrain_outline(stroke: Stroke, terrain_clearance: f32) -> Self {
        Self {
            stroke,
            glow: WorldCircleGlow::None,
            glow_width: 0.0,
            placement: WorldCirclePlacement::Dynamic {
                terrain_clearance,
                avoid_obstacles: false,
            },
        }
    }

    /// Requests a terrain-following outline with obstacle notches and
    /// world-visibility culling when the native collision profile has been
    /// validated. Otherwise it safely renders as a static outline.
    #[must_use]
    pub const fn terrain_obstacle_outline(stroke: Stroke, terrain_clearance: f32) -> Self {
        Self {
            stroke,
            glow: WorldCircleGlow::None,
            glow_width: 0.0,
            placement: WorldCirclePlacement::Dynamic {
                terrain_clearance,
                avoid_obstacles: true,
            },
        }
    }
}

impl Stroke {
    #[must_use]
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }
}

/// Render operation independent of D3D9 and the current render backend.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderCommand {
    /// A circle centered at a world position. Its placement and visual effect
    /// are defined entirely by the constrained [`WorldCircleStyle`].
    WorldCircle {
        center: Position,
        radius: f32,
        style: WorldCircleStyle,
    },
    /// A screen-space line.
    HudLine {
        from: ScreenPoint,
        to: ScreenPoint,
        stroke: Stroke,
    },
    /// A screen-space outlined circle.
    HudCircle {
        center: ScreenPoint,
        radius: f32,
        stroke: Stroke,
    },
    /// Screen-space text rendered by the future backend.
    HudText {
        position: ScreenPoint,
        text: String,
        color: Color,
        size: f32,
    },
}

/// A render command annotated by the host with the plugin that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedRenderCommand {
    pub plugin: PluginId,
    pub command: RenderCommand,
}

/// Per-frame command collection owned by the runtime renderer.
///
/// Plugins cannot construct it directly. They receive a [`PluginRenderFrame`]
/// scoped to their own plugin identity for one render callback.
#[derive(Default)]
pub struct RenderQueue {
    commands: Vec<QueuedRenderCommand>,
}

impl RenderQueue {
    pub(crate) fn frame_for(&mut self, plugin: PluginId) -> PluginRenderFrame<'_> {
        PluginRenderFrame {
            plugin,
            commands: &mut self.commands,
        }
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.commands.append(&mut other.commands);
    }

    pub(crate) fn clear(&mut self) {
        self.commands.clear();
    }

    pub(crate) fn take(&mut self) -> Vec<QueuedRenderCommand> {
        core::mem::take(&mut self.commands)
    }
}

/// Restricted render context available to one plugin for one frame.
pub struct PluginRenderFrame<'queue> {
    plugin: PluginId,
    commands: &'queue mut Vec<QueuedRenderCommand>,
}

impl<'queue> PluginRenderFrame<'queue> {
    /// Starts adding world-space overlay commands.
    pub fn world(&mut self) -> WorldDraw<'_, 'queue> {
        WorldDraw { frame: self }
    }

    /// Starts adding screen-space HUD commands.
    pub fn hud(&mut self) -> HudDraw<'_, 'queue> {
        HudDraw { frame: self }
    }

    fn push(&mut self, command: RenderCommand) {
        self.commands.push(QueuedRenderCommand {
            plugin: self.plugin,
            command,
        });
    }
}

/// World-space drawing namespace for one plugin frame.
pub struct WorldDraw<'frame, 'queue> {
    frame: &'frame mut PluginRenderFrame<'queue>,
}

impl<'frame, 'queue> WorldDraw<'frame, 'queue> {
    /// Draws a static outline around a unit or world position.
    pub fn circle(&mut self, center: Position, radius: f32, stroke: Stroke) {
        self.circle_with_style(center, radius, WorldCircleStyle::static_outline(stroke));
    }

    /// Draws a glowing static circle on the ground around a unit or world
    /// position. Glow variants are retained in the command contract even when
    /// a backend has not implemented them yet.
    pub fn glow_circle(&mut self, center: Position, radius: f32, color: Color) {
        self.circle_with_style(
            center,
            radius,
            WorldCircleStyle::static_full_glow(Stroke::new(color, 1.0), 4.0),
        );
    }

    /// Emits a world circle with an explicit placement and glow contract.
    pub fn circle_with_style(&mut self, center: Position, radius: f32, style: WorldCircleStyle) {
        self.frame.push(RenderCommand::WorldCircle {
            center,
            radius,
            style,
        });
    }
}

/// HUD drawing namespace for one plugin frame.
pub struct HudDraw<'frame, 'queue> {
    frame: &'frame mut PluginRenderFrame<'queue>,
}

impl<'frame, 'queue> HudDraw<'frame, 'queue> {
    pub fn line(&mut self, from: ScreenPoint, to: ScreenPoint, stroke: Stroke) {
        self.frame.push(RenderCommand::HudLine { from, to, stroke });
    }

    pub fn circle(&mut self, center: ScreenPoint, radius: f32, stroke: Stroke) {
        self.frame.push(RenderCommand::HudCircle {
            center,
            radius,
            stroke,
        });
    }

    pub fn text(
        &mut self,
        position: ScreenPoint,
        text: impl Into<String>,
        color: Color,
        size: f32,
    ) {
        self.frame.push(RenderCommand::HudText {
            position,
            text: text.into(),
            color,
            size,
        });
    }
}
