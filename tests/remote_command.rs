use veyr::{
    RemoteCommand, RemoteGraphicsConfiguration, RemoteLaunchOutcome, RemoteLaunchReport,
    RemoteLaunchRequest, GRAPHICS_BACKEND_D3D9, GRAPHICS_BACKEND_OPENGL,
    GRAPHICS_CONFIGURATION_ABI_VERSION, REMOTE_LAUNCH_ABI_VERSION,
};

#[test]
fn remote_command_values_are_stable_for_the_windows_loader_abi() {
    assert_eq!(RemoteCommand::StartDefault as u32, 2);
    assert_eq!(RemoteCommand::StartVisualSmoke as u32, 3);
    assert_eq!(RemoteCommand::Stop as u32, 4);
    assert_eq!(RemoteCommand::FrameCount as u32, 5);
    assert_eq!(RemoteCommand::CallbackPanicCount as u32, 6);
    assert_eq!(RemoteCommand::RenderSubmittedCommands as u32, 7);
    assert_eq!(RemoteCommand::RenderDrawnCommands as u32, 8);
    assert_eq!(RemoteCommand::RenderSkippedCommands as u32, 9);
    assert_eq!(RemoteCommand::RenderDrawFailures as u32, 10);
    assert_eq!(RemoteCommand::RenderStateSetupFailed as u32, 11);
    assert_eq!(RemoteCommand::RenderLastError as u32, 12);
    assert_eq!(RemoteCommand::ConfiguredEndScene as u32, 13);
    assert_eq!(RemoteCommand::ConfiguredReset as u32, 14);
    assert_eq!(RemoteCommand::ConfiguredGraphicsBackend as u32, 15);
    assert_eq!(RemoteCommand::ConfiguredFrameTarget as u32, 16);
    assert_eq!(RemoteCommand::LastHookError as u32, 17);
    assert_eq!(RemoteCommand::ConfiguredAuxiliaryTarget as u32, 18);
    assert_eq!(RemoteCommand::ArmD3d9Capture as u32, 19);
    assert_eq!(RemoteCommand::D3d9CaptureState as u32, 20);
    assert_eq!(RemoteCommand::D3d9FactoryCallCount as u32, 21);
    assert_eq!(RemoteCommand::D3d9CreateDeviceCallCount as u32, 22);
    assert_eq!(RemoteCommand::CapturedD3d9Device as u32, 23);
    assert_eq!(RemoteCommand::CapturedD3d9EndScene as u32, 24);
    assert_eq!(RemoteCommand::CapturedD3d9Reset as u32, 25);
    assert_eq!(RemoteCommand::D3d9CaptureError as u32, 26);
    assert_eq!(RemoteCommand::StartPlayerCircle as u32, 27);
    assert_eq!(RemoteCommand::ArmTerrainProbe as u32, 28);
    assert_eq!(RemoteCommand::TerrainProbeStatus as u32, 29);
    assert_eq!(RemoteCommand::TerrainProbeHitX as u32, 30);
    assert_eq!(RemoteCommand::TerrainProbeHitY as u32, 31);
    assert_eq!(RemoteCommand::TerrainProbeHitZ as u32, 32);
    assert_eq!(RemoteCommand::TerrainProbeNativeResult as u32, 33);
}

#[test]
fn graphics_configuration_abi_is_plain_x86_words() {
    assert_eq!(GRAPHICS_CONFIGURATION_ABI_VERSION, 3);
    assert_eq!(GRAPHICS_BACKEND_D3D9, 1);
    assert_eq!(GRAPHICS_BACKEND_OPENGL, 2);
    assert_eq!(core::mem::size_of::<RemoteGraphicsConfiguration>(), 20);
    assert_eq!(core::mem::align_of::<RemoteGraphicsConfiguration>(), 4);
}

#[test]
fn launch_worker_abi_is_plain_words_and_starts_pending() {
    let request = RemoteLaunchRequest::player_circle(30_000);

    assert_eq!(REMOTE_LAUNCH_ABI_VERSION, 1);
    assert_eq!(request.abi_version, REMOTE_LAUNCH_ABI_VERSION);
    assert_eq!(request.capture_timeout_millis, 30_000);
    assert_eq!(request.report.abi_version, REMOTE_LAUNCH_ABI_VERSION);
    assert_eq!(request.report.outcome, RemoteLaunchOutcome::Pending as u32);
    assert_eq!(core::mem::size_of::<RemoteLaunchReport>(), 48);
    assert_eq!(core::mem::align_of::<RemoteLaunchReport>(), 4);
    assert_eq!(core::mem::size_of::<RemoteLaunchRequest>(), 56);
    assert_eq!(core::mem::align_of::<RemoteLaunchRequest>(), 4);
}
