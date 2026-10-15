#!/usr/bin/env bash
# Does `packages/zstd` decompress what the zstd CLI produced?
#
# Real frames, written by `zstd -1`, with the content checksum present --
# so this checks the header's awkward parts as well as the blocks: the
# two-byte content size that stores its value minus 256, the single-segment
# flag that changes how wide that field is, and the trailing checksum that
# has to be stepped over.
#
# Two compression levels for every input, because the level changes the block
# and entropy choices rather than only the ratio.
#
# `scripts/fixtures/zstd/expect.txt` says which fixtures must decode and which
# must be refused. The refusals are checked too: a decompressor that guesses
# is worse than one that stops, because a wrong answer from a decompressor is
# indistinguishable from a right one until something downstream fails.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
FIX=scripts/fixtures/zstd
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/zstd-demo/src/main.gr --native -o "$work/dec" \
    >"$work/build.log" 2>&1; then
    echo "    the decoder did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

ok=0
refused=0
while read -r name status _rest; do
    case "$name" in ''|\#*) continue ;; esac
    zst="$FIX/$name.zst"
    # The name before the first dot is the source: `text.9` checks `text.bin`.
    src="$FIX/${name%%.*}.bin"
    [ -f "$zst" ] || { echo "    $zst is missing; run $FIX/make.sh"; exit 1; }
    if [ "$status" = "refused" ]; then
        if "$work/dec" "$zst" "$work/out" >/dev/null 2>&1; then
            echo "    $name decoded, but expect.txt says it is refused"
            echo "    if compressed blocks now work, move the line to \`ok\`"
            exit 1
        fi
        refused=$((refused + 1))
        continue
    fi
    if ! "$work/dec" "$zst" "$work/out" >"$work/said" 2>&1; then
        echo "    $name did not decode: $(cat "$work/said")"
        exit 1
    fi
    if ! cmp -s "$src" "$work/out"; then
        echo "    $name does not round-trip:"
        echo "      $(wc -c <"$src") bytes in, $(wc -c <"$work/out") out"
        exit 1
    fi
    ok=$((ok + 1))
done < "$FIX/expect.txt"

echo "    frames: $ok exact   $refused refused as declared"
# A manifest that shrank is a gate that stopped checking (F76).
[ "$ok" -ge 22 ]
