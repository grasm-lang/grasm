#!/usr/bin/env bash
# Build the zstd fixtures `scripts/zstdtest.sh` decodes.
#
#     sh scripts/fixtures/zstd/make.sh
#
# Needs the `zstd` CLI. The output is committed, so this only runs when the
# cases change.
#
# The inputs are chosen for the block types they make zstd emit, which is not
# something a caller controls directly:
#
#   random  incompressible, so zstd gives up and emits a **raw** block
#   tiny    three bytes, also raw
#   empty   a raw block of length zero, which is its own edge
#   zeros   highly compressible, so a **compressed** block
#   text    repetitive, a compressed block with raw literals and sequences
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
PY

for f in random zeros text tiny empty; do
    zstd -q -f -1 "$f.bin" -o "$f.zst"
    printf '%-8s %6s -> %5s bytes\n' "$f" "$(wc -c <"$f.bin")" "$(wc -c <"$f.zst")"
done
