//! Internal developer API and memory map for WoW 3.3.5a build 12340.

#[path = "../offsets.rs"]
pub mod offsets;

/// Runtime orchestration for first-party developer plugins.
pub mod engine;

/// Composition of trusted developer plugins outside the Engine host.
pub mod plugins;

/// Windows injected-runtime building blocks.
pub mod injected;

/// Commands accepted by the x86 Windows loader through [`veyr_remote_command`].
///
/// This is a private bootstrap ABI, not the future public SDK. Its explicit
/// numeric values are stable so an external loader can pass one as the thread
/// parameter to `CreateRemoteThread`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RemoteCommand {
    StartDefault = 2,
    StartVisualSmoke = 3,
    Stop = 4,
    FrameCount = 5,
    CallbackPanicCount = 6,
    RenderSubmittedCommands = 7,
    RenderDrawnCommands = 8,
    RenderSkippedCommands = 9,
    RenderDrawFailures = 10,
    RenderStateSetupFailed = 11,
    RenderLastError = 12,
    ConfiguredEndScene = 13,
    ConfiguredReset = 14,
    ConfiguredGraphicsBackend = 15,
    ConfiguredFrameTarget = 16,
    LastHookError = 17,
    ConfiguredAuxiliaryTarget = 18,
    ArmD3d9Capture = 19,
    D3d9CaptureState = 20,
    D3d9FactoryCallCount = 21,
    D3d9CreateDeviceCallCount = 22,
    CapturedD3d9Device = 23,
    CapturedD3d9EndScene = 24,
    CapturedD3d9Reset = 25,
    D3d9CaptureError = 26,
    StartPlayerCircle = 27,
    ArmTerrainProbe = 28,
    TerrainProbeStatus = 29,
    TerrainProbeHitX = 30,
    TerrainProbeHitY = 31,
    TerrainProbeHitZ = 32,
    TerrainProbeNativeResult = 33,
}

#[cfg(all(windows, target_arch = "x86"))]
static LAST_HOOK_ERROR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Version of the private loader-to-DLL graphics bootstrap structure.
pub const GRAPHICS_CONFIGURATION_ABI_VERSION: u32 = 3;
pub const GRAPHICS_BACKEND_D3D9: u32 = 1;
pub const GRAPHICS_BACKEND_OPENGL: u32 = 2;

/// Backend-neutral graphics targets written into the remote process by the
/// companion DEV loader.
///
/// `backend` deliberately remains a raw integer at the FFI boundary so an
/// unknown value can be rejected without constructing an invalid Rust enum.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RemoteGraphicsConfiguration {
    pub abi_version: u32,
    pub backend: u32,
    pub frame_target: u32,
    /// D3D9 `Reset`, or the second OpenGL swap function when present.
    pub auxiliary_target: u32,
    /// Captured primary `IDirect3DDevice9` pointer for optional passive
    /// shader-constant diagnostics. Zero disables that diagnostic path.
    pub d3d9_device: u32,
}

/// Thread-entry-compatible bootstrap for the selected native graphics backend.
///
/// Return values: `0` success, `1` runtime already running, `2` invalid ABI,
/// backend, or targets, and `3` a Rust panic was contained.
///
/// # Safety
///
/// `parameter` must point to a readable [`RemoteGraphicsConfiguration`] in the
/// current process for the duration of this call.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub unsafe extern "system" fn veyr_remote_configure_graphics(
    parameter: *mut core::ffi::c_void,
) -> u32 {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    if parameter.is_null() {
        return 2;
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let configuration =
            unsafe { core::ptr::read_unaligned(parameter.cast::<RemoteGraphicsConfiguration>()) };
        if configuration.abi_version != GRAPHICS_CONFIGURATION_ABI_VERSION {
            return None;
        }
        let backend = injected::windows::GraphicsBackend::try_from(configuration.backend).ok()?;
        Some(injected::windows::configure_graphics(
            backend,
            configuration.frame_target,
            configuration.auxiliary_target,
            configuration.d3d9_device,
        ))
    })) {
        Ok(Some(Ok(()))) => 0,
        Ok(Some(Err(injected::windows::RuntimeConfigurationError::AlreadyRunning))) => 1,
        Ok(Some(Err(injected::windows::RuntimeConfigurationError::InvalidTargets { .. })))
        | Ok(None) => 2,
        Err(_) => 3,
    }
}

/// Returns the direct `IDirect3DDevice9::EndScene` entry configured by loader.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_end_scene_address() -> u32 {
    injected::windows::configured_d3d9_targets().end_scene
}

/// Thread-entry-compatible bootstrap for loader-resolved D3D9 targets.
///
/// Return values: `0` success, `1` runtime already running, `2` invalid
/// addresses, and `3` a Rust panic was contained. The parameter points to a
/// remote [`offsets::advanced_combat::hooks::Direct3d9Targets`] allocation in
/// this process.
///
/// # Safety
///
/// `parameter` must point to a readable `Direct3d9Targets` value in the current
/// process for the duration of this call.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub unsafe extern "system" fn veyr_remote_configure_d3d9_targets(
    parameter: *mut core::ffi::c_void,
) -> u32 {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    if parameter.is_null() {
        return 2;
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let targets = unsafe {
            core::ptr::read_unaligned(
                parameter.cast::<offsets::advanced_combat::hooks::Direct3d9Targets>(),
            )
        };
        injected::windows::configure_d3d9_targets(targets)
    })) {
        Ok(Ok(())) => 0,
        Ok(Err(injected::windows::RuntimeConfigurationError::AlreadyRunning)) => 1,
        Ok(Err(injected::windows::RuntimeConfigurationError::InvalidTargets { .. })) => 2,
        Err(_) => 3,
    }
}

/// Returns the configured private [`injected::windows::GraphicsBackend`] value.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_graphics_backend() -> u32 {
    injected::windows::configured_graphics_backend() as u32
}

/// Returns the configured frame-hook entry (`EndScene` or `wglSwapBuffers`).
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_graphics_frame_target() -> u32 {
    injected::windows::configured_frame_target()
}

/// Returns the backend-specific optional secondary target.
///
/// For D3D9 this is `Reset`; for OpenGL this is `gdi32!SwapBuffers` when the
/// loader resolved one. It is zero when no secondary target is configured.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_graphics_auxiliary_target() -> u32 {
    injected::windows::configured_auxiliary_target()
}

/// Arms early capture of the real D3D9 device during game initialization.
///
/// The companion loader calls this while the game's primary thread is
/// suspended, then resumes it and polls the capture diagnostics below. This
/// is deliberately separate from starting the renderer: the loader first
/// verifies the actual `IDirect3DDevice9` method entries it captured.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_arm() -> u32 {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    match catch_unwind(AssertUnwindSafe(|| unsafe {
        injected::windows::arm_d3d9_capture()
    })) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.diagnostic_code(),
        Err(_) => u32::MAX - 1,
    }
}

/// Returns the current state of the early D3D9-device capture.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_state() -> u32 {
    injected::windows::d3d9_capture_snapshot().state as u32
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_factory_calls() -> u32 {
    injected::windows::d3d9_capture_snapshot().factory_calls
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_create_device_calls() -> u32 {
    injected::windows::d3d9_capture_snapshot().create_device_calls
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_device() -> u32 {
    injected::windows::d3d9_capture_snapshot().device
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_end_scene() -> u32 {
    injected::windows::d3d9_capture_snapshot().targets.end_scene
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_reset() -> u32 {
    injected::windows::d3d9_capture_snapshot().targets.reset
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_capture_error() -> u32 {
    injected::windows::d3d9_capture_snapshot().error
}

/// Explicitly starts the minimal injected runtime after `LoadLibrary`.
///
/// Return values: `0` success, `1` already running, `2` native hooking
/// failed, and `4` a Rust panic was contained. This must never be invoked from
/// `DllMain`.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_start_default() -> u32 {
    start_runtime(|| unsafe { injected::windows::start_default() })
}

/// Starts the opt-in developer visual smoke test after `LoadLibrary`.
///
/// A cyan circle with a red cross appears near the upper-left corner when the
/// native frame hook and render-command path are both working. It has the same
/// return values and `DllMain` restriction as [`veyr_runtime_start_default`].
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_start_visual_smoke() -> u32 {
    start_runtime(|| unsafe { injected::windows::start_visual_smoke() })
}

/// Starts the first developer world-render milestone: a static radius-20
/// outline around the local player. Until player state is available it waits
/// silently and submits no render commands.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_start_player_circle() -> u32 {
    start_runtime(|| unsafe { injected::windows::start_player_circle() })
}

#[cfg(all(windows, target_arch = "x86"))]
fn start_runtime(start: impl FnOnce() -> Result<(), injected::windows::RuntimeStartError>) -> u32 {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::Ordering;

    LAST_HOOK_ERROR.store(0, Ordering::Release);
    match catch_unwind(AssertUnwindSafe(start)) {
        Ok(Ok(())) => 0,
        Ok(Err(injected::windows::RuntimeStartError::AlreadyRunning)) => 1,
        Ok(Err(injected::windows::RuntimeStartError::Hook(error))) => {
            LAST_HOOK_ERROR.store(error.diagnostic_code(), Ordering::Release);
            2
        }
        Err(_) => 4,
    }
}

#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_last_hook_error() -> u32 {
    use std::sync::atomic::Ordering;

    LAST_HOOK_ERROR.load(Ordering::Acquire)
}

/// Stops the explicit injected runtime before this DLL is unloaded.
///
/// Return values: `0` success, `1` was not running, `2` hook restoration
/// failed, and `3` a Rust panic was contained.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_stop() -> u32 {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    match catch_unwind(AssertUnwindSafe(injected::windows::stop)) {
        Ok(Ok(())) => 0,
        Ok(Err(injected::windows::RuntimeStopError::NotRunning)) => 1,
        Ok(Err(injected::windows::RuntimeStopError::Hook(_))) => 2,
        Err(_) => 3,
    }
}

/// Returns native frame callbacks since runtime start.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_frame_count() -> u32 {
    injected::windows::frame_count()
}

/// Returns Rust panics isolated at the native graphics callback boundary.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_callback_panic_count() -> u32 {
    injected::windows::callback_panic_count()
}

/// Returns render commands submitted to the last native renderer frame.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_submitted_commands() -> u32 {
    injected::windows::last_renderer_stats().submitted_commands
}

/// Returns successfully drawn commands in the last native renderer frame.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_drawn_commands() -> u32 {
    injected::windows::last_renderer_stats().drawn_commands
}

/// Returns skipped commands in the last native renderer frame.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_skipped_commands() -> u32 {
    injected::windows::last_renderer_stats().skipped_commands
}

/// Returns failed native draw calls in the last renderer frame.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_draw_failures() -> u32 {
    injected::windows::last_renderer_stats().draw_failures
}

/// Returns one when native overlay state setup failed in the last frame.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_state_setup_failed() -> u32 {
    u32::from(injected::windows::last_renderer_stats().state_setup_failed)
}

/// Returns the last backend-specific renderer error as a raw 32-bit value.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_runtime_render_last_error() -> u32 {
    injected::windows::last_renderer_stats().last_error
}

/// Arms one render-thread vertical ray from the current player-circle centre.
///
/// This private DEV diagnostic validates the recovered world-collision ABI.
/// It neither changes game state nor starts a second runtime. A request made
/// before entering a world stays armed until the first finite circle centre.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_terrain_probe_arm() -> u32 {
    injected::windows::world_collision::request_probe();
    0
}

/// Last one-shot terrain-probe state. See `world_collision` for DEV codes.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_terrain_probe_status() -> u32 {
    injected::windows::world_collision::probe_status()
}

/// One IEEE-754 component of the last terrain-hit position (`0=x`, `1=y`,
/// `2=z`). This remains a word-only loader ABI by design.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_terrain_probe_hit_component(index: u32) -> u32 {
    injected::windows::world_collision::probe_hit_component_bits(index as usize)
}

/// The raw client collision return code from the latest one-shot probe.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_terrain_probe_native_result() -> u32 {
    injected::windows::world_collision::probe_native_result()
}

/// Returns the direct Reset entry configured by the DEV loader.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_d3d9_reset_address() -> u32 {
    injected::windows::configured_d3d9_targets().reset
}

/// Thread-entry adapter used only by the companion Windows x86 DEV loader.
///
/// `CreateRemoteThread` invokes a `fn(*mut c_void) -> u32`, while the direct
/// exports above deliberately have no arguments. This adapter preserves their
/// simple in-process API without relying on an x86 calling-convention mismatch.
/// It must not be called from `DllMain`.
#[cfg(all(windows, target_arch = "x86"))]
#[no_mangle]
pub extern "system" fn veyr_remote_command(command: *mut core::ffi::c_void) -> u32 {
    let command = command as usize as u32;
    match command {
        value if value == RemoteCommand::StartDefault as u32 => veyr_runtime_start_default(),
        value if value == RemoteCommand::StartVisualSmoke as u32 => {
            veyr_runtime_start_visual_smoke()
        }
        value if value == RemoteCommand::StartPlayerCircle as u32 => {
            veyr_runtime_start_player_circle()
        }
        value if value == RemoteCommand::ArmTerrainProbe as u32 => veyr_terrain_probe_arm(),
        value if value == RemoteCommand::TerrainProbeStatus as u32 => veyr_terrain_probe_status(),
        value if value == RemoteCommand::TerrainProbeHitX as u32 => {
            veyr_terrain_probe_hit_component(0)
        }
        value if value == RemoteCommand::TerrainProbeHitY as u32 => {
            veyr_terrain_probe_hit_component(1)
        }
        value if value == RemoteCommand::TerrainProbeHitZ as u32 => {
            veyr_terrain_probe_hit_component(2)
        }
        value if value == RemoteCommand::TerrainProbeNativeResult as u32 => {
            veyr_terrain_probe_native_result()
        }
        value if value == RemoteCommand::Stop as u32 => veyr_runtime_stop(),
        value if value == RemoteCommand::FrameCount as u32 => veyr_runtime_frame_count(),
        value if value == RemoteCommand::CallbackPanicCount as u32 => {
            veyr_runtime_callback_panic_count()
        }
        value if value == RemoteCommand::RenderSubmittedCommands as u32 => {
            veyr_runtime_render_submitted_commands()
        }
        value if value == RemoteCommand::RenderDrawnCommands as u32 => {
            veyr_runtime_render_drawn_commands()
        }
        value if value == RemoteCommand::RenderSkippedCommands as u32 => {
            veyr_runtime_render_skipped_commands()
        }
        value if value == RemoteCommand::RenderDrawFailures as u32 => {
            veyr_runtime_render_draw_failures()
        }
        value if value == RemoteCommand::RenderStateSetupFailed as u32 => {
            veyr_runtime_render_state_setup_failed()
        }
        value if value == RemoteCommand::RenderLastError as u32 => veyr_runtime_render_last_error(),
        value if value == RemoteCommand::ConfiguredEndScene as u32 => veyr_d3d9_end_scene_address(),
        value if value == RemoteCommand::ConfiguredReset as u32 => veyr_d3d9_reset_address(),
        value if value == RemoteCommand::ConfiguredGraphicsBackend as u32 => {
            veyr_graphics_backend()
        }
        value if value == RemoteCommand::ConfiguredFrameTarget as u32 => {
            veyr_graphics_frame_target()
        }
        value if value == RemoteCommand::LastHookError as u32 => veyr_runtime_last_hook_error(),
        value if value == RemoteCommand::ConfiguredAuxiliaryTarget as u32 => {
            veyr_graphics_auxiliary_target()
        }
        value if value == RemoteCommand::ArmD3d9Capture as u32 => veyr_d3d9_capture_arm(),
        value if value == RemoteCommand::D3d9CaptureState as u32 => veyr_d3d9_capture_state(),
        value if value == RemoteCommand::D3d9FactoryCallCount as u32 => {
            veyr_d3d9_capture_factory_calls()
        }
        value if value == RemoteCommand::D3d9CreateDeviceCallCount as u32 => {
            veyr_d3d9_capture_create_device_calls()
        }
        value if value == RemoteCommand::CapturedD3d9Device as u32 => veyr_d3d9_capture_device(),
        value if value == RemoteCommand::CapturedD3d9EndScene as u32 => {
            veyr_d3d9_capture_end_scene()
        }
        value if value == RemoteCommand::CapturedD3d9Reset as u32 => veyr_d3d9_capture_reset(),
        value if value == RemoteCommand::D3d9CaptureError as u32 => veyr_d3d9_capture_error(),
        _ => u32::MAX,
    }
}
