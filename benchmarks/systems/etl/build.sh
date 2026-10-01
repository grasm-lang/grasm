#!/usr/bin/env bash
# Builds the generator and every implementation into $OUT, and provides
# `ensure_input SIZE PATH`. Sourced by run.sh and verify.sh.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
SCRATCH="${TMPDIR:-/tmp}/grasm-systems-etl"
OUT="${OUT:-$SCRATCH/etl-bin}"
DATA="${ETL_DATA:-$SCRATCH/etl-data}"
SEED="${SEED:-42}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRATCH/systems-target}"
export GOCACHE="${GOCACHE:-$SCRATCH/gocache}"
mkdir -p "$OUT" "$DATA"

(cd "$HERE/gen" && go build -o "$OUT/gen" .)
"$ROOT/target/release/grasm" build "$HERE/grasm/src/main.gr" --native -o "$OUT/grasm" >/dev/null
(cd "$HERE/rust" && cargo build --release --offline -q)
cp "$CARGO_TARGET_DIR/release/etl" "$OUT/rust"
(cd "$HERE/go" && go build -o "$OUT/go" .)
VARIANTS=(grasm rust go)

# ensure_input SIZE PATH: generate PATH at SIZE bytes unless it is already there.
ensure_input() {
    local size="$1" path="$2"
    if [ ! -s "$path" ]; then
        "$OUT/gen" -size "$size" -seed "$SEED" -out "$path"
    fi
}

# A freshly written executable pays a one-off macOS launch cost (~0.5 s) the
# first time it runs; run each binary once on a tiny input so that cost is not
# counted against whichever language happens to be measured.
warm_up() {
    ensure_input 20000 "$DATA/etl-tiny.jsonl"
    for v in "${VARIANTS[@]}"; do "$OUT/$v" "$DATA/etl-tiny.jsonl" >/dev/null; done
}
