//! Backend-neutral quality rules and geometry helpers for circles.
//!
//! World circles need a stable physical sampling density: a larger radius
//! covers more terrain and collision geometry, so a fixed segment count is
//! incorrect. HUD circles use pixels instead and therefore have a separate
//! tessellation rule.

use crate::offsets::api::{ScreenPoint, Vector3};

/// Desired distance between adjacent samples of a world-space circle.
///
/// At radius 20 this produces 848 samples instead of the old fixed 256. The
/// value is deliberately expressed in world units, so doubling the radius
/// approximately doubles the number of terrain and collision samples.
const TARGET_WORLD_CHORD: f32 = 0.15;
const MIN_WORLD_SEGMENTS: usize = 128;
const MAX_WORLD_SEGMENTS: usize = 1_536;

/// Native terrain/model queries are deliberately coarser than final visual
/// tessellation. The resulting polar contour is resampled at render density,
/// so quality scales without multiplying expensive client raycasts by every
/// extra visual vertex.
const TARGET_WORLD_SAMPLE_CHORD: f32 = 0.35;
const MIN_WORLD_SAMPLE_SEGMENTS: usize = 192;
const MAX_WORLD_SAMPLE_SEGMENTS: usize = 768;

/// HUD outlines are tessellated according to their screen-space circumference.
const TARGET_HUD_CHORD_PIXELS: f32 = 1.5;
const MIN_HUD_SEGMENTS: usize = 32;
const MAX_HUD_SEGMENTS: usize = 768;
const SEGMENT_ALIGNMENT: usize = 16;

/// Maximum screen-space miter length relative to half the line width.
pub(crate) const POLYLINE_MITER_LIMIT: f32 = 2.5;

/// Selects physical sampling density for a world circle.
#[must_use]
pub(crate) fn world_segments(radius: f32, requested_floor: usize) -> Option<usize> {
    adaptive_segments(
        radius,
        TARGET_WORLD_CHORD,
        MIN_WORLD_SEGMENTS.max(requested_floor),
        MAX_WORLD_SEGMENTS,
    )
}

/// Selects the native terrain/collision sampling grid for a world circle.
#[must_use]
pub(crate) fn world_sample_segments(radius: f32) -> Option<usize> {
    adaptive_segments(
        radius,
        TARGET_WORLD_SAMPLE_CHORD,
        MIN_WORLD_SAMPLE_SEGMENTS,
        MAX_WORLD_SAMPLE_SEGMENTS,
    )
}

/// Selects visual tessellation density for a screen-space circle.
#[must_use]
pub(crate) fn hud_segments(radius_pixels: f32, requested_floor: usize) -> Option<usize> {
    adaptive_segments(
        radius_pixels,
        TARGET_HUD_CHORD_PIXELS,
        MIN_HUD_SEGMENTS.max(requested_floor),
        MAX_HUD_SEGMENTS,
    )
}

fn adaptive_segments(
    radius: f32,
    target_chord: f32,
    requested_floor: usize,
    maximum: usize,
) -> Option<usize> {
    if !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let raw = (core::f32::consts::TAU * radius / target_chord).ceil();
    if !raw.is_finite() || raw <= 0.0 {
        return None;
    }
    let raw = raw.min(maximum as f32) as usize;
    Some(
        align_up(raw.max(requested_floor.min(maximum)), SEGMENT_ALIGNMENT)
            .min(maximum)
            .max(3),
    )
}

const fn align_up(value: usize, alignment: usize) -> usize {
    value
        .saturating_add(alignment - 1)
        .saturating_div(alignment)
        .saturating_mul(alignment)
}

/// Smooths radial collision limits without ever expanding past a hit.
///
/// `raw` contains independent radial probes. A single unlucky probe can make
/// an isolated needle-shaped inward spike, while a true obstruction should
/// occupy a coherent angular range. This circular morphology pass fills only
/// short *outward* holes (a closing), then limits how quickly the contour can
/// recover after a real inward notch. Every returned radius remains at or
/// inside the corresponding raw collision limit.
///
/// `maximum_step` is expressed in world units between neighbouring angular
/// samples. It must therefore scale with the physical circumference per
/// segment instead of being a fixed magic number for every radius.
pub(crate) fn smooth_radial_limits(raw: &[f32], maximum_step: f32) -> Option<Vec<f32>> {
    if raw.is_empty()
        || !maximum_step.is_finite()
        || maximum_step <= 0.0
        || raw.iter().any(|value| !value.is_finite() || *value < 0.0)
    {
        return None;
    }

    let count = raw.len();
    if count == 1 {
        return Some(raw.to_vec());
    }

    // Isolated high values inside an inward notch are normally a missed ray
    // at an obstacle edge. Fill only holes whose neighbouring samples agree
    // that they are part of the *same* obstruction. In particular, do not
    // erase a legitimate one-segment clear gap between two unrelated objects.
    // The pass is circular, so an obstruction spanning the zero-angle seam
    // behaves like every other one.
    let mut denoised = raw.to_vec();
    for index in 0..count {
        let previous = raw[(index + count - 1) % count];
        let current = raw[index];
        let next = raw[(index + 1) % count];
        let neighbour_ceiling = previous.max(next).min(current);
        let neighbours_agree = (previous - next).abs() <= maximum_step * 2.0;
        // A hole can be arbitrarily deep compared with its neighbours: a
        // collision ray either missed a thin feature or it did not. What
        // matters here is its one-sample angular support, not amplitude.
        if current > neighbour_ceiling && neighbours_agree {
            denoised[index] = neighbour_ceiling;
        }
    }

    // Three copies turn circular distance into ordinary linear distance for
    // every element in the middle copy. A forward/backward min-plus pass then
    // computes min_j(raw[j] + maximum_step * angular_distance(i, j)).
    let mut envelope = Vec::with_capacity(count * 3);
    envelope.extend_from_slice(&denoised);
    envelope.extend_from_slice(&denoised);
    envelope.extend_from_slice(&denoised);

    for index in 1..envelope.len() {
        envelope[index] = envelope[index].min(envelope[index - 1] + maximum_step);
    }
    for index in (0..envelope.len() - 1).rev() {
        envelope[index] = envelope[index].min(envelope[index + 1] + maximum_step);
    }

    Some(envelope[count..count * 2].to_vec())
}

/// Returns an obstacle-contour recovery step appropriate for an angular grid.
///
/// A radial limit is sampled once per angular segment. When the segment count
/// grows with radius, a fixed world-unit recovery step would make larger
/// circles look progressively more angular. Tie it to chord length instead,
/// while retaining a small floor for very small valid circles.
#[must_use]
pub(crate) fn obstacle_recovery_step(radius: f32, segments: usize) -> Option<f32> {
    if !radius.is_finite() || radius <= 0.0 || segments < 3 {
        return None;
    }
    let chord = core::f32::consts::TAU * radius / segments as f32;
    if !chord.is_finite() || chord <= 0.0 {
        return None;
    }
    Some((chord * 1.5).clamp(0.05, 0.75))
}

#[must_use]
pub(crate) fn approximately_same_point(left: ScreenPoint, right: ScreenPoint) -> bool {
    const EPSILON: f32 = 0.01;
    (left.x - right.x).abs() <= EPSILON && (left.y - right.y).abs() <= EPSILON
}

/// Splits a closed world-space ring into its currently visible angular runs.
///
/// `visible_segments[index]` controls the segment from point `index` to the
/// following point. The input may contain its usual repeated closing point;
/// returned runs are deliberately open unless every segment is visible. This
/// preserves real occlusion gaps all the way through screen-space projection
/// rather than reconnecting the ring across a wall or terrain ridge.
pub(crate) fn split_visible_world_ring(
    path: &[Vector3],
    visible_segments: &[bool],
) -> Option<Vec<Vec<Vector3>>> {
    let source = if path.first() == path.last() {
        &path[..path.len().saturating_sub(1)]
    } else {
        path
    };
    if source.len() < 3
        || source.len() != visible_segments.len()
        || source.iter().copied().any(|point| !finite_vector(point))
    {
        return None;
    }

    let Some(first_hidden) = visible_segments.iter().position(|visible| !visible) else {
        let mut closed = source.to_vec();
        closed.push(closed[0]);
        return Some(vec![closed]);
    };
    if visible_segments.iter().all(|visible| !visible) {
        return Some(Vec::new());
    }

    // Begin immediately after a hidden segment. That turns the circular scan
    // into ordinary open runs and prevents a visible run from being split at
    // the arbitrary zero-angle point.
    let mut paths = Vec::new();
    let mut current = Vec::new();
    for offset in 1..=source.len() {
        let index = (first_hidden + offset) % source.len();
        if !visible_segments[index] {
            if current.len() >= 2 {
                paths.push(core::mem::take(&mut current));
            } else {
                current.clear();
            }
            continue;
        }

        let from = source[index];
        let to = source[(index + 1) % source.len()];
        if current.last().copied() != Some(from) {
            current.push(from);
        }
        current.push(to);
    }
    if current.len() >= 2 {
        paths.push(current);
    }
    Some(paths)
}

pub(crate) const fn finite_vector(point: Vector3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_density_scales_with_radius_and_is_high_at_radius_twenty() {
        let small = world_segments(10.0, 0).expect("small circle");
        let radius_twenty = world_segments(20.0, 0).expect("radius twenty");
        let radius_twenty_samples = world_sample_segments(20.0).expect("radius twenty sample grid");
        let large = world_segments(30.0, 0).expect("large circle");

        assert!(small < radius_twenty);
        assert!(radius_twenty < large);
        assert_eq!(radius_twenty, 848);
        assert_eq!(radius_twenty_samples, 368);
        assert!(radius_twenty_samples < radius_twenty);
        assert_eq!(radius_twenty % SEGMENT_ALIGNMENT, 0);
    }

    #[test]
    fn tessellation_is_bounded_for_extreme_inputs() {
        assert_eq!(world_segments(10_000.0, 0), Some(MAX_WORLD_SEGMENTS));
        assert_eq!(hud_segments(10_000.0, 0), Some(MAX_HUD_SEGMENTS));
        assert_eq!(world_segments(f32::NAN, 0), None);
        assert_eq!(world_segments(0.0, 0), None);
    }

    #[test]
    fn collision_envelope_smooths_a_notch_without_exceeding_raw_limits() {
        let raw = [10.0, 10.0, 4.0, 10.0, 10.0, 10.0];
        let smooth = smooth_radial_limits(&raw, 1.0).expect("valid limits");

        assert_eq!(smooth, vec![6.0, 5.0, 4.0, 5.0, 6.0, 7.0]);
        assert!(smooth
            .iter()
            .zip(raw)
            .all(|(smoothed, original)| *smoothed <= original));
    }

    #[test]
    fn collision_envelope_fills_one_missed_ray_inside_a_notch() {
        let raw = [10.0, 4.0, 10.0, 4.0, 10.0];
        let smooth = smooth_radial_limits(&raw, 1.0).expect("valid limits");

        // The central 10 is a one-ray hole between two obstacle hits. It
        // should not create a thin outward spike that cuts through a wall.
        assert_eq!(smooth[2], 4.0);
        assert!(smooth
            .iter()
            .zip(raw)
            .all(|(smoothed, original)| *smoothed <= original));
    }

    #[test]
    fn obstacle_recovery_scales_with_the_angular_grid() {
        let coarse = obstacle_recovery_step(20.0, 128).expect("coarse grid");
        let fine = obstacle_recovery_step(20.0, 848).expect("fine grid");
        let larger = obstacle_recovery_step(40.0, 848).expect("large circle");

        assert!(fine < coarse);
        assert!(larger > fine);
        assert!((0.05..=0.75).contains(&fine));
    }

    #[test]
    fn visibility_split_preserves_open_gaps_and_circular_runs() {
        let ring = [
            Vector3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Vector3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            Vector3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ];

        let runs = split_visible_world_ring(&ring, &[false, true, true, false])
            .expect("valid visibility ring");

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0], vec![ring[1], ring[2], ring[3]]);
        assert_ne!(runs[0].first(), runs[0].last());
    }

    #[test]
    fn fully_visible_ring_stays_closed() {
        let ring = [
            Vector3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Vector3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
        ];
        let runs = split_visible_world_ring(&ring, &[true, true, true, true])
            .expect("valid visibility ring");

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 5);
        assert_eq!(runs[0].first(), runs[0].last());
    }
}
