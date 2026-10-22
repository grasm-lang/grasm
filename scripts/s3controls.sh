#!/usr/bin/env bash
# Mutation controls for the S3 gates: a gate that cannot fail checks nothing.
#
# Each control copies `compiler/` aside, plants one fault in one layer of the
# backend (`scripts/s3_mutate.py`, which refuses a mutation that matches
# nothing), builds that broken driver, and runs the gates named for it with
# `S3=` pointing at it. Every one of those gates must fail, and fail for the
# planted reason: its log must show the symptom that fault produces, so a gate
# that fails for an unrelated reason does not count as having caught it.
#
#   isel       the condition code `slt` selects becomes `le`
#              -> encodetest (the rule table's semantic check of the map)
#              -> s3subset, with the table's checks skipped, by the output
#   encoder    the Rm field of the three-register forms moves up a bit
#              -> encodetest (bytes against the assembler's)
#              -> s3subset (programs compute the wrong thing)
#   objwriter  every relocation is written one instruction late
#              -> objtest (relocations against instruction classes)
#              -> s3subset
#   allocator  a reload reads the spill slot after the one it should
#              -> ra_check (the checker, under register pressure)
#   missing-dec  a scope's end releases nothing -> s3rc (a leak)
#   extra-dec    a read of a boxed local takes no reference of its own, so
#                its release is one too many -> s3rc (a use after free)
#   err-tag      `fail` and a failed `try` answer the `Ok` tag
#                -> s3subset (programs print the wrong thing)
#   rc-select    `inc` is selected as a call of `grasm_val_dec`
#                -> s3rc (a use after free)
#   dataaddr     the address of a struct descriptor takes its page from a
#                string's symbol -> s3subset
#   datarelocs   every pointer relocation in `__data` is a word late
#                -> objtest (descriptor addresses with no relocation)
#   method       `to_upper` calls the runtime's lower-casing
#                -> s3subset
#
# A fault that sends a program into a loop shows as a run stopped at the
# gates' limit (`S3_RUN_SECONDS` in `scripts/lib/s3.sh`, 20 s against well
# under a second for any subset program), and counts as caught.
#
# The unmutated gates are run by `scripts/allgates.sh` on their own; this
# gate only shows that each of them can fail.
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

# control NAME FILE OLD NEW -- GATE SYMPTOM [EXTRA] ...
# Each GATE SYMPTOM EXTRA triple after `--` is a gate script, a fixed string
# its log must contain, and `S3_EXTRA` for that run ("-" for none).
control() {
    local name=$1 file=$2 old=$3 new=$4
    shift 5
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
    while [ $# -ge 3 ]; do
        local gate=$1 symptom=$2 extra=$3
        shift 3
        [ "$extra" = "-" ] && extra=""
        controls=$((controls + 1))
        local log="$WORK/$name-$(basename "$gate" .sh).log"
        S3="$driver" S3_EXTRA="$extra" "$gate" >"$log" 2>&1
        local code=$?
        if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
        elif [ "$code" != 0 ] && grep -ciE -- "timed out after" "$log" >/dev/null; then
            # A program the fault sent into a loop: stopped at the gates' run
            # limit (`S3_RUN_SECONDS`), where its right build had finished in
            # well under a second. A difference found, not a different reason.
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; a program ran past the limit)"
        elif [ "$code" != 0 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"; see its last lines below")
            tail -5 "$log" | sed 's/^/    /'
        else
            MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
        fi
    done
}

control isel compiler/aarch64/src/rules.gr \
    "        mach.C_LT," "        mach.C_LE," -- \
    scripts/encodetest.sh "slt maps to le" - \
    scripts/s3subset.sh "MISMATCH:" --no-rule-check

control encoder compiler/aarch64/src/mach.gr \
    "        return base.bit_or(mm.shl(16)).bit_or(n.shl(5)).bit_or(d)" \
    "        return base.bit_or(mm.shl(17)).bit_or(n.shl(5)).bit_or(d)" -- \
    scripts/encodetest.sh "MISMATCH add-reg" - \
    scripts/s3subset.sh "FAILED:" -

control objwriter compiler/macho/src/main.gr \
    "        put32(rels, o.rel_off[k])" "        put32(rels, o.rel_off[k] + 4)" -- \
    scripts/objtest.sh "has no BRANCH26 relocation" - \
    scripts/s3subset.sh "FAILED:" -

control allocator compiler/aarch64/src/regalloc.gr \
    "let rslot = l - LOC_SPILL" "let rslot = l - LOC_SPILL + 1" -- \
    scripts/ra_check.sh "register allocation check failed" -

# Part two's ownership rules, the lowering families it added, and the
# object's new section.
control missing-dec compiler/mir/src/lower.gr \
    "        while k >= m {" "        while k >= m and false {" -- \
    scripts/s3rc.sh "LEAKS:" -

control extra-dec compiler/mir/src/lower.gr \
    "    emit(v, ir.M_INC, -1, d, -1, 0)" "    ()" -- \
    scripts/s3rc.sh "use after free" -

control err-tag compiler/mir/src/lower.gr \
    "        split_return(v, 1, x)" "        split_return(v, 0, x)" -- \
    scripts/s3subset.sh "MISMATCH:" -

control rc-select compiler/aarch64/src/isel.gr \
    "if op == ir.M_INC { ir.RT_VAL_INC } else { ir.RT_VAL_DEC }" \
    "if op == ir.M_INC { ir.RT_VAL_DEC } else { ir.RT_VAL_DEC }" -- \
    scripts/s3rc.sh "use after free" -

control dataaddr compiler/aarch64/src/rules.gr \
    "    dadrp.sym = Y_DATA" "    dadrp.sym = Y_STR" -- \
    scripts/s3subset.sh "FAILED:" -

control datarelocs compiler/macho/src/main.gr \
    "        put32(drels, o.wrel_off[k])" "        put32(drels, o.wrel_off[k] + 8)" -- \
    scripts/objtest.sh "is an address with no relocation" -

control method compiler/mir/src/lower.gr \
    "\"str.to_upper/0\" -> read_method(info, v, e, \"grasm_str_upper\"" \
    "\"str.to_upper/0\" -> read_method(info, v, e, \"grasm_str_lower\"" -- \
    scripts/s3subset.sh "MISMATCH:" -

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# Fourteen gate runs over eleven faults, and every one must have failed.
if [ "$controls" -ne 14 ] || [ "$caught" -ne 14 ]; then
    echo "expected 14 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
