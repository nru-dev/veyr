//! Windows x86 developer loader for the injected visual smoke test.
//!
//! Usage: `veyr.exe <wow-pid> <absolute-or-relative-path-to-veyr.dll>`.
//! It loads the DLL, then calls its `CreateRemoteThread`-compatible bootstrap
//! export with the visual-smoke command. `status` and `stop` address an
//! already injected matching DLL without loading a second copy.

#[cfg(all(windows, target_arch = "x86"))]
fn main() {
    if let Err(error) = windows_x86::run() {
        eprintln!("veyr: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(windows, target_arch = "x86")))]
fn main() {
    eprintln!("veyr is a Windows x86 developer tool; build it for i686-pc-windows-gnu.");
    std::process::exit(1);
}

#[cfg(all(windows, target_arch = "x86"))]
mod windows_x86 {
    use core::ffi::{c_char, c_void};
    use core::mem::{transmute, zeroed};
    use std::env;
    use std::ffi::OsStr;
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    use veyr::{
        offsets::{
            advanced_combat::{
                camera,
                hooks::{self, loader_discovery},
            },
            memory::{object, object_manager, unit},
            RemoteAddress,
        },
        RemoteCommand, RemoteGraphicsConfiguration, RemoteLaunchOutcome, RemoteLaunchReport,
        RemoteLaunchRequest, GRAPHICS_BACKEND_D3D9, GRAPHICS_BACKEND_OPENGL,
        GRAPHICS_CONFIGURATION_ABI_VERSION, REMOTE_LAUNCH_ABI_VERSION,
    };

    type Handle = *mut c_void;
    type Module = *mut c_void;
    type ThreadEntry = unsafe extern "system" fn(*mut c_void) -> u32;

    const PROCESS_CREATE_THREAD: u32 = 0x0002;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_OPERATION: u32 = 0x0008;
    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_VM_WRITE: u32 = 0x0020;
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RELEASE: u32 = 0x8000;
    const MEM_RESERVE: u32 = 0x2000;
    const PAGE_READWRITE: u32 = 0x04;
    const TH32CS_SNAPMODULE: u32 = 0x0000_0008;
    const TH32CS_SNAPMODULE32: u32 = 0x0000_0010;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const INFINITE: u32 = u32::MAX;
    const DEBUG_ONLY_THIS_PROCESS: u32 = 0x0000_0002;
    const ERROR_BAD_LENGTH: u32 = 24;
    const ERROR_NO_MORE_FILES: u32 = 18;
    const ERROR_PARTIAL_COPY: u32 = 299;
    const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
    const LAUNCH_WORKER_GRACE: Duration = Duration::from_secs(5);
    const STARTUP_GATE_TIMEOUT: Duration = Duration::from_secs(10);
    // The debugger's initial breakpoint is the earliest user-mode point at
    // which the process loader has mapped kernel32 and accepts a remote
    // LoadLibrary thread. It lets us inject and arm before `Wow.exe` executes
    // any application startup code or initializes D3D9.
    const EXCEPTION_BREAKPOINT: u32 = 0x8000_0003;
    const DBG_CONTINUE: u32 = 0x0001_0002;
    const DBG_EXCEPTION_NOT_HANDLED: u32 = 0x8001_0001;
    const EXIT_PROCESS_DEBUG_EVENT: u32 = 5;
    // The ToolHelp module walk can still race a freshly initialized loader.
    // Microsoft documents ERROR_BAD_LENGTH for this case and asks callers to
    // retry the snapshot. Keep that recovery local to every module lookup.
    const MODULE_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
    const MODULE_SNAPSHOT_POLL_INTERVAL: Duration = Duration::from_millis(5);
    const MODULE_SNAPSHOT_MAX_ATTEMPTS: u32 = 1_000;

    const INVALID_HANDLE_VALUE: Handle = -1_isize as Handle;

    #[repr(C)]
    struct ModuleEntry32W {
        size: u32,
        module_id: u32,
        process_id: u32,
        global_usage_count: u32,
        process_usage_count: u32,
        base_address: *mut u8,
        base_size: u32,
        module: Module,
        module_name: [u16; 256],
        executable_path: [u16; 260],
    }

    impl ModuleEntry32W {
        fn new() -> Self {
            Self {
                size: core::mem::size_of::<Self>() as u32,
                module_id: 0,
                process_id: 0,
                global_usage_count: 0,
                process_usage_count: 0,
                base_address: null_mut(),
                base_size: 0,
                module: null_mut(),
                module_name: [0; 256],
                executable_path: [0; 260],
            }
        }
    }

    #[repr(C)]
    struct StartupInfoW {
        size: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_count_chars: u32,
        y_count_chars: u32,
        fill_attribute: u32,
        flags: u32,
        show_window: u16,
        reserved2_count: u16,
        reserved2: *mut u8,
        standard_input: Handle,
        standard_output: Handle,
        standard_error: Handle,
    }

    #[repr(C)]
    struct ProcessInformation {
        process: Handle,
        thread: Handle,
        process_id: u32,
        thread_id: u32,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        nested_record: *mut ExceptionRecord,
        address: *mut c_void,
        parameter_count: u32,
        information: [u32; 15],
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct ExceptionDebugInfo {
        record: ExceptionRecord,
        first_chance: u32,
    }

    #[repr(C)]
    union DebugEventData {
        exception: ExceptionDebugInfo,
        raw: [u8; 164],
    }

    #[repr(C)]
    struct DebugEvent {
        event_code: u32,
        process_id: u32,
        thread_id: u32,
        data: DebugEventData,
    }

    impl DebugEvent {
        fn initial_breakpoint(&self) -> bool {
            if self.event_code != 1 {
                return false;
            }
            // Safety: the active union member is `exception` exactly when
            // `event_code == EXCEPTION_DEBUG_EVENT` (value 1).
            unsafe { self.data.exception.record.code == EXCEPTION_BREAKPOINT }
        }
    }

    extern "system" {
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn CreateRemoteThread(
            process: Handle,
            attributes: *const c_void,
            stack_size: usize,
            start_address: Option<ThreadEntry>,
            parameter: *mut c_void,
            creation_flags: u32,
            thread_id: *mut u32,
        ) -> Handle;
        fn CreateProcessW(
            application_name: *const u16,
            command_line: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit_handles: i32,
            creation_flags: u32,
            environment: *const c_void,
            current_directory: *const u16,
            startup_info: *mut StartupInfoW,
            process_information: *mut ProcessInformation,
        ) -> i32;
        fn ContinueDebugEvent(process_id: u32, thread_id: u32, continue_status: u32) -> i32;
        fn DebugActiveProcessStop(process_id: u32) -> i32;
        fn FreeLibrary(module: Module) -> i32;
        fn GetExitCodeThread(thread: Handle, exit_code: *mut u32) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn GetLastError() -> u32;
        fn GetModuleHandleW(name: *const u16) -> Module;
        fn GetProcAddress(module: Module, name: *const c_char) -> *mut c_void;
        fn LoadLibraryW(path: *const u16) -> Module;
        fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
        fn Module32NextW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn IsWow64Process(process: Handle, wow64_process: *mut i32) -> i32;
        fn ReadProcessMemory(
            process: Handle,
            base_address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            bytes_read: *mut usize,
        ) -> i32;
        fn ResumeThread(thread: Handle) -> u32;
        fn SuspendThread(thread: Handle) -> u32;
        fn VirtualAllocEx(
            process: Handle,
            address: *const c_void,
            size: usize,
            allocation_type: u32,
            protect: u32,
        ) -> *mut c_void;
        fn VirtualFreeEx(process: Handle, address: *mut c_void, size: usize, free_type: u32)
            -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn WaitForDebugEvent(event: *mut DebugEvent, milliseconds: u32) -> i32;
        fn WriteProcessMemory(
            process: Handle,
            base_address: *mut c_void,
            buffer: *const c_void,
            size: usize,
            bytes_written: *mut usize,
        ) -> i32;
    }

    pub fn run() -> Result<(), String> {
        match parse_invocation()? {
            LoaderInvocation::Existing(arguments) => run_existing(arguments),
            LoaderInvocation::Launch(arguments) => run_launch(arguments),
        }
    }

    fn run_existing(arguments: LoaderArguments) -> Result<(), String> {
        let LoaderArguments {
            mode,
            process_id,
            dll_path,
        } = arguments;
        let process = Process::open(process_id)?;

        let live_graphics = (mode == LoaderMode::VisualSmoke)
            .then(|| discover_live_graphics(&process))
            .transpose()?;

        if mode == LoaderMode::VisualSmoke {
            ensure_graphics_not_already_patched(
                &process,
                live_graphics
                    .as_ref()
                    .expect("visual mode resolves a graphics backend"),
            )?;
        }

        let local_module = LocalModule::load(&dll_path)?;
        let remote_command_rva = local_module.export_rva(b"veyr_remote_command\0")?;
        let remote_module = match mode {
            LoaderMode::VisualSmoke => inject(&process, &dll_path)?,
            LoaderMode::Status | LoaderMode::TerrainProbe | LoaderMode::Stop => {
                let module_name = dll_path
                    .file_name()
                    .ok_or_else(|| "veyr.dll path has no file name".to_owned())?
                    .to_string_lossy();
                remote_module_base(&process, &module_name)?
            }
        };
        let remote_command = remote_module
            .checked_add(remote_command_rva)
            .ok_or_else(|| "remote command address overflowed x86 address space".to_owned())?;
        match mode {
            LoaderMode::VisualSmoke => {
                let live = live_graphics.expect("visual mode resolves a graphics backend");
                let configure_rva = local_module.export_rva(b"veyr_remote_configure_graphics\0")?;
                let configure = remote_module.checked_add(configure_rva).ok_or_else(|| {
                    "remote graphics configuration address overflowed x86 space".to_owned()
                })?;
                let configuration = [live.configuration()];
                let remote_configuration = RemoteAllocation::write_slice(&process, &configuration)?;
                let configure_result =
                    call_remote(&process, configure, remote_configuration.address)?;
                println!("graphics configuration result: {configure_result}");
                if configure_result != 0 {
                    return Err(format!(
                        "veyr.dll rejected the live graphics configuration with status {configure_result}"
                    ));
                }

                let result = call_remote(
                    &process,
                    remote_command,
                    RemoteCommand::StartVisualSmoke as u32,
                )?;

                println!("injected module loaded at 0x{remote_module:08X}");
                println!("visual smoke result: {result}");
                if result == 0 {
                    println!("success: look for the cyan circle with red cross near (120, 120)");
                    Ok(())
                } else {
                    if let Some(code) = query_optional_command(
                        &process,
                        remote_command,
                        RemoteCommand::LastHookError,
                    )? {
                        print_hook_error(code);
                    }
                    Err(format!(
                        "visual smoke startup failed with veyr status {result}"
                    ))
                }
            }
            LoaderMode::Status => {
                println!("injected module is already loaded at 0x{remote_module:08X}");
                let configured_backend = required_diagnostic(
                    &process,
                    remote_command,
                    RemoteCommand::ConfiguredGraphicsBackend,
                )?;
                let configured_frame_target = required_diagnostic(
                    &process,
                    remote_command,
                    RemoteCommand::ConfiguredFrameTarget,
                )?;
                let configured_auxiliary_target = required_diagnostic(
                    &process,
                    remote_command,
                    RemoteCommand::ConfiguredAuxiliaryTarget,
                )?;
                if configured_frame_target == 0 {
                    return Err("DLL reports a null configured frame target".to_owned());
                }

                let (backend_name, frame_name) = match configured_backend {
                    GRAPHICS_BACKEND_D3D9 => ("D3D9", "EndScene"),
                    GRAPHICS_BACKEND_OPENGL => ("OpenGL", "wglSwapBuffers"),
                    value => (
                        "unknown",
                        if value == 0 {
                            "frame hook"
                        } else {
                            "frame target"
                        },
                    ),
                };
                println!("DLL configured graphics backend: {backend_name} ({configured_backend})");
                println!("DLL configured {frame_name}: 0x{configured_frame_target:08X}");

                let frames =
                    call_remote(&process, remote_command, RemoteCommand::FrameCount as u32)?;
                println!("graphics runtime frames since startup: {frames}");
                if frames == 0 {
                    println!("no dispatched frames yet; inspecting the configured direct patch");
                } else {
                    println!("frame hook is firing; inspecting the renderer diagnostics");
                }
                print_direct_patch(&process, frame_name, configured_frame_target)?;
                if configured_backend == GRAPHICS_BACKEND_D3D9 {
                    let configured_reset = required_diagnostic(
                        &process,
                        remote_command,
                        RemoteCommand::ConfiguredReset,
                    )?;
                    if configured_reset == 0 {
                        return Err("DLL reports a null configured D3D9 Reset target".to_owned());
                    }
                    println!("DLL configured Reset: 0x{configured_reset:08X}");
                    print_direct_patch(&process, "Reset", configured_reset)?;
                } else if configured_backend == GRAPHICS_BACKEND_OPENGL {
                    if configured_auxiliary_target == 0 {
                        println!("DLL configured gdi32!SwapBuffers: unavailable");
                    } else {
                        println!(
                            "DLL configured gdi32!SwapBuffers: 0x{configured_auxiliary_target:08X}"
                        );
                        print_direct_patch(
                            &process,
                            "gdi32!SwapBuffers",
                            configured_auxiliary_target,
                        )?;
                    }
                }
                let _renderer_result = print_extended_diagnostics(&process, remote_command)?;
                if configured_backend == GRAPHICS_BACKEND_D3D9 {
                    print_build_12340_world_frame_probe(&process);
                    print_world_projection_probe(&process);
                }
                if let Some(code) =
                    query_optional_command(&process, remote_command, RemoteCommand::LastHookError)?
                {
                    print_hook_error(code);
                }
                Ok(())
            }
            LoaderMode::TerrainProbe => {
                println!("injected module is already loaded at 0x{remote_module:08X}");
                let frames =
                    required_diagnostic(&process, remote_command, RemoteCommand::FrameCount)?;
                if frames == 0 {
                    return Err(
                        "the graphics runtime has not dispatched a frame; launch the game through veyr first"
                            .to_owned(),
                    );
                }

                let arm_result = call_remote(
                    &process,
                    remote_command,
                    RemoteCommand::ArmTerrainProbe as u32,
                )?;
                if arm_result != 0 {
                    return Err(format!(
                        "terrain probe arm failed with veyr status {arm_result}"
                    ));
                }
                println!("terrain probe armed; waiting for one in-world render-thread ray");

                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    let status = required_diagnostic(
                        &process,
                        remote_command,
                        RemoteCommand::TerrainProbeStatus,
                    )?;
                    match status {
                        0 | 1 if Instant::now() < deadline => sleep(Duration::from_millis(10)),
                        0 | 1 => {
                            println!("terrain probe is still armed: enter the world so the player circle has a finite centre");
                            return Ok(());
                        }
                        2 => {
                            let native = required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeNativeResult,
                            )?;
                            println!("terrain probe: no terrain hit (client result {native})");
                            return Ok(());
                        }
                        3 => {
                            let x = f32::from_bits(required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeHitX,
                            )?);
                            let y = f32::from_bits(required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeHitY,
                            )?);
                            let z = f32::from_bits(required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeHitZ,
                            )?);
                            let native = required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeNativeResult,
                            )?;
                            println!(
                                "terrain probe: hit at ({x:.3}, {y:.3}, {z:.3}); client result {native}"
                            );
                            return Ok(());
                        }
                        0xE301 => {
                            return Err("terrain probe: CGWorldFrame is unavailable".to_owned())
                        }
                        0xE302 => return Err("terrain probe: generated an invalid ray".to_owned()),
                        0xE303 => {
                            let native = required_diagnostic(
                                &process,
                                remote_command,
                                RemoteCommand::TerrainProbeNativeResult,
                            )?;
                            return Err(format!(
                                "terrain probe: unexpected client result {native}"
                            ));
                        }
                        0xE304 => {
                            return Err(
                                "terrain probe: client returned an invalid hit record".to_owned()
                            )
                        }
                        other => {
                            return Err(format!("terrain probe: unknown status 0x{other:08X}"))
                        }
                    }
                }
            }
            LoaderMode::Stop => {
                let result = call_remote(&process, remote_command, RemoteCommand::Stop as u32)?;
                println!("injected module at 0x{remote_module:08X}: stop result {result}");
                if result == 0 {
                    println!("hooks restored; this DLL remains loaded but is inactive");
                    Ok(())
                } else {
                    Err(format!("runtime stop failed with veyr status {result}"))
                }
            }
        }
    }

    /// Starts WoW under a short-lived debugger gate, arms D3D9 capture at the
    /// initial user-mode breakpoint, then lets exactly one injected worker
    /// configure and start the runtime after capture.
    ///
    /// `CREATE_SUSPENDED` alone is not a usable early-injection gate: after
    /// its first `ResumeThread` the primary thread is no longer suspended, so
    /// a later "resume" is a no-op. The debugger's initial breakpoint occurs
    /// after kernel32 is ready for `LoadLibraryW` but before `Wow.exe` has run
    /// application code, closing the race with `Direct3DCreate9` entirely.
    fn run_launch(arguments: LaunchArguments) -> Result<(), String> {
        let mut launched = StartupGate::start(&arguments.executable_path)?;
        println!(
            "started startup-gated game process: PID {} ({})",
            launched.process_id,
            arguments.executable_path.display()
        );

        let process = Process::open(launched.process_id)?;
        process.ensure_x86_target()?;
        let local_module = LocalModule::load(&arguments.dll_path)?;
        // First leave the debugger while preserving the primary thread's
        // explicit suspend count. Remote `LoadLibraryW` threads created below
        // must not themselves generate debug events that would otherwise
        // remain paused behind the initial breakpoint.
        launched.prepare_injection()?;
        println!("Windows initial loader breakpoint is ready; injecting veyr.dll");
        let remote_module = inject(&process, &arguments.dll_path)?;

        let remote_command_rva = local_module.export_rva(b"veyr_remote_command\0")?;
        let remote_command = remote_module
            .checked_add(remote_command_rva)
            .ok_or_else(|| "remote command address overflowed x86 address space".to_owned())?;
        let arm_result = call_remote(
            &process,
            remote_command,
            RemoteCommand::ArmD3d9Capture as u32,
        )?;
        println!("D3D9 creation capture arm result: {arm_result}");
        if arm_result != 0 {
            print_d3d9_capture_error(arm_result);
            return Err(format!(
                "veyr.dll could not arm D3D9 creation capture (status {arm_result})"
            ));
        }

        let worker_rva = local_module.export_rva(b"veyr_remote_launch_player_circle\0")?;
        let worker_address = remote_module.checked_add(worker_rva).ok_or_else(|| {
            "remote launch worker address overflowed x86 address space".to_owned()
        })?;
        let request = [RemoteLaunchRequest::player_circle(
            CAPTURE_TIMEOUT.as_millis() as u32,
        )];
        let remote_request = RemoteAllocation::write_slice(&process, &request)?;
        let worker = start_remote_thread(&process, worker_address, remote_request.address)
            .map_err(|error| error.describe("D3D9 launch worker"))?;

        println!("D3D9 launch worker is ready; releasing WoW startup gate");
        launched.release()?;
        let worker_result = wait_remote_result_for(
            worker,
            duration_to_wait_millis(CAPTURE_TIMEOUT + LAUNCH_WORKER_GRACE),
        )
        .map_err(|error| error.describe("D3D9 launch worker"))?;
        let report = remote_request.read_value::<RemoteLaunchRequest>("D3D9 launch report")?;
        print_launch_report(&report.report);
        validate_launch_worker_result(worker_result, &report.report)?;

        println!("injected module loaded at 0x{remote_module:08X}");
        println!("player circle result: {}", report.report.runtime_result);
        println!("success: enter the world and look for the wide cyan radius-20 circle around your player");
        Ok(())
    }

    fn print_launch_report(report: &RemoteLaunchReport) {
        println!(
            "D3D9 capture: state {}, factory calls {}, CreateDevice calls {}",
            report.capture_state, report.factory_calls, report.create_device_calls
        );
        if report.device != 0 {
            println!("captured IDirect3DDevice9: 0x{:08X}", report.device);
            println!("captured D3D9 EndScene:    0x{:08X}", report.end_scene);
            println!("captured D3D9 Reset:       0x{:08X}", report.reset);
        }
    }

    fn validate_launch_worker_result(
        worker_result: u32,
        report: &RemoteLaunchReport,
    ) -> Result<(), String> {
        if report.abi_version != REMOTE_LAUNCH_ABI_VERSION {
            return Err(format!(
                "veyr.dll returned incompatible launch-report ABI {}",
                report.abi_version
            ));
        }
        if worker_result != report.outcome {
            return Err(format!(
                "D3D9 launch worker returned {worker_result}, but its report says {}",
                report.outcome
            ));
        }
        match report.outcome {
            value if value == RemoteLaunchOutcome::Started as u32 => Ok(()),
            value if value == RemoteLaunchOutcome::CaptureFailed as u32 => {
                print_d3d9_capture_error(report.capture_error);
                Err("D3D9 creation capture failed inside veyr.dll".to_owned())
            }
            value if value == RemoteLaunchOutcome::CaptureTimedOut as u32 => Err(format!(
                "timed out waiting for D3D9 creation capture (state {}, Direct3DCreate9 calls {}, CreateDevice calls {})",
                report.capture_state, report.factory_calls, report.create_device_calls
            )),
            value if value == RemoteLaunchOutcome::ConfigurationFailed as u32 => Err(format!(
                "veyr.dll rejected the captured D3D9 configuration with status {}",
                report.configuration_result
            )),
            value if value == RemoteLaunchOutcome::RuntimeStartFailed as u32 => {
                print_hook_error(report.hook_error);
                Err(format!(
                    "player circle startup failed with veyr status {}",
                    report.runtime_result
                ))
            }
            value if value == RemoteLaunchOutcome::CaptureCleanupFailed as u32 => {
                print_hook_error(report.capture_error);
                Err("veyr.dll could not restore the temporary D3D9 capture hooks".to_owned())
            }
            value if value == RemoteLaunchOutcome::InvalidRequest as u32 => {
                Err("veyr.dll rejected the private D3D9 launch request".to_owned())
            }
            value if value == RemoteLaunchOutcome::Panicked as u32 => {
                Err("veyr.dll contained a panic in the D3D9 launch worker".to_owned())
            }
            value if value == RemoteLaunchOutcome::Pending as u32 => {
                Err("D3D9 launch worker exited without a final report".to_owned())
            }
            value => Err(format!("veyr.dll reported unknown launch-worker outcome {value}")),
        }
    }

    fn print_d3d9_capture_error(code: u32) {
        let description = match code {
            0 => "none",
            100 => "capture was already armed",
            101 => "d3d9.dll could not be loaded",
            102 => "d3d9!Direct3DCreate9 export is unavailable",
            103 => "captured IDirect3D9 factory has no vtable",
            104 => "IDirect3D9::CreateDevice target is null",
            105 => "captured IDirect3DDevice9 has no vtable",
            106 => "captured device has invalid EndScene/Reset targets",
            1..=13 => "inline hook installation failed (see hook-error code)",
            _ => "unknown D3D9 capture error",
        };
        println!("D3D9 capture error: {code} ({description})");
        if (1..=13).contains(&code) {
            print_hook_error(code);
        }
    }

    #[derive(Debug, Copy, Clone)]
    struct LiveD3d9 {
        owner: RemoteAddress,
        device_slot: RemoteAddress,
        device: RemoteAddress,
        vtable: RemoteAddress,
        present: RemoteAddress,
        targets: hooks::Direct3d9Targets,
    }

    impl LiveD3d9 {
        fn targets(self) -> hooks::Direct3d9Targets {
            self.targets
        }
    }

    #[derive(Debug, Copy, Clone)]
    struct LiveOpenGl {
        wgl_swap_buffers: RemoteFunction,
        gdi_swap_buffers: Option<RemoteFunction>,
    }

    #[derive(Debug, Copy, Clone)]
    enum LiveGraphics {
        D3d9(LiveD3d9),
        OpenGl(LiveOpenGl),
    }

    impl LiveGraphics {
        fn configuration(self) -> RemoteGraphicsConfiguration {
            match self {
                Self::D3d9(live) => RemoteGraphicsConfiguration {
                    abi_version: GRAPHICS_CONFIGURATION_ABI_VERSION,
                    backend: GRAPHICS_BACKEND_D3D9,
                    frame_target: live.targets().end_scene,
                    auxiliary_target: live.targets().reset,
                    d3d9_device: live.device,
                },
                Self::OpenGl(live) => RemoteGraphicsConfiguration {
                    abi_version: GRAPHICS_CONFIGURATION_ABI_VERSION,
                    backend: GRAPHICS_BACKEND_OPENGL,
                    frame_target: live.wgl_swap_buffers.address,
                    auxiliary_target: live
                        .gdi_swap_buffers
                        .map_or(0, |swap_buffers| swap_buffers.address),
                    d3d9_device: 0,
                },
            }
        }
    }

    #[derive(Debug, Copy, Clone)]
    struct RemoteModule {
        base: RemoteAddress,
        size: u32,
    }

    #[derive(Debug, Copy, Clone)]
    struct RemoteFunction {
        module: RemoteModule,
        rva: u32,
        address: RemoteAddress,
        prologue: [u8; 8],
    }

    impl RemoteModule {
        fn contains_range(self, address: RemoteAddress, size: u32) -> bool {
            let Some(module_end) = self.base.checked_add(self.size) else {
                return false;
            };
            let Some(range_end) = address.checked_add(size) else {
                return false;
            };
            address >= self.base && range_end <= module_end
        }

        fn address_from_rva(
            self,
            rva: u32,
            size: u32,
            context: &str,
        ) -> Result<RemoteAddress, String> {
            let address = self
                .base
                .checked_add(rva)
                .ok_or_else(|| format!("{context} RVA overflowed x86 space"))?;
            if self.contains_range(address, size) {
                Ok(address)
            } else {
                Err(format!(
                    "{context} RVA 0x{rva:X} is outside module at 0x{:08X}, size 0x{:X}",
                    self.base, self.size
                ))
            }
        }
    }

    #[derive(Debug, Copy, Clone)]
    struct GraphicsModules {
        d3d9: Option<RemoteModule>,
        opengl32: Option<RemoteModule>,
    }

    fn discover_live_graphics(process: &Process) -> Result<LiveGraphics, String> {
        let graphics_modules = GraphicsModules {
            d3d9: remote_module_optional(process, "d3d9.dll")?,
            opengl32: remote_module_optional(process, "opengl32.dll")?,
        };
        print_graphics_modules(graphics_modules);

        let d3d9_result = graphics_modules
            .d3d9
            .ok_or_else(|| "d3d9.dll is not loaded".to_owned())
            .and_then(|module| discover_live_d3d9(process, module));
        match d3d9_result {
            Ok(live) => {
                println!("selected graphics backend: D3D9 (active device chain is valid)");
                print_live_d3d9(&live);
                return Ok(LiveGraphics::D3d9(live));
            }
            Err(error) => println!("D3D9 probe unavailable: {error}"),
        }

        let opengl32 = graphics_modules.opengl32.ok_or_else(|| {
            "D3D9 discovery failed and the target process does not have opengl32.dll loaded"
                .to_owned()
        })?;
        let live = discover_live_opengl(process, opengl32)?;
        println!("selected graphics backend: OpenGL (D3D9 active device is unavailable)");
        print_live_opengl(&live);
        Ok(LiveGraphics::OpenGl(live))
    }

    fn discover_live_d3d9(
        process: &Process,
        d3d9_module: RemoteModule,
    ) -> Result<LiveD3d9, String> {
        let owner = read_nonzero_u32(process, loader_discovery::GX_DEVICE, "GxDevice owner")?;
        let device_slot = owner
            .checked_add(loader_discovery::DEVICE)
            .ok_or_else(|| "GxDevice + device offset overflowed x86 space".to_owned())?;
        let device = read_remote_u32(process, device_slot, "active graphics device")?;
        if device == 0 {
            return Err(format!(
                "active IDirect3DDevice9 is null at 0x{device_slot:08X}"
            ));
        }
        let vtable = read_nonzero_u32(process, device, "IDirect3DDevice9 vtable")?;
        let reset = read_vtable_entry(
            process,
            vtable,
            loader_discovery::RESET_INDEX,
            "IDirect3DDevice9::Reset",
        )?;
        let present = read_vtable_entry(
            process,
            vtable,
            loader_discovery::PRESENT_INDEX,
            "IDirect3DDevice9::Present",
        )?;
        let end_scene = read_vtable_entry(
            process,
            vtable,
            loader_discovery::END_SCENE_INDEX,
            "IDirect3DDevice9::EndScene",
        )?;
        validate_module_target(d3d9_module, reset, "IDirect3DDevice9::Reset")?;
        validate_module_target(d3d9_module, present, "IDirect3DDevice9::Present")?;
        validate_module_target(d3d9_module, end_scene, "IDirect3DDevice9::EndScene")?;
        let _ = read_remote_bytes::<1>(process, reset, "IDirect3DDevice9::Reset target")?;
        let _ = read_remote_bytes::<1>(process, present, "IDirect3DDevice9::Present target")?;
        let _ = read_remote_bytes::<1>(process, end_scene, "IDirect3DDevice9::EndScene target")?;

        Ok(LiveD3d9 {
            owner,
            device_slot,
            device,
            vtable,
            present,
            targets: hooks::Direct3d9Targets { end_scene, reset },
        })
    }

    fn validate_module_target(
        module: RemoteModule,
        target: RemoteAddress,
        name: &str,
    ) -> Result<(), String> {
        let module_end = module
            .base
            .checked_add(module.size)
            .ok_or_else(|| "remote d3d9.dll image range overflowed x86 space".to_owned())?;
        if module.contains_range(target, 1) {
            Ok(())
        } else {
            Err(format!(
                "{name} target 0x{target:08X} is outside d3d9.dll at 0x{:08X}..0x{:08X}",
                module.base, module_end
            ))
        }
    }

    fn discover_live_opengl(
        process: &Process,
        opengl32: RemoteModule,
    ) -> Result<LiveOpenGl, String> {
        let wgl_swap_buffers = resolve_remote_export(process, opengl32, "wglSwapBuffers", 0)?;
        let gdi_swap_buffers = match remote_module_optional(process, "gdi32.dll")? {
            Some(gdi32) => match resolve_remote_export(process, gdi32, "SwapBuffers", 0) {
                Ok(function) => Some(function),
                Err(error) => {
                    println!("gdi32!SwapBuffers probe unavailable: {error}");
                    None
                }
            },
            None => {
                println!("gdi32!SwapBuffers probe unavailable: gdi32.dll is not loaded");
                None
            }
        };

        Ok(LiveOpenGl {
            wgl_swap_buffers,
            gdi_swap_buffers,
        })
    }

    /// Resolves a named PE export from the target process itself.
    ///
    /// This intentionally does not derive an RVA from the loader's copy of a
    /// system DLL: side-by-side DLL versions can expose different RVAs.
    fn resolve_remote_export(
        process: &Process,
        module: RemoteModule,
        export_name: &str,
        forward_depth: u8,
    ) -> Result<RemoteFunction, String> {
        const PE_SIGNATURE: u32 = 0x0000_4550;
        const PE32_MAGIC: u16 = 0x010B;
        const DOS_NT_OFFSET: u32 = 0x3C;
        const FILE_HEADER_SIZE: u32 = 20;
        const FILE_HEADER_SIZE_OF_OPTIONAL_HEADER: u32 = 16;
        const PE_SIGNATURE_SIZE: u32 = 4;
        const OPTIONAL_HEADER_MAGIC_OFFSET: u32 = 0;
        const PE32_EXPORT_DIRECTORY_OFFSET: u32 = 96;
        const EXPORT_DIRECTORY_SIZE: u32 = 40;
        const EXPORT_NUMBER_OF_FUNCTIONS: u32 = 20;
        const EXPORT_NUMBER_OF_NAMES: u32 = 24;
        const EXPORT_ADDRESS_OF_FUNCTIONS: u32 = 28;
        const EXPORT_ADDRESS_OF_NAMES: u32 = 32;
        const EXPORT_ADDRESS_OF_ORDINALS: u32 = 36;
        const MAX_EXPORT_NAMES: u32 = 65_536;

        if forward_depth >= 4 {
            return Err(format!(
                "export forwarding depth exceeded while resolving {export_name}"
            ));
        }

        let dos_offset = module.address_from_rva(DOS_NT_OFFSET, 4, "DOS e_lfanew")?;
        let nt_rva = read_remote_u32(process, dos_offset, "DOS e_lfanew")?;
        let nt_header = module.address_from_rva(nt_rva, PE_SIGNATURE_SIZE, "PE header")?;
        if read_remote_u32(process, nt_header, "PE signature")? != PE_SIGNATURE {
            return Err(format!(
                "module at 0x{:08X} does not contain a PE signature",
                module.base
            ));
        }

        let optional_rva = nt_rva
            .checked_add(PE_SIGNATURE_SIZE)
            .and_then(|value| value.checked_add(FILE_HEADER_SIZE))
            .ok_or_else(|| "PE optional-header RVA overflowed x86 space".to_owned())?;
        let optional_header = module.address_from_rva(
            optional_rva,
            PE32_EXPORT_DIRECTORY_OFFSET + 8,
            "PE optional header",
        )?;
        let size_of_optional_header = read_remote_u16(
            process,
            nt_header
                .checked_add(PE_SIGNATURE_SIZE + FILE_HEADER_SIZE_OF_OPTIONAL_HEADER)
                .ok_or_else(|| "PE optional-header-size address overflowed".to_owned())?,
            "SizeOfOptionalHeader",
        )?;
        if u32::from(size_of_optional_header) < PE32_EXPORT_DIRECTORY_OFFSET + 8 {
            return Err("PE optional header has no export-directory entry".to_owned());
        }
        if read_remote_u16(
            process,
            optional_header
                .checked_add(OPTIONAL_HEADER_MAGIC_OFFSET)
                .ok_or_else(|| "PE optional-header magic address overflowed".to_owned())?,
            "PE optional-header magic",
        )? != PE32_MAGIC
        {
            return Err("target graphics module is not a PE32 image".to_owned());
        }

        let export_rva = read_remote_u32(
            process,
            optional_header
                .checked_add(PE32_EXPORT_DIRECTORY_OFFSET)
                .ok_or_else(|| "PE export-directory RVA address overflowed".to_owned())?,
            "export directory RVA",
        )?;
        let export_size = read_remote_u32(
            process,
            optional_header
                .checked_add(PE32_EXPORT_DIRECTORY_OFFSET + 4)
                .ok_or_else(|| "PE export-directory size address overflowed".to_owned())?,
            "export directory size",
        )?;
        if export_rva == 0 || export_size < EXPORT_DIRECTORY_SIZE {
            return Err("target graphics module has no usable export directory".to_owned());
        }
        let export_directory =
            module.address_from_rva(export_rva, EXPORT_DIRECTORY_SIZE, "export directory")?;
        let export_end = export_rva
            .checked_add(export_size)
            .ok_or_else(|| "export-directory range overflowed x86 space".to_owned())?;

        let function_count = read_remote_u32(
            process,
            export_directory
                .checked_add(EXPORT_NUMBER_OF_FUNCTIONS)
                .ok_or_else(|| "NumberOfFunctions address overflowed".to_owned())?,
            "NumberOfFunctions",
        )?;
        let name_count = read_remote_u32(
            process,
            export_directory
                .checked_add(EXPORT_NUMBER_OF_NAMES)
                .ok_or_else(|| "NumberOfNames address overflowed".to_owned())?,
            "NumberOfNames",
        )?;
        if function_count == 0 || name_count == 0 || name_count > MAX_EXPORT_NAMES {
            return Err("target graphics module has an invalid export-name table".to_owned());
        }

        let functions_rva = read_remote_u32(
            process,
            export_directory
                .checked_add(EXPORT_ADDRESS_OF_FUNCTIONS)
                .ok_or_else(|| "AddressOfFunctions address overflowed".to_owned())?,
            "AddressOfFunctions",
        )?;
        let names_rva = read_remote_u32(
            process,
            export_directory
                .checked_add(EXPORT_ADDRESS_OF_NAMES)
                .ok_or_else(|| "AddressOfNames address overflowed".to_owned())?,
            "AddressOfNames",
        )?;
        let ordinals_rva = read_remote_u32(
            process,
            export_directory
                .checked_add(EXPORT_ADDRESS_OF_ORDINALS)
                .ok_or_else(|| "AddressOfNameOrdinals address overflowed".to_owned())?,
            "AddressOfNameOrdinals",
        )?;

        let functions = module.address_from_rva(
            functions_rva,
            function_count
                .checked_mul(4)
                .ok_or_else(|| "function-table size overflowed".to_owned())?,
            "function table",
        )?;
        let names = module.address_from_rva(
            names_rva,
            name_count
                .checked_mul(4)
                .ok_or_else(|| "name-table size overflowed".to_owned())?,
            "name table",
        )?;
        let ordinals = module.address_from_rva(
            ordinals_rva,
            name_count
                .checked_mul(2)
                .ok_or_else(|| "ordinal-table size overflowed".to_owned())?,
            "ordinal table",
        )?;

        for index in 0..name_count {
            let name_slot = names
                .checked_add(
                    index
                        .checked_mul(4)
                        .ok_or_else(|| "name-table index overflowed x86 space".to_owned())?,
                )
                .ok_or_else(|| "name-table slot overflowed x86 space".to_owned())?;
            let name_rva = read_remote_u32(process, name_slot, "export name RVA")?;
            if read_remote_ascii(process, module, name_rva, "export name")? != export_name {
                continue;
            }

            let ordinal_slot = ordinals
                .checked_add(
                    index
                        .checked_mul(2)
                        .ok_or_else(|| "ordinal-table index overflowed x86 space".to_owned())?,
                )
                .ok_or_else(|| "ordinal-table slot overflowed x86 space".to_owned())?;
            let ordinal = u32::from(read_remote_u16(process, ordinal_slot, "export ordinal")?);
            if ordinal >= function_count {
                return Err(format!("{export_name} has an invalid export ordinal"));
            }
            let function_slot = functions
                .checked_add(
                    ordinal
                        .checked_mul(4)
                        .ok_or_else(|| "function-table index overflowed x86 space".to_owned())?,
                )
                .ok_or_else(|| "function-table slot overflowed x86 space".to_owned())?;
            let function_rva = read_remote_u32(process, function_slot, "export function RVA")?;
            if function_rva >= export_rva && function_rva < export_end {
                let forwarder =
                    read_remote_ascii(process, module, function_rva, "export forwarder")?;
                let (forward_module, forwarded_name) =
                    forwarder.rsplit_once('.').ok_or_else(|| {
                        format!(
                            "{export_name} has an invalid forwarded-export string {forwarder:?}"
                        )
                    })?;
                if forwarded_name.starts_with('#') {
                    return Err(format!(
                        "{export_name} forwards by ordinal, which this DEV loader does not support"
                    ));
                }
                let forward_module = if forward_module.contains('.') {
                    forward_module.to_owned()
                } else {
                    format!("{forward_module}.dll")
                };
                let module = remote_module(process, &forward_module)?;
                return resolve_remote_export(process, module, forwarded_name, forward_depth + 1);
            }

            let address = module.address_from_rva(function_rva, 8, export_name)?;
            let prologue = read_remote_bytes::<8>(process, address, export_name)?;
            return Ok(RemoteFunction {
                module,
                rva: function_rva,
                address,
                prologue,
            });
        }

        Err(format!(
            "module at 0x{:08X} does not export {export_name}",
            module.base
        ))
    }

    fn read_remote_ascii(
        process: &Process,
        module: RemoteModule,
        rva: u32,
        context: &str,
    ) -> Result<String, String> {
        const MAX_EXPORT_STRING: usize = 256;

        let address = module.address_from_rva(rva, 1, context)?;
        let module_end = module
            .base
            .checked_add(module.size)
            .ok_or_else(|| format!("{context} module range overflowed x86 space"))?;
        let remaining = usize::try_from(module_end - address)
            .map_err(|_| format!("{context} size does not fit the host"))?;
        let mut bytes = vec![0_u8; remaining.min(MAX_EXPORT_STRING)];
        read_remote_into(process, address, &mut bytes, context)?;
        let length = bytes.iter().position(|byte| *byte == 0).ok_or_else(|| {
            format!("{context} exceeds {MAX_EXPORT_STRING} bytes or has no terminator")
        })?;
        String::from_utf8(bytes[..length].to_vec())
            .map_err(|_| format!("{context} is not ASCII/UTF-8"))
    }

    fn print_graphics_modules(modules: GraphicsModules) {
        print_optional_module("d3d9.dll", modules.d3d9);
        print_optional_module("opengl32.dll", modules.opengl32);
    }

    fn print_optional_module(name: &str, module: Option<RemoteModule>) {
        match module {
            Some(module) => println!(
                "renderer module {name}: loaded at 0x{:08X}, size 0x{:X}",
                module.base, module.size
            ),
            None => println!("renderer module {name}: not loaded"),
        }
    }

    fn print_live_d3d9(live: &LiveD3d9) {
        println!("GxDevice owner:        0x{:08X}", live.owner);
        println!("D3D9 device slot:      0x{:08X}", live.device_slot);
        println!("IDirect3DDevice9:      0x{:08X}", live.device);
        println!("D3D9 vtable:           0x{:08X}", live.vtable);
        println!("D3D9 Reset target:     0x{:08X}", live.targets.reset);
        println!("D3D9 Present target:   0x{:08X}", live.present);
        println!("D3D9 EndScene target:  0x{:08X}", live.targets.end_scene);
    }

    fn print_live_opengl(live: &LiveOpenGl) {
        print_remote_function("wglSwapBuffers", live.wgl_swap_buffers);
        match live.gdi_swap_buffers {
            Some(function) => print_remote_function("gdi32!SwapBuffers", function),
            None => println!("gdi32!SwapBuffers: unavailable; using wglSwapBuffers only"),
        }
    }

    fn print_remote_function(name: &str, function: RemoteFunction) {
        println!(
            "{name} module:      0x{:08X}, size 0x{:X}",
            function.module.base, function.module.size
        );
        println!("{name} RVA:         0x{:08X}", function.rva);
        println!("{name} target:      0x{:08X}", function.address);
        print!("{name} prologue:");
        for byte in function.prologue {
            print!(" {byte:02X}");
        }
        println!();
    }

    fn read_vtable_entry(
        process: &Process,
        vtable: RemoteAddress,
        index: u32,
        context: &str,
    ) -> Result<RemoteAddress, String> {
        let offset = index
            .checked_mul(loader_discovery::POINTER_SIZE)
            .ok_or_else(|| format!("{context} vtable offset overflowed"))?;
        let slot = vtable
            .checked_add(offset)
            .ok_or_else(|| format!("{context} vtable slot overflowed x86 space"))?;
        read_nonzero_u32(process, slot, context)
    }

    fn read_nonzero_u32(
        process: &Process,
        address: RemoteAddress,
        context: &str,
    ) -> Result<RemoteAddress, String> {
        let value = read_remote_u32(process, address, context)?;
        if value == 0 {
            Err(format!("{context} is null at 0x{address:08X}"))
        } else {
            Ok(value)
        }
    }

    fn read_remote_u32(
        process: &Process,
        address: RemoteAddress,
        context: &str,
    ) -> Result<RemoteAddress, String> {
        Ok(u32::from_le_bytes(read_remote_bytes::<4>(
            process, address, context,
        )?))
    }

    fn read_remote_u64(process: &Process, address: u32, context: &str) -> Result<u64, String> {
        Ok(u64::from_le_bytes(read_remote_bytes::<8>(
            process, address, context,
        )?))
    }

    fn read_remote_f32(process: &Process, address: u32, context: &str) -> Result<f32, String> {
        Ok(f32::from_le_bytes(read_remote_bytes::<4>(
            process, address, context,
        )?))
    }

    fn read_remote_u16(
        process: &Process,
        address: RemoteAddress,
        context: &str,
    ) -> Result<u16, String> {
        Ok(u16::from_le_bytes(read_remote_bytes::<2>(
            process, address, context,
        )?))
    }

    fn ensure_not_already_patched(
        process: &Process,
        name: &str,
        target: RemoteAddress,
    ) -> Result<(), String> {
        let first = read_remote_bytes::<1>(process, target, name)?[0];
        if first == 0xE9 {
            Err(format!(
                "live {name} at 0x{target:08X} already starts with E9; stop the previously injected runtime first"
            ))
        } else {
            Ok(())
        }
    }

    fn ensure_graphics_not_already_patched(
        process: &Process,
        live: &LiveGraphics,
    ) -> Result<(), String> {
        match live {
            LiveGraphics::D3d9(live) => {
                let targets = live.targets();
                ensure_not_already_patched(process, "EndScene", targets.end_scene)?;
                ensure_not_already_patched(process, "Reset", targets.reset)
            }
            LiveGraphics::OpenGl(live) => {
                ensure_not_already_patched(
                    process,
                    "wglSwapBuffers",
                    live.wgl_swap_buffers.address,
                )?;
                if let Some(gdi_swap_buffers) = live.gdi_swap_buffers {
                    ensure_not_already_patched(
                        process,
                        "gdi32!SwapBuffers",
                        gdi_swap_buffers.address,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn print_extended_diagnostics(process: &Process, remote_command: u32) -> Result<u32, String> {
        let Some(callback_panics) =
            query_optional_command(process, remote_command, RemoteCommand::CallbackPanicCount)?
        else {
            println!("extended renderer diagnostics are unavailable in this older DLL");
            return Ok(0);
        };
        let submitted = required_diagnostic(
            process,
            remote_command,
            RemoteCommand::RenderSubmittedCommands,
        )?;
        let drawn =
            required_diagnostic(process, remote_command, RemoteCommand::RenderDrawnCommands)?;
        let skipped = required_diagnostic(
            process,
            remote_command,
            RemoteCommand::RenderSkippedCommands,
        )?;
        let draw_failures =
            required_diagnostic(process, remote_command, RemoteCommand::RenderDrawFailures)?;
        let state_setup_failed = required_diagnostic(
            process,
            remote_command,
            RemoteCommand::RenderStateSetupFailed,
        )?;
        let last_result =
            required_diagnostic(process, remote_command, RemoteCommand::RenderLastError)?;

        println!("callback panics: {callback_panics}");
        println!(
            "renderer: submitted={submitted}, drawn={drawn}, skipped={skipped}, draw_failures={draw_failures}, state_setup_failed={state_setup_failed}, last_result=0x{last_result:08X} ({})",
            describe_renderer_result(last_result),
        );
        Ok(last_result)
    }

    /// Reads the camera chain verified against the exact supported executable.
    ///
    /// This is diagnostics only: it does not make an unverified projection
    /// claim and never writes to the game process.
    fn print_build_12340_world_frame_probe(process: &Process) {
        let world_frame =
            match read_remote_u32(process, camera::CURRENT_WORLD_FRAME, "current CGWorldFrame") {
                Ok(value) if value != 0 => value,
                Ok(_) => {
                    println!(
                        "build-12340 camera: unavailable (CGWorldFrame is null at 0x{:08X})",
                        camera::CURRENT_WORLD_FRAME,
                    );
                    return;
                }
                Err(error) => {
                    println!("build-12340 camera: unavailable ({error})");
                    return;
                }
            };
        let Some(camera_slot) = world_frame.checked_add(camera::CAMERA_OFFSET) else {
            println!("build-12340 camera: camera-slot address overflowed");
            return;
        };
        let camera_address = match read_remote_u32(process, camera_slot, "CGWorldFrame camera") {
            Ok(value) if value != 0 => value,
            Ok(_) => {
                println!(
                    "build-12340 camera: frame=0x{world_frame:08X}; camera is null at 0x{camera_slot:08X}"
                );
                return;
            }
            Err(error) => {
                println!("build-12340 camera: frame=0x{world_frame:08X}; {error}");
                return;
            }
        };
        let read = |offset, label| {
            address_with_offset(camera_address, offset, label)
                .and_then(|address| read_remote_f32(process, address, label))
        };
        let values = (
            read(camera::EYE_POSITION_OFFSET, "camera eye X"),
            read(camera::EYE_POSITION_OFFSET + 4, "camera eye Y"),
            read(camera::EYE_POSITION_OFFSET + 8, "camera eye Z"),
            read(camera::FORWARD_BASIS_OFFSET, "camera forward X"),
            read(camera::FORWARD_BASIS_OFFSET + 4, "camera forward Y"),
            read(camera::FORWARD_BASIS_OFFSET + 8, "camera forward Z"),
            read(camera::LEFT_BASIS_OFFSET, "camera left X"),
            read(camera::LEFT_BASIS_OFFSET + 4, "camera left Y"),
            read(camera::LEFT_BASIS_OFFSET + 8, "camera left Z"),
            read(camera::YAW_OFFSET, "camera yaw"),
            read(camera::PITCH_OFFSET, "camera pitch"),
            read(camera::FOV_OFFSET, "camera FOV"),
        );
        match values {
            (
                Ok(x),
                Ok(y),
                Ok(z),
                Ok(ax),
                Ok(ay),
                Ok(az),
                Ok(bx),
                Ok(by),
                Ok(bz),
                Ok(yaw),
                Ok(pitch),
                Ok(fov),
            ) if [x, y, z, ax, ay, az, bx, by, bz, yaw, pitch, fov]
                .into_iter()
                .all(f32::is_finite) =>
            {
                println!(
                    "build-12340 camera: frame=0x{world_frame:08X}; camera=0x{camera_address:08X}; eye=({x:.3}, {y:.3}, {z:.3}); forward=({ax:.6}, {ay:.6}, {az:.6}); left=({bx:.6}, {by:.6}, {bz:.6}); yaw={yaw:.6}; pitch={pitch:.6}; fov={fov:.6}"
                );
            }
            _ => println!(
                "build-12340 camera: frame=0x{world_frame:08X}; camera=0x{camera_address:08X}; snapshot is unavailable or non-finite"
            ),
        }
    }

    fn describe_renderer_result(result: u32) -> &'static str {
        match result {
            0 => "no renderer error",
            0xE104 => "world circle: D3D9 viewport unavailable",
            0xE105 => "world circle: requested mode is not implemented yet",
            0xE201 => "world circle: invalid input or viewport",
            0xE202 => "world circle: all sampled segments are outside the camera view",
            0xE207 => "world circle: live CGCamera snapshot is unavailable or invalid",
            _ => "native renderer result (HRESULT or backend-specific code)",
        }
    }

    /// Prints the exact inputs consumed by the static world-circle projection.
    ///
    /// This is loader-only developer diagnostics.  It never writes to the game
    /// and makes a client-build offset mismatch visible in one status report.
    fn print_world_projection_probe(process: &Process) {
        match WorldProjectionProbe::read(process) {
            Ok(probe) => probe.print(),
            Err(error) => println!("world projection probe unavailable: {error}"),
        }
    }

    struct WorldProjectionProbe {
        world_frame: RemoteAddress,
        camera: RemoteAddress,
        eye: [f32; 3],
        roll: f32,
        yaw: f32,
        pitch: f32,
        field_of_view: f32,
        player: RemoteAddress,
        player_position: [f32; 3],
    }

    impl WorldProjectionProbe {
        fn read(process: &Process) -> Result<Self, String> {
            let world_frame =
                read_nonzero_u32(process, camera::CURRENT_WORLD_FRAME, "current world frame")?;
            let camera_slot = address_with_offset(
                world_frame,
                camera::CAMERA_OFFSET,
                "world-frame camera slot",
            )?;
            let camera = read_nonzero_u32(process, camera_slot, "world-frame camera")?;
            let eye = [
                read_remote_f32(
                    process,
                    address_with_offset(camera, camera::EYE_POSITION_OFFSET, "camera eye X")?,
                    "camera eye X",
                )?,
                read_remote_f32(
                    process,
                    address_with_offset(camera, camera::EYE_POSITION_OFFSET + 4, "camera eye Y")?,
                    "camera eye Y",
                )?,
                read_remote_f32(
                    process,
                    address_with_offset(camera, camera::EYE_POSITION_OFFSET + 8, "camera eye Z")?,
                    "camera eye Z",
                )?,
            ];
            let (player, player_position) = read_local_player_position(process)?;
            Ok(Self {
                world_frame,
                camera,
                eye,
                roll: read_remote_f32(
                    process,
                    address_with_offset(camera, camera::ROLL_OFFSET, "camera roll")?,
                    "camera roll",
                )?,
                yaw: read_remote_f32(
                    process,
                    address_with_offset(camera, camera::YAW_OFFSET, "camera yaw")?,
                    "camera yaw",
                )?,
                pitch: read_remote_f32(
                    process,
                    address_with_offset(camera, camera::PITCH_OFFSET, "camera pitch")?,
                    "camera pitch",
                )?,
                field_of_view: read_remote_f32(
                    process,
                    address_with_offset(camera, camera::FOV_OFFSET, "camera field of view")?,
                    "camera field of view",
                )?,
                player,
                player_position,
            })
        }

        fn print(&self) {
            println!("world projection inputs (read-only):");
            println!(
                "world frame: 0x{:08X}; camera: 0x{:08X}",
                self.world_frame, self.camera
            );
            println!(
                "camera eye: ({:.6}, {:.6}, {:.6}); roll={:.6}; yaw={:.6}; pitch={:.6}; fov={:.6}",
                self.eye[0],
                self.eye[1],
                self.eye[2],
                self.roll,
                self.yaw,
                self.pitch,
                self.field_of_view,
            );
            println!(
                "local player: 0x{:08X}; position: ({:.6}, {:.6}, {:.6})",
                self.player,
                self.player_position[0],
                self.player_position[1],
                self.player_position[2],
            );
        }
    }

    fn read_local_player_position(process: &Process) -> Result<(RemoteAddress, [f32; 3]), String> {
        const OBJECT_LIMIT: usize = 10_000;

        let connection = read_nonzero_u32(
            process,
            object_manager::CLIENT_CONNECTION,
            "client connection",
        )?;
        let manager = read_nonzero_u32(
            process,
            address_with_offset(
                connection,
                object_manager::OBJECT_MANAGER,
                "object manager slot",
            )?,
            "object manager",
        )?;
        let local_guid = read_remote_u64(
            process,
            address_with_offset(manager, object_manager::LOCAL_GUID, "local player GUID")?,
            "local player GUID",
        )?;
        if local_guid == 0 {
            return Err("local player GUID is null".to_owned());
        }
        let mut current = read_nonzero_u32(
            process,
            address_with_offset(manager, object_manager::FIRST_OBJECT, "first object slot")?,
            "first world object",
        )?;

        for _ in 0..OBJECT_LIMIT {
            let descriptor = read_nonzero_u32(
                process,
                address_with_offset(current, object::DESCRIPTOR_ARRAY, "object descriptor slot")?,
                "object descriptor array",
            )?;
            let guid = read_remote_u64(process, descriptor, "object GUID")?;
            if guid == local_guid {
                let position = [
                    read_remote_f32(
                        process,
                        address_with_offset(current, unit::POSITION_X, "player position X")?,
                        "player position X",
                    )?,
                    read_remote_f32(
                        process,
                        address_with_offset(current, unit::POSITION_Y, "player position Y")?,
                        "player position Y",
                    )?,
                    read_remote_f32(
                        process,
                        address_with_offset(current, unit::POSITION_Z, "player position Z")?,
                        "player position Z",
                    )?,
                ];
                return Ok((current, position));
            }

            let next = read_remote_u32(
                process,
                address_with_offset(current, object::NEXT_OBJECT, "next world object")?,
                "next world object",
            )?;
            if next == 0 {
                break;
            }
            current = next;
        }

        Err(format!(
            "local player was not found in the first {OBJECT_LIMIT} world objects"
        ))
    }

    fn address_with_offset(
        base: RemoteAddress,
        offset: u32,
        context: &str,
    ) -> Result<RemoteAddress, String> {
        base.checked_add(offset)
            .ok_or_else(|| format!("{context} address overflows x86 space"))
    }

    fn print_hook_error(code: u32) {
        if code == 0 {
            println!("last hook installation error: none");
            return;
        }
        let description = match code {
            1 => "null target",
            2 => "replacement equals target",
            3 => "target prologue read failed",
            4 => "x86 address overflow",
            5 => "unsupported or relative instruction in target prologue",
            6 => "trampoline allocation failed",
            7 => "relative jump is outside x86 range",
            8 => "VirtualProtect failed",
            9 => "original page protection could not be restored",
            10 => "target/trampoline write failed",
            11 => "FlushInstructionCache failed",
            12 => "thread snapshot failed",
            13 => "another process thread could not be suspended",
            _ => "unknown inline-hook failure",
        };
        println!("last hook installation error: {code} ({description})");
    }

    fn required_diagnostic(
        process: &Process,
        remote_command: u32,
        command: RemoteCommand,
    ) -> Result<u32, String> {
        query_optional_command(process, remote_command, command)?.ok_or_else(|| {
            "the injected DLL did not return a complete diagnostics snapshot".to_owned()
        })
    }

    fn query_optional_command(
        process: &Process,
        remote_command: u32,
        command: RemoteCommand,
    ) -> Result<Option<u32>, String> {
        let result = call_remote(process, remote_command, command as u32)?;
        Ok((result != u32::MAX).then_some(result))
    }

    fn print_direct_patch(process: &Process, name: &str, target: u32) -> Result<(), String> {
        let bytes = read_remote_bytes::<8>(process, target, name)?;
        print!("{name} bytes at 0x{target:08X}:");
        for byte in bytes {
            print!(" {byte:02X}");
        }
        println!();

        if bytes[0] == 0xE9 {
            let displacement = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let destination = i64::from(target) + 5 + i64::from(displacement);
            if let Ok(destination) = u32::try_from(destination) {
                println!("{name} direct JMP destination: 0x{destination:08X}");
            } else {
                println!("{name} has an invalid x86 JMP destination");
            }
        } else {
            println!("{name} is not currently patched with a direct E9 jump");
        }
        Ok(())
    }

    fn read_remote_bytes<const SIZE: usize>(
        process: &Process,
        address: u32,
        context: &str,
    ) -> Result<[u8; SIZE], String> {
        let mut bytes = [0_u8; SIZE];
        read_remote_into(process, address, &mut bytes, context)?;
        Ok(bytes)
    }

    fn read_remote_into(
        process: &Process,
        address: u32,
        bytes: &mut [u8],
        context: &str,
    ) -> Result<(), String> {
        let mut bytes_read = 0_usize;
        let succeeded = unsafe {
            ReadProcessMemory(
                process.handle,
                address as usize as *const c_void,
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut bytes_read,
            )
        };
        if succeeded == 0 || bytes_read != bytes.len() {
            return Err(format!(
                "could not read {context} at 0x{address:08X}: Win32 error {}",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum LoaderMode {
        VisualSmoke,
        Status,
        TerrainProbe,
        Stop,
    }

    struct LoaderArguments {
        mode: LoaderMode,
        process_id: u32,
        dll_path: PathBuf,
    }

    struct LaunchArguments {
        executable_path: PathBuf,
        dll_path: PathBuf,
    }

    enum LoaderInvocation {
        Existing(LoaderArguments),
        Launch(LaunchArguments),
    }

    fn parse_invocation() -> Result<LoaderInvocation, String> {
        let mut arguments = env::args_os().skip(1);
        let first = arguments.next().ok_or_else(usage)?;
        let first_text = first.to_string_lossy();
        if first_text.eq_ignore_ascii_case("launch") {
            let executable_path =
                canonicalize_argument(arguments.next().ok_or_else(usage)?, "WoW executable")?;
            let dll_path = canonicalize_argument(arguments.next().ok_or_else(usage)?, "veyr.dll")?;
            if arguments.next().is_some() {
                return Err(usage());
            }
            return Ok(LoaderInvocation::Launch(LaunchArguments {
                executable_path,
                dll_path,
            }));
        }
        let (mode, process_id_argument) = if first_text.eq_ignore_ascii_case("status") {
            (LoaderMode::Status, arguments.next().ok_or_else(usage)?)
        } else if first_text.eq_ignore_ascii_case("terrain") {
            (
                LoaderMode::TerrainProbe,
                arguments.next().ok_or_else(usage)?,
            )
        } else if first_text.eq_ignore_ascii_case("stop") {
            (LoaderMode::Stop, arguments.next().ok_or_else(usage)?)
        } else {
            (LoaderMode::VisualSmoke, first)
        };
        let process_id = process_id_argument
            .to_string_lossy()
            .parse::<u32>()
            .map_err(|_| "the WoW process ID must be decimal".to_owned())?;
        let requested_path = arguments.next().ok_or_else(usage)?;
        if arguments.next().is_some() {
            return Err(usage());
        }

        canonicalize_argument(requested_path, "veyr.dll").map(|dll_path| {
            LoaderInvocation::Existing(LoaderArguments {
                mode,
                process_id,
                dll_path,
            })
        })
    }

    fn canonicalize_argument(value: std::ffi::OsString, label: &str) -> Result<PathBuf, String> {
        fs::canonicalize(value).map_err(|error| format!("could not resolve {label} path: {error}"))
    }

    fn usage() -> String {
        "usage: veyr.exe launch <path-to-WoW.exe> <path-to-veyr.dll>\n       veyr.exe <wow-pid> <path-to-veyr.dll>\n       veyr.exe status <wow-pid> <path-to-currently-injected-veyr.dll>\n       veyr.exe terrain <wow-pid> <path-to-currently-injected-veyr.dll>\n       veyr.exe stop <wow-pid> <path-to-currently-injected-veyr.dll>".to_owned()
    }

    fn inject(process: &Process, dll_path: &Path) -> Result<u32, String> {
        let wide_path = wide_null(dll_path.as_os_str());
        let remote_path = RemoteAllocation::write_slice(process, &wide_path)?;
        let remote_load_library = remote_load_library_w(process)?;

        let remote_base = call_remote(process, remote_load_library, remote_path.address)
            .map_err(|error| format!("LoadLibraryW(veyr.dll): {error}"))?;
        if remote_base == 0 {
            return Err("the target rejected LoadLibraryW for veyr.dll".to_owned());
        }

        Ok(remote_base)
    }

    fn remote_load_library_w(process: &Process) -> Result<RemoteAddress, String> {
        let kernel_name = wide_null(OsStr::new("kernel32.dll"));
        let kernel = unsafe { GetModuleHandleW(kernel_name.as_ptr()) };
        if kernel.is_null() {
            return Err(last_error("GetModuleHandleW(kernel32.dll)"));
        }

        let load_library = unsafe { GetProcAddress(kernel, c"LoadLibraryW".as_ptr()) };
        if load_library.is_null() {
            return Err(last_error("GetProcAddress(LoadLibraryW)"));
        }
        let load_library_rva = (load_library as usize)
            .checked_sub(kernel as usize)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "LoadLibraryW RVA does not fit the x86 address space".to_owned())?;
        let remote_kernel = remote_module_base(process, "kernel32.dll")?;
        remote_kernel
            .checked_add(load_library_rva)
            .ok_or_else(|| "remote LoadLibraryW address overflowed x86 address space".to_owned())
    }

    fn remote_module_base(process: &Process, expected_name: &str) -> Result<u32, String> {
        remote_module(process, expected_name).map(|module| module.base)
    }

    fn remote_module(process: &Process, expected_name: &str) -> Result<RemoteModule, String> {
        remote_module_optional(process, expected_name)?
            .ok_or_else(|| format!("target process does not have {expected_name} loaded"))
    }

    fn remote_module_optional(
        process: &Process,
        expected_name: &str,
    ) -> Result<Option<RemoteModule>, String> {
        let deadline = Instant::now() + MODULE_SNAPSHOT_TIMEOUT;
        let mut attempts = 0_u32;
        loop {
            attempts = attempts.saturating_add(1);
            let snapshot = unsafe {
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process.id)
            };
            if snapshot == INVALID_HANDLE_VALUE {
                let error = unsafe { GetLastError() };
                if is_retryable_module_snapshot_error(error)
                    && attempts < MODULE_SNAPSHOT_MAX_ATTEMPTS
                    && Instant::now() < deadline
                {
                    sleep(MODULE_SNAPSHOT_POLL_INTERVAL);
                    continue;
                }
                if is_retryable_module_snapshot_error(error) {
                    return Err(format!(
                        "CreateToolhelp32Snapshot kept reporting retryable Win32 error {error} while reading modules for PID {} (attempts {attempts}, up to {} ms)",
                        process.id,
                        MODULE_SNAPSHOT_TIMEOUT.as_millis(),
                    ));
                }
                return Err(format!(
                    "CreateToolhelp32Snapshot failed with Win32 error {error}"
                ));
            }

            return find_module_in_snapshot(HandleGuard(snapshot), expected_name);
        }
    }

    fn is_retryable_module_snapshot_error(error: u32) -> bool {
        // ERROR_BAD_LENGTH is the documented concurrent-loader result.
        // ERROR_PARTIAL_COPY can occur while a freshly created WOW64 process
        // is still mapping its initial modules, so it is treated identically
        // during the bounded bootstrap window.
        error == ERROR_BAD_LENGTH || error == ERROR_PARTIAL_COPY
    }

    fn find_module_in_snapshot(
        snapshot: HandleGuard,
        expected_name: &str,
    ) -> Result<Option<RemoteModule>, String> {
        let mut entry = ModuleEntry32W::new();
        if unsafe { Module32FirstW(snapshot.0, &mut entry) } == 0 {
            let error = unsafe { GetLastError() };
            return if error == ERROR_NO_MORE_FILES {
                Ok(None)
            } else {
                Err(format!("Module32FirstW failed with Win32 error {error}"))
            };
        }

        loop {
            if utf16_name(&entry.module_name).eq_ignore_ascii_case(expected_name) {
                let base = u32::try_from(entry.base_address as usize).map_err(|_| {
                    "remote module base does not fit the x86 address space".to_owned()
                })?;
                if entry.base_size == 0 {
                    return Err(format!(
                        "target process reports a zero image size for {expected_name}"
                    ));
                }
                base.checked_add(entry.base_size).ok_or_else(|| {
                    format!("remote {expected_name} image range overflowed x86 space")
                })?;
                return Ok(Some(RemoteModule {
                    base,
                    size: entry.base_size,
                }));
            }

            entry = ModuleEntry32W::new();
            if unsafe { Module32NextW(snapshot.0, &mut entry) } == 0 {
                let error = unsafe { GetLastError() };
                return if error == ERROR_NO_MORE_FILES {
                    Ok(None)
                } else {
                    Err(format!("Module32NextW failed with Win32 error {error}"))
                };
            }
        }
    }

    fn call_remote_raw(
        process: &Process,
        address: u32,
        parameter: u32,
    ) -> Result<u32, RemoteCallError> {
        let thread = start_remote_thread(process, address, parameter)?;
        wait_remote_result(thread)
    }

    /// Starts a private thread-entry export without waiting for it.
    ///
    /// This is used only for the pre-created launch worker. It lets that one
    /// worker wait inside `veyr.dll` while the loader releases WoW startup,
    /// rather than polling by creating a sequence of extra remote threads.
    fn start_remote_thread(
        process: &Process,
        address: u32,
        parameter: u32,
    ) -> Result<HandleGuard, RemoteCallError> {
        let entry: ThreadEntry = unsafe { transmute(address as usize) };
        let thread = unsafe {
            CreateRemoteThread(
                process.handle,
                null(),
                0,
                Some(entry),
                parameter as usize as *mut c_void,
                0,
                null_mut(),
            )
        };
        if thread.is_null() {
            return Err(RemoteCallError::CreateThread {
                win32_error: unsafe { GetLastError() },
            });
        }
        Ok(HandleGuard(thread))
    }

    fn call_remote(process: &Process, address: u32, parameter: u32) -> Result<u32, String> {
        call_remote_raw(process, address, parameter)
            .map_err(|error| error.describe("remote command"))
    }

    fn wait_remote_result(thread: HandleGuard) -> Result<u32, RemoteCallError> {
        wait_remote_result_for(thread, INFINITE)
    }

    fn wait_remote_result_for(
        thread: HandleGuard,
        milliseconds: u32,
    ) -> Result<u32, RemoteCallError> {
        let wait_result = unsafe { WaitForSingleObject(thread.0, milliseconds) };
        if wait_result == WAIT_TIMEOUT {
            return Err(RemoteCallError::Timeout { milliseconds });
        }
        if wait_result != WAIT_OBJECT_0 {
            return Err(RemoteCallError::Wait {
                win32_error: unsafe { GetLastError() },
            });
        }

        let mut result = 0;
        if unsafe { GetExitCodeThread(thread.0, &mut result) } == 0 {
            return Err(RemoteCallError::ExitCode {
                win32_error: unsafe { GetLastError() },
            });
        }
        Ok(result)
    }

    #[derive(Debug, Copy, Clone)]
    enum RemoteCallError {
        CreateThread { win32_error: u32 },
        Wait { win32_error: u32 },
        ExitCode { win32_error: u32 },
        Timeout { milliseconds: u32 },
    }

    impl RemoteCallError {
        fn describe(self, operation: &str) -> String {
            match self {
                Self::CreateThread { win32_error } => {
                    format!("{operation}: CreateRemoteThread failed with Win32 error {win32_error}")
                }
                Self::Wait { win32_error } => {
                    format!(
                        "{operation}: WaitForSingleObject failed with Win32 error {win32_error}"
                    )
                }
                Self::ExitCode { win32_error } => {
                    format!("{operation}: GetExitCodeThread failed with Win32 error {win32_error}")
                }
                Self::Timeout { milliseconds } => {
                    format!("{operation}: did not finish within {milliseconds} ms")
                }
            }
        }
    }

    fn duration_to_wait_millis(duration: Duration) -> u32 {
        u32::try_from(duration.as_millis()).unwrap_or(u32::MAX - 1)
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn utf16_name(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|code_unit| *code_unit == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }

    fn last_error(operation: &str) -> String {
        format!("{operation} failed with Win32 error {}", unsafe {
            GetLastError()
        })
    }

    /// Owns a newly created WoW process held at the debugger's initial
    /// breakpoint. This is the first safe user-mode point for injection: the
    /// target loader is initialized, but the WoW entry point has not run.
    ///
    /// Dropping the gate always continues its pending event and detaches the
    /// debugger. Thus any loader-side error still lets the user start WoW
    /// normally instead of leaving an invisible suspended process behind.
    struct StartupGate {
        process_id: u32,
        primary_thread: Handle,
        pending_event: Option<(u32, u32)>,
        primary_thread_suspended: bool,
        debugger_attached: bool,
        released: bool,
    }

    impl StartupGate {
        fn start(executable_path: &Path) -> Result<Self, String> {
            let application_name = wide_null(executable_path.as_os_str());
            let working_directory = executable_path
                .parent()
                .ok_or_else(|| "WoW executable has no parent directory".to_owned())?;
            let current_directory = wide_null(working_directory.as_os_str());
            let mut startup_info: StartupInfoW = unsafe { zeroed() };
            startup_info.size = core::mem::size_of::<StartupInfoW>() as u32;
            let mut process_information: ProcessInformation = unsafe { zeroed() };
            let created = unsafe {
                CreateProcessW(
                    application_name.as_ptr(),
                    null_mut(),
                    null(),
                    null(),
                    0,
                    DEBUG_ONLY_THIS_PROCESS,
                    null(),
                    current_directory.as_ptr(),
                    &mut startup_info,
                    &mut process_information,
                )
            };
            if created == 0 {
                return Err(last_error("CreateProcessW"));
            }
            if process_information.process.is_null() || process_information.thread.is_null() {
                if !process_information.process.is_null() {
                    unsafe {
                        let _ = CloseHandle(process_information.process);
                    }
                }
                if !process_information.thread.is_null() {
                    unsafe {
                        let _ = CloseHandle(process_information.thread);
                    }
                }
                return Err("CreateProcessW returned null process/thread handles".to_owned());
            }
            unsafe {
                let _ = CloseHandle(process_information.process);
            }
            let mut gate = Self {
                process_id: process_information.process_id,
                primary_thread: process_information.thread,
                pending_event: None,
                primary_thread_suspended: false,
                debugger_attached: true,
                released: false,
            };
            if let Err(error) = gate.wait_for_initial_breakpoint() {
                drop(gate);
                return Err(error);
            }
            Ok(gate)
        }

        fn wait_for_initial_breakpoint(&mut self) -> Result<(), String> {
            let deadline = Instant::now() + STARTUP_GATE_TIMEOUT;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(
                        "timed out waiting for WoW's initial Windows loader breakpoint".to_owned(),
                    );
                }
                let mut event: DebugEvent = unsafe { zeroed() };
                if unsafe { WaitForDebugEvent(&mut event, duration_to_wait_millis(remaining)) } == 0
                {
                    return Err(last_error("WaitForDebugEvent"));
                }
                if event.process_id != self.process_id {
                    let _ = unsafe {
                        ContinueDebugEvent(
                            event.process_id,
                            event.thread_id,
                            DBG_EXCEPTION_NOT_HANDLED,
                        )
                    };
                    continue;
                }
                if event.event_code == EXIT_PROCESS_DEBUG_EVENT {
                    let _ = unsafe {
                        ContinueDebugEvent(event.process_id, event.thread_id, DBG_CONTINUE)
                    };
                    return Err(
                        "WoW exited before reaching its initial loader breakpoint".to_owned()
                    );
                }
                if event.initial_breakpoint() {
                    // Keep the original primary thread stopped after we
                    // continue this debug event. Windows otherwise resumes
                    // it as part of ContinueDebugEvent, and calling
                    // DebugActiveProcessStop immediately afterward lets WoW
                    // race D3D9 creation before the hook is armed.
                    if unsafe { SuspendThread(self.primary_thread) } == u32::MAX {
                        return Err(last_error("SuspendThread(primary thread)"));
                    }
                    self.primary_thread_suspended = true;
                    self.pending_event = Some((event.process_id, event.thread_id));
                    return Ok(());
                }
                if unsafe {
                    ContinueDebugEvent(event.process_id, event.thread_id, DBG_EXCEPTION_NOT_HANDLED)
                } == 0
                {
                    return Err(last_error("ContinueDebugEvent"));
                }
            }
        }

        /// Continues the initial debugger event and detaches while preserving
        /// the primary thread's suspend count. After this returns it is safe
        /// to create ordinary remote threads: no debug events remain pending,
        /// but WoW itself still cannot advance into its entry point.
        fn prepare_injection(&mut self) -> Result<(), String> {
            let (process_id, thread_id) = self
                .pending_event
                .take()
                .ok_or_else(|| "WoW startup gate has no pending debug event".to_owned())?;
            if unsafe { ContinueDebugEvent(process_id, thread_id, DBG_CONTINUE) } == 0 {
                return Err(last_error("ContinueDebugEvent(initial loader breakpoint)"));
            }
            if unsafe { DebugActiveProcessStop(self.process_id) } == 0 {
                return Err(last_error("DebugActiveProcessStop"));
            }
            self.debugger_attached = false;
            Ok(())
        }

        fn release(&mut self) -> Result<(), String> {
            if self.released {
                return Ok(());
            }
            if self.debugger_attached {
                return Err("WoW startup debugger has not been detached".to_owned());
            }
            if self.primary_thread_suspended
                && unsafe { ResumeThread(self.primary_thread) } == u32::MAX
            {
                return Err(last_error("ResumeThread(primary thread)"));
            }
            self.primary_thread_suspended = false;
            self.released = true;
            Ok(())
        }
    }

    impl Drop for StartupGate {
        fn drop(&mut self) {
            if let Some((process_id, thread_id)) = self.pending_event.take() {
                unsafe {
                    let _ = ContinueDebugEvent(process_id, thread_id, DBG_CONTINUE);
                }
            }
            if self.debugger_attached {
                unsafe {
                    let _ = DebugActiveProcessStop(self.process_id);
                }
            }
            if self.primary_thread_suspended {
                unsafe {
                    let _ = ResumeThread(self.primary_thread);
                }
            }
            unsafe {
                let _ = CloseHandle(self.primary_thread);
            }
        }
    }

    struct Process {
        id: u32,
        handle: Handle,
    }

    impl Process {
        fn open(process_id: u32) -> Result<Self, String> {
            let access = PROCESS_CREATE_THREAD
                | PROCESS_QUERY_INFORMATION
                | PROCESS_VM_OPERATION
                | PROCESS_VM_READ
                | PROCESS_VM_WRITE;
            let handle = unsafe { OpenProcess(access, 0, process_id) };
            HandleGuard::new(handle, "OpenProcess").map(|handle| Self {
                id: process_id,
                handle: handle.into_raw(),
            })
        }

        fn ensure_x86_target(&self) -> Result<(), String> {
            let mut loader_is_wow64 = 0_i32;
            let mut target_is_wow64 = 0_i32;
            let loader_checked =
                unsafe { IsWow64Process(GetCurrentProcess(), &mut loader_is_wow64) };
            let target_checked = unsafe { IsWow64Process(self.handle, &mut target_is_wow64) };
            if loader_checked == 0 || target_checked == 0 {
                return Err(last_error("IsWow64Process"));
            }
            if loader_is_wow64 != 0 && target_is_wow64 == 0 {
                return Err(
                    "the selected WoW executable is 64-bit, but veyr.dll and veyr.exe are x86; choose the 32-bit client executable"
                        .to_owned(),
                );
            }
            Ok(())
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    struct HandleGuard(Handle);

    impl HandleGuard {
        fn new(handle: Handle, operation: &str) -> Result<Self, String> {
            if handle.is_null() {
                Err(last_error(operation))
            } else {
                Ok(Self(handle))
            }
        }

        fn into_raw(self) -> Handle {
            let handle = self.0;
            core::mem::forget(self);
            handle
        }
    }

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct RemoteAllocation<'process> {
        process: &'process Process,
        address: u32,
    }

    impl<'process> RemoteAllocation<'process> {
        fn write_slice<T: Copy>(process: &'process Process, value: &[T]) -> Result<Self, String> {
            let size = core::mem::size_of_val(value);
            let address = unsafe {
                VirtualAllocEx(
                    process.handle,
                    null(),
                    size,
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_READWRITE,
                )
            };
            if address.is_null() {
                return Err(last_error("VirtualAllocEx"));
            }

            let remote_address = u32::try_from(address as usize).map_err(|_| {
                unsafe {
                    let _ = VirtualFreeEx(process.handle, address, 0, MEM_RELEASE);
                }
                "VirtualAllocEx returned an address outside x86 space".to_owned()
            })?;
            let allocation = Self {
                process,
                address: remote_address,
            };
            let mut bytes_written = 0;
            let written = unsafe {
                WriteProcessMemory(
                    process.handle,
                    address,
                    value.as_ptr().cast(),
                    size,
                    &mut bytes_written,
                )
            };
            if written == 0 || bytes_written != size {
                return Err(last_error("WriteProcessMemory"));
            }
            Ok(allocation)
        }

        fn read_value<T: Copy>(&self, context: &str) -> Result<T, String> {
            let mut value = core::mem::MaybeUninit::<T>::uninit();
            let bytes = unsafe {
                core::slice::from_raw_parts_mut(
                    value.as_mut_ptr().cast::<u8>(),
                    core::mem::size_of::<T>(),
                )
            };
            read_remote_into(self.process, self.address, bytes, context)?;
            // `read_remote_into` filled the exact object representation and
            // callers use only plain `repr(C)` word structures.
            Ok(unsafe { value.assume_init() })
        }
    }

    impl Drop for RemoteAllocation<'_> {
        fn drop(&mut self) {
            unsafe {
                let _ = VirtualFreeEx(
                    self.process.handle,
                    self.address as usize as *mut c_void,
                    0,
                    MEM_RELEASE,
                );
            }
        }
    }

    struct LocalModule {
        module: Module,
        label: String,
    }

    impl LocalModule {
        fn load(path: &Path) -> Result<Self, String> {
            let label = path.display().to_string();
            let wide_path = wide_null(path.as_os_str());
            let module = unsafe { LoadLibraryW(wide_path.as_ptr()) };
            if module.is_null() {
                Err(last_error(&format!("LoadLibraryW({label})")))
            } else {
                Ok(Self { module, label })
            }
        }

        fn export_rva(&self, name: &[u8]) -> Result<u32, String> {
            let export_name = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
            let address = unsafe { GetProcAddress(self.module, name.as_ptr().cast()) };
            if address.is_null() {
                return Err(format!("{} does not export {export_name}", self.label));
            }
            let rva = (address as usize)
                .checked_sub(self.module as usize)
                .ok_or_else(|| {
                    format!("{export_name} address precedes local {} base", self.label)
                })?;
            u32::try_from(rva)
                .map_err(|_| format!("{export_name} RVA in {} does not fit x86 space", self.label))
        }
    }

    impl Drop for LocalModule {
        fn drop(&mut self) {
            unsafe {
                let _ = FreeLibrary(self.module);
            }
        }
    }
}
