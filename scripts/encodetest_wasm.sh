#!/usr/bin/env bash
# The Grasm backend's wasm32 encoder against LLVM's (docs/self-hosting.md
# §6.24, decision (11)): every instruction form `compiler/wasm32` writes,
# encoded by `compiler/wasm32/src/enc.gr` and by `llvm-mc
# -triple=wasm32-unknown-unknown -show-encoding` from the same instruction in
# LLVM's assembly syntax, the bytes compared case by case. Immediates are
# generated across each LEB128 width's edges -- 63/64, 127/128, the most
# negative and positive of each type -- since a LEB128 that is one byte long
# or short is a valid instruction of another meaning.
#
# The forms LLVM writes as relocations (`call`, `call_indirect`, `global.get`,
# `global.set`, `throw`: a padded five-byte index for a linker to patch) and
# `try_table` (llvm-mc 21.1.8 crashes printing it) are held instead by V8's
# validation and `scripts/wasm_validate.py` in `wasmsubset`, which decode each
# of them in every module it builds.
#
# Its controls are `scripts/wasmcontrols.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver

MC=${LLVM_MC:-/opt/homebrew/opt/llvm@21/bin/llvm-mc}
[ -x "$MC" ] || { echo "error: $MC is missing (Homebrew's llvm@21; LLVM_MC= names another)"; exit 1; }

if ! "$S3" --encode-test "$WORK/enc" --target wasm32 >"$WORK/gen.log" 2>&1; then
    echo "s3 --encode-test --target wasm32 failed:"
    sed 's/^/  /' "$WORK/gen.log"
    exit 1
fi
if ! "$MC" -triple=wasm32-unknown-unknown -mattr=+bulk-memory,+nontrapping-fptoint,+sign-ext \
        -show-encoding --no-type-check "$WORK/enc.s" >"$WORK/mc.txt" 2>"$WORK/mc.err"; then
    echo "llvm-mc refuses what the encoder's cases print:"
    head -20 "$WORK/mc.err" | sed 's/^/  /'
    exit 1
fi
python3 - "$WORK/enc.tsv" "$WORK/mc.txt" <<'EOF'
import re, sys
cases = [l.rstrip("\n").split("\t") for l in open(sys.argv[1]) if l.strip()]
encs = []
for line in open(sys.argv[2]):
    m = re.search(r"# encoding: \[([^\]]*)\]", line)
    if m:
        encs.append("".join(x.strip()[2:].rjust(2, "0") for x in m.group(1).split(",") if x.strip()))
# The function's own `end` is the last encoding; the cases come first.
want_lines = sum(len(c[2].split(";")) for c in cases)
if len(encs) != want_lines + 1:
    print(f"llvm-mc encoded {len(encs)} instructions, the cases have {want_lines} and the end")
    sys.exit(1)
k = 0
bad = 0
forms = set()
for form, ours, asm in cases:
    n = len(asm.split(";"))
    theirs = "".join(encs[k:k + n])
    k += n
    forms.add(form)
    if theirs != ours:
        bad += 1
        if bad <= 20:
            print(f"DIFFERS {asm}: ours {ours}, llvm-mc's {theirs}")
print(f"cases: {len(cases)}   forms: {len(forms)}   agreeing: {len(cases) - bad}   disagreeing: {bad}")
# Floors: a shrinking case list should be noticed.
if len(cases) < 234 or len(forms) < 91:
    print("fewer cases or forms than the floor (234 cases, 91 forms)")
    sys.exit(1)
sys.exit(1 if bad else 0)
EOF
