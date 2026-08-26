//! Direct x86 entry hooks for the injected D3D9 runtime.
//!
//! The runtime deliberately hooks the configured `EndScene` and `Reset`
//! entries themselves. This is the architecture used by the known-working
//! build: [`DirectHook`] copies the complete original prologue to a trampoline,
//! installs a five-byte detour, and the dispatchers call that trampoline after
//! running the Rust callback. The captured device and current CreateDevice ABI
//! are only used to discover/configure the entries; no device-vtable slot is
//! rewritten here.

use core::ffi::c_void;
use std::sync::atomic::AtomicU32;

use crate::offsets::RemoteAddress;

use super::{direct_hook::DirectHook, memory::LocalProcessMemoryError};

/// ABI of `IDirect3DDevice9::EndScene` on the supported x86 target.
pub type EndSceneFn = unsafe extern "system" fn(device: *mut c_void) -> i32;

/// ABI of `IDirect3DDevice9::Reset` on the supported x86 target.
pub type ResetFn = unsafe extern "system" fn(device: *mut c_void, parameters: *mut c_void) -> i32;

/// Restorable direct-entry hook for `IDirect3DDevice9::EndScene`.
///
/// Unlike a per-device vtable replacement, this hooks the explicitly
/// configured method entry. It therefore follows every WoW device that uses
/// that D3D9 implementation and does not depend on an undocumented vtable
/// layout. The original prologue is invoked through the trampoline owned by
/// [`DirectHook`].
pub struct EndSceneHook {
    hook: DirectHook,
}

impl EndSceneHook {
    /// Installs a direct x86 detour at the absolute `EndScene` entry.
    ///
    /// # Safety
    ///
    /// `target` must be the current process's live
    /// `IDirect3DDevice9::EndScene` entry. `replacement` must remain valid
    /// until this hook is uninstalled, use the exact ABI, and never unwind a
    /// panic into the native client.
    pub unsafe fn install(
        target: RemoteAddress,
        replacement: EndSceneFn,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        let hook =
            unsafe { DirectHook::install(target, function_address(replacement), original_slot) }?;
        Ok(Self { hook })
    }

    /// Calls the trampoline containing the original `EndScene` prologue.
    ///
    /// # Safety
    ///
    /// `device` must be the live device passed to this hook invocation.
    pub unsafe fn call_original(&self, device: *mut c_void) -> i32 {
        unsafe { call_end_scene(self.hook.trampoline(), device) }
    }

    /// Address of the trampoline used to invoke the original method.
    #[must_use]
    pub const fn original_address(&self) -> RemoteAddress {
        self.hook.trampoline()
    }

    /// Restores the target's original bytes.
    pub fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        self.hook.uninstall()
    }

    /// Whether this direct hook is installed.
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

/// Restorable direct-entry hook for `IDirect3DDevice9::Reset`.
pub struct ResetHook {
    hook: DirectHook,
}

impl ResetHook {
    /// Installs a direct x86 detour at the absolute `Reset` entry.
    ///
    /// # Safety
    ///
    /// `target` must be the current process's live `Reset` entry.
    /// `replacement` must remain valid until this hook is uninstalled, use the
    /// exact ABI, and never unwind a panic into the native client.
    pub unsafe fn install(
        target: RemoteAddress,
        replacement: ResetFn,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        let hook = unsafe {
            DirectHook::install(target, reset_function_address(replacement), original_slot)
        }?;
        Ok(Self { hook })
    }

    /// Calls the trampoline containing the original `Reset` prologue.
    ///
    /// # Safety
    ///
    /// `device` and `parameters` must be the values passed to this invocation.
    pub unsafe fn call_original(&self, device: *mut c_void, parameters: *mut c_void) -> i32 {
        unsafe { call_reset(self.hook.trampoline(), device, parameters) }
    }

    /// Address of the trampoline used to invoke the original method.
    #[must_use]
    pub const fn original_address(&self) -> RemoteAddress {
        self.hook.trampoline()
    }

    /// Restores the target's original bytes.
    pub fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        self.hook.uninstall()
    }

    /// Whether this direct hook is installed.
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

/// Failure while installing or removing a D3D9 direct hook.
///
/// The `NullDevice`/`NullVtable` variants are retained for the short-lived
/// D3D9 factory/device capture path, which uses the same diagnostic type for
/// pointer-cell edits. The active frame hooks use the direct-entry variants
/// below through [`DirectHook`].
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
    Read(LocalProcessMemoryError),
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
            Self::Write(_) => 8,
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

impl From<LocalProcessMemoryError> for EndSceneHookError {
    fn from(error: LocalProcessMemoryError) -> Self {
        Self::Read(error)
    }
}

/// Invokes an `EndScene` address at the FFI boundary.
///
/// # Safety
///
/// `address` must point to the active original method or its trampoline, and
/// `device` must be the matching live D3D9 device pointer.
pub(crate) unsafe fn call_end_scene(address: RemoteAddress, device: *mut c_void) -> i32 {
    unsafe { (function_from_address(address))(device) }
}

/// Invokes a `Reset` address at the FFI boundary.
///
/// # Safety
///
/// `address` must point to the active original method or its trampoline, and
/// its arguments must come from the matching native `Reset` call.
pub(crate) unsafe fn call_reset(
    address: RemoteAddress,
    device: *mut c_void,
    parameters: *mut c_void,
) -> i32 {
    unsafe { (reset_function_from_address(address))(device, parameters) }
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
