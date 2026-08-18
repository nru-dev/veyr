//! Early D3D9-device discovery for the injected developer runtime.
//!
//! This module is intentionally separate from the renderer. It is armed while
//! the game process is suspended, observes `Direct3DCreate9` and the returned
//! factory's `CreateDevice` call, then publishes the *actual* device method
//! entries. No WoW memory layout or pre-recorded D3D9 RVA is involved.

use core::ffi::{c_char, c_void};
use core::mem::transmute;
use core::ptr::null;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::offsets::{advanced_combat::hooks::Direct3d9Targets, RemoteAddress};

use super::d3d9_hook::{DirectHook, EndSceneHookError};

/// Status returned through the private loader ABI while the early capture is
/// armed. Values are explicit so the loader can print them without sharing
/// Rust types across the process boundary.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum D3d9CaptureState {
    Idle = 0,
    Armed = 1,
    FactoryObserved = 2,
    DeviceCaptured = 3,
    Failed = 4,
}

impl D3d9CaptureState {
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::Armed,
            2 => Self::FactoryObserved,
            3 => Self::DeviceCaptured,
            4 => Self::Failed,
            _ => Self::Idle,
        }
    }
}

/// Snapshot of one early D3D9 capture attempt.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct D3d9CaptureSnapshot {
    pub state: D3d9CaptureState,
    pub factory_calls: u32,
    pub create_device_calls: u32,
    pub device: RemoteAddress,
    pub targets: Direct3d9Targets,
    /// Zero means that installing and observing the capture hooks succeeded.
    pub error: u32,
}

type Direct3dCreate9Fn = unsafe extern "system" fn(sdk_version: u32) -> *mut c_void;
/// `IDirect3D9::CreateDevice` is a COM `STDMETHODCALLTYPE` method. On x86
/// this is `__stdcall`: the interface pointer and six documented arguments
/// are all stack arguments, and the callee pops seven words. Do not use Rust
/// `thiscall` here just because the native caller was compiled from C++; the
/// vtable ABI is COM, not an ordinary C++ member-function ABI.
type CreateDeviceFn = unsafe extern "system" fn(
    factory: *mut c_void,
    adapter: u32,
    device_type: u32,
    focus_window: *mut c_void,
    behavior_flags: u32,
    presentation_parameters: *mut c_void,
    returned_device: *mut *mut c_void,
) -> i32;

const IDIRECT3D9_CREATE_DEVICE_INDEX: usize = 16;
const IDIRECT3DDEVICE9_RESET_INDEX: usize = 16;
const IDIRECT3DDEVICE9_END_SCENE_INDEX: usize = 42;
const ERROR_ALREADY_ARMED: u32 = 100;
const ERROR_D3D9_MODULE: u32 = 101;
const ERROR_DIRECT3D_CREATE9_EXPORT: u32 = 102;
const ERROR_FACTORY_VTABLE: u32 = 103;
const ERROR_CREATE_DEVICE_TARGET: u32 = 104;
const ERROR_CAPTURED_DEVICE_VTABLE: u32 = 105;
const ERROR_CAPTURED_DEVICE_TARGETS: u32 = 106;
/// An ordinary Rust panic escaped the observer section of a callback.
const ERROR_CALLBACK_FAULT: u32 = 107;

static STATE: AtomicU32 = AtomicU32::new(D3d9CaptureState::Idle as u32);
static FACTORY_CALLS: AtomicU32 = AtomicU32::new(0);
static CREATE_DEVICE_CALLS: AtomicU32 = AtomicU32::new(0);
static CAPTURED_DEVICE: AtomicU32 = AtomicU32::new(0);
static CAPTURED_END_SCENE: AtomicU32 = AtomicU32::new(0);
static CAPTURED_RESET: AtomicU32 = AtomicU32::new(0);
static LAST_ERROR: AtomicU32 = AtomicU32::new(0);
static ORIGINAL_DIRECT3D_CREATE9: AtomicU32 = AtomicU32::new(0);
static ORIGINAL_CREATE_DEVICE: AtomicU32 = AtomicU32::new(0);
static CAPTURE_HOOKS: OnceLock<Mutex<Option<CaptureHooks>>> = OnceLock::new();

struct CaptureHooks {
    direct3d_create9: DirectHook,
    create_device: Option<DirectHook>,
}

fn capture_hooks() -> &'static Mutex<Option<CaptureHooks>> {
    CAPTURE_HOOKS.get_or_init(|| Mutex::new(None))
}

/// Loads D3D9 when necessary and arms the pre-device capture hook.
///
/// This must be called from the loader's remote bootstrap thread, never from
/// `DllMain`. `LoadLibraryW("d3d9.dll")` only maps the system runtime; it does
/// not create a factory or device. Arming immediately after the ordinary
/// user-mode bootstrap is therefore deliberately earlier than waiting for a
/// possibly unrelated client-side D3D9 module probe.
///
/// # Safety
///
/// The current process must be the supported 32-bit Windows game process. The
/// DLL must remain loaded until [`stop`] restores the capture hooks.
pub unsafe fn arm() -> Result<(), D3d9CaptureError> {
    let mut hooks = recover_lock(capture_hooks().lock());
    if hooks.is_some() {
        return Err(D3d9CaptureError::AlreadyArmed);
    }

    reset_observation();
    let d3d9 = unsafe { load_d3d9_module() }.ok_or(D3d9CaptureError::D3d9Module)?;
    let create9 = unsafe { GetProcAddress(d3d9, c"Direct3DCreate9".as_ptr()) };
    let create9 = function_address(create9).ok_or(D3d9CaptureError::Direct3dCreate9Export)?;
    let hook = unsafe {
        DirectHook::install(
            create9,
            callback_address(direct3d_create9_dispatch as *const ()),
            &ORIGINAL_DIRECT3D_CREATE9,
        )
    }
    .map_err(D3d9CaptureError::Hook)?;

    *hooks = Some(CaptureHooks {
        direct3d_create9: hook,
        create_device: None,
    });
    STATE.store(D3d9CaptureState::Armed as u32, Ordering::Release);
    Ok(())
}

/// Removes capture hooks. The normal EndScene/Reset runtime is independent and
/// can stay active after the factory hooks have been removed.
pub fn stop() -> Result<(), EndSceneHookError> {
    let mut hooks = recover_lock(capture_hooks().lock());
    if let Some(mut hooks) = hooks.take() {
        if let Some(create_device) = hooks.create_device.as_mut() {
            create_device.uninstall()?;
        }
        hooks.direct3d_create9.uninstall()?;
    }
    ORIGINAL_DIRECT3D_CREATE9.store(0, Ordering::Release);
    ORIGINAL_CREATE_DEVICE.store(0, Ordering::Release);
    STATE.store(D3d9CaptureState::Idle as u32, Ordering::Release);
    Ok(())
}

#[must_use]
pub fn snapshot() -> D3d9CaptureSnapshot {
    D3d9CaptureSnapshot {
        state: D3d9CaptureState::from_raw(STATE.load(Ordering::Acquire)),
        factory_calls: FACTORY_CALLS.load(Ordering::Acquire),
        create_device_calls: CREATE_DEVICE_CALLS.load(Ordering::Acquire),
        device: CAPTURED_DEVICE.load(Ordering::Acquire),
        targets: Direct3d9Targets {
            end_scene: CAPTURED_END_SCENE.load(Ordering::Acquire),
            reset: CAPTURED_RESET.load(Ordering::Acquire),
        },
        error: LAST_ERROR.load(Ordering::Acquire),
    }
}

#[derive(Debug)]
pub enum D3d9CaptureError {
    AlreadyArmed,
    D3d9Module,
    Direct3dCreate9Export,
    Hook(EndSceneHookError),
}

impl D3d9CaptureError {
    #[must_use]
    pub const fn diagnostic_code(&self) -> u32 {
        match self {
            Self::AlreadyArmed => ERROR_ALREADY_ARMED,
            Self::D3d9Module => ERROR_D3D9_MODULE,
            Self::Direct3dCreate9Export => ERROR_DIRECT3D_CREATE9_EXPORT,
            Self::Hook(error) => error.diagnostic_code(),
        }
    }
}

unsafe extern "system" fn direct3d_create9_dispatch(sdk_version: u32) -> *mut c_void {
    // Do not touch Rust allocation/locks or the factory object until the
    // client call has returned. In particular, the original Create9 must run
    // exactly once even if our observation path rejects it.
    let original = ORIGINAL_DIRECT3D_CREATE9.load(Ordering::Acquire);
    if original == 0 {
        return null::<c_void>() as *mut c_void;
    }
    let factory = unsafe { call_direct3d_create9(original, sdk_version) };
    if factory.is_null() {
        return factory;
    }

    // A Rust panic is not an access-violation firewall, but it prevents an
    // ordinary observation error from unwinding across the Win32 ABI. The
    // actual factory is still returned unmodified.
    // Never allow a second factory instance to replace the first hook while
    // D3D9 is still constructing devices. One factory is sufficient for the
    // bootstrap, and avoiding a lock/patch on later calls removes a startup
    // race with libraries that probe D3D9 more than once.
    if STATE.load(Ordering::Acquire) == D3d9CaptureState::Armed as u32 {
        let observed = catch_unwind(AssertUnwindSafe(|| unsafe { observe_factory(factory) }));
        match observed {
            Ok(Ok(())) => {}
            Ok(Err(error)) => fail(error),
            Err(_) => fail(ERROR_CALLBACK_FAULT),
        }
    }
    factory
}

unsafe extern "system" fn create_device_dispatch(
    factory: *mut c_void,
    adapter: u32,
    device_type: u32,
    focus_window: *mut c_void,
    behavior_flags: u32,
    presentation_parameters: *mut c_void,
    returned_device: *mut *mut c_void,
) -> i32 {
    // The real CreateDevice is called before we inspect its output. This is
    // intentionally a pure post-call observer; no capture fault can prevent
    // WoW from receiving the device it requested.
    let original = ORIGINAL_CREATE_DEVICE.load(Ordering::Acquire);
    if original == 0 {
        return -1;
    }
    let result = unsafe {
        call_create_device(
            original,
            factory,
            adapter,
            device_type,
            focus_window,
            behavior_flags,
            presentation_parameters,
            returned_device,
        )
    };
    if result < 0 || returned_device.is_null() {
        return result;
    }

    // A valid pointer is owned by D3D9 for the duration of this call. Copying
    // it here is the only direct dereference in the callback; the remaining
    // work is guarded so it cannot cross the native boundary as a panic.
    let device = unsafe { returned_device.read() };
    if device.is_null() {
        return result;
    }
    // Capture only the first successfully created device. Later devices can
    // belong to a failed-mode fallback or reset path and must never make us
    // patch another factory while this callback is active.
    if STATE.load(Ordering::Acquire) == D3d9CaptureState::FactoryObserved as u32 {
        let observed = catch_unwind(AssertUnwindSafe(|| unsafe {
            observe_created_device(device)
        }));
        match observed {
            Ok(Ok(())) => {}
            Ok(Err(error)) => fail(error),
            Err(_) => fail(ERROR_CALLBACK_FAULT),
        }
    }
    result
}

unsafe fn observe_factory(factory: *mut c_void) -> Result<(), u32> {
    FACTORY_CALLS.fetch_add(1, Ordering::Relaxed);
    STATE.store(D3d9CaptureState::FactoryObserved as u32, Ordering::Release);
    unsafe { install_create_device_hook(factory) }
}

unsafe fn observe_created_device(device: *mut c_void) -> Result<(), u32> {
    CREATE_DEVICE_CALLS.fetch_add(1, Ordering::Relaxed);
    unsafe { publish_device(device) }
}

unsafe fn install_create_device_hook(factory: *mut c_void) -> Result<(), u32> {
    let mut hooks = recover_lock(capture_hooks().lock());
    let Some(hooks) = hooks.as_mut() else {
        return Err(ERROR_ALREADY_ARMED);
    };
    if hooks.create_device.is_some() {
        return Ok(());
    }

    let vtable = unsafe { read_vtable(factory) }.ok_or(ERROR_FACTORY_VTABLE)?;
    let create_device = unsafe { read_vtable_entry(vtable, IDIRECT3D9_CREATE_DEVICE_INDEX) }
        .ok_or(ERROR_CREATE_DEVICE_TARGET)?;
    let hook = unsafe {
        DirectHook::install(
            create_device,
            callback_address(create_device_dispatch as *const ()),
            &ORIGINAL_CREATE_DEVICE,
        )
    }
    .map_err(|error| error.diagnostic_code())?;
    hooks.create_device = Some(hook);
    Ok(())
}

unsafe fn publish_device(device: *mut c_void) -> Result<(), u32> {
    let vtable = unsafe { read_vtable(device) }.ok_or(ERROR_CAPTURED_DEVICE_VTABLE)?;
    let reset = unsafe { read_vtable_entry(vtable, IDIRECT3DDEVICE9_RESET_INDEX) }
        .ok_or(ERROR_CAPTURED_DEVICE_TARGETS)?;
    let end_scene = unsafe { read_vtable_entry(vtable, IDIRECT3DDEVICE9_END_SCENE_INDEX) }
        .ok_or(ERROR_CAPTURED_DEVICE_TARGETS)?;
    let targets = Direct3d9Targets { end_scene, reset };
    if !targets.is_valid() {
        return Err(ERROR_CAPTURED_DEVICE_TARGETS);
    }

    CAPTURED_DEVICE.store(device as usize as RemoteAddress, Ordering::Release);
    CAPTURED_END_SCENE.store(end_scene, Ordering::Release);
    CAPTURED_RESET.store(reset, Ordering::Release);
    STATE.store(D3d9CaptureState::DeviceCaptured as u32, Ordering::Release);
    Ok(())
}

unsafe fn read_vtable(instance: *mut c_void) -> Option<*const RemoteAddress> {
    let vtable = unsafe { (instance as *const *const RemoteAddress).read() };
    (!vtable.is_null()).then_some(vtable)
}

unsafe fn read_vtable_entry(vtable: *const RemoteAddress, index: usize) -> Option<RemoteAddress> {
    let target = unsafe { vtable.add(index).read() };
    (target != 0).then_some(target)
}

unsafe fn call_direct3d_create9(address: RemoteAddress, sdk_version: u32) -> *mut c_void {
    let function: Direct3dCreate9Fn = unsafe { transmute(address as usize) };
    unsafe { function(sdk_version) }
}

#[allow(clippy::too_many_arguments)]
unsafe fn call_create_device(
    address: RemoteAddress,
    factory: *mut c_void,
    adapter: u32,
    device_type: u32,
    focus_window: *mut c_void,
    behavior_flags: u32,
    presentation_parameters: *mut c_void,
    returned_device: *mut *mut c_void,
) -> i32 {
    let function: CreateDeviceFn = unsafe { transmute(address as usize) };
    unsafe {
        function(
            factory,
            adapter,
            device_type,
            focus_window,
            behavior_flags,
            presentation_parameters,
            returned_device,
        )
    }
}

unsafe fn load_d3d9_module() -> Option<*mut c_void> {
    let name = [
        b'd' as u16,
        b'3' as u16,
        b'd' as u16,
        b'9' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if !module.is_null() {
        return Some(module);
    }
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    (!module.is_null()).then_some(module)
}

fn function_address(address: *mut c_void) -> Option<RemoteAddress> {
    u32::try_from(address as usize)
        .ok()
        .filter(|address| *address != 0)
}

fn callback_address(callback: *const ()) -> RemoteAddress {
    u32::try_from(callback as usize).expect("the supported injected DLL is x86")
}

fn reset_observation() {
    FACTORY_CALLS.store(0, Ordering::Release);
    CREATE_DEVICE_CALLS.store(0, Ordering::Release);
    CAPTURED_DEVICE.store(0, Ordering::Release);
    CAPTURED_END_SCENE.store(0, Ordering::Release);
    CAPTURED_RESET.store(0, Ordering::Release);
    LAST_ERROR.store(0, Ordering::Release);
    ORIGINAL_DIRECT3D_CREATE9.store(0, Ordering::Release);
    ORIGINAL_CREATE_DEVICE.store(0, Ordering::Release);
}

fn fail(error: u32) {
    LAST_ERROR.store(error, Ordering::Release);
    STATE.store(D3d9CaptureState::Failed as u32, Ordering::Release);
}

fn recover_lock<T>(result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
}
