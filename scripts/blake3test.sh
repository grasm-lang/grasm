#!/usr/bin/env bash
# Does `packages/blake3` agree with the BLAKE3 reference implementation?
#
# The digests in `scripts/fixtures/blake3.txt` came from the reference, not
# from this repository's hash, so this is a comparison rather than a
# self-portrait. They are pinned rather than computed so the gate needs no
# network and no Python package.
#
# The lengths matter more than the count. BLAKE3 is a binary tree over 1 KiB
# chunks, and the tree is *not* balanced when the input is not a power of two
# chunks: the left subtree takes the largest power of two below the total. A
# hash with that wrong is correct for every input up to 2048 bytes and wrong
# at 3072, so a test that stops at one chunk proves almost nothing.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
VEC=scripts/fixtures/blake3.txt
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/blake3-demo/src/main.gr --native -o "$work/b3" \
    >"$work/build.log" 2>&1; then
    echo "    the hash did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
tree=0
while read -r n want; do
    case "$n" in ''|\#*) continue ;; esac
    got=$("$work/b3" "$n")
    if [ "$got" != "$want" ]; then
        echo "    $n bytes:"
        echo "      got  $got"
        echo "      want $want"
        exit 1
    fi
    checked=$((checked + 1))
    [ "$n" -gt 2048 ] && tree=$((tree + 1))
done < "$VEC"

echo "    lengths: $checked   past one chunk: $tree   all match the reference"
# A vector file that shrank is a gate that stopped checking (F76), and one
# with no multi-chunk case does not check the tree at all.
[ "$checked" -ge 18 ] && [ "$tree" -ge 3 ]
