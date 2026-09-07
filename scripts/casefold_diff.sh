#!/usr/bin/env bash
# Does the case-folding table agree with Unicode, over the whole of Unicode?
#
# `packages/casefold` carries a generated table of every code point that folds.
# The version before it was written by hand from the block layouts, and it was
# wrong in both directions at once: 35 characters folded that should not have
# been, and 445 that fold were missed. A sample would not have found either —
# both live in the parts of Latin Extended-B and Cyrillic where a block's
# parity flips partway through.
#
# So this compares the entire range, all 1,112,064 assignable code points,
# against Python's `str.casefold`, which is the same Unicode full mapping the
# table is generated from.
#
# It also checks the generator's output still matches the committed file: a
# table nobody can regenerate is a table that has stopped being derived.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
WORK="${TMPDIR:-/tmp}/grasm_casefolddiff_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

SRC=packages/casefold/src/main.gr
cp "$SRC" "$WORK/committed.gr"
if ! python3 scripts/casefold_table.py >"$WORK/regen.log" 2>&1; then
    echo "the generator would not run"
    cat "$WORK/regen.log"
    exit 1
fi
if ! diff -q "$WORK/committed.gr" "$SRC" >/dev/null; then
    diff "$WORK/committed.gr" "$SRC" | head -20
    cp "$WORK/committed.gr" "$SRC"
    echo "the committed table is not what the generator produces"
    echo "run: python3 scripts/casefold_table.py"
    exit 1
fi

if ! "$GRASM" build packages/casefolddiff/src/main.gr --native -o "$WORK/run" >/dev/null 2>&1; then
    echo "the harness would not build"
    exit 1
fi
"$WORK/run" > "$WORK/grasm.txt" || {
    echo "the harness did not finish"
    exit 1
}

python3 - "$WORK/grasm.txt" <<'PY'
import sys

mine = {}
for line in open(sys.argv[1]):
    line = line.strip()
    if not line:
        continue
    code, folded = line.split("=")
    mine[int(code)] = [int(x) for x in folded.split(",")]

theirs = {}
for code in range(0x110000):
    if 0xD800 <= code <= 0xDFFF:
        continue
    one = chr(code)
    folded = one.casefold()
    if folded != one:
        theirs[code] = [ord(c) for c in folded]

wrong = sorted(c for c in set(mine) & set(theirs) if mine[c] != theirs[c])
missing = sorted(set(theirs) - set(mine))
extra = sorted(set(mine) - set(theirs))
print(f"folding: {len(theirs)}   wrong: {len(wrong)}   "
      f"missing: {len(missing)}   extra: {len(extra)}")
for code in (wrong + missing + extra)[:12]:
    print(f"  U+{code:04X}: unicode {theirs.get(code)} grasm {mine.get(code)}")
sys.exit(1 if (wrong or missing or extra) else 0)
PY
