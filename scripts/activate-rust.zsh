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
# %N resolves to this file even when it is sourced from an interactive shell.
VEYR_SCRIPTS_DIR="${${(%):-%N}:A:h}"
VEYR_PROJECT_ROOT_DEFAULT="${VEYR_SCRIPTS_DIR:h}"

# The target client and injector are 32-bit MinGW binaries.  Do not silently
# switch this to the much smaller MSVC artifact: it has a different CRT setup
# and is not the known-good test build for this project.
export CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER:-$VEYR_SCRIPTS_DIR/zig-cc-i686-windows.sh}"

veyr_sha256() {
    shasum -a 256 "$1" | awk '{ print $1 }'
}

veyr_file_size() {
    wc -c < "$1" | tr -d '[:space:]'
}

veyr_assert_windows_x86_artifact() {
    local artifact="$1"
    local description

    if ! description="$(file -b "$artifact")"; then
        print -u2 -- "Could not inspect Windows artifact: $artifact"
        return 1
    fi

    if [[ "$description" != *"PE32 executable"* ||
        "$description" != *"Intel 80386"* ||
        "$description" != *"for MS Windows"* ]]; then
        print -u2 -- "Unexpected artifact format for $artifact: $description"
        return 1
    fi
}

veyr_write_artifact_manifest() {
    local manifest_path="$1"
    local source_commit="$2"
    local source_ref="$3"
    local source_tree="$4"
    local dll_path="$5"
    local loader_path="$6"
    local dll_sha loader_sha dll_bytes loader_bytes

    dll_sha="$(veyr_sha256 "$dll_path")" || return
    loader_sha="$(veyr_sha256 "$loader_path")" || return
    dll_bytes="$(veyr_file_size "$dll_path")" || return
    loader_bytes="$(veyr_file_size "$loader_path")" || return

    printf '%s\n' \
        '{' \
        '  "schema_version": 1,' \
        '  "channel": "dev",' \
        "  \"source_commit\": \"$source_commit\"," \
        "  \"source_ref\": \"$source_ref\"," \
        "  \"source_tree\": \"$source_tree\"," \
        '  "target": "i686-pc-windows-gnu",' \
        '  "profile": "release",' \
        '  "artifacts": {' \
        "    \"veyr.dll\": { \"sha256\": \"$dll_sha\", \"bytes\": $dll_bytes }," \
        "    \"veyr.exe\": { \"sha256\": \"$loader_sha\", \"bytes\": $loader_bytes }" \
        '  }' \
        '}' > "$manifest_path"
}

veyr_build_windows() {
    local project_root="${VEYR_PROJECT_ROOT:-$VEYR_PROJECT_ROOT_DEFAULT}"
    local linker_wrapper="${CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER:-$VEYR_SCRIPTS_DIR/zig-cc-i686-windows.sh}"
    local output_dir="$project_root/dist"
    local stage_dir
    local build_dir="$CARGO_TARGET_DIR/i686-pc-windows-gnu/release"
    local source_commit source_ref source_tree

    if [[ ! -d "$project_root" ]]; then
        print -u2 "Veyr project directory does not exist: $project_root"
        return 1
    fi

    if [[ ! -x "$linker_wrapper" ]]; then
        print -u2 "Windows GNU linker wrapper is missing or not executable: $linker_wrapper"
        return 1
    fi

    if ! source_commit="$(git -C "$project_root" rev-parse --verify HEAD)"; then
        print -u2 "Could not identify the source commit for the Windows build"
        return 1
    fi
    source_ref="$(git -C "$project_root" branch --show-current)" || return
    [[ -n "$source_ref" ]] || source_ref="detached"
    if [[ -n "$(git -C "$project_root" status --porcelain)" ]]; then
        source_tree="dirty"
    else
        source_tree="clean"
    fi

    (
        cd "$project_root" || exit
        CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER="$linker_wrapper" \
            cargo build --release --target i686-pc-windows-gnu "$@"
    ) || return

    if [[ ! -f "$build_dir/veyr.dll" || ! -f "$build_dir/veyr.exe" ]]; then
        print -u2 "Windows build completed without both release deliverables"
        return 1
    fi

    mkdir -p "$output_dir"
    if ! stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/veyr-dist-staging.XXXXXX")"; then
        print -u2 "Could not create a staging directory for Windows artifacts"
        return 1
    fi

    cp "$build_dir/veyr.dll" "$stage_dir/veyr.dll" || return
    cp "$build_dir/veyr.exe" "$stage_dir/veyr.exe" || return
    veyr_assert_windows_x86_artifact "$stage_dir/veyr.dll" || return
    veyr_assert_windows_x86_artifact "$stage_dir/veyr.exe" || return
    veyr_write_artifact_manifest \
        "$stage_dir/manifest.json" \
        "$source_commit" \
        "$source_ref" \
        "$source_tree" \
        "$stage_dir/veyr.dll" \
        "$stage_dir/veyr.exe" || return

    # Publish only after both copies have succeeded, so dist/ never contains
    # an unverified binary. The manifest is published last: it is the
    # provenance record for the exact DLL/loader pair currently in dist/.
    mv -f "$stage_dir/veyr.dll" "$output_dir/veyr.dll"
    mv -f "$stage_dir/veyr.exe" "$output_dir/veyr.exe"
    mv -f "$stage_dir/manifest.json" "$output_dir/manifest.json"
    rmdir "$stage_dir" 2>/dev/null || true
}
