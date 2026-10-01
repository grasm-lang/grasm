#!/usr/bin/env bash
# Builds the upstream, the load generator and the three proxies into $BUILD_DIR.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
BUILD_DIR="${BUILD_DIR:-${TMPDIR:-/tmp}/proxybench-build}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$BUILD_DIR/cargo-target}"
GRASM="${GRASM:-$ROOT/target/release/grasm}"
mkdir -p "$BUILD_DIR"

(cd "$HERE/upstream" && go build -o "$BUILD_DIR/upstream" .)
(cd "$HERE/loadgen" && go build -o "$BUILD_DIR/loadgen" .)
(cd "$HERE/go" && go build -o "$BUILD_DIR/proxy-go" .)
(cd "$HERE/rust" && cargo build --release --offline --quiet)
cp "$CARGO_TARGET_DIR/release/proxybench-rust" "$BUILD_DIR/proxy-rust"
"$GRASM" build "$HERE/grasm/proxy.gr" --native -o "$BUILD_DIR/proxy-grasm" >/dev/null
echo "built into $BUILD_DIR" >&2
