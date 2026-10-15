#!/usr/bin/env bash
# Build the zstd fixtures `scripts/zstdtest.sh` decodes.
#
#     sh scripts/fixtures/zstd/make.sh
#
# Needs the `zstd` CLI. The output is committed, so this only runs when the
# cases change.
#
# The inputs are chosen for what they make zstd *emit*, which is not something
# a caller controls directly. Several compression levels, because the level
# changes the block and entropy choices rather than only the ratio:
#
#   random    incompressible, so zstd gives up and writes a **raw** block
#   tiny      three bytes, also raw
#   empty     a raw block of length zero, which is its own edge
#   zeros     a compressed block: RLE-ish, one long match at offset 1
#   text      a compressed block with raw literals and two sequences
#   runs      many short runs, which pushes the match lengths around
#   halfrand  incompressible then compressible, so the block types differ
#             inside one frame
#   onebyte   a single byte, the smallest frame that carries anything
#
# `expect.txt` says which of them this decoder handles today. A fixture that
# must fail is as much a check as one that must pass: it is how "refused by
# name" stays true rather than becoming "silently wrong".
set -eu
cd "$(dirname "$0")"

python3 - <<'PY'
import random
random.seed(11)
open("random.bin", "wb").write(bytes(random.randrange(256) for _ in range(4000)))
open("zeros.bin", "wb").write(b"\0" * 4000)
open("text.bin", "wb").write(("the quick brown fox jumps over the lazy dog. " * 120).encode())
open("tiny.bin", "wb").write(b"abc")
open("empty.bin", "wb").write(b"")
open("onebyte.bin", "wb").write(b"Q")
random.seed(23)
open("runs.bin", "wb").write(
    b"".join(bytes([random.randrange(256)]) * random.randrange(1, 40) for _ in range(300)))
open("halfrand.bin", "wb").write(
    bytes(random.randrange(256) for _ in range(1500)) + b"x" * 1500)
PY

for f in random zeros text tiny empty onebyte runs halfrand; do
    for lvl in 1 9; do
        out="$f.$lvl.zst"
        [ "$lvl" = 1 ] && out="$f.zst"
        zstd -q -f "-$lvl" "$f.bin" -o "$out"
    done
    printf '%-10s %7s -> %5s / %5s bytes (level 1 / 9)\n' \
        "$f" "$(wc -c <"$f.bin")" "$(wc -c <"$f.zst")" "$(wc -c <"$f.9.zst")"
done
