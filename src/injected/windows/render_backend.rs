//! Backend-neutral contract between the Engine and privileged native renderers.
//!
//! Neither the Engine nor plugins can construct or inspect a [`NativeFrame`].
//! They submit constrained render commands; the selected injected backend owns
//! every native graphics handle and translates those commands at frame time.

use core::ffi::c_void;
use core::marker::PhantomData;
use core::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::offsets::api::QueuedRenderCommand;

/// Native renderer selected for the current injected runtime.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GraphicsBackend {
    D3d9 = crate::GRAPHICS_BACKEND_D3D9,
    OpenGl = crate::GRAPHICS_BACKEND_OPENGL,
}

impl TryFrom<u32> for GraphicsBackend {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            value if value == Self::D3d9 as u32 => Ok(Self::D3d9),
            value if value == Self::OpenGl as u32 => Ok(Self::OpenGl),
            _ => Err(()),
        }
    }
}

/// Opaque borrow of one native graphics callback.
pub struct NativeFrame<'callback> {
    source: NativeFrameSource,
    callback: PhantomData<&'callback mut ()>,
}

enum NativeFrameSource {
    D3d9 { device: NonNull<c_void> },
    OpenGl { device_context: NonNull<c_void> },
}

impl NativeFrame<'_> {
    pub(crate) fn from_d3d9(device: *mut c_void) -> Option<Self> {
        NonNull::new(device).map(|device| Self {
            source: NativeFrameSource::D3d9 { device },
            callback: PhantomData,
        })
    }

    pub(crate) fn from_opengl(device_context: *mut c_void) -> Option<Self> {
        NonNull::new(device_context).map(|device_context| Self {
            source: NativeFrameSource::OpenGl { device_context },
            callback: PhantomData,
        })
    }

    pub(crate) const fn d3d9_device(&self) -> Option<*mut c_void> {
        match self.source {
            NativeFrameSource::D3d9 { device } => Some(device.as_ptr()),
            NativeFrameSource::OpenGl { .. } => None,
        }
    }

    pub(crate) const fn opengl_device_context(&self) -> Option<*mut c_void> {
        match self.source {
            NativeFrameSource::D3d9 { .. } => None,
            NativeFrameSource::OpenGl { device_context } => Some(device_context.as_ptr()),
        }
    }
}

/// Privileged consumer of Engine-owned render commands.
///
/// This is injected-runtime infrastructure, not developer API. Implementations
/// must restore every native graphics state they change before returning.
pub trait NativeRenderer: Send {
    fn render(&mut self, frame: NativeFrame<'_>, commands: &[QueuedRenderCommand]);

    fn before_device_reset(&mut self, _frame: NativeFrame<'_>) {}

    fn after_device_reset(&mut self, _frame: NativeFrame<'_>, _succeeded: bool) {}
}

/// Backend-neutral diagnostics for the most recently submitted overlay frame.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct RendererStats {
    pub submitted_commands: u32,
    pub drawn_commands: u32,
    pub skipped_commands: u32,
    pub draw_failures: u32,
    pub state_setup_failed: bool,
    /// Backend-specific raw error (`HRESULT` for D3D9, internal code for GL).
    pub last_error: u32,
}

static LAST_SUBMITTED_COMMANDS: AtomicU32 = AtomicU32::new(0);
static LAST_DRAWN_COMMANDS: AtomicU32 = AtomicU32::new(0);
static LAST_SKIPPED_COMMANDS: AtomicU32 = AtomicU32::new(0);
static LAST_DRAW_FAILURES: AtomicU32 = AtomicU32::new(0);
static LAST_STATE_SETUP_FAILED: AtomicU32 = AtomicU32::new(0);
static LAST_ERROR: AtomicU32 = AtomicU32::new(0);

pub(crate) fn publish_renderer_stats(stats: RendererStats) {
    LAST_SUBMITTED_COMMANDS.store(stats.submitted_commands, Ordering::Release);
    LAST_DRAWN_COMMANDS.store(stats.drawn_commands, Ordering::Release);
    LAST_SKIPPED_COMMANDS.store(stats.skipped_commands, Ordering::Release);
    LAST_DRAW_FAILURES.store(stats.draw_failures, Ordering::Release);
    LAST_STATE_SETUP_FAILED.store(u32::from(stats.state_setup_failed), Ordering::Release);
    LAST_ERROR.store(stats.last_error, Ordering::Release);
}

pub(crate) fn reset_renderer_stats() {
    publish_renderer_stats(RendererStats::default());
}

/// Snapshot used only by the private DEV loader diagnostics ABI.
#[must_use]
pub fn last_renderer_stats() -> RendererStats {
    RendererStats {
        submitted_commands: LAST_SUBMITTED_COMMANDS.load(Ordering::Acquire),
        drawn_commands: LAST_DRAWN_COMMANDS.load(Ordering::Acquire),
        skipped_commands: LAST_SKIPPED_COMMANDS.load(Ordering::Acquire),
        draw_failures: LAST_DRAW_FAILURES.load(Ordering::Acquire),
        state_setup_failed: LAST_STATE_SETUP_FAILED.load(Ordering::Acquire) != 0,
        last_error: LAST_ERROR.load(Ordering::Acquire),
    }
}
