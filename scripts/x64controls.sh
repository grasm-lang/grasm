#!/usr/bin/env bash
# Mutation controls for the x86-64 target's gates (docs/self-hosting.md §6.16,
# stage S7): a gate that cannot fail checks nothing.
#
# Each control copies `compiler/` aside, plants one fault in the x86-64
# target (`scripts/s3_mutate.py`, which refuses a mutation that matches
# nothing), builds that broken driver, and runs the gate named for it with
# `S3=` pointing at it. The gate must fail, and fail for the planted reason:
# its log must show the symptom that fault produces.
#
#   opcode-bit   `add`'s register form is encoded with its opcode one bit off
#                (03, `add` the other way round, for 01) -> encodetest_x64
#   rex-r        the REX prefix's R bit, the fourth bit of the ModRM `reg`
#                field, is written where W goes -> encodetest_x64
#   imm8-edge    an immediate of 128 is taken to fit in 8 bits, so it is
#                written as -128 -> encodetest_x64
#   sib          a base of rsp or r12 gets no SIB byte, so the address means
#                another register -> encodetest_x64
#   disp-rbp     a displacement of 0 from rbp or r13 is left out, which the
#                processor reads as RIP-relative -> encodetest_x64
#   lock-order   a 16-bit locked instruction puts `lock` before the
#                operand-size prefix, the other order to the assembler's
#                -> encodetest_x64
#   jmp-rel      a short branch's displacement is written one byte long
#                -> encodetest_x64
#   reloc-kind   the RIP-relative address of a symbol asks for a GOT load's
#                relocation -> encodetest_x64 (the relocation check)
#   coverage     the target says it can emit a form the encoder has no cases
#                for -> encodetest_x64 (the coverage check)
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

controls=0
caught=0
declare -a MISSED=()

# control NAME FILE OLD NEW -- GATE SYMPTOM
control() {
    local name=$1 file=$2 old=$3 new=$4 gate=$6 symptom=$7
    controls=$((controls + 1))
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src"
    cp -R compiler "$WORK/src/"
    if ! python3 scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    local driver="$WORK/s3-$name"
    if ! "$GRASM" build "$WORK/src/compiler/s3/src/main.gr" --native -o "$driver" >"$WORK/build-$name.log" 2>&1; then
        MISSED+=("$name: the mutated backend does not build")
        sed 's/^/    /' "$WORK/build-$name.log" | head -5
        return
    fi
    local log="$WORK/$name.log"
    S3="$driver" "$gate" >"$log" 2>&1
    local code=$?
    if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
    elif [ "$code" != 0 ]; then
        MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"; see its last lines below")
        tail -5 "$log" | sed 's/^/    /'
    else
        MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
    fi
}

M=compiler/x86_64/src/mach.gr

control opcode-bit $M \
    $'        let op = match m {\n            1 -> 1\n' \
    $'        let op = match m {\n            1 -> 3\n' -- \
    scripts/encodetest_x64.sh "MISMATCH add-rr"

control rex-r $M \
    "    v = v.bit_or(r.shr(3).bit_and(1).shl(2))" \
    "    v = v.bit_or(r.shr(3).bit_and(1).shl(3))" -- \
    scripts/encodetest_x64.sh "MISMATCH add-rr"

control imm8-edge $M \
    "    v >= -128 and v <= 127" "    v >= -128 and v <= 128" -- \
    scripts/encodetest_x64.sh "MISMATCH add-ri"

control sib $M \
    $'    if lo == 4 {\n        b.push(36)' $'    if lo == 12 {\n        b.push(36)' -- \
    scripts/encodetest_x64.sh "MISMATCH load"

control disp-rbp $M \
    "    if disp == 0 and lo != 5 {" "    if disp == 0 {" -- \
    scripts/encodetest_x64.sh "MISMATCH load"

control lock-order $M \
    $'        if sz == 2 {\n            pre.push(102)\n        }\n        if m >= X_XADD8 {\n            pre.push(240)\n        }' \
    $'        if m >= X_XADD8 {\n            pre.push(240)\n        }\n        if sz == 2 {\n            pre.push(102)\n        }' -- \
    scripts/encodetest_x64.sh "MISMATCH xadd16"

control jmp-rel $M \
    "        return [if m == X_JMP8 { 235 } else { 112 + cc }, rel.bit_and(255)]" \
    "        return [if m == X_JMP8 { 235 } else { 112 + cc }, (rel + 1).bit_and(255)]" -- \
    scripts/encodetest_x64.sh "MISMATCH jmp8"

control reloc-kind $M \
    $'    } else if m == X_LEA_RIP {\n        REL_SIGNED' $'    } else if m == X_LEA_RIP {\n        REL_GOT_LOAD' -- \
    scripts/encodetest_x64.sh "RELOC lea-rip"

control coverage compiler/x86_64/src/main.gr \
    $'        out.push(mach.form_name(m))\n    }\n    out' \
    $'        out.push(mach.form_name(m))\n    }\n    out.push(mach.form_name(mach.P_RETURN))\n    out' -- \
    scripts/encodetest_x64.sh "with no test cases"

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# Nine faults, and every one must have been caught.
if [ "$controls" -ne 9 ] || [ "$caught" -ne 9 ]; then
    echo "expected 9 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
