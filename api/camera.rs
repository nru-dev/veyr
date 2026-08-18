use super::super::advanced_combat::camera;
use super::memory::with_offset;
use super::{ApiError, ApiResult, GameApi, Memory};

/// Semantic camera access for rendering and world-to-screen work.
pub struct Camera<'api, M> {
    api: &'api GameApi<M>,
}

impl<'api, M> Camera<'api, M> {
    pub(crate) const fn new(api: &'api GameApi<M>) -> Self {
        Self { api }
    }
}

impl<'api, M: Memory> Camera<'api, M> {
    /// Reads the verified camera state for the supported client executable.
    ///
    /// This only snapshots client memory. A missing world frame or camera is
    /// reported as a normal API error, which makes login, character selection,
    /// and loading screens safe for plugins to ignore.
    pub fn state(&self) -> ApiResult<CameraState, M::Error> {
        let world_frame = self
            .api
            .memory()
            .read_u32(camera::CURRENT_WORLD_FRAME)
            .map_err(ApiError::Memory)?;
        if world_frame == 0 {
            return Err(ApiError::NullPointer {
                context: "current world frame",
            });
        }

        let camera_slot =
            with_offset(world_frame, camera::CAMERA_OFFSET).ok_or(ApiError::AddressOverflow {
                base: world_frame,
                offset: camera::CAMERA_OFFSET,
            })?;
        let address = self
            .api
            .memory()
            .read_u32(camera_slot)
            .map_err(ApiError::Memory)?;
        if address == 0 {
            return Err(ApiError::NullPointer {
                context: "world-frame camera",
            });
        }

        let eye = self.read_vector3(address, camera::EYE_POSITION_OFFSET)?;
        Ok(CameraState {
            eye,
            forward: self.read_vector3(address, camera::FORWARD_BASIS_OFFSET)?,
            left: self.read_vector3(address, camera::LEFT_BASIS_OFFSET)?,
            roll: self.read_f32(address, camera::ROLL_OFFSET)?,
            yaw: self.read_f32(address, camera::YAW_OFFSET)?,
            pitch: self.read_f32(address, camera::PITCH_OFFSET)?,
            field_of_view: self.read_f32(address, camera::FOV_OFFSET)?,
        })
    }

    fn read_f32(&self, base: u32, offset: u32) -> ApiResult<f32, M::Error> {
        let address =
            with_offset(base, offset).ok_or(ApiError::AddressOverflow { base, offset })?;
        self.api
            .memory()
            .read_f32(address)
            .map_err(ApiError::Memory)
    }

    fn read_vector3(&self, base: u32, offset: u32) -> ApiResult<Vector3, M::Error> {
        Ok(Vector3 {
            x: self.read_f32(base, offset)?,
            y: self.read_f32(base, offset + 4)?,
            z: self.read_f32(base, offset + 8)?,
        })
    }
}

/// Three-dimensional point or direction used by camera data.
///
/// Its C layout is intentional: the trusted Windows runtime also passes this
/// value to verified client methods on the x86 render thread. It remains a
/// plain value type in the developer API; no game pointer is hidden in it.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Snapshot of the client camera in world coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CameraState {
    pub eye: Vector3,
    /// Unit vector along the visible camera direction.
    pub forward: Vector3,
    /// Unit vector pointing toward the camera's left side.
    pub left: Vector3,
    /// Camera roll in radians.
    pub roll: f32,
    /// Camera yaw in radians.
    pub yaw: f32,
    /// Camera pitch in radians.
    pub pitch: f32,
    pub field_of_view: f32,
}
