#!/usr/bin/env bash
# Builds every variant into $BUILD_DIR. Sourced by run.sh and verify.sh.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
GRASM="${GRASM:-$REPO/target/release/grasm}"
BUILD_DIR="${BUILD_DIR:-${TMPDIR:-/tmp}/grasm-systems-ecs}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$BUILD_DIR/cargo-target}"
mkdir -p "$BUILD_DIR"

[ -x "$GRASM" ] || { echo "error: grasm compiler not found at $GRASM" >&2; exit 1; }

"$GRASM" build "$HERE/grasm/ecs.gr" --native -o "$BUILD_DIR/grasm_aos" >/dev/null
"$GRASM" build "$HERE/grasm/ecs_soa.gr" --native -o "$BUILD_DIR/grasm_soa" >/dev/null
(cd "$HERE/rust" && cargo build --release --offline --quiet)
cp "$CARGO_TARGET_DIR/release/systems_ecs" "$BUILD_DIR/rust"
(cd "$HERE/go" && go build -o "$BUILD_DIR/go" .)

VARIANTS=(grasm_aos grasm_soa rust go)
