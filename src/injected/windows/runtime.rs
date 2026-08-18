//! Explicit lifecycle for the injected Windows x86 graphics runtime.
//!
//! `DllMain` is deliberately not involved. The DEV loader configures one
//! native graphics backend after injection, then starts the runtime through an
//! exported bootstrap. Engine and plugins remain backend-neutral throughout.

use core::cell::Cell;
use core::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use crate::engine::{DeveloperPlugin, Engine, PluginRegistrationError};
use crate::offsets::{advanced_combat::hooks, api::Memory, RemoteAddress};
use crate::plugins::{PlayerCirclePlugin, RenderSmokePlugin};

use super::render_backend::{reset_renderer_stats, GraphicsBackend, NativeFrame, NativeRenderer};
use super::world_collision;
use super::{
    call_end_scene, call_reset, call_swap_buffers, D3d9OverlayRenderer, EndSceneHook,
    EndSceneHookError, LocalProcessMemory, OpenGlOverlayRenderer, ResetHook, SwapBuffersHook,
};

/// Internal receiver of native frame and device-lifecycle events.
///
/// The host contains Rust panics at the native boundary. Native access
/// violations remain unrecoverable and therefore all graphics handles stay in
/// the privileged renderer layer.
pub trait FrameRuntime: Send {
    fn on_frame(&mut self, frame: NativeFrame<'_>);

    fn on_before_device_reset(&mut self, _frame: NativeFrame<'_>) {}

    fn on_after_device_reset(&mut self, _frame: NativeFrame<'_>, _succeeded: bool) {}
}

/// Bridges Engine callbacks to one privileged native renderer.
pub struct EngineRuntime<M: Memory, R> {
    engine: Engine<M>,
    renderer: R,
    update_interval: Duration,
    last_update: Option<Instant>,
}

/// Built-in renderer selected from the loader-supplied graphics target.
pub enum RuntimeOverlayRenderer {
    D3d9(D3d9OverlayRenderer),
    OpenGl(OpenGlOverlayRenderer),
}

impl RuntimeOverlayRenderer {
    fn selected() -> Self {
        match configured_graphics_backend() {
            GraphicsBackend::D3d9 => Self::D3d9(D3d9OverlayRenderer::new()),
            GraphicsBackend::OpenGl => Self::OpenGl(OpenGlOverlayRenderer::new()),
        }
    }
}

impl NativeRenderer for RuntimeOverlayRenderer {
    fn render(
        &mut self,
        frame: NativeFrame<'_>,
        commands: &[crate::offsets::api::QueuedRenderCommand],
    ) {
        match self {
            Self::D3d9(renderer) => renderer.render(frame, commands),
            Self::OpenGl(renderer) => renderer.render(frame, commands),
        }
    }

    fn before_device_reset(&mut self, frame: NativeFrame<'_>) {
        match self {
            Self::D3d9(renderer) => renderer.before_device_reset(frame),
            Self::OpenGl(renderer) => renderer.before_device_reset(frame),
        }
    }

    fn after_device_reset(&mut self, frame: NativeFrame<'_>, succeeded: bool) {
        match self {
            Self::D3d9(renderer) => renderer.after_device_reset(frame, succeeded),
            Self::OpenGl(renderer) => renderer.after_device_reset(frame, succeeded),
        }
    }
}

/// Assembles the built-in Windows Engine before installing native hooks.
pub struct RuntimeBuilder<R = RuntimeOverlayRenderer> {
    engine: Engine<LocalProcessMemory>,
    renderer: R,
}

impl Default for RuntimeBuilder<RuntimeOverlayRenderer> {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder<RuntimeOverlayRenderer> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_renderer(RuntimeOverlayRenderer::selected())
    }
}

impl<R> RuntimeBuilder<R> {
    #[must_use]
    pub fn with_renderer(renderer: R) -> Self {
        Self {
            engine: Engine::new(LocalProcessMemory),
            renderer,
        }
    }

    pub fn register_plugin<P>(&mut self, plugin: P) -> Result<(), PluginRegistrationError>
    where
        P: DeveloperPlugin<LocalProcessMemory> + 'static,
    {
        self.engine.register_plugin(plugin)
    }

    #[must_use]
    pub fn engine(&self) -> &Engine<LocalProcessMemory> {
        &self.engine
    }

    #[must_use]
    pub fn engine_mut(&mut self) -> &mut Engine<LocalProcessMemory> {
        &mut self.engine
    }

    /// Installs the configured native graphics hook with the assembled Engine.
    ///
    /// # Safety
    ///
    /// The configured targets must belong to this supported x86 process and
    /// the DLL must remain loaded until [`stop`] succeeds.
    pub unsafe fn start(self) -> Result<(), RuntimeStartError>
    where
        R: NativeRenderer + 'static,
    {
        unsafe { start(EngineRuntime::new(self.engine, self.renderer)) }
    }
}

impl<M: Memory, R> EngineRuntime<M, R> {
    #[must_use]
    pub fn new(engine: Engine<M>, renderer: R) -> Self {
        Self {
            engine,
            renderer,
            update_interval: Duration::from_millis(16),
            last_update: None,
        }
    }

    /// Limits temporary update ticks. Render dispatch still follows every
    /// graphics callback and therefore tracks the game's actual frame rate.
    #[must_use]
    pub fn with_update_interval(mut self, interval: Duration) -> Self {
        self.update_interval = interval;
        self
    }

    #[must_use]
    pub fn engine(&self) -> &Engine<M> {
        &self.engine
    }

    #[must_use]
    pub fn engine_mut(&mut self) -> &mut Engine<M> {
        &mut self.engine
    }

    #[must_use]
    pub fn renderer(&self) -> &R {
        &self.renderer
    }

    #[must_use]
    pub fn renderer_mut(&mut self) -> &mut R {
        &mut self.renderer
    }
}

impl<M, R> FrameRuntime for EngineRuntime<M, R>
where
    M: Memory + Send,
    R: NativeRenderer,
{
    fn on_frame(&mut self, frame: NativeFrame<'_>) {
        let now = Instant::now();
        if self
            .last_update
            .is_none_or(|last_update| now.duration_since(last_update) >= self.update_interval)
        {
            let _report = self.engine.update();
            self.last_update = Some(now);
        }

        let _report = self.engine.render();
        let commands = self.engine.take_render_commands();
        // A private loader command can arm exactly one terrain ray. Running it
        // here guarantees the call shares WoW's graphics/game thread; it is
        // never performed from the loader's remote thread.
        if let Some(center) = commands.iter().find_map(|queued| match &queued.command {
            crate::offsets::api::RenderCommand::WorldCircle { center, .. } => Some(*center),
            _ => None,
        }) {
            world_collision::run_pending_probe(center);
        }
        self.renderer.render(frame, &commands);
    }

    fn on_before_device_reset(&mut self, frame: NativeFrame<'_>) {
        self.renderer.before_device_reset(frame);
    }

    fn on_after_device_reset(&mut self, frame: NativeFrame<'_>, succeeded: bool) {
        self.renderer.after_device_reset(frame, succeeded);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum GraphicsTargets {
    D3d9 {
        targets: hooks::Direct3d9Targets,
        device: RemoteAddress,
    },
    OpenGl {
        wgl_swap_buffers: RemoteAddress,
        gdi_swap_buffers: Option<RemoteAddress>,
    },
}

impl GraphicsTargets {
    const fn backend(self) -> GraphicsBackend {
        match self {
            Self::D3d9 { .. } => GraphicsBackend::D3d9,
            Self::OpenGl { .. } => GraphicsBackend::OpenGl,
        }
    }

    const fn frame_target(self) -> RemoteAddress {
        match self {
            Self::D3d9 { targets, .. } => targets.end_scene,
            Self::OpenGl {
                wgl_swap_buffers, ..
            } => wgl_swap_buffers,
        }
    }

    const fn auxiliary_target(self) -> RemoteAddress {
        match self {
            Self::D3d9 { targets, .. } => targets.reset,
            Self::OpenGl {
                gdi_swap_buffers, ..
            } => match gdi_swap_buffers {
                Some(target) => target,
                None => 0,
            },
        }
    }
}

enum ActiveHooks {
    D3d9 {
        end_scene: EndSceneHook,
        reset: ResetHook,
    },
    OpenGl {
        wgl_swap_buffers: SwapBuffersHook,
        gdi_swap_buffers: Option<SwapBuffersHook>,
    },
}

impl ActiveHooks {
    fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        match self {
            Self::D3d9 { end_scene, reset } => {
                reset.uninstall()?;
                ORIGINAL_RESET.store(0, Ordering::Release);
                end_scene.uninstall()?;
                ORIGINAL_END_SCENE.store(0, Ordering::Release);
            }
            Self::OpenGl {
                wgl_swap_buffers,
                gdi_swap_buffers,
            } => {
                if let Some(gdi_swap_buffers) = gdi_swap_buffers {
                    gdi_swap_buffers.uninstall()?;
                    ORIGINAL_GDI_SWAP_BUFFERS.store(0, Ordering::Release);
                }
                wgl_swap_buffers.uninstall()?;
                ORIGINAL_WGL_SWAP_BUFFERS.store(0, Ordering::Release);
            }
        }
        Ok(())
    }
}

struct ActiveRuntime {
    hooks: ActiveHooks,
    runtime: Box<dyn FrameRuntime>,
}

// `ACTIVE_RUNTIME` is initialized before either entry patch is made visible.
// The callback path must never lazily create its mutex: a native callback can
// arrive immediately after the jump is written and the allocator/OnceLock
// path is not a safe dependency at that ABI boundary.
static ACTIVE_RUNTIME: OnceLock<Mutex<Option<ActiveRuntime>>> = OnceLock::new();
static GRAPHICS_TARGETS: OnceLock<Mutex<GraphicsTargets>> = OnceLock::new();
static ORIGINAL_END_SCENE: AtomicU32 = AtomicU32::new(0);
static ORIGINAL_RESET: AtomicU32 = AtomicU32::new(0);
static ORIGINAL_WGL_SWAP_BUFFERS: AtomicU32 = AtomicU32::new(0);
static ORIGINAL_GDI_SWAP_BUFFERS: AtomicU32 = AtomicU32::new(0);
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
static CALLBACK_PANIC_COUNT: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static INSIDE_SWAP_BUFFERS: Cell<bool> = const { Cell::new(false) };
}

fn active_runtime() -> &'static Mutex<Option<ActiveRuntime>> {
    ACTIVE_RUNTIME.get_or_init(|| Mutex::new(None))
}

fn initialize_callback_runtime_slot() -> &'static Mutex<Option<ActiveRuntime>> {
    active_runtime()
}

fn graphics_targets() -> &'static Mutex<GraphicsTargets> {
    // A runtime cannot start successfully before the loader supplies live
    // targets. Zeroes intentionally fail the native hook validation instead
    // of retaining stale addresses from an earlier client session.
    GRAPHICS_TARGETS.get_or_init(|| {
        Mutex::new(GraphicsTargets::D3d9 {
            targets: hooks::Direct3d9Targets {
                end_scene: 0,
                reset: 0,
            },
            device: 0,
        })
    })
}

/// Configures exactly one native graphics backend for the next start.
pub fn configure_graphics(
    backend: GraphicsBackend,
    frame_target: RemoteAddress,
    auxiliary_target: RemoteAddress,
    d3d9_device: RemoteAddress,
) -> Result<(), RuntimeConfigurationError> {
    let targets = match backend {
        GraphicsBackend::D3d9 => {
            let targets = hooks::Direct3d9Targets {
                end_scene: frame_target,
                reset: auxiliary_target,
            };
            if !targets.is_valid() {
                return Err(RuntimeConfigurationError::InvalidTargets {
                    backend,
                    frame_target,
                    auxiliary_target,
                });
            }
            GraphicsTargets::D3d9 {
                targets,
                device: d3d9_device,
            }
        }
        GraphicsBackend::OpenGl
            if frame_target != 0 && (auxiliary_target == 0 || auxiliary_target != frame_target) =>
        {
            GraphicsTargets::OpenGl {
                wgl_swap_buffers: frame_target,
                gdi_swap_buffers: (auxiliary_target != 0).then_some(auxiliary_target),
            }
        }
        GraphicsBackend::OpenGl => {
            return Err(RuntimeConfigurationError::InvalidTargets {
                backend,
                frame_target,
                auxiliary_target,
            });
        }
    };

    let active = recover_lock(active_runtime().lock());
    if active.is_some() {
        return Err(RuntimeConfigurationError::AlreadyRunning);
    }
    *recover_lock(graphics_targets().lock()) = targets;
    Ok(())
}

/// Compatibility helper for the previous D3D9-only bootstrap.
pub fn configure_d3d9_targets(
    targets: hooks::Direct3d9Targets,
) -> Result<(), RuntimeConfigurationError> {
    configure_graphics(GraphicsBackend::D3d9, targets.end_scene, targets.reset, 0)
}

pub fn configure_opengl_target(target: RemoteAddress) -> Result<(), RuntimeConfigurationError> {
    configure_graphics(GraphicsBackend::OpenGl, target, 0, 0)
}

#[must_use]
pub fn configured_graphics_backend() -> GraphicsBackend {
    recover_lock(graphics_targets().lock()).backend()
}

#[must_use]
pub fn configured_frame_target() -> RemoteAddress {
    recover_lock(graphics_targets().lock()).frame_target()
}

#[must_use]
pub fn configured_reset_target() -> RemoteAddress {
    match *recover_lock(graphics_targets().lock()) {
        GraphicsTargets::D3d9 { targets, .. } => targets.reset,
        GraphicsTargets::OpenGl { .. } => 0,
    }
}

/// Secondary direct entry selected by the current backend.
///
/// This is D3D9 `Reset` for D3D9, or optional `gdi32!SwapBuffers` for OpenGL.
#[must_use]
pub fn configured_auxiliary_target() -> RemoteAddress {
    recover_lock(graphics_targets().lock()).auxiliary_target()
}

#[must_use]
pub fn configured_d3d9_targets() -> hooks::Direct3d9Targets {
    match *recover_lock(graphics_targets().lock()) {
        GraphicsTargets::D3d9 { targets, .. } => targets,
        GraphicsTargets::OpenGl { .. } => hooks::Direct3d9Targets {
            end_scene: 0,
            reset: 0,
        },
    }
}

#[must_use]
pub fn configured_opengl_target() -> RemoteAddress {
    match *recover_lock(graphics_targets().lock()) {
        GraphicsTargets::D3d9 { .. } => 0,
        GraphicsTargets::OpenGl {
            wgl_swap_buffers, ..
        } => wgl_swap_buffers,
    }
}

/// Installs the currently configured graphics hook.
///
/// # Safety
///
/// Targets must point to the matching live functions in this x86 process.
pub unsafe fn start<R>(runtime: R) -> Result<(), RuntimeStartError>
where
    R: FrameRuntime + 'static,
{
    let runtime_slot = initialize_callback_runtime_slot();
    let mut slot = recover_lock(runtime_slot.lock());
    if slot.is_some() {
        return Err(RuntimeStartError::AlreadyRunning);
    }

    let runtime: Box<dyn FrameRuntime> = Box::new(runtime);
    let targets = *recover_lock(graphics_targets().lock());
    FRAME_COUNT.store(0, Ordering::Release);
    CALLBACK_PANIC_COUNT.store(0, Ordering::Release);
    reset_renderer_stats();
    let hooks = match targets {
        GraphicsTargets::D3d9 { targets, .. } => {
            let mut reset =
                match unsafe { ResetHook::install(targets.reset, reset_dispatch, &ORIGINAL_RESET) }
                {
                    Ok(hook) => hook,
                    Err(error) => {
                        ORIGINAL_RESET.store(0, Ordering::Release);
                        return Err(RuntimeStartError::Hook(error));
                    }
                };

            let end_scene = match unsafe {
                EndSceneHook::install(targets.end_scene, end_scene_dispatch, &ORIGINAL_END_SCENE)
            } {
                Ok(hook) => hook,
                Err(error) => {
                    let _ = reset.uninstall();
                    ORIGINAL_END_SCENE.store(0, Ordering::Release);
                    ORIGINAL_RESET.store(0, Ordering::Release);
                    return Err(RuntimeStartError::Hook(error));
                }
            };
            ActiveHooks::D3d9 { end_scene, reset }
        }
        GraphicsTargets::OpenGl {
            wgl_swap_buffers,
            gdi_swap_buffers,
        } => {
            let mut wgl_swap_buffers = unsafe {
                SwapBuffersHook::install(
                    wgl_swap_buffers,
                    wgl_swap_buffers_dispatch,
                    &ORIGINAL_WGL_SWAP_BUFFERS,
                )
            }
            .map_err(RuntimeStartError::Hook)?;
            let gdi_swap_buffers = if let Some(gdi_swap_buffers) = gdi_swap_buffers {
                match unsafe {
                    SwapBuffersHook::install(
                        gdi_swap_buffers,
                        gdi_swap_buffers_dispatch,
                        &ORIGINAL_GDI_SWAP_BUFFERS,
                    )
                } {
                    Ok(hook) => Some(hook),
                    Err(error) => {
                        let _ = wgl_swap_buffers.uninstall();
                        ORIGINAL_WGL_SWAP_BUFFERS.store(0, Ordering::Release);
                        ORIGINAL_GDI_SWAP_BUFFERS.store(0, Ordering::Release);
                        return Err(RuntimeStartError::Hook(error));
                    }
                }
            } else {
                None
            };
            ActiveHooks::OpenGl {
                wgl_swap_buffers,
                gdi_swap_buffers,
            }
        }
    };

    *slot = Some(ActiveRuntime { hooks, runtime });
    Ok(())
}

/// Starts the minimal built-in runtime with no developer plugins.
///
/// # Safety
///
/// The loader-configured target must be the matching live function in this
/// x86 process, and the DLL must remain loaded until [`stop`] succeeds.
pub unsafe fn start_default() -> Result<(), RuntimeStartError> {
    unsafe { RuntimeBuilder::new().start() }
}

/// Starts the first-party cyan-circle visual smoke plugin.
///
/// # Safety
///
/// Has the same requirements as [`start_default`].
pub unsafe fn start_visual_smoke() -> Result<(), RuntimeStartError> {
    let mut builder = RuntimeBuilder::new();
    builder
        .register_plugin(RenderSmokePlugin)
        .expect("the visual smoke plugin is registered into an empty bootstrap");
    unsafe { builder.start() }
}

/// Starts the first-party static player-circle world-render milestone.
///
/// The plugin emits no command until a local player and finite position are
/// available, so character-select and loading screens remain inert. During
/// the world-projection milestone it also installs the deterministic HUD
/// marker: this proves the D3D9 overlay independently of camera/W2S data.
///
/// # Safety
///
/// Has the same requirements as [`start_default`].
pub unsafe fn start_player_circle() -> Result<(), RuntimeStartError> {
    let mut builder = RuntimeBuilder::new();
    builder
        .register_plugin(PlayerCirclePlugin::default())
        .expect("the player-circle plugin is registered into an empty bootstrap");
    builder
        .register_plugin(RenderSmokePlugin)
        .expect("the HUD diagnostic plugin is registered into an empty bootstrap");
    unsafe { builder.start() }
}

/// Restores the active native function entries.
pub fn stop() -> Result<(), RuntimeStopError> {
    let mut slot = recover_lock(active_runtime().lock());
    let active = slot.as_mut().ok_or(RuntimeStopError::NotRunning)?;
    active.hooks.uninstall().map_err(RuntimeStopError::Hook)?;
    *slot = None;
    Ok(())
}

#[must_use]
pub fn is_running() -> bool {
    recover_lock(active_runtime().lock()).is_some()
}

/// Dispatched native render frames since the last successful start.
#[must_use]
pub fn frame_count() -> u32 {
    FRAME_COUNT.load(Ordering::Acquire)
}

#[must_use]
pub fn callback_panic_count() -> u32 {
    CALLBACK_PANIC_COUNT.load(Ordering::Acquire)
}

#[derive(Debug)]
pub enum RuntimeStartError {
    AlreadyRunning,
    Hook(EndSceneHookError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RuntimeConfigurationError {
    AlreadyRunning,
    InvalidTargets {
        backend: GraphicsBackend,
        frame_target: RemoteAddress,
        auxiliary_target: RemoteAddress,
    },
}

#[derive(Debug)]
pub enum RuntimeStopError {
    NotRunning,
    Hook(EndSceneHookError),
}

unsafe extern "system" fn end_scene_dispatch(device: *mut c_void) -> i32 {
    if let Some(frame) = NativeFrame::from_d3d9(device) {
        dispatch_frame(frame);
    }

    let original = ORIGINAL_END_SCENE.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    unsafe { call_end_scene(original, device) }
}

unsafe extern "system" fn reset_dispatch(device: *mut c_void, parameters: *mut c_void) -> i32 {
    if let Some(frame) = NativeFrame::from_d3d9(device) {
        dispatch_reset(frame, |runtime, frame| {
            runtime.on_before_device_reset(frame)
        });
    }

    let original = ORIGINAL_RESET.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let result = unsafe { call_reset(original, device, parameters) };

    if let Some(frame) = NativeFrame::from_d3d9(device) {
        dispatch_reset(frame, |runtime, frame| {
            runtime.on_after_device_reset(frame, result >= 0)
        });
    }
    result
}

unsafe extern "system" fn wgl_swap_buffers_dispatch(device_context: *mut c_void) -> i32 {
    dispatch_swap_buffers(
        device_context,
        ORIGINAL_WGL_SWAP_BUFFERS.load(Ordering::Acquire),
    )
}

unsafe extern "system" fn gdi_swap_buffers_dispatch(device_context: *mut c_void) -> i32 {
    dispatch_swap_buffers(
        device_context,
        ORIGINAL_GDI_SWAP_BUFFERS.load(Ordering::Acquire),
    )
}

fn dispatch_swap_buffers(device_context: *mut c_void, original: RemoteAddress) -> i32 {
    let entered = SwapReentryGuard::enter();
    if entered.is_some() {
        if let Some(frame) = NativeFrame::from_opengl(device_context) {
            dispatch_frame(frame);
        }
    }

    if original == 0 {
        return 0;
    }
    unsafe { call_swap_buffers(original, device_context) }
}

struct SwapReentryGuard;

impl SwapReentryGuard {
    fn enter() -> Option<Self> {
        INSIDE_SWAP_BUFFERS.with(|inside| {
            if inside.replace(true) {
                None
            } else {
                Some(Self)
            }
        })
    }
}

impl Drop for SwapReentryGuard {
    fn drop(&mut self) {
        INSIDE_SWAP_BUFFERS.with(|inside| inside.set(false));
    }
}

fn dispatch_frame(frame: NativeFrame<'_>) {
    let Some(runtime_slot) = ACTIVE_RUNTIME.get() else {
        return;
    };
    let mut slot = match runtime_slot.try_lock() {
        Ok(slot) => slot,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let Some(active) = slot.as_mut() else {
        return;
    };

    FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if catch_unwind(AssertUnwindSafe(|| active.runtime.on_frame(frame))).is_err() {
        CALLBACK_PANIC_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

fn dispatch_reset(
    frame: NativeFrame<'_>,
    callback: impl FnOnce(&mut dyn FrameRuntime, NativeFrame<'_>),
) {
    let Some(runtime_slot) = ACTIVE_RUNTIME.get() else {
        return;
    };
    let mut slot = match runtime_slot.try_lock() {
        Ok(slot) => slot,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let Some(active) = slot.as_mut() else {
        return;
    };

    if catch_unwind(AssertUnwindSafe(|| {
        callback(active.runtime.as_mut(), frame)
    }))
    .is_err()
    {
        CALLBACK_PANIC_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}
