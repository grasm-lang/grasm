#!/usr/bin/env bash
# Build a sample of the corpus natively, keeping symbols, so the emitted code
# can be mined. Each binary is disassembled once and thrown away.
cd "$(dirname "$0")/.."
S=${TMPDIR:-/tmp}/grasm-mine/mine
mkdir -p "$S" "$S/mine"
: > $S/calls.txt
n=0
for d in packages/*/; do
    f="$d/src/main.gr"
    [ -f "$f" ] || continue
    grep -q "^fn main" "$f" || continue
    name=$(basename "$d")
    (cd "$d" && GRASM_KEEP_SYMBOLS=1 "$(dirname "$0")/../target/release/grasm" \
        build --native src/main.gr -o "$S/bin" >/dev/null 2>&1) || continue
    [ -f "$S/bin" ] || continue
    otool -tV "$S/bin" 2>/dev/null | awk -v p="$name" '
        /^_/ { fn=$0; sub(":","",fn) }
        /	bl	_grasm_/ { c=$NF; sub("^_","",c); print p"\t"fn"\t"c }
    ' >> $S/calls.txt
    rm -f "$S/bin"
    n=$((n+1))
done
echo "built $n programs"
wc -l < $S/calls.txt
