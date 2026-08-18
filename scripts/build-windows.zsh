#!/usr/bin/env zsh
# Build the x86 Windows deliverables and atomically publish them to dist/.
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
PROJECT_ROOT="${SCRIPT_DIR:h}"
source "$SCRIPT_DIR/activate-rust.zsh"

cd "$PROJECT_ROOT"
veyr_build_windows "$@"
