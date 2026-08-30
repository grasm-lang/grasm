#!/usr/bin/env bash
# Does the DEFLATE codec agree with a real zlib, both ways?
#
# The package's own tests use ten streams somebody chose. The shapes that break
# a decoder are the ones nobody chooses — a distance reaching exactly to the
# first byte, a length symbol at the top of its range, a dynamic table whose
# repeat codes land on a boundary — and they turn up in random data at every
# compression level, which is what this generates.
#
# Both directions, because a codec that only agrees with itself has not been
# checked. Python's zlib compresses and Grasm decompresses; Grasm compresses
# and Python decompresses.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
CASES=${ZLIB_DIFF_CASES:-60}
SEED=${ZLIB_DIFF_SEED:-20260830}
WORK="${TMPDIR:-/tmp}/grasm_zlibdiff_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if ! "$GRASM" build packages/zlibdiff/src/main.gr --native -o "$WORK/run" >/dev/null 2>&1; then
    echo "the harness would not build"
    exit 1
fi

python3 - "$SEED" "$CASES" "$WORK" <<'PY'
import random, sys, zlib, os
random.seed(int(sys.argv[1])); N = int(sys.argv[2]); OUT = sys.argv[3]
cases, want = [], []

def body(kind, n):
    if kind == 0:   # random, essentially incompressible
        return bytes(random.randrange(256) for _ in range(n))
    if kind == 1:   # long runs, which exercise overlapping copies
        return bytes([random.randrange(4)]) * n
    if kind == 2:   # repeated text, which produces dynamic tables
        return (b"the quick brown fox jumps over the lazy dog. " * (n // 44 + 1))[:n]
    if kind == 3:   # a small alphabet, which makes short codes
        return bytes(random.choice(b"abcab") for _ in range(n))
    return bytes((i * 37 + 11) % 256 for i in range(n))

for i in range(N):
    n = random.choice([0, 1, 2, 3, 17, 255, 256, 257, 1000, 4096, 32768, 65535, 65536, 70000])
    raw = body(i % 5, n)
    level = random.choice([0, 1, 6, 9])
    cases.append("inflate " + zlib.compress(raw, level).hex())
    # A sentinel rather than the empty string: an empty payload is a real case
    # and a blank line would be skipped by the comparison below, which is how
    # the first version of this gate quietly checked 118 of its 120 cases.
    want.append("INFLATE:" + raw.hex())
    # And the other way: what Grasm writes, a real zlib must read.
    cases.append("deflate " + raw.hex())
    want.append("ROUNDTRIP:" + raw.hex())

open(os.path.join(OUT, "cases.txt"), "w").write("\n".join(cases) + "\n")
open(os.path.join(OUT, "want.txt"), "w").write("\n".join(want) + "\n")
print(f"{len(cases)} cases written")
PY

if ! "$WORK/run" "$WORK/cases.txt" >"$WORK/got.txt" 2>"$WORK/err.txt"; then
    echo "the harness failed:"; head -5 "$WORK/err.txt"; exit 1
fi

python3 - "$WORK" <<'PY'
import sys, zlib, os
W = sys.argv[1]
want = open(os.path.join(W, "want.txt")).read().split("\n")
got = open(os.path.join(W, "got.txt")).read().split("\n")
agree = 0; bad = []
for i, w in enumerate(want):
    if not w: continue
    g = got[i] if i < len(got) else "<missing>"
    if w.startswith("ROUNDTRIP:"):
        raw = bytes.fromhex(w[10:])
        try:
            back = zlib.decompress(bytes.fromhex(g))
            if back == raw: agree += 1
            else: bad.append(f"line {i}: real zlib read what Grasm wrote as different bytes")
        except Exception as e:
            bad.append(f"line {i}: real zlib refused a {len(raw)}-byte stream Grasm wrote — {e}")
    else:
        expect = w[8:] if w.startswith("INFLATE:") else w
        if g == expect: agree += 1
        else: bad.append(f"line {i}: inflate disagreed — wanted {len(expect)//2} bytes, got {g[:40]!r}")
total = agree + len(bad)
print(f"cases: {total}   agreeing: {agree}   disagreeing: {len(bad)}")
for b in bad[:8]: print("  " + b)
sys.exit(1 if bad else 0)
PY
