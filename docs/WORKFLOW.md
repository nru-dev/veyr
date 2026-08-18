# Veyr development and release workflow

## Channels

- dev is the only active development branch. Each verified work block ends
  with a checkpoint commit and a GitHub Desktop push.
- release is reserved for a complete, user-accepted Windows release. It is
  never advanced automatically.

## Windows artifacts

Run ./scripts/build-windows.zsh from the repository root. It builds the
32-bit GNU Windows DLL and loader, verifies that both are PE32 x86 files, and
publishes them with provenance and SHA-256 values in dist/manifest.json.

Run ./scripts/verify-windows-artifacts.zsh before sharing or testing a
candidate. It checks the files in dist/ against that manifest.

- dist/ is the latest development candidate.
- dist/stable/ is the last manually confirmed rollback baseline. Build
  scripts must never replace it automatically.

## Block completion rule

Before the next work block starts:

1. formatting, tests, and the Windows build pass;
2. the current dist/ candidate verifies against its manifest;
3. the changes are committed on dev and pushed through GitHub Desktop;
4. for runtime-sensitive changes, the candidate is also checked in the real
   Windows client.

World-circle rendering is intentionally outside the current release path and
remains paused until the post-release update.
