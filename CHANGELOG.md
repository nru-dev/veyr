# Changelog

## Unreleased — restored D3D9 runtime

This development candidate brings back the known-working World of Warcraft
3.3.5a (build 12340) rendering path. It is a Windows **x86** candidate, not a
portable or production release.

### What changed

- **Capture the live device early.** `veyr.exe launch` starts WoW suspended,
  waits for the normal Windows loader bootstrap, injects `veyr.dll`, and arms
  temporary `Direct3DCreate9`/`IDirect3D9::CreateDevice` hooks before WoW's
  first device is created. The runtime uses the actual `IDirect3DDevice9`
  pointer and its live `EndScene`/`Reset` entries rather than guessed driver
  addresses.
- **Restore the working frame-hook ABI.** The long-lived D3D9 runtime uses
  direct x86 entry hooks with the `__stdcall` COM signatures for
  `EndScene(device)` and `Reset(device, parameters)`. `Present` is not used by
  the restored path. Original prologues are kept in trampolines and are always
  called after the Veyr callback.
- **Make startup and teardown fail closed.** Temporary capture hooks are
  removed before the normal renderer hooks are installed; `Reset` is removed
  before `EndScene` during teardown. Hook lifecycle state, cancellation, and
  capture counters/HRESULTs are exposed through the private loader diagnostics
  ABI so a failed launch can be cleaned up without leaving WoW suspended.
- **Keep target discovery defensive.** The legacy `GxDevice` offset is only a
  validated probe (mapped-module and vtable checks); it is not trusted as the
  source of the active frame-hook entries.
- **Preserve renderer safety fixes.** D3D9 stream-source state uses the correct
  `GetStreamSource`/`SetStreamSource` slots, and callback initialization is
  completed before a direct hook becomes visible.
- **Publish reproducible artifacts.** `dist/manifest.json` records the source
  tree, target, byte sizes, and SHA-256 values for the matching `veyr.dll` and
  `veyr.exe` pair. FTP deployment uploads the binaries first and the manifest
  last.

### Use the candidate

On a 32-bit Windows installation containing the supported WoW client:

```bat
cd /d "D:\World of Warcraft 3.3.5a"
veyr.exe launch "D:\World of Warcraft 3.3.5a\Wow.exe" veyr.dll
```

The successful launch output should include the captured device, `EndScene`,
and `Reset` addresses, followed by a player-circle startup result of `0`.
For an already injected process, use:

```bat
veyr.exe status <PID> veyr.dll
veyr.exe stop <PID> veyr.dll
```

`status` prints the direct-patch bytes and renderer diagnostics; `stop`
restores the native entries while leaving the DLL loaded and inactive.

### Build and publish

From the repository root, use the pinned Rust toolchain and the x86 Windows
target:

```zsh
# Build locally without FTP (the build helper uploads by default when enabled).
VEYR_FTP_UPLOAD=0 ./scripts/build-windows.zsh
./scripts/verify-windows-artifacts.zsh
```

To publish the verified pair, set `VEYR_FTP_URL`, `VEYR_FTP_USER`,
`VEYR_FTP_PASSWORD`, and optionally `VEYR_FTP_REMOTE_DIR` (or put them in the
ignored `.ftp-deploy.env`), then run:

```zsh
./scripts/upload-windows-ftp.zsh
```

Never commit `.ftp-deploy.env`, `target/`, or ad-hoc build output. Keep
`dist/veyr.dll`, `dist/veyr.exe`, and their matching `manifest.json` together;
`dist/stable/` is the manually confirmed rollback baseline.

### Verification and scope

Before pushing a checkpoint, run formatting, tests, linting, the Windows x86
build, and manifest verification. The runtime-sensitive result still needs a
real Windows/WoW check; it cannot be reproduced by the macOS development
host.

The player-circle renderer is intentionally conservative: static camera-
projected rendering is enabled, while experimental native terrain/model
collision remains fail-closed until the exact live client profile is
revalidated. Other WoW builds, 64-bit clients, and non-D3D9 backends are
outside this candidate's support statement.
