#!/usr/bin/env bash
# Builds every variant into $OUT (default: a temp dir). Sourced by run.sh and verify.sh.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="${OUT:-${TMPDIR:-/tmp}/dirtyheap-bin}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$OUT/rust-target}"
mkdir -p "$OUT"
"$ROOT/target/release/grasm" build "$HERE/grasm/refs.gr" --native -o "$OUT/grasm-refs" >/dev/null
"$ROOT/target/release/grasm" build "$HERE/grasm/values.gr" --native -o "$OUT/grasm-values" >/dev/null
(cd "$HERE/rust" && CARGO_TARGET_DIR="$CARGO_TARGET_DIR" cargo build --release --offline -q)
cp "$CARGO_TARGET_DIR/release/dirtyheap" "$OUT/rust-arena"
cp "$CARGO_TARGET_DIR/release/dirtyheap_rc" "$OUT/rust-rc"
(cd "$HERE/go" && go build -o "$OUT/go-pointers" .)
VARIANTS=(grasm-refs grasm-values rust-arena rust-rc go-pointers)
