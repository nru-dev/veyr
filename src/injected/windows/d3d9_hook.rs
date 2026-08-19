//! Per-device D3D9 vtable hooks for the injected x86 runtime.
//!
//! The old implementation patched the shared driver method entry and copied
//! its opening instructions into a trampoline. That is unsafe for vendor
//! drivers: even a decoder which accepts the bytes cannot preserve every
//! entry-point assumption. A captured `IDirect3DDevice9` already supplies the
//! exact vtable WoW calls, so replace only the two pointers in that one table
//! and call the saved native pointers directly.

use core::ffi::c_void;
use core::mem::size_of;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::offsets::RemoteAddress;

use super::memory::LocalProcessMemoryError;

/// ABI of `IDirect3DDevice9::EndScene` on the supported x86 target.
pub type EndSceneFn = unsafe extern "system" fn(device: *mut c_void) -> i32;

/// ABI of `IDirect3DDevice9::Reset` on the supported x86 target.
pub type ResetFn = unsafe extern "system" fn(device: *mut c_void, parameters: *mut c_void) -> i32;

const IDIRECT3DDEVICE9_RESET_INDEX: usize = 16;
const IDIRECT3DDEVICE9_END_SCENE_INDEX: usize = 42;
const POINTER_SIZE: usize = size_of::<RemoteAddress>();
const PAGE_EXECUTE_READWRITE: u32 = 0x40;

/// One restorable slot replacement in the captured device's vtable.
///
/// This does not patch nor decode any driver instruction. The original method
/// pointer remains callable directly with the COM ABI after the callback.
struct VtableSlotHook {
    slot: RemoteAddress,
    original: RemoteAddress,
    active: bool,
}

impl VtableSlotHook {
    /// # Safety
    ///
    /// `device` must be the live captured D3D9 device and the chosen slot must
    /// be one of its documented `IDirect3DDevice9` method entries.
    unsafe fn install(
        device: *mut c_void,
        index: usize,
        replacement: RemoteAddress,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        if device.is_null() {
            return Err(EndSceneHookError::NullDevice);
        }
        if replacement == 0 {
            return Err(EndSceneHookError::NullReplacement);
        }
        let vtable = unsafe { (device.cast::<*mut RemoteAddress>()).read() };
        if vtable.is_null() {
            return Err(EndSceneHookError::NullVtable);
        }
        let slot = unsafe { vtable.add(index) };
        let slot_address = slot as usize as RemoteAddress;
        let original = unsafe { slot.read() };
        if original == 0 {
            return Err(EndSceneHookError::NullTarget);
        }
        if original == replacement {
            return Err(EndSceneHookError::ReplacementIsOriginal);
        }

        // Publish before the pointer replacement, so a native call which sees
        // the new vtable entry can always call the real original target.
        original_slot.store(original, Ordering::Release);
        if let Err(error) = write_vtable_slot(slot_address, replacement) {
            original_slot.store(0, Ordering::Release);
            return Err(error);
        }
        // The patch is complete. A concurrent `stop` cannot restore a zero
        // original once the callback slot has become visible.
        std::sync::atomic::fence(Ordering::SeqCst);

        Ok(Self {
            slot: slot_address,
            original,
            active: true,
        })
    }

    fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        if !self.active {
            return Ok(());
        }
        // Restore the exact pointer captured from this device. The runtime's
        // lifecycle serializes start/stop under one mutex, so no independent
        // Veyr writer can race this slot.
        write_vtable_slot(self.slot, self.original)?;
        self.active = false;
        Ok(())
    }

    const fn original(&self) -> RemoteAddress {
        self.original
    }

    const fn is_active(&self) -> bool {
        self.active
    }
}

/// Restorable replacement of the captured device's `EndScene` vtable slot.
pub struct EndSceneHook {
    hook: VtableSlotHook,
}

impl EndSceneHook {
    /// # Safety
    ///
    /// `device` must remain the active D3D9 device for the lifetime of this
    /// hook; `replacement` must use the COM `__stdcall` ABI.
    pub unsafe fn install(
        device: *mut c_void,
        replacement: EndSceneFn,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        let hook = unsafe {
            VtableSlotHook::install(
                device,
                IDIRECT3DDEVICE9_END_SCENE_INDEX,
                function_address(replacement),
                original_slot,
            )
        }?;
        Ok(Self { hook })
    }

    #[must_use]
    pub const fn original_address(&self) -> RemoteAddress {
        self.hook.original()
    }

    pub fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        self.hook.uninstall()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.hook.is_active()
    }
}

impl Drop for EndSceneHook {
    fn drop(&mut self) {
        let _ = self.uninstall();
    }
}

/// Restorable replacement of the captured device's `Reset` vtable slot.
pub struct ResetHook {
    hook: VtableSlotHook,
}

impl ResetHook {
    /// # Safety
    ///
    /// `device` must remain the active D3D9 device for the lifetime of this
    /// hook; `replacement` must use the COM `__stdcall` ABI.
    pub unsafe fn install(
        device: *mut c_void,
        replacement: ResetFn,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        let hook = unsafe {
            VtableSlotHook::install(
                device,
                IDIRECT3DDEVICE9_RESET_INDEX,
                reset_function_address(replacement),
                original_slot,
            )
        }?;
        Ok(Self { hook })
    }

    #[must_use]
    pub const fn original_address(&self) -> RemoteAddress {
        self.hook.original()
    }

    pub fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        self.hook.uninstall()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.hook.is_active()
    }
}

impl Drop for ResetHook {
    fn drop(&mut self) {
        let _ = self.uninstall();
    }
}

/// Failure while installing or removing a D3D9 vtable replacement.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EndSceneHookError {
    NullDevice,
    NullVtable,
    NullTarget,
    NullReplacement,
    ReplacementIsOriginal,
    Protect {
        target: RemoteAddress,
        win32_error: u32,
    },
    RestoreProtection {
        target: RemoteAddress,
        win32_error: u32,
    },
    Write(LocalProcessMemoryError),
    FlushInstructionCache {
        target: RemoteAddress,
        win32_error: u32,
    },
    // Legacy inline-hook diagnostics retained only for the short-lived
    // Direct3DCreate9 capture and optional OpenGL path. D3D9 device callbacks
    // no longer use these code paths.
    Read(super::memory::LocalProcessMemoryError),
    AddressOverflow {
        address: RemoteAddress,
        offset: u32,
    },
    UnsupportedInstruction {
        target: RemoteAddress,
        opcode: u8,
    },
    TrampolineAllocation {
        win32_error: u32,
    },
    RelativeJumpOutOfRange {
        source: RemoteAddress,
        destination: RemoteAddress,
    },
    ThreadSnapshot {
        win32_error: u32,
    },
    ThreadSuspend {
        thread_id: u32,
        win32_error: u32,
    },
}

impl EndSceneHookError {
    /// Stable private DEV diagnostic used across the loader ABI.
    #[must_use]
    pub const fn diagnostic_code(&self) -> u32 {
        match self {
            Self::NullDevice => 1,
            Self::NullVtable => 2,
            Self::NullTarget => 3,
            Self::NullReplacement => 4,
            Self::ReplacementIsOriginal => 5,
            Self::Protect { .. } => 6,
            Self::RestoreProtection { .. } => 7,
            Self::Write { .. } => 8,
            Self::FlushInstructionCache { .. } => 9,
            Self::Read(_) => 10,
            Self::AddressOverflow { .. } => 11,
            Self::UnsupportedInstruction { .. } => 12,
            Self::TrampolineAllocation { .. } => 13,
            Self::RelativeJumpOutOfRange { .. } => 14,
            Self::ThreadSnapshot { .. } => 15,
            Self::ThreadSuspend { .. } => 16,
        }
    }
}

impl From<super::memory::LocalProcessMemoryError> for EndSceneHookError {
    fn from(error: super::memory::LocalProcessMemoryError) -> Self {
        Self::Read(error)
    }
}

/// Invokes a saved original `IDirect3DDevice9::EndScene` target.
///
/// # Safety
///
/// `address` must be a live original method pointer for `device`.
pub(crate) unsafe fn call_end_scene(address: RemoteAddress, device: *mut c_void) -> i32 {
    unsafe { (function_from_address(address))(device) }
}

/// Invokes a saved original `IDirect3DDevice9::Reset` target.
///
/// # Safety
///
/// `address` must be a live original method pointer for `device`.
pub(crate) unsafe fn call_reset(
    address: RemoteAddress,
    device: *mut c_void,
    parameters: *mut c_void,
) -> i32 {
    unsafe { (reset_function_from_address(address))(device, parameters) }
}

fn write_vtable_slot(
    slot: RemoteAddress,
    replacement: RemoteAddress,
) -> Result<(), EndSceneHookError> {
    let slot_pointer = slot as usize as *mut RemoteAddress;
    let mut old_protection = 0_u32;
    if unsafe {
        VirtualProtect(
            slot_pointer.cast::<c_void>(),
            POINTER_SIZE,
            PAGE_EXECUTE_READWRITE,
            &mut old_protection,
        )
    } == 0
    {
        return Err(EndSceneHookError::Protect {
            target: slot,
            win32_error: unsafe { GetLastError() },
        });
    }

    let mut bytes_written = 0_usize;
    let written = unsafe {
        WriteProcessMemory(
            GetCurrentProcess(),
            slot_pointer.cast::<c_void>(),
            core::ptr::addr_of!(replacement).cast(),
            POINTER_SIZE,
            &mut bytes_written,
        )
    };
    let write_error = (written == 0 || bytes_written != POINTER_SIZE).then(|| {
        EndSceneHookError::Write(LocalProcessMemoryError::Write {
            address: slot,
            win32_error: unsafe { GetLastError() },
        })
    });

    let mut ignored = 0_u32;
    if unsafe {
        VirtualProtect(
            slot_pointer.cast::<c_void>(),
            POINTER_SIZE,
            old_protection,
            &mut ignored,
        )
    } == 0
    {
        return Err(EndSceneHookError::RestoreProtection {
            target: slot,
            win32_error: unsafe { GetLastError() },
        });
    }
    if let Some(error) = write_error {
        return Err(error);
    }
    if unsafe {
        FlushInstructionCache(
            GetCurrentProcess(),
            slot_pointer.cast::<c_void>(),
            POINTER_SIZE,
        )
    } == 0
    {
        return Err(EndSceneHookError::FlushInstructionCache {
            target: slot,
            win32_error: unsafe { GetLastError() },
        });
    }
    Ok(())
}

fn function_address(function: EndSceneFn) -> RemoteAddress {
    function as usize as RemoteAddress
}

fn reset_function_address(function: ResetFn) -> RemoteAddress {
    function as usize as RemoteAddress
}

fn function_from_address(address: RemoteAddress) -> EndSceneFn {
    unsafe { core::mem::transmute::<RemoteAddress, EndSceneFn>(address) }
}

fn reset_function_from_address(address: RemoteAddress) -> ResetFn {
    unsafe { core::mem::transmute::<RemoteAddress, ResetFn>(address) }
}

#[link(name = "kernel32")]
extern "system" {
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetLastError() -> u32;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protection: u32,
        old_protection: *mut u32,
    ) -> i32;
    fn WriteProcessMemory(
        process: *mut c_void,
        base_address: *mut c_void,
        buffer: *const c_void,
        size: usize,
        bytes_written: *mut usize,
    ) -> i32;
}
