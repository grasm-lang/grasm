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

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# Seven gate runs over four faults, and every one must have failed.
if [ "$controls" -ne 7 ] || [ "$caught" -ne 7 ]; then
    echo "expected 7 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
