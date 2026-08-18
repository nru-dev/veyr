//! Narrow wrapper around the live client's `CGWorldFrame` intersection path.
//!
//! This is deliberately a native-runtime detail, not a plugin API. WoW has
//! already resolved the active map's terrain, WMOs, M2 collision and custom
//! client assets by the time this method runs. Querying it is therefore more
//! reliable than attempting to reopen `terrain.MPQ` (or a custom patch) from
//! an injected DLL.
//!
//! The call is permitted only from the game render thread. In particular, it
//! must never be made by a loader-created remote thread: client world state is
//! not synchronised for that use.

use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::offsets::{
    advanced_combat::camera,
    api::{Memory, Position, Vector3},
    functions,
};

use super::LocalProcessMemory;

/// The low flag used by the build-12340 `CGWorldFrame::Intersect` terrain
/// branch. It is intentionally kept private until the live probe validates
/// the result semantics on the exact supported executable.
const INTERSECT_TERRAIN: u32 = 0x01;
/// The build-12340 model/WMO branch. The recovered client code tests this
/// exact mask before it invokes its closest-model traversal.
const INTERSECT_MODELS: u32 = 0x7C;
/// Static world visibility is terrain plus model/WMO geometry.
const INTERSECT_WORLD: u32 = INTERSECT_TERRAIN | INTERSECT_MODELS;

/// Terrain-probe diagnostic states exposed only through the private DEV
/// loader ABI. Values above `0xE300` are errors, not collision return codes.
pub(crate) const PROBE_IDLE: u32 = 0;
pub(crate) const PROBE_ARMED: u32 = 1;
pub(crate) const PROBE_MISS: u32 = 2;
pub(crate) const PROBE_HIT: u32 = 3;
pub(crate) const PROBE_WORLD_UNAVAILABLE: u32 = 0xE301;
pub(crate) const PROBE_INVALID_RAY: u32 = 0xE302;
pub(crate) const PROBE_UNEXPECTED_RESULT: u32 = 0xE303;
pub(crate) const PROBE_INVALID_HIT: u32 = 0xE304;

static PROBE_REQUESTED: AtomicU32 = AtomicU32::new(0);
static PROBE_STATUS: AtomicU32 = AtomicU32::new(PROBE_IDLE);
static PROBE_NATIVE_RESULT: AtomicU32 = AtomicU32::new(0);
static PROBE_HIT_X: AtomicU32 = AtomicU32::new(0);
static PROBE_HIT_Y: AtomicU32 = AtomicU32::new(0);
static PROBE_HIT_Z: AtomicU32 = AtomicU32::new(0);

/// A hit record written by build-12340's `CGWorldFrame::Intersect`.
///
/// Static analysis shows `position` at `+0x08` and `distance` at `+0x14` for
/// every successful terrain result (`1` or `2`). The two leading words are deliberately opaque;
/// a terrain caller needs neither object identity nor client internals.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct RawIntersectHit {
    _unknown_00: u32,
    _unknown_04: u32,
    position: Vector3,
    distance: f32,
}

impl Default for RawIntersectHit {
    fn default() -> Self {
        Self {
            _unknown_00: 0,
            _unknown_04: 0,
            position: Vector3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            distance: 0.0,
        }
    }
}

/// A validated world-space terrain, model, or WMO contact.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct WorldHit {
    pub(crate) position: Vector3,
    pub(crate) distance: f32,
}

/// One completed client intersection, including its raw return code.
#[derive(Debug, Copy, Clone, PartialEq)]
struct Intersection {
    native_result: u32,
    hit: Option<WorldHit>,
}

/// Why a terrain query was not usable this frame.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum GroundQueryError {
    /// Character select, loading, or a world transition left no live frame.
    WorldFrameUnavailable,
    /// The caller supplied a non-finite or degenerate ray.
    InvalidRay,
    /// The client returned a result code not documented by the recovered ABI.
    UnexpectedResult(u32),
    /// The client returned a terrain result without a plausible finite hit.
    InvalidHit,
}

/// Requests exactly one terrain probe once an in-world render command reaches
/// the runtime. Calling this is harmless at login: it remains armed until a
/// finite player-centred world command exists.
pub(crate) fn request_probe() {
    PROBE_NATIVE_RESULT.store(0, Ordering::Release);
    PROBE_HIT_X.store(0, Ordering::Release);
    PROBE_HIT_Y.store(0, Ordering::Release);
    PROBE_HIT_Z.store(0, Ordering::Release);
    PROBE_STATUS.store(PROBE_ARMED, Ordering::Release);
    PROBE_REQUESTED.store(1, Ordering::Release);
}

#[must_use]
pub(crate) fn probe_status() -> u32 {
    PROBE_STATUS.load(Ordering::Acquire)
}

#[must_use]
pub(crate) fn probe_native_result() -> u32 {
    PROBE_NATIVE_RESULT.load(Ordering::Acquire)
}

#[must_use]
pub(crate) fn probe_hit_component_bits(index: usize) -> u32 {
    match index {
        0 => PROBE_HIT_X.load(Ordering::Acquire),
        1 => PROBE_HIT_Y.load(Ordering::Acquire),
        2 => PROBE_HIT_Z.load(Ordering::Acquire),
        _ => 0,
    }
}

/// Performs the requested probe exactly once, from the EndScene/swap render
/// thread. No query occurs unless [`request_probe`] armed it first.
pub(crate) fn run_pending_probe(center: Position) {
    if PROBE_REQUESTED.swap(0, Ordering::AcqRel) == 0 {
        return;
    }

    let start = Vector3 {
        x: center.x,
        y: center.y,
        z: center.z + 500.0,
    };
    let end = Vector3 {
        x: center.x,
        y: center.y,
        z: center.z - 500.0,
    };
    let result = unsafe { intersect_with_flags(start, end, INTERSECT_TERRAIN) };
    match result {
        Ok(Intersection {
            native_result,
            hit: Some(hit),
        }) => {
            PROBE_HIT_X.store(hit.position.x.to_bits(), Ordering::Release);
            PROBE_HIT_Y.store(hit.position.y.to_bits(), Ordering::Release);
            PROBE_HIT_Z.store(hit.position.z.to_bits(), Ordering::Release);
            PROBE_NATIVE_RESULT.store(native_result, Ordering::Release);
            PROBE_STATUS.store(PROBE_HIT, Ordering::Release);
        }
        Ok(Intersection {
            native_result,
            hit: None,
        }) => {
            PROBE_NATIVE_RESULT.store(native_result, Ordering::Release);
            PROBE_STATUS.store(PROBE_MISS, Ordering::Release);
        }
        Err(error) => {
            let status = match error {
                GroundQueryError::WorldFrameUnavailable => PROBE_WORLD_UNAVAILABLE,
                GroundQueryError::InvalidRay => PROBE_INVALID_RAY,
                GroundQueryError::UnexpectedResult(result) => {
                    PROBE_NATIVE_RESULT.store(result, Ordering::Release);
                    PROBE_UNEXPECTED_RESULT
                }
                GroundQueryError::InvalidHit => PROBE_INVALID_HIT,
            };
            PROBE_STATUS.store(status, Ordering::Release);
        }
    }
}

/// Calls the client's terrain branch with a finite vertical or arbitrary ray.
///
/// `Ok(None)` means no terrain contact. The caller should treat every error
/// as a transient condition and draw nothing for that sample, rather than
/// retrying or dereferencing any client pointer itself.
///
/// # Safety
///
/// Must execute on WoW's render thread while the supported build is running.
/// The supplied vectors must remain valid for this direct x86 client call;
/// they are ordinary stack values in the injected module.
pub(crate) unsafe fn intersect_terrain(
    start: Vector3,
    end: Vector3,
) -> Result<Option<WorldHit>, GroundQueryError> {
    unsafe { intersect_with_flags(start, end, INTERSECT_TERRAIN) }
        .map(|intersection| intersection.hit)
}

/// Traces only static model/WMO geometry. It deliberately excludes terrain so
/// a radial circle probe can make a notch at a wall without treating an
/// ordinary uphill slope as an obstacle.
pub(crate) fn obstacle_between(
    start: Vector3,
    end: Vector3,
) -> Result<Option<WorldHit>, GroundQueryError> {
    // Safety: called only by the D3D render callback; the public safe wrapper
    // never exposes this client call outside the native runtime.
    unsafe { intersect_with_flags(start, end, INTERSECT_MODELS) }
        .map(|intersection| intersection.hit)
}

/// Returns whether terrain or static model geometry blocks a camera-to-world
/// segment. An unsupported transient client result leaves the segment visible
/// rather than creating an incorrect hole in the overlay.
pub(crate) fn world_obstructs(start: Vector3, end: Vector3) -> bool {
    // Safety: see [`obstacle_between`].
    unsafe { intersect_with_flags(start, end, INTERSECT_WORLD) }
        .ok()
        .is_some_and(|intersection| intersection.hit.is_some())
}

/// Calls the recovered client method with one validated collision mask.
///
/// # Safety
///
/// Must execute on WoW's render thread while the supported build is running.
unsafe fn intersect_with_flags(
    start: Vector3,
    end: Vector3,
    flags: u32,
) -> Result<Intersection, GroundQueryError> {
    if !finite(start) || !finite(end) || same_point(start, end) {
        return Err(GroundQueryError::InvalidRay);
    }

    let world_frame = LocalProcessMemory
        .read_u32(camera::CURRENT_WORLD_FRAME)
        .map_err(|_| GroundQueryError::WorldFrameUnavailable)?;
    if world_frame == 0 {
        return Err(GroundQueryError::WorldFrameUnavailable);
    }

    let mut raw_hit = RawIntersectHit::default();
    // `CGWorldFrame::Intersect(CGWorldFrame* this, Vector3 const& start,
    // Vector3 const& end, u32 flags, Hit* out) -> u32` was recovered from the
    // function's `ret 0x10`, parameter loads and its direct in-client caller.
    // On x86 Rust's `thiscall` passes `this` in ECX and pops the four explicit
    // stack parameters exactly as the client expects.
    type IntersectFn = unsafe extern "thiscall" fn(
        *mut c_void,
        *const Vector3,
        *const Vector3,
        u32,
        *mut RawIntersectHit,
    ) -> u32;
    let intersect: IntersectFn = unsafe {
        core::mem::transmute::<usize, IntersectFn>(
            functions::world::CG_WORLD_FRAME_INTERSECT as usize,
        )
    };
    let result = unsafe {
        intersect(
            world_frame as usize as *mut c_void,
            &start,
            &end,
            flags,
            &mut raw_hit,
        )
    };

    match result {
        0 => Ok(Intersection {
            native_result: result,
            hit: None,
        }),
        // The recovered epilogue emits `1` for a terrain contact, `2` for a
        // static model/WMO contact, and `3` for a secondary static-world
        // branch. All three initialise position (+0x08) and distance (+0x14).
        1..=3 => valid_hit(raw_hit).map(|hit| Intersection {
            native_result: result,
            hit: Some(hit),
        }),
        other => Err(GroundQueryError::UnexpectedResult(other)),
    }
}

/// Returns the loaded client's terrain height at one XY point.
///
/// This is intentionally available only to the native render path. It uses
/// the same live world that the client has already assembled from base or
/// custom map assets; no MPQ path, map name, or tile file is involved.
pub(crate) fn terrain_at(x: f32, y: f32, reference_z: f32) -> Option<Vector3> {
    let start = Vector3 {
        x,
        y,
        z: reference_z + 500.0,
    };
    let end = Vector3 {
        x,
        y,
        z: reference_z - 500.0,
    };
    unsafe { intersect_terrain(start, end) }
        .ok()
        .flatten()
        .map(|hit| hit.position)
}

fn valid_hit(raw: RawIntersectHit) -> Result<WorldHit, GroundQueryError> {
    (finite(raw.position) && raw.distance.is_finite() && raw.distance >= 0.0)
        .then_some(WorldHit {
            position: raw.position,
            distance: raw.distance,
        })
        .ok_or(GroundQueryError::InvalidHit)
}

fn finite(value: Vector3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn same_point(left: Vector3, right: Vector3) -> bool {
    left.x == right.x && left.y == right.y && left.z == right.z
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_rays_are_rejected_before_touching_client_memory() {
        let point = Vector3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        assert!(same_point(point, point));
        assert!(finite(point));
        assert!(!finite(Vector3 {
            x: f32::NAN,
            y: 0.0,
            z: 0.0,
        }));
    }

    #[test]
    fn raw_terrain_hit_requires_finite_values() {
        let hit = RawIntersectHit {
            _unknown_00: 0,
            _unknown_04: 0,
            position: Vector3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            distance: 4.0,
        };
        assert_eq!(
            valid_hit(hit),
            Ok(WorldHit {
                position: hit.position,
                distance: 4.0,
            })
        );
    }
}
