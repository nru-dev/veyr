//! Typed wrapper around the shared x86 inline detour for OpenGL buffer swaps.

use core::ffi::c_void;
use std::sync::atomic::AtomicU32;

use crate::offsets::RemoteAddress;

use super::{d3d9_hook::EndSceneHookError, direct_hook::DirectHook};

/// ABI shared by `wglSwapBuffers` and `gdi32!SwapBuffers` on Windows x86.
pub type SwapBuffersFn = unsafe extern "system" fn(device_context: *mut c_void) -> i32;

/// Compatibility name for the OpenGL wrapper export.
pub type WglSwapBuffersFn = SwapBuffersFn;

/// Restorable direct-entry hook for a Windows buffer-swap function.
pub struct SwapBuffersHook {
    hook: DirectHook,
}

impl SwapBuffersHook {
    /// Installs an x86 detour at a loader-resolved buffer-swap export.
    ///
    /// # Safety
    ///
    /// `target` must be the current process's live `wglSwapBuffers` entry and
    /// `replacement` must retain the exact system ABI until uninstallation.
    pub unsafe fn install(
        target: RemoteAddress,
        replacement: SwapBuffersFn,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        let replacement = replacement as usize as RemoteAddress;
        let hook = unsafe { DirectHook::install(target, replacement, original_slot) }?;
        Ok(Self { hook })
    }

    #[must_use]
    pub const fn original_address(&self) -> RemoteAddress {
        self.hook.trampoline()
    }

    pub fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        self.hook.uninstall()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.hook.is_active()
    }
}

impl Drop for SwapBuffersHook {
    fn drop(&mut self) {
        let _ = self.uninstall();
    }
}

/// Invokes the copied original buffer-swap prologue.
///
/// # Safety
///
/// `address` must be this hook's trampoline and `device_context` must be the
/// native HDC supplied by the intercepted call.
pub(crate) unsafe fn call_swap_buffers(address: RemoteAddress, device_context: *mut c_void) -> i32 {
    let function: SwapBuffersFn = unsafe { core::mem::transmute(address as usize) };
    unsafe { function(device_context) }
}

/// Compatibility type for callers that specifically hook `wglSwapBuffers`.
pub type WglSwapBuffersHook = SwapBuffersHook;
