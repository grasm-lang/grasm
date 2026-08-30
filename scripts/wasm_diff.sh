#!/usr/bin/env bash
# Does the WebAssembly interpreter agree with a real one?
#
# The package's own tests use modules somebody assembled by hand, which proves
# the instructions somebody thought of. The ones that go wrong are where a
# 64-bit host's habits leak into a 32-bit machine: which way division
# truncates, whose sign a remainder follows, the most negative number divided
# by minus one. Those turn up in random expressions.
#
# The reference is the WebAssembly engine in Node, which is V8's — the same one
# in a browser. It also *validates* each module, so a generator that emitted
# something illegal is caught rather than compared against a guess.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
CASES=${WASM_DIFF_CASES:-150}
SEED=${WASM_DIFF_SEED:-20260830}
WORK="${TMPDIR:-/tmp}/grasm_wasmdiff_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if ! command -v node >/dev/null 2>&1; then
    echo "node is not installed, so there is no reference engine to compare against"
    echo "cases: 0   agreeing: 0   skipped: 1"
    exit 0
fi
if ! "$GRASM" build packages/wasmdiff/src/main.gr --native -o "$WORK/run" >/dev/null 2>&1; then
    echo "the harness would not build"
    exit 1
fi

python3 scripts/wasm_diff.py "$SEED" "$CASES" "$WORK" >/dev/null || {
    echo "the generator failed"; exit 1; }

"$WORK/run" "$WORK/cases.txt" > "$WORK/ours.txt" 2>"$WORK/err.txt" || {
    echo "the harness failed:"; head -3 "$WORK/err.txt"; exit 1; }

node -e '
const fs = require("fs");
const lines = fs.readFileSync(process.argv[1], "utf8").split("\n").filter(Boolean);
const out = [];
for (const line of lines) {
  const p = line.split(" ");
  const buf = Buffer.from(p[0], "hex");
  try {
    const inst = new WebAssembly.Instance(new WebAssembly.Module(buf));
    try { out.push(String(inst.exports[p[1]](Number(p[2]), Number(p[3])) >>> 0)); }
    catch (e) { out.push("TRAP"); }
  } catch (e) { out.push("REFUSED"); }
}
fs.writeFileSync(process.argv[2], out.join("\n") + "\n");
' "$WORK/cases.txt" "$WORK/theirs.txt"

total=$(wc -l < "$WORK/theirs.txt" | tr -d ' ')
# A run where the reference refused everything would compare nothing and pass,
# so what it accepted is counted rather than assumed.
valid=$(grep -cv '^REFUSED$' "$WORK/theirs.txt" || true)
if [ "$valid" -lt $((total / 2)) ]; then
    echo "the reference engine refused $((total - valid)) of $total modules — the generator is emitting something illegal"
    exit 1
fi

if diff -q "$WORK/ours.txt" "$WORK/theirs.txt" >/dev/null 2>&1; then
    echo "cases: $total   agreeing: $total   disagreeing: 0   traps: $(grep -c '^TRAP$' "$WORK/theirs.txt" || true)"
    exit 0
fi
bad=$(diff "$WORK/ours.txt" "$WORK/theirs.txt" | grep -c '^<' || true)
echo "cases: $total   agreeing: $((total - bad))   disagreeing: $bad"
paste -d' ' <(cut -c1-20 "$WORK/cases.txt") "$WORK/ours.txt" "$WORK/theirs.txt" \
    | awk '$2 != $3 {print "  ours " $2 "  theirs " $3}' | head -6
exit 1
