#!/usr/bin/env bash
# `bytes` and the builders on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/bytesfuzz.gr` makes buffers, windows sharing them,
# writes into shared and unshared ones (a shared one is copied first, and the
# others must not see the write), fills, copies, joins and searches them,
# makes text of them valid and not (each invalid sequence one U+FFFD, as
# Rust's `from_utf8_lossy` replaces it), and grows builders of text and of
# bytes, shared and not. It is built twice, linked as it ships and with the
# Grasm runtime interposed (`scripts/lib/rtswap.sh`); the swapped build must
# take the family's names from the Grasm runtime, and every seed must print
# and exit the same from both -- and so must each of the family's panics
# the program can end in.
#
#   RTBYTES_SEEDS (20) seeds of RTBYTES_ROUNDS (300) rounds. `RT_OBJ=`, `S3=`
#   and `LINK=` as for `scripts/rtswap.sh`.
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

SEEDS=${RTBYTES_SEEDS:-20}
ROUNDS=${RTBYTES_ROUNDS:-300}

if ! "$GRASM" build compiler/rt/probe/bytesfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/bytesfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in bytes_of bytes_slice bytes_set bytes_to_str bytes_concat bytebuf_push_byte builder_push_str builder_to_str; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done
taken=$(grep -cE '^grasm (bytes|bytebuf|builder|strbuf)' "$WORK/f.swaplog")

# Runs `$@` after the seed on both builds; fails saying how they differ.
both() {
    local k=$1
    shift
    "$WORK/f" "$k" "$@" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    "$WORK/f.swap" "$k" "$@" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    if [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.out" "$WORK/b.out" || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $k ($*): the two runtimes differ (exit $ac and $bc)"
        diff "$WORK/a.out" "$WORK/b.out" | head -6 | cut -c1-200 | sed 's/^/    /'
        diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
        return 1
    fi
}

same=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    both "$k" "$ROUNDS" || exit 1
    # A round's text may hold a newline of its own, so its lines are not
    # counted; it must finish, and say something.
    if [ "$ac" != 0 ] || [ ! -s "$WORK/a.out" ]; then
        echo "FAIL: seed $k: exit $ac"
        exit 1
    fi
    same=$((same + 1))
    k=$((k + 1))
done
panics=0
p=0
while [ "$p" -le 5 ]; do
    both 1 3 "$p" || exit 1
    if [ "$ac" != 101 ] || ! grep -q '^panic: ' "$WORK/a.err"; then
        echo "FAIL: panic case $p did not panic (exit $ac)"
        exit 1
    fi
    panics=$((panics + 1))
    p=$((p + 1))
done
echo "seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds)   panics identical: $panics of 6   names of the family taken from Grasm: $taken"
[ "$same" -eq "$SEEDS" ] && [ "$SEEDS" -ge 1 ]
