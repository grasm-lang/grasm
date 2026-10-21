#!/usr/bin/env bash
# The Grasm backend's AArch64 encoder against the system assembler
# (docs/self-hosting.md, stage S3: "an encoding gate ... covering every form
# the rules emit").
#
# An encoder bug is silent: a wrong bit in a register field still makes a
# valid instruction, and only some program, some day, computes the wrong
# thing. So every instruction form is encoded twice, by `compiler/aarch64`
# and by `clang -c -arch arm64` from the assembly the backend prints for the
# same operands, and the bytes must be identical. The operands are generated
# across their ranges -- every register in every field, immediates at both
# ends and between, every condition, branch offsets to the limit of their
# reach -- not one example each.
#
# Three checks:
#
#   * the rule table's own checks (`s3 --check-rules`): no rule shadowed by
#     an earlier one, every MIR operation covered, every template well-formed,
#     and the condition map checked against what each condition means;
#   * coverage: every form compiled code can contain (`s3 --forms`, derived
#     from the rules and the frame code) has test cases;
#   * agreement: the assembler's words and the encoder's, case by case.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver

command -v clang >/dev/null 2>&1 || { echo "clang is needed as the oracle"; exit 1; }

failed=0
if ! "$S3" --check-rules >"$WORK/rules.log" 2>&1; then
    echo "the rule table fails its checks:"
    sed 's/^/  /' "$WORK/rules.log"
    failed=1
else
    cat "$WORK/rules.log"
fi

if ! "$S3" --encode-test "$WORK/enc" >"$WORK/gen.log" 2>&1; then
    echo "the encoder refuses its own test cases:"
    sed 's/^/  /' "$WORK/gen.log"
    exit 1
fi
if ! clang -c -arch arm64 "$WORK/enc.s" -o "$WORK/enc.o" 2>"$WORK/as.log"; then
    echo "the assembler refuses what the backend prints:"
    head -20 "$WORK/as.log" | sed 's/^/  /'
    exit 1
fi

# Coverage: every form the backend can emit has cases.
if ! "$S3" --forms >"$WORK/forms.txt" 2>&1; then
    echo "s3 --forms failed"
    exit 1
fi
cut -f1 "$WORK/enc.tsv" | sort -u >"$WORK/tested.txt"
emittable=$(wc -l <"$WORK/forms.txt" | tr -d ' ')
sort -u "$WORK/forms.txt" | comm -23 - "$WORK/tested.txt" >"$WORK/missing.txt"
nmissing=$(wc -l <"$WORK/missing.txt" | tr -d ' ')
if [ "$nmissing" -gt 0 ]; then
    echo "forms the backend emits with no test cases:"
    sed 's/^/  /' "$WORK/missing.txt"
    failed=1
fi

if ! python3 scripts/s3_macho.py compare "$WORK/enc.o" "$WORK/enc.tsv"; then
    failed=1
fi
echo "emittable forms: $emittable   with no cases: $nmissing"

# A gate that compared a handful has compared nothing: the forms the rules
# reach are about forty, at sixty-odd cases each.
cases=$(wc -l <"$WORK/enc.tsv" | tr -d ' ')
if [ "$cases" -lt 2000 ] || [ "$emittable" -lt 40 ]; then
    echo "only $cases cases over $emittable forms; expected over 2000 over 40"
    exit 1
fi
exit $failed
