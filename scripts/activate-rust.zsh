#!/usr/bin/env zsh
# Usage: source scripts/activate-rust.zsh
#
# Keeps the Rust toolchain, Cargo cache, and build artifacts outside the repo
# and outside the user's default Rust directories.

export VEYR_RUST_ROOT="/Users/deadsec/Developer/toolchains/veyr-rust"
export CARGO_HOME="$VEYR_RUST_ROOT/cargo"
export RUSTUP_HOME="$VEYR_RUST_ROOT/rustup"
export CARGO_TARGET_DIR="$VEYR_RUST_ROOT/target"
# cargo-xwin uses this cache for the Windows MSVC sysroot. Keeping it below
# VEYR_RUST_ROOT makes the entire development toolchain disposable as one
# directory instead of writing into ~/Library/Caches.
export XWIN_CACHE_DIR="$VEYR_RUST_ROOT/xwin"

mkdir -p "$CARGO_TARGET_DIR" "$XWIN_CACHE_DIR"

case ":$PATH:" in
    *":$CARGO_HOME/bin:"*) ;;
    *) export PATH="$CARGO_HOME/bin:$PATH" ;;
esac

# The checked-in toolchain includes an ARM64 macOS Zig binary. It is used by
# cargo-xwin's clang backend to produce the actual 32-bit Windows DLL without
# relying on a global Homebrew installation.
export XWIN_CROSS_COMPILER="${XWIN_CROSS_COMPILER:-clang}"

# Resolve the repository path before callers change directories.
VEYR_SCRIPTS_DIR="${0:A:h}"
VEYR_PROJECT_ROOT_DEFAULT="${VEYR_SCRIPTS_DIR:h}"

# The target client and injector are 32-bit MinGW binaries.  Do not silently
# switch this to the much smaller MSVC artifact: it has a different CRT setup
# and is not the known-good test build for this project.
export CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER:-$VEYR_SCRIPTS_DIR/zig-cc-i686-windows.sh}"

veyr_build_windows() {
    local project_root="${VEYR_PROJECT_ROOT:-$VEYR_PROJECT_ROOT_DEFAULT}"
    local linker_wrapper="${CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER:-$VEYR_SCRIPTS_DIR/zig-cc-i686-windows.sh}"
    local output_dir="$project_root/dist"
    local stage_dir="${TMPDIR:-/tmp}/veyr-dist-staging-$$"
    local build_dir="$CARGO_TARGET_DIR/i686-pc-windows-gnu/release"

    if [[ ! -d "$project_root" ]]; then
        print -u2 "Veyr project directory does not exist: $project_root"
        return 1
    fi

    if [[ ! -x "$linker_wrapper" ]]; then
        print -u2 "Windows GNU linker wrapper is missing or not executable: $linker_wrapper"
        return 1
    fi

    (
        cd "$project_root" || exit
        CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER="$linker_wrapper" \
            cargo build --release --target i686-pc-windows-gnu "$@"
    ) || return

    if [[ ! -f "$build_dir/veyr.dll" || ! -f "$build_dir/veyr_loader.exe" ]]; then
        print -u2 "Windows build completed without both release deliverables"
        return 1
    fi

    mkdir -p "$output_dir" "$stage_dir"
    cp "$build_dir/veyr.dll" "$stage_dir/veyr.dll" || return
    cp "$build_dir/veyr_loader.exe" "$stage_dir/veyr_loader.exe" || return

    # Publish only after both copies have succeeded, so dist/ never contains
    # an old DLL paired with a new loader (or vice versa).  Staging lives in
    # the system temp area and leaves no transient files in dist/.
    mv -f "$stage_dir/veyr.dll" "$output_dir/veyr.dll"
    mv -f "$stage_dir/veyr_loader.exe" "$output_dir/veyr_loader.exe"
    rmdir "$stage_dir" 2>/dev/null || true
}
