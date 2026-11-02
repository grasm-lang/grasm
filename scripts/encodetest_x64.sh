#!/usr/bin/env bash
# The Grasm backend's x86-64 encoder against the system assembler
# (docs/self-hosting.md §6.16, stage S7: "an encoding gate against `llvm-mc`
# covering every form the rules emit"; `as -arch x86_64` here, the same LLVM
# assembler).
#
# An encoder bug is silent, and on x86-64 more so than on AArch64: an
# instruction is 1 to 15 bytes, so a wrong length shifts everything after it,
# while a wrong REX bit or ModRM field still makes a valid instruction that
# names the wrong register. So every form is encoded twice, by
# `compiler/x86_64` and by `as -arch x86_64` from the AT&T assembly the
# backend prints for the same operands, and the bytes must be identical, case
# by case. Where several encodings spell one instruction the encoder must
# choose the one the assembler chooses (an 8-bit immediate or displacement
# when it fits, none for a displacement of 0 but from rbp or r13, a SIB byte
# for rsp and r12, the short `rax` forms, `d1` for a shift by one), and the
# operands are generated across those edges -- every register in every field,
# immediates, displacements and branch offsets at both ends of each width and
# between -- not one example each.
#
# Three checks:
#
#   * coverage: every form compiled code for the target can contain (`s3
#     --forms --target x86_64`) has test cases;
#   * agreement: the assembler's bytes and the encoder's, case by case, and
#     the total length;
#   * relocations: every case naming a symbol (a call, a RIP-relative
#     address, a GOT load, a thread-local descriptor) carries exactly the
#     relocation the encoder says its last four bytes take, of that type,
#     PC-relative, four bytes wide, against that symbol; and nothing else is
#     relocated.
#
# Its controls are `scripts/x64controls.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver

command -v as >/dev/null 2>&1 || { echo "as is needed as the oracle"; exit 1; }

failed=0
if ! "$S3" --encode-test "$WORK/enc" --target x86_64 >"$WORK/gen.log" 2>&1; then
    echo "the encoder refuses its own test cases:"
    sed 's/^/  /' "$WORK/gen.log"
    exit 1
fi
if ! as -arch x86_64 "$WORK/enc.s" -o "$WORK/enc.o" 2>"$WORK/as.log"; then
    echo "the assembler refuses what the backend prints:"
    head -20 "$WORK/as.log" | sed 's/^/  /'
    exit 1
fi

# Coverage: every form the target can emit has cases.
if ! "$S3" --forms --target x86_64 >"$WORK/forms.txt" 2>&1; then
    echo "s3 --forms --target x86_64 failed"
    sed 's/^/  /' "$WORK/forms.txt"
    exit 1
fi
cut -f1 "$WORK/enc.tsv" | sort -u >"$WORK/tested.txt"
emittable=$(wc -l <"$WORK/forms.txt" | tr -d ' ')
sort -u "$WORK/forms.txt" | comm -23 - "$WORK/tested.txt" >"$WORK/missing.txt"
nmissing=$(wc -l <"$WORK/missing.txt" | tr -d ' ')
if [ "$nmissing" -gt 0 ]; then
    echo "forms the target emits with no test cases:"
    sed 's/^/  /' "$WORK/missing.txt"
    failed=1
fi

python3 scripts/x64_macho.py compare "$WORK/enc.o" "$WORK/enc.tsv" >"$WORK/compare.log" 2>&1
code=$?
cat "$WORK/compare.log"
if [ "$code" != 0 ]; then
    failed=1
fi
echo "emittable forms: $emittable   with no cases: $nmissing"

# A gate that compared a handful has compared nothing. The counts it prints
# are derived, and held to what was generated: the comparison saw every form
# and every case the generator wrote, and the floors are what the encoder had
# when the gate was written (102 forms, 5558 cases, 51 of them relocated).
cases=$(wc -l <"$WORK/enc.tsv" | tr -d ' ')
relocated=$(awk -F'\t' '$5 >= 0' "$WORK/enc.tsv" | wc -l | tr -d ' ')
seen_forms=$(grep -oE '^forms: [0-9]+' "$WORK/compare.log" | grep -oE '[0-9]+')
seen_cases=$(grep -oE 'cases: [0-9]+' "$WORK/compare.log" | grep -oE '[0-9]+')
seen_relocated=$(grep -oE 'relocated: [0-9]+' "$WORK/compare.log" | grep -oE '[0-9]+')
if [ "${seen_forms:-0}" != "$emittable" ] || [ "${seen_cases:-0}" != "$cases" ] || [ "${seen_relocated:-0}" != "$relocated" ]; then
    echo "the comparison saw ${seen_forms:-0} forms, ${seen_cases:-0} cases and ${seen_relocated:-0} relocated;"
    echo "the generator wrote $emittable forms, $cases cases and $relocated relocated"
    exit 1
fi
if [ "$cases" -lt 5000 ] || [ "$emittable" -lt 100 ] || [ "$relocated" -lt 40 ]; then
    echo "only $cases cases over $emittable forms ($relocated relocated); expected over 5000 over 100 (40 relocated)"
    exit 1
fi
exit $failed
