//! Windows x86-only injected-runtime primitives.

#[allow(dead_code)]
mod client_assets;
mod d3d9_capture;
mod d3d9_hook;
mod d3d9_renderer;
mod direct_hook;
mod memory;
mod opengl_hook;
mod opengl_renderer;
mod render_backend;
mod runtime;
pub(crate) mod world_collision;
mod world_projection;

pub use d3d9_capture::{
    arm as arm_d3d9_capture, is_active as d3d9_capture_active, snapshot as d3d9_capture_snapshot,
    stop as stop_d3d9_capture, D3d9CaptureError, D3d9CaptureSnapshot, D3d9CaptureState,
};
pub(crate) use d3d9_hook::{call_end_scene, call_reset};
pub use d3d9_hook::{EndSceneFn, EndSceneHook, EndSceneHookError, ResetFn, ResetHook};
pub use d3d9_renderer::{last_d3d9_render_stats, D3d9OverlayRenderer, D3d9RenderStats};
pub use memory::{LocalProcessMemory, LocalProcessMemoryError};
pub(crate) use opengl_hook::call_swap_buffers;
pub use opengl_hook::{SwapBuffersFn, SwapBuffersHook, WglSwapBuffersFn, WglSwapBuffersHook};
pub use opengl_renderer::OpenGlOverlayRenderer;
pub use render_backend::{
    last_renderer_stats, GraphicsBackend, NativeFrame, NativeRenderer, RendererStats,
};
pub use runtime::{
    callback_panic_count, configure_d3d9_targets, configure_graphics, configure_opengl_target,
    configured_auxiliary_target, configured_d3d9_device, configured_d3d9_targets,
    configured_frame_target, configured_graphics_backend, configured_opengl_target,
    configured_reset_target, frame_count, is_running, start, start_default, start_player_circle,
    start_visual_smoke, stop, EngineRuntime, FrameRuntime, RuntimeBuilder,
    RuntimeConfigurationError, RuntimeOverlayRenderer, RuntimeStartError, RuntimeStopError,
};
