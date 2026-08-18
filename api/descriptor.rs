use super::super::{update_fields, FieldIndex, RemoteAddress};
use super::{ApiError, ApiResult, Memory};

/// Handle to an object's update-field descriptor array.
pub struct Descriptor<'memory, M> {
    memory: &'memory M,
    address: RemoteAddress,
}

impl<'memory, M> Descriptor<'memory, M> {
    pub(crate) const fn new(memory: &'memory M, address: RemoteAddress) -> Self {
        Self { memory, address }
    }

    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.address
    }
}

impl<'memory, M: Memory> Descriptor<'memory, M> {
    /// Reads one `u32` update field.
    pub fn read_u32(&self, field: impl update_fields::Field) -> ApiResult<u32, M::Error> {
        self.memory
            .read_u32(self.field_address(field)?)
            .map_err(ApiError::Memory)
    }

    /// Reads one `u64` update field (for example a GUID).
    pub fn read_u64(&self, field: impl update_fields::Field) -> ApiResult<u64, M::Error> {
        self.memory
            .read_u64(self.field_address(field)?)
            .map_err(ApiError::Memory)
    }

    /// Reads one `f32` update field.
    pub fn read_f32(&self, field: impl update_fields::Field) -> ApiResult<f32, M::Error> {
        self.memory
            .read_f32(self.field_address(field)?)
            .map_err(ApiError::Memory)
    }

    fn field_address(
        &self,
        field: impl update_fields::Field,
    ) -> ApiResult<RemoteAddress, M::Error> {
        let index: FieldIndex = field.index();
        update_fields::checked_address_of(self.address, field).ok_or(ApiError::AddressOverflow {
            base: self.address,
            offset: update_fields::byte_offset(index),
        })
    }
}
