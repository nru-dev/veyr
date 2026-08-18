#!/bin/sh
# GNU-driver compatibility wrapper for Rust's i686-pc-windows-gnu target.
#
# Rust emits GNU-ld spellings; the bundled macOS Zig invokes lld-link, which
# needs the .def file and static archives passed as regular files instead.
set -eu

VEYR_RUST_ROOT="${VEYR_RUST_ROOT:-/Users/deadsec/Developer/toolchains/veyr-rust}"
ZIG="$VEYR_RUST_ROOT/tools/zig-0.14.1/zig"
MINGW="$VEYR_RUST_ROOT/tools/mingw-w64/mingw-w64/14.0.0_3/toolchain-i686/i686-w64-mingw32/lib"
MSYS="$VEYR_RUST_ROOT/tools/msys2/gcc-static/mingw32"
GCC_EH="$MSYS/lib/gcc/i686-w64-mingw32/15.2.0/libgcc_eh.a"
GCC="$MSYS/lib/gcc/i686-w64-mingw32/15.2.0/libgcc.a"

for required in "$ZIG" "$MINGW/libpthread.a" "$MINGW/libmsvcrt.a" "$GCC_EH" "$GCC"; do
    if [ ! -e "$required" ]; then
        printf 'missing Windows GNU build dependency: %s\n' "$required" >&2
        exit 1
    fi
done

# Project and toolchain paths are intentionally whitespace-free. Cargo/rustc
# pass one linker option per argv item, so normalizing them line-by-line is
# safe here and preserves all remaining arguments unchanged.
#
# Keep the GCC DW2 unwinder static: the injected DLL must not require a
# sidecar libgcc_s_dw2-1.dll beside the game executable.
# shellcheck disable=SC2046
exec "$ZIG" cc -target x86-windows-gnu -L"$MINGW" $(printf '%s\n' "$@" | sed \
    -e 's#^-Wl,\(.*\.def\)$#\1#' \
    -e '/^-Wl,--large-address-aware$/d' \
    -e 's#^-l:libpthread\.a$#'"$MINGW"'/libpthread.a#' \
    -e 's#^-lmsvcrt$#'"$MINGW"'/libmsvcrt.a#' \
    -e 's#^-lgcc_eh$#'"$GCC_EH"'#' \
    -e 's#^-lgcc$#'"$GCC"'#')
