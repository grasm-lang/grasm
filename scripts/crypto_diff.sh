#!/usr/bin/env bash
# Do the crypto primitives agree with an implementation that shares nothing
# with them?
#
# The packages' own tests check every published vector: RFC 8439, RFC 7693,
# RFC 7748, RFC 5869. That proves agreement at the points somebody chose to
# write down, which is a small and non-random set. The defects that survive a
# vector list are the ones that need an input nobody picked — a carry that
# only propagates past the eighth limb, a message that ends exactly on a block
# boundary, a scalar whose top bit makes the last ladder step differ.
#
# So this generates random cases and diffs them against `crypto_diff.py`, which
# computes the same answers using Python's arbitrary-precision integers. That
# reference has no limbs and no carry chain, so it cannot share a bug with the
# ten-limb arithmetic under test — which is the entire point of using it rather
# than a second copy of the same idea.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
CASES=${CRYPTO_DIFF_CASES:-120}
SEED=${CRYPTO_DIFF_SEED:-20260830}
WORK="${TMPDIR:-/tmp}/grasm_cryptodiff_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

if ! python3 -c 'import hashlib; hashlib.blake2s' 2>/dev/null; then
    echo "error: this python has no blake2s, so there is no oracle to compare against"
    exit 1
fi

if ! python3 scripts/crypto_diff.py "$SEED" "$CASES" "$WORK" >"$WORK/gen.log" 2>&1; then
    echo "the reference implementation would not run:"
    cat "$WORK/gen.log"
    exit 1
fi

# Natively, because the interpreter spends about half a second on each scalar
# multiplication and this gate does hundreds of them.
if ! "$GRASM" build packages/cryptodiff/src/main.gr --native -o "$WORK/run" >/dev/null 2>&1; then
    echo "the harness would not build"
    exit 1
fi

if ! "$WORK/run" "$WORK/cases.txt" >"$WORK/got.txt" 2>"$WORK/err.txt"; then
    echo "the harness failed:"
    head -5 "$WORK/err.txt"
    exit 1
fi

total=$(wc -l <"$WORK/want.txt" | tr -d ' ')
if diff -q "$WORK/want.txt" "$WORK/got.txt" >/dev/null 2>&1; then
    echo "cases: $total   agreeing: $total   disagreeing: 0"
    exit 0
fi

bad=$(diff "$WORK/want.txt" "$WORK/got.txt" | grep -c '^<' || true)
echo "cases: $total   agreeing: $((total - bad))   disagreeing: $bad"
echo
echo "the first few, as (case) then (reference) then (grasm):"
diff "$WORK/want.txt" "$WORK/got.txt" | head -20
exit 1
