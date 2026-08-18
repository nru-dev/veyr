#!/usr/bin/env zsh
# Verify the current dev candidate in dist/ against its generated manifest.
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
PROJECT_ROOT="${SCRIPT_DIR:h}"
DIST_DIR="${1:-$PROJECT_ROOT/dist}"
MANIFEST="$DIST_DIR/manifest.json"
DLL="$DIST_DIR/veyr.dll"
LOADER="$DIST_DIR/veyr.exe"

require_windows_x86() {
    local artifact="$1"
    local description
    description="$(file -b "$artifact")"
    if [[ "$description" != *"PE32 executable"* ||
        "$description" != *"Intel 80386"* ||
        "$description" != *"for MS Windows"* ]]; then
        print -u2 -- "Unexpected artifact format for $artifact: $description"
        return 1
    fi
}

manifest_value() {
    local artifact="$1"
    local field="$2"
    local pattern

    case "$artifact:$field" in
        veyr.dll:sha256)
            pattern='/"veyr\.dll"/s/.*"sha256": "([0-9a-f]{64})".*/\1/p'
            ;;
        veyr.dll:bytes)
            pattern='/"veyr\.dll"/s/.*"bytes": ([0-9]+).*/\1/p'
            ;;
        veyr.exe:sha256)
            pattern='/"veyr\.exe"/s/.*"sha256": "([0-9a-f]{64})".*/\1/p'
            ;;
        veyr.exe:bytes)
            pattern='/"veyr\.exe"/s/.*"bytes": ([0-9]+).*/\1/p'
            ;;
        *)
            print -u2 -- "Unsupported manifest lookup: $artifact $field"
            return 2
            ;;
    esac

    sed -nE "$pattern" "$MANIFEST"
}

verify_artifact() {
    local artifact="$1"
    local name="$2"
    local expected_sha expected_bytes actual_sha actual_bytes

    [[ -f "$artifact" ]] || {
        print -u2 -- "Missing artifact: $artifact"
        return 1
    }

    require_windows_x86 "$artifact"
    expected_sha="$(manifest_value "$name" sha256)"
    expected_bytes="$(manifest_value "$name" bytes)"
    actual_sha="$(shasum -a 256 "$artifact" | awk '{ print $1 }')"
    actual_bytes="$(wc -c < "$artifact" | tr -d '[:space:]')"

    [[ -n "$expected_sha" && -n "$expected_bytes" ]] || {
        print -u2 -- "Manifest has no usable entry for $name"
        return 1
    }
    [[ "$actual_sha" == "$expected_sha" ]] || {
        print -u2 -- "SHA-256 mismatch for $name"
        return 1
    }
    [[ "$actual_bytes" == "$expected_bytes" ]] || {
        print -u2 -- "Size mismatch for $name"
        return 1
    }
}

[[ -f "$MANIFEST" ]] || {
    print -u2 -- "Missing artifact manifest: $MANIFEST"
    exit 1
}

verify_artifact "$DLL" "veyr.dll"
verify_artifact "$LOADER" "veyr.exe"
print -- "Verified Windows x86 artifacts in $DIST_DIR"
