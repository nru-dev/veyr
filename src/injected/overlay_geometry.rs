//! Backend-neutral geometry for simple screen-space overlay primitives.

use crate::offsets::api::{Color, ScreenPoint, Stroke};

use super::circle_geometry::{approximately_same_point, POLYLINE_MITER_LIMIT};

// Overlay coordinates are ultimately handed to fixed-function D3D9 as
// transformed pixels. Keep malformed plugin input well inside a finite,
// driver-safe range instead of allowing a huge but finite float to explode
// during tessellation.
const MAX_OVERLAY_COORDINATE: f32 = 1_000_000.0;
const MAX_OVERLAY_STROKE_WIDTH: f32 = 100_000.0;
const MAX_OVERLAY_RADIUS: f32 = 1_000_000.0;

/// Backend-neutral point in a tessellated screen-space primitive.
///
/// Native renderers own any API-specific vertex layout, colour packing, and
/// pixel-centre adjustment.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct OverlayVertex {
    pub x: f32,
    pub y: f32,
    pub color: Color,
}

impl OverlayVertex {
    const fn at(point: ScreenPoint, color: Color) -> Self {
        Self {
            x: point.x,
            y: point.y,
            color,
        }
    }
}

/// Builds a two-triangle strip for one screen-space line.
pub(crate) fn line_strip(
    from: ScreenPoint,
    to: ScreenPoint,
    stroke: Stroke,
) -> Option<[OverlayVertex; 4]> {
    if !finite_point(from) || !finite_point(to) || !valid_stroke(stroke) {
        return None;
    }
    if !offsettable_point(from, stroke.width) || !offsettable_point(to, stroke.width) {
        return None;
    }

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length_squared = dx.mul_add(dx, dy * dy);
    if length_squared <= f32::EPSILON {
        return None;
    }

    let half_width = stroke.width * 0.5;
    if !half_width.is_finite() {
        return None;
    }
    let inverse_length = length_squared.sqrt().recip();
    let offset_x = -dy * inverse_length * half_width;
    let offset_y = dx * inverse_length * half_width;
    let color = stroke.color;

    Some([
        OverlayVertex::at(
            ScreenPoint::new(from.x + offset_x, from.y + offset_y),
            color,
        ),
        OverlayVertex::at(
            ScreenPoint::new(from.x - offset_x, from.y - offset_y),
            color,
        ),
        OverlayVertex::at(ScreenPoint::new(to.x + offset_x, to.y + offset_y), color),
        OverlayVertex::at(ScreenPoint::new(to.x - offset_x, to.y - offset_y), color),
    ])
}

/// Builds a joined triangle list for one screen-space polyline.
///
/// Unlike independently tessellating every segment, this shares one pair of
/// vertices at each path point. Miter joins remove visible pinholes and keep
/// thick/glowing world circles continuous while turning sharply around an
/// obstacle. Extreme miters fall back to a bevel-like bounded join.
pub(crate) fn polyline_triangles(
    points: &[ScreenPoint],
    stroke: Stroke,
) -> Option<Vec<OverlayVertex>> {
    if points.len() < 2
        || points.iter().copied().any(|point| !finite_point(point))
        || !valid_stroke(stroke)
    {
        return None;
    }
    let half_width = stroke.width * 0.5;

    let closed = points.len() >= 3
        && approximately_same_point(points[0], *points.last().expect("non-empty path"));
    // Projection and viewport clipping can emit the same point from two
    // neighbouring source segments (especially at a viewport corner). A
    // zero-length segment has no normal, but it must not make us discard the
    // rest of an otherwise valid circle or polyline. Canonicalise those
    // adjacent points before calculating joins.
    let source = if closed {
        &points[..points.len() - 1]
    } else {
        points
    };
    let mut unique = Vec::with_capacity(source.len());
    for point in source.iter().copied() {
        if unique
            .last()
            .is_none_or(|last| !approximately_same_point(*last, point))
        {
            unique.push(point);
        }
    }
    // A duplicate closing point can survive above when the input contains
    // more than one copy of it before its conventional final copy.
    while unique.len() > 1
        && approximately_same_point(
            unique[0],
            *unique.last().expect("more than one unique point"),
        )
    {
        unique.pop();
    }
    if (closed && unique.len() < 3) || (!closed && unique.len() < 2) {
        return None;
    }

    let mut pairs = Vec::with_capacity(unique.len());
    for index in 0..unique.len() {
        let point = unique[index];
        let previous = if index > 0 {
            Some(unique[index - 1])
        } else if closed {
            unique.last().copied()
        } else {
            None
        };
        let next = if index + 1 < unique.len() {
            Some(unique[index + 1])
        } else if closed {
            unique.first().copied()
        } else {
            None
        };
        let offset = joined_offset(previous, point, next, half_width)?;
        pairs.push((
            OverlayVertex::at(
                ScreenPoint::new(point.x + offset.0, point.y + offset.1),
                stroke.color,
            ),
            OverlayVertex::at(
                ScreenPoint::new(point.x - offset.0, point.y - offset.1),
                stroke.color,
            ),
        ));
    }

    let segment_count = if closed {
        pairs.len()
    } else {
        pairs.len().saturating_sub(1)
    };
    let mut vertices = Vec::with_capacity(segment_count * 6);
    for index in 0..segment_count {
        let next = (index + 1) % pairs.len();
        let (left_a, right_a) = pairs[index];
        let (left_b, right_b) = pairs[next];
        vertices.extend_from_slice(&[left_a, right_a, left_b, left_b, right_a, right_b]);
    }
    (!vertices.is_empty()).then_some(vertices)
}

fn joined_offset(
    previous: Option<ScreenPoint>,
    point: ScreenPoint,
    next: Option<ScreenPoint>,
    half_width: f32,
) -> Option<(f32, f32)> {
    let incoming = previous.and_then(|value| unit_direction(value, point));
    let outgoing = next.and_then(|value| unit_direction(point, value));
    let direction = match (incoming, outgoing) {
        (Some(incoming), Some(outgoing)) => {
            let x = incoming.0 + outgoing.0;
            let y = incoming.1 + outgoing.1;
            let length_squared = x.mul_add(x, y * y);
            if length_squared <= 1.0e-6 {
                outgoing
            } else {
                let inverse = length_squared.sqrt().recip();
                (x * inverse, y * inverse)
            }
        }
        (Some(direction), None) | (None, Some(direction)) => direction,
        (None, None) => return None,
    };
    let normal = (-direction.1, direction.0);

    if let (Some(incoming), Some(_)) = (incoming, outgoing) {
        let incoming_normal = (-incoming.1, incoming.0);
        let denominator = normal
            .0
            .mul_add(incoming_normal.0, normal.1 * incoming_normal.1);
        if denominator.abs() > 0.2 {
            let miter = (half_width / denominator.abs()).min(half_width * POLYLINE_MITER_LIMIT);
            return Some((normal.0 * miter, normal.1 * miter));
        }
    }
    Some((normal.0 * half_width, normal.1 * half_width))
}

fn unit_direction(from: ScreenPoint, to: ScreenPoint) -> Option<(f32, f32)> {
    let x = to.x - from.x;
    let y = to.y - from.y;
    let length_squared = x.mul_add(x, y * y);
    if !length_squared.is_finite() || length_squared <= 1.0e-6 {
        return None;
    }
    let inverse = length_squared.sqrt().recip();
    Some((x * inverse, y * inverse))
}

/// Builds a closed triangle strip for one screen-space circle outline.
pub(crate) fn circle_strip(
    center: ScreenPoint,
    radius: f32,
    stroke: Stroke,
    segments: usize,
) -> Option<Vec<OverlayVertex>> {
    if !finite_point(center)
        || !radius.is_finite()
        || !(0.0..=MAX_OVERLAY_RADIUS).contains(&radius)
        || !valid_stroke(stroke)
        || segments < 3
    {
        return None;
    }

    let half_width = stroke.width * 0.5;
    let outer_radius = radius + half_width;
    let inner_radius = (radius - half_width).max(0.0);
    if !outer_radius.is_finite()
        || !inner_radius.is_finite()
        || !offsettable_point(center, outer_radius * 2.0)
    {
        return None;
    }
    let mut vertices = Vec::with_capacity((segments + 1) * 2);

    for segment in 0..=segments {
        let angle = core::f32::consts::TAU * segment as f32 / segments as f32;
        let (sin, cos) = angle.sin_cos();
        vertices.push(OverlayVertex::at(
            ScreenPoint::new(center.x + cos * outer_radius, center.y + sin * outer_radius),
            stroke.color,
        ));
        vertices.push(OverlayVertex::at(
            ScreenPoint::new(center.x + cos * inner_radius, center.y + sin * inner_radius),
            stroke.color,
        ));
    }

    Some(vertices)
}

fn finite_point(point: ScreenPoint) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x.abs() <= MAX_OVERLAY_COORDINATE
        && point.y.abs() <= MAX_OVERLAY_COORDINATE
}

fn valid_stroke(stroke: Stroke) -> bool {
    stroke.width.is_finite() && (0.0..=MAX_OVERLAY_STROKE_WIDTH).contains(&stroke.width)
}

fn offsettable_point(point: ScreenPoint, width: f32) -> bool {
    let half_width = width * 0.5;
    (point.x.abs() + half_width).is_finite() && (point.y.abs() + half_width).is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_is_a_two_triangle_strip_with_the_requested_width() {
        let vertices = line_strip(
            ScreenPoint::new(10.0, 20.0),
            ScreenPoint::new(30.0, 20.0),
            Stroke::new(Color::RED, 4.0),
        )
        .expect("valid line");

        assert_eq!(vertices.len(), 4);
        assert_eq!(vertices[0].x, 10.0);
        assert_eq!(vertices[0].y, 22.0);
        assert_eq!(vertices[1].y, 18.0);
        assert_eq!(vertices[0].color, Color::RED);
    }

    #[test]
    fn circle_closes_its_triangle_strip() {
        let vertices = circle_strip(
            ScreenPoint::new(100.0, 100.0),
            20.0,
            Stroke::new(Color::CYAN, 2.0),
            12,
        )
        .expect("valid circle");

        assert_eq!(vertices.len(), 26);
        assert_eq!(vertices.first(), vertices.get(24));
        assert_eq!(vertices.get(1), vertices.last());
    }

    #[test]
    fn joined_polyline_has_shared_bounded_corners() {
        let vertices = polyline_triangles(
            &[
                ScreenPoint::new(0.0, 0.0),
                ScreenPoint::new(10.0, 0.0),
                ScreenPoint::new(10.0, 10.0),
            ],
            Stroke::new(Color::CYAN, 4.0),
        )
        .expect("valid joined path");

        assert_eq!(vertices.len(), 12);
        assert!(vertices
            .iter()
            .all(|vertex| vertex.x.is_finite() && vertex.y.is_finite()));
        assert!(vertices
            .iter()
            .all(|vertex| vertex.x.abs() <= 15.0 && vertex.y.abs() <= 15.0));
    }

    #[test]
    fn joined_polyline_discards_duplicate_clip_points_without_dropping_the_path() {
        // Viewport clipping can produce this exact sequence when adjacent
        // world segments meet on one screen edge. The duplicate must not turn
        // the entire visible run into an invalid zero-length polyline.
        let vertices = polyline_triangles(
            &[
                ScreenPoint::new(0.0, 0.0),
                ScreenPoint::new(0.0, 0.0),
                ScreenPoint::new(10.0, 0.0),
                ScreenPoint::new(10.0, 10.0),
            ],
            Stroke::new(Color::CYAN, 4.0),
        )
        .expect("duplicate point is harmless");

        assert_eq!(vertices.len(), 12);
        assert!(vertices
            .iter()
            .all(|vertex| vertex.x.is_finite() && vertex.y.is_finite()));
    }

    #[test]
    fn closed_polyline_discards_duplicate_points_and_remains_closed() {
        let vertices = polyline_triangles(
            &[
                ScreenPoint::new(0.0, 0.0),
                ScreenPoint::new(10.0, 0.0),
                ScreenPoint::new(10.0, 0.0),
                ScreenPoint::new(10.0, 10.0),
                ScreenPoint::new(0.0, 10.0),
                ScreenPoint::new(0.0, 0.0),
            ],
            Stroke::new(Color::CYAN, 2.0),
        )
        .expect("duplicate point is harmless");

        // Four unique corners, each producing two triangles in the closed
        // ring. An open fallback would have only three segments.
        assert_eq!(vertices.len(), 24);
    }

    #[test]
    fn joined_polyline_rejects_a_width_that_overflows_its_half_width() {
        assert!(polyline_triangles(
            &[ScreenPoint::new(0.0, 0.0), ScreenPoint::new(1.0, 0.0)],
            Stroke::new(Color::CYAN, f32::MAX),
        )
        .is_none());
    }
}
