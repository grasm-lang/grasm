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
#   prose     words from a small vocabulary, which is what makes zstd reach
#             for Huffman-coded literals rather than storing them
#   skew      bytes from a skewed sixteen-letter alphabet with few repeats, so
#             most of it stays literal and the literal section grows past the
#             point where zstd splits it into **four streams**
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
words = "alpha beta gamma delta epsilon zeta eta theta iota kappa".split()
open("prose.bin", "wb").write(
    " ".join(random.choice(words) for _ in range(900)).encode())
random.seed(5)
alpha = bytes(range(97, 113))
weights = [40, 25, 15, 9, 5, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1]
skew = bytearray()
while len(skew) < 9000:
    skew.append(random.choices(alpha, weights=weights)[0])
open("skew.bin", "wb").write(bytes(skew))
# A longer skewed run, for the one fixture that catches a repeat offset
# crossing a block boundary. The parameters are not arbitrary: they were found
# by building a decoder with the bug and searching for the smallest input the
# two disagree on.
random.seed(1)
alpha2 = bytes(range(97, 97 + random.choice([6, 10, 16])))
w2 = [max(1, 40 // (i + 1)) for i in range(len(alpha2))]
n2 = random.choice([6000, 12000, 20000])
carry = bytearray()
while len(carry) < n2:
    carry.append(random.choices(alpha2, weights=w2)[0])
open("carry.bin", "wb").write(bytes(carry))
PY

for f in random zeros text tiny empty onebyte runs halfrand prose skew; do
    for lvl in 1 9; do
        out="$f.$lvl.zst"
        [ "$lvl" = 1 ] && out="$f.zst"
        zstd -q -f "-$lvl" "$f.bin" -o "$out"
    done
    printf '%-10s %7s -> %5s / %5s bytes (level 1 / 9)\n' \
        "$f" "$(wc -c <"$f.bin")" "$(wc -c <"$f.zst")" "$(wc -c <"$f.9.zst")"
done

# Several blocks from a small input, so a block can say "the same Huffman tree
# as last time" rather than describing one.
zstd -q -f -5 --target-compressed-block-size=1500 skew.bin -o skew.blocks.zst
printf '%-12s %7s -> %5s bytes (several blocks, reused trees)\n' \
    "skew.blocks" "$(wc -c <skew.bin)" "$(wc -c <skew.blocks.zst)"

# And the one fixture that catches a *repeat offset* crossing a block
# boundary. The three most recent offsets are frame state, not block state,
# and resetting them per block is a bug every other fixture here misses --
# including skew.blocks, which has the reused trees but never names a repeat
# offset as a block's first sequence. This input was found by building a
# decoder with the bug and searching for the smallest file the two disagree
# on, which is the only honest way to know a gate covers something.
zstd -q -f -5 --target-compressed-block-size=1500 carry.bin -o carry.blocks.zst
printf '%-12s %7s -> %5s bytes (repeat offset across a block)\n' \
    "carry.blocks" "$(wc -c <carry.bin)" "$(wc -c <carry.blocks.zst)"
