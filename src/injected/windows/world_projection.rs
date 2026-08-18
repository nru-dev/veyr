//! Projection of world primitives through the verified build-12340 camera.
//!
//! World geometry is emitted as ordinary, viewport-clipped overlay triangles.
//! The renderer therefore never changes WoW's own transforms or depth state.
//! A bad snapshot simply rejects one frame rather than producing unbounded
//! screen geometry.

use crate::injected::overlay_geometry::{polyline_triangles, OverlayVertex};
use crate::offsets::api::{CameraState, Position, ScreenPoint, Stroke, Vector3};

/// Why a world primitive could not be converted into bounded screen geometry.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum ProjectionError {
    InvalidInput,
    InvalidCamera,
    NoVisibleSegments,
}

impl ProjectionError {
    pub(crate) const fn diagnostic_code(self) -> u32 {
        match self {
            Self::InvalidInput => 0xE201,
            Self::InvalidCamera => 0xE207,
            Self::NoVisibleSegments => 0xE202,
        }
    }
}

/// Viewport used to convert camera-space positions into pixels.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

// Projecting a point a few centimetres in front of the eye is numerically
// valid but visually disastrous: a short ground segment becomes a line across
// the whole viewport. Clip in world space before the perspective divide.
const NEAR_PLANE: f32 = 0.5;

/// Projects one static circle in WoW's XY ground plane (`Z` is height).
pub(crate) fn project_static_circle(
    camera: CameraState,
    viewport: Viewport,
    center: Position,
    radius: f32,
    stroke: Stroke,
    segments: usize,
) -> Result<Vec<OverlayVertex>, ProjectionError> {
    if viewport.width == 0
        || viewport.height == 0
        || !finite_position(center)
        || !radius.is_finite()
        || radius <= 0.0
        || !stroke.width.is_finite()
        || stroke.width <= 0.0
        || segments < 3
    {
        return Err(ProjectionError::InvalidInput);
    }
    let points: Vec<_> = (0..=segments)
        .map(|segment| {
            let angle = core::f32::consts::TAU * segment as f32 / segments as f32;
            let (sin, cos) = angle.sin_cos();
            Vector3 {
                x: center.x + cos * radius,
                y: center.y + sin * radius,
                z: center.z,
            }
        })
        .collect();

    project_world_path(camera, viewport, &points, stroke)
}

/// Projects a contiguous world-space polyline through the live client camera.
///
/// Every segment is clipped against the camera near plane before perspective
/// division. This keeps points moving behind the camera from producing the
/// diagonal full-screen artefacts seen in the original circle diagnostic.
pub(crate) fn project_world_path(
    camera: CameraState,
    viewport: Viewport,
    points: &[Vector3],
    stroke: Stroke,
) -> Result<Vec<OverlayVertex>, ProjectionError> {
    if viewport.width == 0
        || viewport.height == 0
        || points.len() < 2
        || !points.iter().copied().all(finite_vector)
        || !stroke.width.is_finite()
        || stroke.width <= 0.0
    {
        return Err(ProjectionError::InvalidInput);
    }
    let camera = ValidatedCamera::try_from(camera)?;

    let mut vertices = Vec::with_capacity((points.len() - 1) * 6);
    let mut visible_path = Vec::new();
    for pair in points.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        let Some((from, to)) = clip_segment_to_near_plane(camera, *from, *to) else {
            flush_projected_path(&mut visible_path, stroke, &mut vertices);
            continue;
        };
        let Some(from) = project_point(camera, viewport, from) else {
            flush_projected_path(&mut visible_path, stroke, &mut vertices);
            continue;
        };
        let Some(to) = project_point(camera, viewport, to) else {
            flush_projected_path(&mut visible_path, stroke, &mut vertices);
            continue;
        };
        let Some((from, to)) = clip_segment_to_viewport(from, to, viewport) else {
            flush_projected_path(&mut visible_path, stroke, &mut vertices);
            continue;
        };
        if visible_path
            .last()
            .is_some_and(|last| !approximately_same(*last, from))
        {
            flush_projected_path(&mut visible_path, stroke, &mut vertices);
        }
        if visible_path.is_empty() {
            visible_path.push(from);
        }
        visible_path.push(to);
    }
    flush_projected_path(&mut visible_path, stroke, &mut vertices);

    (!vertices.is_empty())
        .then_some(vertices)
        .ok_or(ProjectionError::NoVisibleSegments)
}

fn flush_projected_path(
    path: &mut Vec<ScreenPoint>,
    stroke: Stroke,
    vertices: &mut Vec<OverlayVertex>,
) {
    if let Some(path_vertices) = polyline_triangles(path, stroke) {
        vertices.extend(path_vertices);
    }
    path.clear();
}

fn approximately_same(left: ScreenPoint, right: ScreenPoint) -> bool {
    const EPSILON: f32 = 0.01;
    (left.x - right.x).abs() <= EPSILON && (left.y - right.y).abs() <= EPSILON
}

#[derive(Debug, Copy, Clone)]
struct ValidatedCamera {
    eye: Vector3,
    forward: Vector3,
    left: Vector3,
    up: Vector3,
    tangent_horizontal: f32,
}

impl TryFrom<CameraState> for ValidatedCamera {
    type Error = ProjectionError;

    fn try_from(camera: CameraState) -> Result<Self, Self::Error> {
        if !finite_vector(camera.eye)
            || !finite_vector(camera.forward)
            || !finite_vector(camera.left)
            || !camera.field_of_view.is_finite()
            || !(0.1..3.0).contains(&camera.field_of_view)
            || !finite_vector(Vector3 {
                x: camera.roll,
                y: camera.yaw,
                z: camera.pitch,
            })
        {
            return Err(ProjectionError::InvalidCamera);
        }

        let forward_length = length(camera.forward);
        let left_length = length(camera.left);
        if !(0.8..=1.2).contains(&forward_length)
            || !(0.8..=1.2).contains(&left_length)
            || dot(camera.forward, camera.left).abs() > 0.2
        {
            return Err(ProjectionError::InvalidCamera);
        }

        let forward = scale(camera.forward, forward_length.recip());
        let left = scale(camera.left, left_length.recip());
        let up = cross(forward, left);
        let up_length = length(up);
        if !(0.9..=1.1).contains(&up_length) {
            return Err(ProjectionError::InvalidCamera);
        }
        let tangent_horizontal = (camera.field_of_view * 0.5).tan();
        if !tangent_horizontal.is_finite() || tangent_horizontal <= 0.0 {
            return Err(ProjectionError::InvalidCamera);
        }

        Ok(Self {
            eye: camera.eye,
            forward,
            left,
            up: scale(up, up_length.recip()),
            tangent_horizontal,
        })
    }
}

fn project_point(
    camera: ValidatedCamera,
    viewport: Viewport,
    point: Vector3,
) -> Option<ScreenPoint> {
    let relative = Vector3 {
        x: point.x - camera.eye.x,
        y: point.y - camera.eye.y,
        z: point.z - camera.eye.z,
    };
    let depth = dot(relative, camera.forward);
    if !depth.is_finite() || depth < NEAR_PLANE {
        return None;
    }

    // `left` is positive toward the visible left side, hence the minus sign
    // when mapping it into conventional screen coordinates.
    let normal_x = -dot(relative, camera.left) / (depth * camera.tangent_horizontal);
    let aspect = viewport.width as f32 / viewport.height as f32;
    let tangent_vertical = camera.tangent_horizontal / aspect;
    if !tangent_vertical.is_finite() || tangent_vertical <= 0.0 {
        return None;
    }
    let normal_y = dot(relative, camera.up) / (depth * tangent_vertical);
    let x = viewport.x as f32 + (normal_x + 1.0) * viewport.width as f32 * 0.5;
    let y = viewport.y as f32 + (1.0 - normal_y) * viewport.height as f32 * 0.5;
    (x.is_finite() && y.is_finite() && x.abs() <= 1_000_000.0 && y.abs() <= 1_000_000.0)
        .then_some(ScreenPoint::new(x, y))
}

/// Clips one world-space segment to the camera near plane.
fn clip_segment_to_near_plane(
    camera: ValidatedCamera,
    from: Vector3,
    to: Vector3,
) -> Option<(Vector3, Vector3)> {
    let from_depth = depth(camera, from);
    let to_depth = depth(camera, to);
    if !from_depth.is_finite() || !to_depth.is_finite() {
        return None;
    }
    if from_depth < NEAR_PLANE && to_depth < NEAR_PLANE {
        return None;
    }
    if from_depth >= NEAR_PLANE && to_depth >= NEAR_PLANE {
        return Some((from, to));
    }

    let delta = to_depth - from_depth;
    if delta.abs() <= f32::EPSILON {
        return None;
    }
    let factor = (NEAR_PLANE - from_depth) / delta;
    if !(0.0..=1.0).contains(&factor) {
        return None;
    }
    let intersection = lerp(from, to, factor);
    if from_depth < NEAR_PLANE {
        Some((intersection, to))
    } else {
        Some((from, intersection))
    }
}

fn depth(camera: ValidatedCamera, point: Vector3) -> f32 {
    dot(
        Vector3 {
            x: point.x - camera.eye.x,
            y: point.y - camera.eye.y,
            z: point.z - camera.eye.z,
        },
        camera.forward,
    )
}

const fn lerp(from: Vector3, to: Vector3, factor: f32) -> Vector3 {
    Vector3 {
        x: from.x + (to.x - from.x) * factor,
        y: from.y + (to.y - from.y) * factor,
        z: from.z + (to.z - from.z) * factor,
    }
}

/// Clips a finite segment to the native viewport.
fn clip_segment_to_viewport(
    from: ScreenPoint,
    to: ScreenPoint,
    viewport: Viewport,
) -> Option<(ScreenPoint, ScreenPoint)> {
    let min_x = viewport.x as f32;
    let max_x = min_x + viewport.width as f32;
    let min_y = viewport.y as f32;
    let max_y = min_y + viewport.height as f32;
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let mut enter = 0.0_f32;
    let mut leave = 1.0_f32;

    for (p, q) in [
        (-dx, from.x - min_x),
        (dx, max_x - from.x),
        (-dy, from.y - min_y),
        (dy, max_y - from.y),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let parameter = q / p;
        if p < 0.0 {
            if parameter > leave {
                return None;
            }
            enter = enter.max(parameter);
        } else {
            if parameter < enter {
                return None;
            }
            leave = leave.min(parameter);
        }
    }

    (enter <= leave).then(|| {
        (
            ScreenPoint::new(from.x + dx * enter, from.y + dy * enter),
            ScreenPoint::new(from.x + dx * leave, from.y + dy * leave),
        )
    })
}

const fn dot(a: Vector3, b: Vector3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

const fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

const fn scale(vector: Vector3, scalar: f32) -> Vector3 {
    Vector3 {
        x: vector.x * scalar,
        y: vector.y * scalar,
        z: vector.z * scalar,
    }
}

fn length(vector: Vector3) -> f32 {
    dot(vector, vector).sqrt()
}

const fn finite_position(position: Position) -> bool {
    position.x.is_finite() && position.y.is_finite() && position.z.is_finite()
}

const fn finite_vector(vector: Vector3) -> bool {
    vector.x.is_finite() && vector.y.is_finite() && vector.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offsets::api::Color;

    const CAMERA: CameraState = CameraState {
        eye: Vector3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        forward: Vector3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        left: Vector3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        roll: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        field_of_view: core::f32::consts::FRAC_PI_2,
    };

    const VIEWPORT: Viewport = Viewport {
        x: 0,
        y: 0,
        width: 1600,
        height: 900,
    };

    #[test]
    fn projects_a_visible_circle_to_triangles() {
        let vertices = project_static_circle(
            CAMERA,
            VIEWPORT,
            Position {
                x: 20.0,
                y: 0.0,
                z: 0.0,
                rotation: 0.0,
            },
            5.0,
            Stroke::new(Color::GREEN, 2.0),
            12,
        )
        .expect("visible circle");

        assert_eq!(vertices.len(), 12 * 6);
        assert!(vertices
            .iter()
            .all(|vertex| vertex.x.is_finite() && vertex.y.is_finite()));
    }

    #[test]
    fn rejects_a_circle_behind_the_camera() {
        assert_eq!(
            project_static_circle(
                CAMERA,
                VIEWPORT,
                Position {
                    x: -20.0,
                    y: 0.0,
                    z: 0.0,
                    rotation: 0.0,
                },
                5.0,
                Stroke::new(Color::GREEN, 2.0),
                12,
            ),
            Err(ProjectionError::NoVisibleSegments)
        );
    }
}
