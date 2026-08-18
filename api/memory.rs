use core::marker::PhantomData;

use super::super::RemoteAddress;

/// Transport abstraction for reading the x86 client process.
///
/// Backends may use `ReadProcessMemory`, an injected reader, or a deterministic
/// test double. The API intentionally exposes only primitive reads: remote
/// layouts are decoded by the domain modules that own those layouts.
pub trait Memory {
    type Error;

    fn read_u8(&self, address: RemoteAddress) -> Result<u8, Self::Error>;
    fn read_u32(&self, address: RemoteAddress) -> Result<u32, Self::Error>;
    fn read_u64(&self, address: RemoteAddress) -> Result<u64, Self::Error>;
    fn read_f32(&self, address: RemoteAddress) -> Result<f32, Self::Error>;
}

/// Borrowed readers retain the same transport capability as their owner.
///
/// This lets the Engine own a reader while creating a short-lived API context
/// for each plugin callback, instead of making the API borrow Engine from
/// inside itself.
impl<M: Memory + ?Sized> Memory for &M {
    type Error = M::Error;

    fn read_u8(&self, address: RemoteAddress) -> Result<u8, Self::Error> {
        (**self).read_u8(address)
    }

    fn read_u32(&self, address: RemoteAddress) -> Result<u32, Self::Error> {
        (**self).read_u32(address)
    }

    fn read_u64(&self, address: RemoteAddress) -> Result<u64, Self::Error> {
        (**self).read_u64(address)
    }

    fn read_f32(&self, address: RemoteAddress) -> Result<f32, Self::Error> {
        (**self).read_f32(address)
    }
}

/// Optional capability for a backend that can modify remote process memory.
pub trait WritableMemory: Memory {
    fn write_u8(&self, address: RemoteAddress, value: u8) -> Result<(), Self::Error>;
    fn write_u32(&self, address: RemoteAddress, value: u32) -> Result<(), Self::Error>;
    fn write_u64(&self, address: RemoteAddress, value: u64) -> Result<(), Self::Error>;
    fn write_f32(&self, address: RemoteAddress, value: f32) -> Result<(), Self::Error>;
}

/// A typed pointer into the 32-bit game process.
///
/// The marker type is compile-time-only; it never changes the x86 pointer size.
#[repr(transparent)]
pub struct RemotePtr<T> {
    address: RemoteAddress,
    marker: PhantomData<fn() -> T>,
}

impl<T> Copy for RemotePtr<T> {}

impl<T> Clone for RemotePtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> RemotePtr<T> {
    pub const NULL: Self = Self::new(0);

    #[must_use]
    pub const fn new(address: RemoteAddress) -> Self {
        Self {
            address,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn address(self) -> RemoteAddress {
        self.address
    }

    #[must_use]
    pub const fn is_null(self) -> bool {
        self.address == 0
    }

    #[must_use]
    pub const fn cast<U>(self) -> RemotePtr<U> {
        RemotePtr::new(self.address)
    }
}

/// Adds an in-structure byte offset without leaving the x86 address space.
#[must_use]
pub(crate) const fn with_offset(base: RemoteAddress, offset: u32) -> Option<RemoteAddress> {
    base.checked_add(offset)
}
