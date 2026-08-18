use core::ffi::c_void;
use core::mem::{size_of, MaybeUninit};

use crate::offsets::{
    api::{Memory, WritableMemory},
    RemoteAddress,
};

/// Same-process memory reader/writer for the injected x86 DLL.
///
/// `ReadProcessMemory`/`WriteProcessMemory` are used even in-process so an
/// invalid client address becomes a normal API error instead of an unchecked
/// Rust pointer dereference. Calls made from a graphics callback can therefore
/// request `LocalProcessMemory` without treating a callback-owned COM pointer
/// as a Rust reference.
#[derive(Debug, Copy, Clone, Default)]
pub struct LocalProcessMemory;

/// Win32 failure produced by [`LocalProcessMemory`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LocalProcessMemoryError {
    Read {
        address: RemoteAddress,
        win32_error: u32,
    },
    Write {
        address: RemoteAddress,
        win32_error: u32,
    },
}

impl LocalProcessMemory {
    fn read<T: Copy>(address: RemoteAddress) -> Result<T, LocalProcessMemoryError> {
        let mut value = MaybeUninit::<T>::uninit();
        let mut bytes_read = 0_usize;
        let expected = size_of::<T>();
        let succeeded = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                as_local_pointer(address),
                value.as_mut_ptr().cast(),
                expected,
                &mut bytes_read,
            )
        };

        if succeeded == 0 || bytes_read != expected {
            return Err(LocalProcessMemoryError::Read {
                address,
                win32_error: unsafe { GetLastError() },
            });
        }

        Ok(unsafe { value.assume_init() })
    }

    fn write<T: Copy>(address: RemoteAddress, value: T) -> Result<(), LocalProcessMemoryError> {
        let mut bytes_written = 0_usize;
        let expected = size_of::<T>();
        let succeeded = unsafe {
            WriteProcessMemory(
                GetCurrentProcess(),
                as_local_mut_pointer(address),
                (&value as *const T).cast(),
                expected,
                &mut bytes_written,
            )
        };

        if succeeded == 0 || bytes_written != expected {
            return Err(LocalProcessMemoryError::Write {
                address,
                win32_error: unsafe { GetLastError() },
            });
        }

        Ok(())
    }
}

impl Memory for LocalProcessMemory {
    type Error = LocalProcessMemoryError;

    fn read_u8(&self, address: RemoteAddress) -> Result<u8, Self::Error> {
        Self::read(address)
    }

    fn read_u32(&self, address: RemoteAddress) -> Result<u32, Self::Error> {
        Self::read(address)
    }

    fn read_u64(&self, address: RemoteAddress) -> Result<u64, Self::Error> {
        Self::read(address)
    }

    fn read_f32(&self, address: RemoteAddress) -> Result<f32, Self::Error> {
        Self::read(address)
    }
}

impl WritableMemory for LocalProcessMemory {
    fn write_u8(&self, address: RemoteAddress, value: u8) -> Result<(), Self::Error> {
        Self::write(address, value)
    }

    fn write_u32(&self, address: RemoteAddress, value: u32) -> Result<(), Self::Error> {
        Self::write(address, value)
    }

    fn write_u64(&self, address: RemoteAddress, value: u64) -> Result<(), Self::Error> {
        Self::write(address, value)
    }

    fn write_f32(&self, address: RemoteAddress, value: f32) -> Result<(), Self::Error> {
        Self::write(address, value)
    }
}

/// Converts a validated x86 remote address only at the Win32 FFI boundary.
///
/// No remote pointer is stored as `usize`; Win32 needs a local raw pointer to
/// call `ReadProcessMemory` in this same x86 process.
fn as_local_pointer(address: RemoteAddress) -> *const c_void {
    address as usize as *const c_void
}

fn as_local_mut_pointer(address: RemoteAddress) -> *mut c_void {
    address as usize as *mut c_void
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetLastError() -> u32;
    fn ReadProcessMemory(
        process: *mut c_void,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> i32;
    fn WriteProcessMemory(
        process: *mut c_void,
        base_address: *mut c_void,
        buffer: *const c_void,
        size: usize,
        bytes_written: *mut usize,
    ) -> i32;
}
