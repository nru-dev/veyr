//! Privileged D3D9 implementation of the API render-command backend.
//!
//! This module accepts only commands already attributed to plugins by Engine.
//! D3D9 discovery remains outside it. Static world primitives are projected
//! through the verified client camera before they become overlay geometry.

use core::ffi::c_void;
use core::ptr;

use crate::injected::circle_geometry::{
    hud_segments, obstacle_recovery_step, smooth_radial_limits, split_visible_world_ring,
    world_sample_segments, world_segments,
};
use crate::injected::overlay_geometry::{circle_strip, line_strip, OverlayVertex};
use crate::offsets::api::{
    CameraState, GameApi, Position, QueuedRenderCommand, RenderCommand, Stroke, Vector3,
    WorldCircleGlow, WorldCirclePlacement, WorldCircleStyle,
};

use super::memory::LocalProcessMemory;
use super::render_backend::{
    last_renderer_stats, publish_renderer_stats, NativeFrame, NativeRenderer, RendererStats,
};
use super::world_collision;
use super::world_projection::{
    project_static_circle, project_world_path, ProjectionError, Viewport,
};

const D3DFVF_XYZRHW: u32 = 0x0004;
const D3DFVF_DIFFUSE: u32 = 0x0040;
const OVERLAY_FVF: u32 = D3DFVF_XYZRHW | D3DFVF_DIFFUSE;

const D3DPT_TRIANGLELIST: u32 = 4;
const D3DPT_TRIANGLESTRIP: u32 = 5;

const D3DRS_ZENABLE: u32 = 7;
const D3DRS_ZWRITEENABLE: u32 = 14;
const D3DRS_ALPHATESTENABLE: u32 = 15;
const D3DRS_SRCBLEND: u32 = 19;
const D3DRS_DESTBLEND: u32 = 20;
const D3DRS_CULLMODE: u32 = 22;
const D3DRS_ALPHABLENDENABLE: u32 = 27;
const D3DRS_FOGENABLE: u32 = 28;
const D3DRS_LIGHTING: u32 = 137;
const D3DRS_SCISSORTESTENABLE: u32 = 174;

const D3DBLEND_SRCALPHA: u32 = 5;
const D3DBLEND_INVSRCALPHA: u32 = 6;
const D3DCULL_NONE: u32 = 1;

const D3DTSS_COLOROP: u32 = 1;
const D3DTSS_COLORARG1: u32 = 2;
const D3DTSS_ALPHAOP: u32 = 4;
const D3DTSS_ALPHAARG1: u32 = 5;
const D3DTOP_SELECTARG1: u32 = 2;
const D3DTA_DIFFUSE: u32 = 0x0000_0002;

const OVERLAY_RENDER_STATES: [u32; 10] = [
    D3DRS_ZENABLE,
    D3DRS_ZWRITEENABLE,
    D3DRS_ALPHATESTENABLE,
    D3DRS_ALPHABLENDENABLE,
    D3DRS_SRCBLEND,
    D3DRS_DESTBLEND,
    D3DRS_CULLMODE,
    D3DRS_FOGENABLE,
    D3DRS_LIGHTING,
    D3DRS_SCISSORTESTENABLE,
];
const OVERLAY_TEXTURE_STAGE_STATES: [u32; 4] = [
    D3DTSS_COLOROP,
    D3DTSS_COLORARG1,
    D3DTSS_ALPHAOP,
    D3DTSS_ALPHAARG1,
];

// `IDirect3DDevice9` COM vtable slots, including the three IUnknown entries.
const DEVICE_SET_RENDER_STATE: usize = 57;
const DEVICE_GET_RENDER_STATE: usize = 58;
const DEVICE_GET_VIEWPORT: usize = 48;
const DEVICE_GET_TEXTURE: usize = 64;
const DEVICE_SET_TEXTURE: usize = 65;
const DEVICE_GET_TEXTURE_STAGE_STATE: usize = 66;
const DEVICE_SET_TEXTURE_STAGE_STATE: usize = 67;
const DEVICE_SET_VERTEX_DECLARATION: usize = 87;
const DEVICE_GET_VERTEX_DECLARATION: usize = 88;
const DEVICE_SET_FVF: usize = 89;
const DEVICE_GET_FVF: usize = 90;
const DEVICE_SET_VERTEX_SHADER: usize = 92;
const DEVICE_GET_VERTEX_SHADER: usize = 93;
const DEVICE_SET_PIXEL_SHADER: usize = 107;
const DEVICE_GET_PIXEL_SHADER: usize = 108;
const DEVICE_DRAW_PRIMITIVE_UP: usize = 83;
const DEVICE_GET_STREAM_SOURCE: usize = 103;
const DEVICE_SET_STREAM_SOURCE: usize = 102;

const IUNKNOWN_RELEASE: usize = 2;

type SetRenderStateFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;
type GetRenderStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> i32;
type GetViewportFn = unsafe extern "system" fn(*mut c_void, *mut D3dViewport) -> i32;
type SetTextureFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
type GetTextureFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
type SetTextureStageStateFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32) -> i32;
type GetTextureStageStateFn = unsafe extern "system" fn(*mut c_void, u32, u32, *mut u32) -> i32;
type SetVertexDeclarationFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type GetVertexDeclarationFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type SetFvfFn = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type GetFvfFn = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type SetVertexShaderFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type GetVertexShaderFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type SetPixelShaderFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type GetPixelShaderFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type DrawPrimitiveUpFn =
    unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void, u32) -> i32;
type SetStreamSourceFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, u32) -> i32;
type GetStreamSourceFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32, *mut u32) -> i32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

const E_FAIL: i32 = 0x8000_4005_u32 as i32;
const WORLD_CIRCLE_VIEWPORT_UNAVAILABLE: u32 = 0xE104;
const WORLD_CIRCLE_OBSTACLE_CONTOUR_UNAVAILABLE: u32 = 0xE105;
const WORLD_CIRCLE_TERRAIN_SAMPLES_PENDING: u32 = 0xE208;
const WORLD_CIRCLE_INVALID_TESSELLATION: u32 = 0xE209;
// The first player-circle must be safe on every compatible client build.
// Direct `CGWorldFrame::Intersect` calls are experimental until revalidated
// against the exact executable being run; a wrong ABI causes an unrecoverable
// client access violation. Keep native terrain/model sampling disabled by
// default and render the verified camera-projected static ring instead.
const ENABLE_EXPERIMENTAL_WORLD_COLLISION: bool = false;
// Native terrain/model queries are bounded separately from visual
// tessellation. Terrain can be queried much more cheaply than arbitrary model
// and camera rays, so it receives a larger per-frame budget. Both caches stay
// double-buffered: no partial ring is ever rendered.
const TERRAIN_SAMPLES_PER_FRAME: usize = 128;
// Ignore sub-pixel client position jitter, but prepare a new terrain contour
// before a visibly translated old one can drift away from the ground.
const TERRAIN_REFRESH_DISTANCE: f32 = 0.25;
// One collision index needs one model ray and one camera-visibility ray. Keep
// this finite even if a plugin requests a very large circle.
const COLLISION_SAMPLES_PER_FRAME: usize = 64;

/// Compatibility name for callers that still identify these diagnostics with
/// the original D3D9-only runtime.
pub type D3d9RenderStats = RendererStats;

/// Snapshot of the most recent privileged renderer frame.
///
/// This is developer-runtime diagnostics, not SDK API. It lets the loader
/// distinguish a hook issue from a rejected D3D9 state or draw call.
#[must_use]
pub fn last_d3d9_render_stats() -> D3d9RenderStats {
    last_renderer_stats()
}

/// D3D9 fixed-function backend for the first HUD command set.
///
/// It draws HUD primitives. Text and world primitives remain safely skipped
/// until their dedicated render paths are implemented.
pub struct D3d9OverlayRenderer {
    circle_segments: usize,
    terrain_circle: Option<DynamicTerrainCircle>,
    collision_circle: Option<DynamicCollisionCircle>,
    last_frame: D3d9RenderStats,
}

impl Default for D3d9OverlayRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl D3d9OverlayRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            // A radius-20 circle needs sub-unit segments to follow both the
            // ground and a curved visual contour without visible faceting.
            // The public setter caps this to a bounded render cost.
            circle_segments: 256,
            terrain_circle: None,
            collision_circle: None,
            last_frame: RendererStats {
                submitted_commands: 0,
                drawn_commands: 0,
                skipped_commands: 0,
                draw_failures: 0,
                state_setup_failed: false,
                last_error: 0,
            },
        }
    }

    /// Selects circle smoothness, clamped to a predictable render cost.
    ///
    /// This is a minimum quality floor. Normal world and HUD circles scale
    /// above it automatically with their world-space or pixel radius.
    #[must_use]
    pub const fn with_circle_segments(mut self, segments: usize) -> Self {
        self.circle_segments = if segments < 32 {
            32
        } else if segments > 1_536 {
            1_536
        } else {
            segments
        };
        self
    }

    #[must_use]
    pub const fn last_frame(&self) -> D3d9RenderStats {
        self.last_frame
    }

    fn record_draw(&mut self, result: i32) {
        if succeeded(result) {
            self.last_frame.drawn_commands += 1;
        } else {
            self.last_frame.draw_failures += 1;
            self.last_frame.last_error = result as u32;
        }
    }

    fn publish_last_frame(&self) {
        publish_renderer_stats(self.last_frame);
    }

    fn draw_world_circle(
        &mut self,
        device: *mut c_void,
        center: Position,
        radius: f32,
        style: WorldCircleStyle,
    ) {
        let viewport = match unsafe { d3d_viewport(device) } {
            Ok(viewport) => viewport,
            Err(_) => {
                self.last_frame.skipped_commands += 1;
                self.last_frame.last_error = WORLD_CIRCLE_VIEWPORT_UNAVAILABLE;
                return;
            }
        };
        let api = GameApi::new(LocalProcessMemory);
        let camera = match api.camera().state() {
            Ok(camera) => camera,
            Err(_) => {
                self.last_frame.skipped_commands += 1;
                self.last_frame.last_error = ProjectionError::InvalidCamera.diagnostic_code();
                return;
            }
        };

        let Some(render_segments) = world_segments(radius, self.circle_segments) else {
            self.last_frame.skipped_commands += 1;
            self.last_frame.last_error = WORLD_CIRCLE_INVALID_TESSELLATION;
            return;
        };
        let sample_segments = if ENABLE_EXPERIMENTAL_WORLD_COLLISION {
            let Some(segments) = world_sample_segments(radius) else {
                self.last_frame.skipped_commands += 1;
                self.last_frame.last_error = WORLD_CIRCLE_INVALID_TESSELLATION;
                return;
            };
            segments
        } else {
            0
        };

        let terrain_paths = match style.placement {
            WorldCirclePlacement::Static => {
                self.collision_circle = None;
                None
            }
            WorldCirclePlacement::Dynamic {
                terrain_clearance,
                avoid_obstacles,
            } => {
                if !terrain_clearance.is_finite() || terrain_clearance < 0.0 {
                    self.last_frame.skipped_commands += 1;
                    self.last_frame.last_error = WORLD_CIRCLE_OBSTACLE_CONTOUR_UNAVAILABLE;
                    return;
                }
                if !ENABLE_EXPERIMENTAL_WORLD_COLLISION {
                    self.terrain_circle = None;
                    self.collision_circle = None;
                    None
                } else {
                    let circle = self.terrain_circle.get_or_insert_with(|| {
                        DynamicTerrainCircle::new(
                            center,
                            radius,
                            terrain_clearance,
                            sample_segments,
                        )
                    });
                    circle.update(
                        center,
                        radius,
                        terrain_clearance,
                        sample_segments,
                        TERRAIN_SAMPLES_PER_FRAME,
                    );
                    let Some(path) = circle.completed_path(center) else {
                        self.last_frame.skipped_commands += 1;
                        self.last_frame.last_error = WORLD_CIRCLE_TERRAIN_SAMPLES_PENDING;
                        return;
                    };
                    let paths = if avoid_obstacles {
                        let source_center = circle.active_center();
                        let needs_collision_cache =
                            self.collision_circle.as_ref().is_none_or(|circle| {
                                !circle.matches_configuration(
                                    radius,
                                    terrain_clearance,
                                    sample_segments,
                                )
                            });
                        if needs_collision_cache {
                            self.collision_circle = DynamicCollisionCircle::new(
                                source_center,
                                radius,
                                terrain_clearance,
                                sample_segments,
                                &path,
                            );
                        }
                        if let Some(collision) = self.collision_circle.as_mut() {
                            collision.update(
                                source_center,
                                radius,
                                terrain_clearance,
                                sample_segments,
                                &path,
                                camera,
                                COLLISION_SAMPLES_PER_FRAME,
                            );
                            collision.apply(center, &path)
                        } else {
                            vec![path]
                        }
                    } else {
                        self.collision_circle = None;
                        vec![path]
                    };
                    Some(paths)
                }
            }
        };

        let terrain_paths = terrain_paths
            .map(|paths| resample_world_paths(&paths, render_segments, sample_segments));

        for stroke in world_circle_strokes(style) {
            let projection = match &terrain_paths {
                Some(paths) => project_world_paths(camera, viewport, paths, stroke),
                None => {
                    project_static_circle(camera, viewport, center, radius, stroke, render_segments)
                }
            };
            let vertices = match projection {
                Ok(vertices) => vertices,
                Err(error) => {
                    self.last_frame.skipped_commands += 1;
                    self.last_frame.last_error = error.diagnostic_code();
                    return;
                }
            };
            self.record_draw(unsafe { draw_triangle_list(device, &vertices) });
        }
    }
}

/// Double-buffered incremental terrain snapshot for one dynamic circle.
///
/// The completed path stays on screen while an updated path is sampled. This
/// prevents a moving player from making the circle blink between cache refreshes.
struct DynamicTerrainCircle {
    active: TerrainCircleCache,
    pending: Option<TerrainCircleCache>,
}

impl DynamicTerrainCircle {
    fn new(center: Position, radius: f32, clearance: f32, segments: usize) -> Self {
        Self {
            active: TerrainCircleCache::new(center, radius, clearance, segments),
            pending: None,
        }
    }

    fn update(
        &mut self,
        center: Position,
        radius: f32,
        clearance: f32,
        segments: usize,
        budget: usize,
    ) {
        // A style change cannot reuse the old vertices. This only happens for
        // an actual API command change, never during normal movement.
        if !self
            .active
            .matches_configuration(radius, clearance, segments)
        {
            self.active = TerrainCircleCache::new(center, radius, clearance, segments);
            self.pending = None;
        }

        if !self.active.complete() {
            self.active.sample_next(budget);
            return;
        }

        if self.pending.is_none()
            && horizontal_center_distance(self.active.center, center) > TERRAIN_REFRESH_DISTANCE
        {
            self.pending = Some(TerrainCircleCache::new(center, radius, clearance, segments));
        }

        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        pending.sample_next(budget);
        if pending.complete() {
            // Atomically exchange only full contours: no transient blank frame.
            if let Some(ready) = self.pending.take() {
                self.active = ready;
            }
        }
    }

    #[must_use]
    fn active_center(&self) -> Position {
        self.active.center
    }

    /// Returns the latest complete contour, translated to this frame's player
    /// position. The normal dynamic path refreshes the complete terrain
    /// contour in that same callback whenever the player has moved.
    fn completed_path(&self, requested_center: Position) -> Option<Vec<Vector3>> {
        let offset_x = requested_center.x - self.active.center.x;
        let offset_y = requested_center.y - self.active.center.y;
        self.active.completed_path().map(|mut path| {
            for point in &mut path {
                point.x += offset_x;
                point.y += offset_y;
            }
            path
        })
    }
}

/// One incrementally sampled terrain contour.
struct TerrainCircleCache {
    center: Position,
    radius: f32,
    clearance: f32,
    samples: Vec<Option<Vector3>>,
    attempts: Vec<u8>,
    next_sample: usize,
    completed_samples: usize,
}

const MAX_TERRAIN_SAMPLE_ATTEMPTS: u8 = 3;
const MAX_TERRAIN_GAP: usize = 2;

impl TerrainCircleCache {
    fn new(center: Position, radius: f32, clearance: f32, segments: usize) -> Self {
        Self {
            center,
            radius,
            clearance,
            samples: vec![None; segments],
            attempts: vec![0; segments],
            next_sample: 0,
            completed_samples: 0,
        }
    }

    fn matches_configuration(&self, radius: f32, clearance: f32, segments: usize) -> bool {
        self.samples.len() == segments && self.radius == radius && self.clearance == clearance
    }

    fn sample_next(&mut self, budget: usize) {
        for _ in 0..budget {
            if self.complete() || self.samples.is_empty() {
                return;
            }
            let index = self.next_sample;
            self.next_sample = (self.next_sample + 1) % self.samples.len();
            if self.samples[index].is_some() || self.attempts[index] >= MAX_TERRAIN_SAMPLE_ATTEMPTS
            {
                continue;
            }
            self.attempts[index] += 1;
            let angle = core::f32::consts::TAU * index as f32 / self.samples.len() as f32;
            let (sin, cos) = angle.sin_cos();
            let x = self.center.x + cos * self.radius;
            let y = self.center.y + sin * self.radius;
            let sample = world_collision::terrain_at(x, y, self.center.z)
                .filter(|point| point.x.is_finite() && point.y.is_finite() && point.z.is_finite())
                .map(|mut point| {
                    point.z += self.clearance;
                    point
                });
            if sample.is_some() {
                self.samples[index] = sample;
                self.completed_samples += 1;
            }

            if self.completed_samples < self.samples.len()
                && self
                    .attempts
                    .iter()
                    .all(|attempts| *attempts >= MAX_TERRAIN_SAMPLE_ATTEMPTS)
            {
                self.repair_small_gaps();
            }
        }
    }

    fn complete(&self) -> bool {
        self.completed_samples == self.samples.len()
    }

    fn completed_path(&self) -> Option<Vec<Vector3>> {
        if !self.complete() {
            return None;
        }
        let mut path: Vec<_> = self.samples.iter().copied().collect::<Option<_>>()?;
        path.push(path[0]);
        Some(path)
    }

    fn repair_small_gaps(&mut self) {
        let count = self.samples.len();
        if count < 3 {
            return;
        }

        // Examine runs starting immediately after a valid circular anchor.
        // This also repairs a miss spanning the last and first sample, which
        // the old linear scan incorrectly treated as two edge gaps.
        for anchor in 0..count {
            let Some(left) = self.samples[anchor] else {
                continue;
            };
            let first_gap = (anchor + 1) % count;
            if self.samples[first_gap].is_some() {
                continue;
            }
            let mut length = 0;
            while length < count && self.samples[(first_gap + length) % count].is_none() {
                length += 1;
            }
            if length == 0 || length > MAX_TERRAIN_GAP || length == count {
                continue;
            }
            let right_index = (first_gap + length) % count;
            let Some(right) = self.samples[right_index] else {
                continue;
            };
            for offset in 0..length {
                let index = (first_gap + offset) % count;
                if self.samples[index].is_some() {
                    continue;
                }
                let factor = (offset + 1) as f32 / (length + 1) as f32;
                self.samples[index] = Some(lerp_vector(left, right, factor));
                self.completed_samples += 1;
            }
        }
    }
}

fn horizontal_center_distance(left: Position, right: Position) -> f32 {
    let x = left.x - right.x;
    let y = left.y - right.y;
    (x * x + y * y).sqrt()
}

/// Resamples one closed angular contour at the visual tessellation density.
///
/// Terrain/collision probes intentionally run on a coarser grid. Because both
/// grids are uniformly spaced around the same centre, circular interpolation
/// preserves the sampled contour without multiplying native client calls.
fn resample_closed_path(path: &[Vector3], segments: usize) -> Option<Vec<Vector3>> {
    if segments < 3 || path.len() < 4 {
        return None;
    }
    let source = if path.first() == path.last() {
        &path[..path.len() - 1]
    } else {
        path
    };
    if source.len() < 3 || source.iter().copied().any(|point| !finite_vector(point)) {
        return None;
    }
    if source.len() == segments {
        let mut result = source.to_vec();
        result.push(result[0]);
        return Some(result);
    }

    let mut result = Vec::with_capacity(segments + 1);
    for index in 0..segments {
        let source_position = index as f32 * source.len() as f32 / segments as f32;
        let left = source_position.floor() as usize % source.len();
        let right = (left + 1) % source.len();
        let factor = source_position - source_position.floor();
        result.push(lerp_vector(source[left], source[right], factor));
    }
    result.push(result[0]);
    Some(result)
}

fn resample_world_paths(
    paths: &[Vec<Vector3>],
    render_segments: usize,
    sample_segments: usize,
) -> Vec<Vec<Vector3>> {
    paths
        .iter()
        .filter_map(|path| {
            let closed = path.len() >= 4 && path.first() == path.last();
            if closed {
                return resample_closed_path(path, render_segments).or_else(|| Some(path.clone()));
            }

            // Occlusion has already split the polar ring into open angular
            // runs. Preserve those boundaries and distribute the final
            // density in proportion to the run's number of sampled arcs.
            if path.len() < 2 || sample_segments < 3 {
                return None;
            }
            let arcs = path.len() - 1;
            let target_arcs = ((arcs as f32 * render_segments as f32 / sample_segments as f32)
                .ceil() as usize)
                .max(arcs);
            Some(resample_open_path(path, target_arcs))
        })
        .collect()
}

fn resample_open_path(path: &[Vector3], target_arcs: usize) -> Vec<Vector3> {
    if target_arcs <= path.len().saturating_sub(1) {
        return path.to_vec();
    }
    let source_arcs = path.len() - 1;
    let mut result = Vec::with_capacity(target_arcs + 1);
    for index in 0..=target_arcs {
        let source_position = index as f32 * source_arcs as f32 / target_arcs as f32;
        let left = (source_position.floor() as usize).min(source_arcs - 1);
        let right = left + 1;
        let factor = if index == target_arcs {
            1.0
        } else {
            source_position - source_position.floor()
        };
        result.push(lerp_vector(path[left], path[right], factor));
    }
    result
}

const fn lerp_vector(from: Vector3, to: Vector3, factor: f32) -> Vector3 {
    Vector3 {
        x: from.x + (to.x - from.x) * factor,
        y: from.y + (to.y - from.y) * factor,
        z: from.z + (to.z - from.z) * factor,
    }
}

const fn finite_vector(point: Vector3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    const fn point(x: f32, y: f32, z: f32) -> Vector3 {
        Vector3 { x, y, z }
    }

    #[test]
    fn closed_contour_resampling_keeps_closure_and_requested_density() {
        let source = [
            point(1.0, 0.0, 0.0),
            point(0.0, 1.0, 1.0),
            point(-1.0, 0.0, 2.0),
            point(0.0, -1.0, 1.0),
            point(1.0, 0.0, 0.0),
        ];
        let result = resample_closed_path(&source, 16).expect("closed contour");

        assert_eq!(result.len(), 17);
        assert_eq!(result.first(), result.last());
        assert!(result.iter().copied().all(finite_vector));
    }

    #[test]
    fn open_visibility_run_stays_open_when_upsampled() {
        let source = vec![
            point(0.0, 0.0, 0.0),
            point(1.0, 1.0, 1.0),
            point(2.0, 0.0, 2.0),
        ];
        let result = resample_world_paths(core::slice::from_ref(&source), 16, 4);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].first(), source.first());
        assert_eq!(result[0].last(), source.last());
        assert_ne!(result[0].first(), result[0].last());
    }
}

/// Distance kept between a radial obstacle hit and the circle line. It avoids
/// z-fighting and makes the resulting inward notch visually legible.
const OBSTACLE_MARGIN: f32 = 0.25;
/// Horizontal model probes sit at a stable character-height slice instead of
/// scraping terrain slopes, which are already handled by vertical samples.
const OBSTACLE_PROBE_HEIGHT: f32 = 1.0;
/// Raise only the visibility-ray target slightly. This keeps an endpoint on
/// terrain from self-occluding while preserving terrain and model occlusion
/// between the camera and the actual line.
const VISIBILITY_TARGET_LIFT: f32 = 0.15;
/// Double-buffered model/visibility state for a terrain circle.
///
/// Terrain samples can be computed in a compact burst because they are
/// vertical rays. Obstacle and camera visibility rays are substantially more
/// expensive, so this cache advances incrementally and only atomically swaps
/// fully sampled collision state. The previously completed state remains
/// usable while player movement starts the next snapshot.
struct DynamicCollisionCircle {
    active: CollisionCircleCache,
    pending: Option<CollisionCircleCache>,
}

impl DynamicCollisionCircle {
    fn new(
        center: Position,
        radius: f32,
        clearance: f32,
        segments: usize,
        path: &[Vector3],
    ) -> Option<Self> {
        let active = CollisionCircleCache::new(center, radius, clearance, segments, path)?;
        Some(Self {
            active,
            pending: None,
        })
    }

    fn matches_configuration(&self, radius: f32, clearance: f32, segments: usize) -> bool {
        self.active
            .matches_configuration(radius, clearance, segments)
    }

    fn update(
        &mut self,
        center: Position,
        radius: f32,
        clearance: f32,
        segments: usize,
        path: &[Vector3],
        camera: CameraState,
        budget: usize,
    ) {
        debug_assert!(self
            .active
            .matches_configuration(radius, clearance, segments));

        if !self.active.complete() {
            self.active.sample_next(camera, budget);
            return;
        }

        if self.pending.is_none()
            && horizontal_center_distance(self.active.center, center) > TERRAIN_REFRESH_DISTANCE
        {
            self.pending = CollisionCircleCache::new(center, radius, clearance, segments, path);
        }

        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        pending.sample_next(camera, budget);
        if pending.complete() {
            if let Some(ready) = self.pending.take() {
                self.active = ready;
            }
        }
    }

    /// Applies the most recent complete collision snapshot to the current
    /// terrain path. Until the first collision snapshot is ready we retain
    /// the terrain-only circle rather than letting setup produce a visual
    /// blank or a burst of unchecked client calls.
    fn apply(&self, center: Position, path: &[Vector3]) -> Vec<Vec<Vector3>> {
        self.active
            .apply(center, path)
            .unwrap_or_else(|| vec![path.to_vec()])
    }
}

/// One angular snapshot of obstacle radial limits and world visibility.
///
/// Every index corresponds to a segment of the terrain ring. The radial
/// limit is later combined with the *current* terrain path, which lets the
/// displayed contour follow the player at render speed without re-running
/// all model queries every frame.
struct CollisionCircleCache {
    center: Position,
    radius: f32,
    clearance: f32,
    base_points: Vec<Vector3>,
    radial_limits: Vec<Option<f32>>,
    visible_segments: Vec<Option<bool>>,
    next_obstacle_sample: usize,
    next_visibility_sample: usize,
    center_ground_z: Option<f32>,
}

impl CollisionCircleCache {
    fn new(
        center: Position,
        radius: f32,
        clearance: f32,
        segments: usize,
        path: &[Vector3],
    ) -> Option<Self> {
        let base_points = unique_ring_points(path, segments)?;
        Some(Self {
            center,
            radius,
            clearance,
            base_points,
            radial_limits: vec![None; segments],
            visible_segments: vec![None; segments],
            next_obstacle_sample: 0,
            next_visibility_sample: 0,
            center_ground_z: None,
        })
    }

    fn matches_configuration(&self, radius: f32, clearance: f32, segments: usize) -> bool {
        self.radial_limits.len() == segments
            && self.visible_segments.len() == segments
            && self.radius == radius
            && self.clearance == clearance
    }

    fn complete(&self) -> bool {
        self.center_ground_z.is_some()
            && self.radial_limits.iter().all(Option::is_some)
            && self.visible_segments.iter().all(Option::is_some)
    }

    fn sample_next(&mut self, camera: CameraState, budget: usize) {
        if self.radial_limits.is_empty() {
            return;
        }
        let center_ground_z = *self.center_ground_z.get_or_insert_with(|| {
            world_collision::terrain_at(self.center.x, self.center.y, self.center.z)
                .map_or(self.center.z, |point| point.z)
        });

        let obstacle_budget = budget.saturating_div(2).max(1);
        for _ in 0..obstacle_budget {
            if self.complete() {
                return;
            }
            let index = self.next_obstacle_sample;
            self.next_obstacle_sample = (self.next_obstacle_sample + 1) % self.radial_limits.len();
            if self.radial_limits[index].is_some() {
                continue;
            }

            let target = self.base_points[index];

            self.radial_limits[index] = Some(obstacle_radial_limit(
                self.center,
                center_ground_z,
                self.radius,
                self.clearance,
                target,
            ));
        }

        for _ in 0..budget.saturating_sub(obstacle_budget) {
            if self.complete() {
                return;
            }
            let index = self.next_visibility_sample;
            self.next_visibility_sample =
                (self.next_visibility_sample + 1) % self.visible_segments.len();
            if self.visible_segments[index].is_some() {
                continue;
            }
            let target = self.base_points[index];
            let radial_limit = self.radial_limits[index].unwrap_or(self.radius);
            let (direction_x, direction_y) =
                radial_direction(self.center, target).unwrap_or((1.0, 0.0));
            let midpoint = Vector3 {
                x: self.center.x + direction_x * radial_limit * 0.5,
                y: self.center.y + direction_y * radial_limit * 0.5,
                z: (self.center.z + target.z) * 0.5 + VISIBILITY_TARGET_LIFT,
            };
            self.visible_segments[index] =
                Some(!world_collision::world_obstructs(camera.eye, midpoint));
        }
    }

    fn apply(&self, requested_center: Position, path: &[Vector3]) -> Option<Vec<Vec<Vector3>>> {
        if !self.complete() || path.len() < 4 {
            return None;
        }
        let source = unique_ring_points(path, self.radial_limits.len())?;

        let raw_limits: Vec<_> = self.radial_limits.iter().copied().collect::<Option<_>>()?;
        let recovery_step = obstacle_recovery_step(self.radius, raw_limits.len())?;
        let smoothed_limits = smooth_radial_limits(&raw_limits, recovery_step)?;
        let mut contoured = Vec::with_capacity(source.len() + 1);
        for (index, (point, radial_limit)) in
            source.iter().copied().zip(smoothed_limits).enumerate()
        {
            if (radial_limit - self.radius).abs() <= 0.001 {
                contoured.push(point);
                continue;
            }
            let (direction_x, direction_y) =
                radial_direction(self.center, self.base_points[index])?;
            let x = requested_center.x + direction_x * radial_limit;
            let y = requested_center.y + direction_y * radial_limit;
            contoured.push(
                world_collision::terrain_at(x, y, requested_center.z)
                    .filter(|sample| {
                        sample.x.is_finite() && sample.y.is_finite() && sample.z.is_finite()
                    })
                    .map(|mut sample| {
                        sample.z += self.clearance;
                        sample
                    })
                    .unwrap_or(Vector3 { x, y, z: point.z }),
            );
        }
        contoured.push(contoured[0]);

        let visible: Vec<_> = self
            .visible_segments
            .iter()
            .copied()
            .collect::<Option<_>>()?;
        split_visible_world_ring(&contoured, &visible)
    }
}

/// Converts a closed or open angular ring into exactly its unique perimeter
/// points. The collision cache must use the actual terrain-contour directions,
/// rather than assuming an XY-perfect circle on sloped/custom maps.
fn unique_ring_points(path: &[Vector3], expected_count: usize) -> Option<Vec<Vector3>> {
    let source = if path.first() == path.last() {
        &path[..path.len().saturating_sub(1)]
    } else {
        path
    };
    (source.len() == expected_count && source.iter().copied().all(finite_vector))
        .then(|| source.to_vec())
}

fn radial_direction(center: Position, point: Vector3) -> Option<(f32, f32)> {
    let x = point.x - center.x;
    let y = point.y - center.y;
    let length_squared = x.mul_add(x, y * y);
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return None;
    }
    let inverse_length = length_squared.sqrt().recip();
    Some((x * inverse_length, y * inverse_length))
}

fn obstacle_radial_limit(
    center: Position,
    center_ground_z: f32,
    radius: f32,
    terrain_clearance: f32,
    target: Vector3,
) -> f32 {
    let delta_x = target.x - center.x;
    let delta_y = target.y - center.y;
    let radius_squared = delta_x * delta_x + delta_y * delta_y;
    if !radius_squared.is_finite() || radius_squared <= OBSTACLE_MARGIN * OBSTACLE_MARGIN {
        return radius;
    }

    let start = Vector3 {
        x: center.x,
        y: center.y,
        z: center_ground_z + terrain_clearance + OBSTACLE_PROBE_HEIGHT,
    };
    let end = Vector3 {
        x: target.x,
        y: target.y,
        z: target.z + OBSTACLE_PROBE_HEIGHT,
    };
    let Ok(Some(hit)) = world_collision::obstacle_between(start, end) else {
        return radius;
    };

    let hit_x = hit.position.x - center.x;
    let hit_y = hit.position.y - center.y;
    let hit_factor = (hit_x * delta_x + hit_y * delta_y) / radius_squared;
    if !hit_factor.is_finite() || !(0.0..1.0).contains(&hit_factor) {
        return radius;
    }

    let target_radius = radius_squared.sqrt();
    (target_radius * hit_factor - OBSTACLE_MARGIN).clamp(radius * 0.025, radius)
}

fn project_world_paths(
    camera: CameraState,
    viewport: Viewport,
    paths: &[Vec<Vector3>],
    stroke: Stroke,
) -> Result<Vec<OverlayVertex>, ProjectionError> {
    let mut vertices = Vec::new();
    for path in paths {
        match project_world_path(camera, viewport, path, stroke) {
            Ok(path_vertices) => vertices.extend(path_vertices),
            Err(ProjectionError::NoVisibleSegments) => {}
            Err(error) => return Err(error),
        }
    }
    (!vertices.is_empty())
        .then_some(vertices)
        .ok_or(ProjectionError::NoVisibleSegments)
}

fn world_circle_strokes(style: WorldCircleStyle) -> Vec<Stroke> {
    let mut strokes = Vec::with_capacity(4);
    if !matches!(style.glow, WorldCircleGlow::None) {
        let glow_width = style.glow_width.max(1.0);
        strokes.push(Stroke::new(
            style.stroke.color.with_alpha(40),
            style.stroke.width + glow_width * 2.0,
        ));
        strokes.push(Stroke::new(
            style.stroke.color.with_alpha(100),
            style.stroke.width + glow_width,
        ));
    }
    strokes.push(style.stroke);
    strokes
}

impl NativeRenderer for D3d9OverlayRenderer {
    fn render(&mut self, frame: NativeFrame<'_>, commands: &[QueuedRenderCommand]) {
        self.last_frame = RendererStats {
            submitted_commands: commands.len().try_into().unwrap_or(u32::MAX),
            ..RendererStats::default()
        };

        if commands.is_empty() {
            self.publish_last_frame();
            return;
        }

        let Some(device) = frame.d3d9_device() else {
            self.last_frame.state_setup_failed = true;
            self.last_frame.skipped_commands = self.last_frame.submitted_commands;
            self.last_frame.last_error = E_FAIL as u32;
            self.publish_last_frame();
            return;
        };
        let _saved_state = match unsafe { SavedState::capture(device) } {
            Ok(state) => state,
            Err(result) => {
                self.last_frame.state_setup_failed = true;
                self.last_frame.skipped_commands = self.last_frame.submitted_commands;
                self.last_frame.last_error = result as u32;
                self.publish_last_frame();
                return;
            }
        };

        if let Err(result) = unsafe { configure_overlay(device) } {
            self.last_frame.state_setup_failed = true;
            self.last_frame.skipped_commands = self.last_frame.submitted_commands;
            self.last_frame.last_error = result as u32;
            self.publish_last_frame();
            return;
        }

        for queued in commands {
            match &queued.command {
                RenderCommand::WorldCircle {
                    center,
                    radius,
                    style,
                } => {
                    self.draw_world_circle(device, *center, *radius, *style);
                }
                RenderCommand::HudLine { from, to, stroke } => {
                    let Some(vertices) = line_strip(*from, *to, *stroke) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    self.record_draw(unsafe { draw_strip(device, &vertices) });
                }
                RenderCommand::HudCircle {
                    center,
                    radius,
                    stroke,
                } => {
                    let Some(segments) = hud_segments(*radius, 0) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    let Some(vertices) = circle_strip(*center, *radius, *stroke, segments) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    self.record_draw(unsafe { draw_strip(device, &vertices) });
                }
                RenderCommand::HudText { .. } => {
                    self.last_frame.skipped_commands += 1;
                }
            }
        }
        self.publish_last_frame();
    }
}

struct SavedState {
    device: *mut c_void,
    render_states: [(u32, u32); OVERLAY_RENDER_STATES.len()],
    texture_stage_states: [(u32, u32); OVERLAY_TEXTURE_STAGE_STATES.len()],
    texture: *mut c_void,
    vertex_declaration: *mut c_void,
    fvf: u32,
    vertex_shader: *mut c_void,
    pixel_shader: *mut c_void,
    stream_zero: *mut c_void,
    stream_zero_offset: u32,
    stream_zero_stride: u32,
}

impl SavedState {
    unsafe fn capture(device: *mut c_void) -> Result<Self, i32> {
        let get_render_state: GetRenderStateFn =
            unsafe { device_method(device, DEVICE_GET_RENDER_STATE) };
        let get_texture: GetTextureFn = unsafe { device_method(device, DEVICE_GET_TEXTURE) };
        let get_texture_stage_state: GetTextureStageStateFn =
            unsafe { device_method(device, DEVICE_GET_TEXTURE_STAGE_STATE) };
        let get_vertex_declaration: GetVertexDeclarationFn =
            unsafe { device_method(device, DEVICE_GET_VERTEX_DECLARATION) };
        let get_fvf: GetFvfFn = unsafe { device_method(device, DEVICE_GET_FVF) };
        let get_vertex_shader: GetVertexShaderFn =
            unsafe { device_method(device, DEVICE_GET_VERTEX_SHADER) };
        let get_pixel_shader: GetPixelShaderFn =
            unsafe { device_method(device, DEVICE_GET_PIXEL_SHADER) };
        let get_stream_source: GetStreamSourceFn =
            unsafe { device_method(device, DEVICE_GET_STREAM_SOURCE) };

        let mut render_states = [(0_u32, 0_u32); OVERLAY_RENDER_STATES.len()];
        for (saved, state) in render_states.iter_mut().zip(OVERLAY_RENDER_STATES) {
            let mut value = 0_u32;
            let result = unsafe { get_render_state(device, state, &mut value) };
            if !succeeded(result) {
                return Err(result);
            }
            *saved = (state, value);
        }

        let mut texture_stage_states = [(0_u32, 0_u32); OVERLAY_TEXTURE_STAGE_STATES.len()];
        for (saved, state) in texture_stage_states
            .iter_mut()
            .zip(OVERLAY_TEXTURE_STAGE_STATES)
        {
            let mut value = 0_u32;
            let result = unsafe { get_texture_stage_state(device, 0, state, &mut value) };
            if !succeeded(result) {
                return Err(result);
            }
            *saved = (state, value);
        }

        let mut texture = ptr::null_mut();
        let result = unsafe { get_texture(device, 0, &mut texture) };
        if !succeeded(result) {
            return Err(result);
        }
        let mut vertex_declaration = ptr::null_mut();
        let result = unsafe { get_vertex_declaration(device, &mut vertex_declaration) };
        if !succeeded(result) {
            release_com(texture);
            return Err(result);
        }
        let mut fvf = 0_u32;
        let result = unsafe { get_fvf(device, &mut fvf) };
        if !succeeded(result) {
            release_com(texture);
            release_com(vertex_declaration);
            return Err(result);
        }
        let mut vertex_shader = ptr::null_mut();
        let result = unsafe { get_vertex_shader(device, &mut vertex_shader) };
        if !succeeded(result) {
            release_com(texture);
            release_com(vertex_declaration);
            return Err(result);
        }
        let mut pixel_shader = ptr::null_mut();
        let result = unsafe { get_pixel_shader(device, &mut pixel_shader) };
        if !succeeded(result) {
            release_com(texture);
            release_com(vertex_declaration);
            release_com(vertex_shader);
            return Err(result);
        }
        let mut stream_zero = ptr::null_mut();
        let mut stream_zero_offset = 0_u32;
        let mut stream_zero_stride = 0_u32;
        let result = unsafe {
            get_stream_source(
                device,
                0,
                &mut stream_zero,
                &mut stream_zero_offset,
                &mut stream_zero_stride,
            )
        };
        if !succeeded(result) {
            release_com(texture);
            release_com(vertex_declaration);
            release_com(vertex_shader);
            release_com(pixel_shader);
            return Err(result);
        }

        Ok(Self {
            device,
            render_states,
            texture_stage_states,
            texture,
            vertex_declaration,
            fvf,
            vertex_shader,
            pixel_shader,
            stream_zero,
            stream_zero_offset,
            stream_zero_stride,
        })
    }
}

impl Drop for SavedState {
    fn drop(&mut self) {
        unsafe {
            let set_render_state: SetRenderStateFn =
                device_method(self.device, DEVICE_SET_RENDER_STATE);
            let set_texture: SetTextureFn = device_method(self.device, DEVICE_SET_TEXTURE);
            let set_texture_stage_state: SetTextureStageStateFn =
                device_method(self.device, DEVICE_SET_TEXTURE_STAGE_STATE);
            let set_vertex_declaration: SetVertexDeclarationFn =
                device_method(self.device, DEVICE_SET_VERTEX_DECLARATION);
            let set_fvf: SetFvfFn = device_method(self.device, DEVICE_SET_FVF);
            let set_vertex_shader: SetVertexShaderFn =
                device_method(self.device, DEVICE_SET_VERTEX_SHADER);
            let set_pixel_shader: SetPixelShaderFn =
                device_method(self.device, DEVICE_SET_PIXEL_SHADER);
            let set_stream_source: SetStreamSourceFn =
                device_method(self.device, DEVICE_SET_STREAM_SOURCE);

            for &(state, value) in &self.render_states {
                let _ = set_render_state(self.device, state, value);
            }
            for &(state, value) in &self.texture_stage_states {
                let _ = set_texture_stage_state(self.device, 0, state, value);
            }
            let _ = set_fvf(self.device, self.fvf);
            let _ = set_vertex_declaration(self.device, self.vertex_declaration);
            let _ = set_vertex_shader(self.device, self.vertex_shader);
            let _ = set_pixel_shader(self.device, self.pixel_shader);
            let _ = set_texture(self.device, 0, self.texture);
            // DrawPrimitiveUP clears stream zero as a documented side effect.
            // Restore it before releasing the reference obtained by
            // GetStreamSource, otherwise the game's next draw can inherit a
            // null vertex stream after our overlay returns.
            let _ = set_stream_source(
                self.device,
                0,
                self.stream_zero,
                self.stream_zero_offset,
                self.stream_zero_stride,
            );

            release_com(self.texture);
            release_com(self.vertex_declaration);
            release_com(self.vertex_shader);
            release_com(self.pixel_shader);
            release_com(self.stream_zero);
        }
    }
}

unsafe fn release_com(value: *mut c_void) {
    if !value.is_null() {
        let release: ReleaseFn = unsafe { com_method(value, IUNKNOWN_RELEASE) };
        let _ = unsafe { release(value) };
    }
}

unsafe fn configure_overlay(device: *mut c_void) -> Result<(), i32> {
    let set_render_state: SetRenderStateFn =
        unsafe { device_method(device, DEVICE_SET_RENDER_STATE) };
    let set_texture: SetTextureFn = unsafe { device_method(device, DEVICE_SET_TEXTURE) };
    let set_texture_stage_state: SetTextureStageStateFn =
        unsafe { device_method(device, DEVICE_SET_TEXTURE_STAGE_STATE) };
    let set_vertex_declaration: SetVertexDeclarationFn =
        unsafe { device_method(device, DEVICE_SET_VERTEX_DECLARATION) };
    let set_vertex_shader: SetVertexShaderFn =
        unsafe { device_method(device, DEVICE_SET_VERTEX_SHADER) };
    let set_pixel_shader: SetPixelShaderFn =
        unsafe { device_method(device, DEVICE_SET_PIXEL_SHADER) };
    let set_fvf: SetFvfFn = unsafe { device_method(device, DEVICE_SET_FVF) };

    let render_states = [
        (D3DRS_ZENABLE, 0),
        (D3DRS_ZWRITEENABLE, 0),
        (D3DRS_ALPHATESTENABLE, 0),
        (D3DRS_ALPHABLENDENABLE, 1),
        (D3DRS_SRCBLEND, D3DBLEND_SRCALPHA),
        (D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA),
        (D3DRS_CULLMODE, D3DCULL_NONE),
        (D3DRS_FOGENABLE, 0),
        (D3DRS_LIGHTING, 0),
        (D3DRS_SCISSORTESTENABLE, 0),
    ];
    let texture_states = [
        (D3DTSS_COLOROP, D3DTOP_SELECTARG1),
        (D3DTSS_COLORARG1, D3DTA_DIFFUSE),
        (D3DTSS_ALPHAOP, D3DTOP_SELECTARG1),
        (D3DTSS_ALPHAARG1, D3DTA_DIFFUSE),
    ];

    macro_rules! require_d3d_ok {
        ($call:expr) => {
            let result = unsafe { $call };
            if !succeeded(result) {
                return Err(result);
            }
        };
    }

    require_d3d_ok!(set_vertex_declaration(device, ptr::null_mut()));
    require_d3d_ok!(set_vertex_shader(device, ptr::null_mut()));
    require_d3d_ok!(set_pixel_shader(device, ptr::null_mut()));
    require_d3d_ok!(set_fvf(device, OVERLAY_FVF));
    require_d3d_ok!(set_texture(device, 0, ptr::null_mut()));
    for &(state, value) in &render_states {
        require_d3d_ok!(set_render_state(device, state, value));
    }
    for &(state, value) in &texture_states {
        require_d3d_ok!(set_texture_stage_state(device, 0, state, value));
    }
    Ok(())
}

unsafe fn draw_strip(device: *mut c_void, vertices: &[OverlayVertex]) -> i32 {
    let Some(primitive_count) = vertices
        .len()
        .checked_sub(2)
        .and_then(|count| count.try_into().ok())
    else {
        return E_FAIL;
    };
    let vertices: Vec<D3d9Vertex> = vertices
        .iter()
        .copied()
        .map(D3d9Vertex::from_overlay)
        .collect();
    let draw: DrawPrimitiveUpFn = unsafe { device_method(device, DEVICE_DRAW_PRIMITIVE_UP) };
    let stride = u32::try_from(core::mem::size_of::<D3d9Vertex>()).expect("vertex stride fits u32");
    unsafe {
        draw(
            device,
            D3DPT_TRIANGLESTRIP,
            primitive_count,
            vertices.as_ptr().cast(),
            stride,
        )
    }
}

unsafe fn draw_triangle_list(device: *mut c_void, vertices: &[OverlayVertex]) -> i32 {
    let Some(primitive_count) = vertices
        .len()
        .checked_div(3)
        .and_then(|count| count.try_into().ok())
    else {
        return E_FAIL;
    };
    if primitive_count == 0 || vertices.len() % 3 != 0 {
        return E_FAIL;
    }
    let vertices: Vec<D3d9Vertex> = vertices
        .iter()
        .copied()
        .map(D3d9Vertex::from_overlay)
        .collect();
    let draw: DrawPrimitiveUpFn = unsafe { device_method(device, DEVICE_DRAW_PRIMITIVE_UP) };
    let stride = u32::try_from(core::mem::size_of::<D3d9Vertex>()).expect("vertex stride fits u32");
    unsafe {
        draw(
            device,
            D3DPT_TRIANGLELIST,
            primitive_count,
            vertices.as_ptr().cast(),
            stride,
        )
    }
}

unsafe fn d3d_viewport(device: *mut c_void) -> Result<Viewport, i32> {
    let get_viewport: GetViewportFn = unsafe { device_method(device, DEVICE_GET_VIEWPORT) };
    let mut viewport = D3dViewport::default();
    let result = unsafe { get_viewport(device, &mut viewport) };
    if !succeeded(result) {
        return Err(result);
    }
    if viewport.width == 0 || viewport.height == 0 {
        return Err(E_FAIL);
    }
    Ok(Viewport {
        x: viewport.x,
        y: viewport.y,
        width: viewport.width,
        height: viewport.height,
    })
}

/// Exact layout of D3D9's `D3DVIEWPORT9` structure.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct D3dViewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    _min_z: f32,
    _max_z: f32,
}

impl Default for D3dViewport {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            _min_z: 0.0,
            _max_z: 0.0,
        }
    }
}

/// Transformed fixed-function vertex owned solely by the D3D9 backend.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct D3d9Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
}

impl D3d9Vertex {
    const fn from_overlay(vertex: OverlayVertex) -> Self {
        Self {
            // D3D9 transformed vertices address pixel centres at half-pixel
            // coordinates. Other backends must not inherit this adjustment.
            x: vertex.x - 0.5,
            y: vertex.y - 0.5,
            z: 0.0,
            rhw: 1.0,
            color: u32::from_be_bytes([
                vertex.color.a,
                vertex.color.r,
                vertex.color.g,
                vertex.color.b,
            ]),
        }
    }
}

unsafe fn device_method<T>(device: *mut c_void, slot: usize) -> T {
    let address = unsafe { vtable_entry(device, slot) };
    // This is a Windows x86-only COM call boundary. `T` is always one of the
    // exact function-pointer types above, all pointer-sized on this target.
    unsafe { core::mem::transmute_copy(&address) }
}

unsafe fn com_method<T>(interface: *mut c_void, slot: usize) -> T {
    let address = unsafe { vtable_entry(interface, slot) };
    // See `device_method`: this is local COM pointer handling, unrelated to
    // the game's 32-bit RemoteAddress representation.
    unsafe { core::mem::transmute_copy(&address) }
}

unsafe fn vtable_entry(interface: *mut c_void, slot: usize) -> *const c_void {
    let vtable = unsafe { *(interface.cast::<*const *const c_void>()) };
    unsafe { *vtable.add(slot) }
}

const fn succeeded(result: i32) -> bool {
    result >= 0
}
