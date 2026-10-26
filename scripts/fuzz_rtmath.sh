#!/usr/bin/env bash
# The math family in the Grasm runtime must answer the Rust runtime's bits
# (docs/self-hosting.md §6.7).
#
# `compiler/rt/probe/mathfuzz.gr` calls every one of the family's entry
# points by its C name -- the backends inline `sqrt`, `floor`, `ceil` and
# `abs`, so no corpus program reaches those -- over any 64 bits, the special
# values (zeros, infinities, quiet and signalling NaNs with payloads,
# subnormals, the extremes), ties and ordinary values, and prints every
# answer's bits. It is built twice, linked as it ships and with the Grasm
# runtime interposed (`scripts/lib/rtswap.sh`), and each chunk of draws must
# give the same bytes and exit status from both; the last call panics, so
# the exit status and the message are compared too.
#
#   RTMATH_CHUNKS (10) x RTMATH_COUNT (50000) draws, from RTMATH_SEED (1) up.
# `RT_OBJ=`, `S3=` and `LINK=` as for `scripts/rtswap.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver
rtswap_runtime
rtswap_families
rtswap_linker

CHUNKS=${RTMATH_CHUNKS:-10}
COUNT=${RTMATH_COUNT:-50000}
SEED=${RTMATH_SEED:-1}

if ! "$GRASM" build compiler/rt/probe/mathfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/mathfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
# Every math name but the two it cannot declare must have been taken from
# Grasm, or the comparison compared nothing for that name.
missing=0
for need in $(awk '$1 == "math" { print $2 }' compiler/rt/families.txt); do
    case "$need" in f64_is_nan | f64_is_finite) continue ;; esac
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime"
        missing=$((missing + 1))
    fi
done
[ "$missing" = 0 ] || exit 1
taken=$(grep -c '^grasm ' "$WORK/f.swaplog")

same=0
k=0
while [ "$k" -lt "$CHUNKS" ]; do
    seed=$((SEED + k))
    a=$("$WORK/f" "$seed" "$COUNT" </dev/null 2>"$WORK/a.err" | shasum -a 256)
    ac=${PIPESTATUS[0]}
    b=$("$WORK/f.swap" "$seed" "$COUNT" </dev/null 2>"$WORK/b.err" | shasum -a 256)
    bc=${PIPESTATUS[0]}
    if [ "$a" != "$b" ] || [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $seed: the two runtimes differ (exit $ac and $bc)"
        "$WORK/f" "$seed" "$COUNT" >"$WORK/a.out" 2>/dev/null </dev/null
        "$WORK/f.swap" "$seed" "$COUNT" >"$WORK/b.out" 2>/dev/null </dev/null
        diff "$WORK/a.out" "$WORK/b.out" | head -8 | cut -c1-200 | sed 's/^/    /'
        exit 1
    fi
    if [ "$ac" != 101 ] || ! grep -q '^panic: negative integer exponent' "$WORK/a.err"; then
        echo "FAIL: seed $seed: the run did not end in the negative-exponent panic (exit $ac)"
        exit 1
    fi
    same=$((same + 1))
    k=$((k + 1))
done
draws=$((CHUNKS * COUNT))
echo "chunks identical: $same of $CHUNKS ($draws draws, $((draws * 33)) calls of 33 entry points on each runtime); names taken from Grasm: $taken"
[ "$same" -eq "$CHUNKS" ] && [ "$CHUNKS" -ge 1 ]
