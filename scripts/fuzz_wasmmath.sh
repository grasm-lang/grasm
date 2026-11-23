#!/usr/bin/env bash
# The wasm C library's libm against the Rust playground's (docs/self-hosting.md
# §6.24, decision (6)): `compiler/rtwasm/src/math.gr` is Rust's `libm`
# crate's functions ported to Grasm, and must answer as the crate does, bit
# for bit, on every function the runtime calls.
#
# The oracle is the crate itself as the playground has it: `scripts/wasm/
# libmprobe.rs`, `f64`'s methods built by rustc for `wasm32-unknown-unknown`
# (the code rustc links into `grasm_wasm.wasm`). The subject is
# `scripts/fixtures/wasmmath/mathfuzz.gr`, built by `s3 --target wasm32`,
# calling the library's functions by their C names on seeded arguments --
# a quarter any 64 bits at all, the rest across each function's range -- and
# printing each call's bits; `scripts/wasm/mathcmp.mjs` gives the probe the
# same arguments. A NaN is any NaN.
#
#   FUZZ_WASMMATH_SEEDS (default "1 2 3 4 5"), FUZZ_WASMMATH_CALLS (calls per
#   function per seed, default 40000): 24 functions, so 4,800,000 calls.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
SEEDS=${FUZZ_WASMMATH_SEEDS:-1 2 3 4 5}
CALLS=${FUZZ_WASMMATH_CALLS:-40000}
RUSTC=${RUSTC:-rustc}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver
command -v node >/dev/null 2>&1 || { echo "error: node is needed"; exit 1; }
if ! "$RUSTC" --target wasm32-unknown-unknown --crate-type cdylib -O scripts/wasm/libmprobe.rs -o "$WORK/probe.wasm" >"$WORK/rustc.log" 2>&1; then
    echo "error: rustc cannot build the probe for wasm32-unknown-unknown (RUSTC= names another):"
    head -5 "$WORK/rustc.log"
    exit 1
fi
if ! "$GRASM" tir compiler/rt/src/main.gr >"$WORK/rt.tir" ||
    ! "$GRASM" tir "${WASM_LIBC:-compiler/rtwasm}/src/main.gr" >"$WORK/rtwasm.tir" ||
    ! "$GRASM" tir scripts/fixtures/wasmmath/mathfuzz.gr >"$WORK/fuzz.tir" ||
    ! "$S3" "$WORK/fuzz.tir" -o "$WORK/fuzz.wasm" --target wasm32 --runtime-tir "$WORK/rt.tir" --libc-tir "$WORK/rtwasm.tir" >"$WORK/s3.log" 2>&1; then
    echo "error: the fuzzer does not build:"
    head -5 "$WORK/s3.log"
    exit 1
fi
total=0
status=0
for seed in $SEEDS; do
    node scripts/wasm/runprog.mjs "$WORK/fuzz.wasm" "$seed" "$CALLS" >"$WORK/calls.txt" 2>"$WORK/err.txt" || {
        echo "the fuzzer failed on seed $seed:"; head -5 "$WORK/err.txt"; exit 1; }
    n=$(wc -l <"$WORK/calls.txt" | tr -d ' ')
    if [ "$n" -ne $((24 * CALLS)) ]; then
        echo "seed $seed: $n calls, not $((24 * CALLS))"
        exit 1
    fi
    total=$((total + n))
    echo "seed $seed: $(node scripts/wasm/mathcmp.mjs "$WORK/probe.wasm" <"$WORK/calls.txt" | tee "$WORK/cmp.txt" | tail -1)"
    if grep -c '^DIFFERS' "$WORK/cmp.txt" >/dev/null; then
        grep '^DIFFERS' "$WORK/cmp.txt" | head -10
        status=1
    fi
done
echo "fuzz_wasmmath: $total calls compared with the Rust playground's libm"
exit $status
