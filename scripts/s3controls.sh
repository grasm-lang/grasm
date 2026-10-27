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
#                -> fuzz_methods, through its third engine (S3 part five)
#
# Part three's doubles, function values and generics:
#
#   fp-cc        a comparison of doubles' `lt` reads the integer `lt`
#                condition, which an unordered comparison (a NaN) satisfies
#                -> encodetest (the floating-point map checked against IEEE)
#                -> fuzz_s3, with the table's checks skipped (a NaN compared)
#   fp-class     every move is the integer `mov`, doubles' too: a double in a
#                general register's field
#                -> s3subset (the allocation checker's class check)
#   capture-rc   a closure's boxed capture goes into its environment without
#                a reference of its own -> s3rc (a use after free)
#   mono         an instance is named for its function alone, so every use
#                of a generic function reaches whichever instance came first
#                -> s3subset
#
# Part four's references, atomics, effects, ranges and tasks:
#
#   ref-cell     `r.value = x` writes the next slot of the shared heap, not
#                the one `r` names -> s3subset
#   atomic-plain a fetch-add is selected as a plain 64-bit load, add and
#                store, whatever its width -> s3subset (a narrow add carries
#                into the next byte)
#   handler      an effect operation answers one more than its handler did
#                -> s3subset
#   range-bound  a range held as a value is made with its end one past
#                -> s3subset
#   task-result  a parallel map's worker answers 0 for every integer result
#                -> s3subset
#
# Part five's line table, and the generators' third engine
# (`scripts/lib/s3engine.py`), whose faults are planted in a copy of
# `scripts/` instead:
#
#   dwarf-line     every line the line table records is one past the right
#                  one -> objtest (a literal's load maps to the wrong line)
#   engine-skip    the engine declines every program it builds
#                  -> fuzz_operators (the Grasm backend compared nothing)
#   engine-unused  a generator never hands a program to the engine
#                  -> fuzz_operators (the same)
#
# The release allocator (`backtrack.gr`), the default:
#
#   backtrack      a register is free wherever no physical register's own
#                  range is, whoever holds it -> fuzz_s3 (the allocation
#                  checker)
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
        local code stop
        # First with `S3_STOP_AT_FIRST=1`, which `s3subset` and `s3rc` take
        # to stop at the first program that fails (the others ignore it).
        # A run that stopped early and shows the symptom settles the control:
        # the full run prints the same first failure, and more. One that does
        # not is run again in full and judged on that, so an early stop can
        # catch a fault but never decides that one was missed.
        for stop in 1 ""; do
            git ls-files --others --exclude-standard | sort >"$WORK/untracked-before"
            case "$gate" in
                *.py) S3="$driver" S3_EXTRA="$extra" S3_STOP_AT_FIRST="$stop" python3 "$gate" >"$log" 2>&1 ;;
                *) S3="$driver" S3_EXTRA="$extra" S3_STOP_AT_FIRST="$stop" "$gate" >"$log" 2>&1 ;;
            esac
            code=$?
            # A mutated backend can make a program write wherever it can
            # reach. The extra-dec and rc-select faults free a string still
            # in use, and under them L-LIB-FS wrote files named `c` and `b`
            # into the repository root, which `litter` then reported. Remove
            # what this run created, and say so.
            git ls-files --others --exclude-standard | sort | comm -13 "$WORK/untracked-before" - |
                while read -r stray; do
                    rm -f -- "$stray"
                    echo "    removed $stray, written by the $name build"
                done
            if [ "$code" != 0 ] && grep -qF -- "$symptom" "$log"; then
                break
            fi
            if [ "$code" != 0 ] && grep -qiE -- "timed out after" "$log"; then
                break
            fi
        done
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
    scripts/s3subset.sh "MISMATCH:" - \
    scripts/fuzz_methods.py "S3 DISAGREES" -

control fp-cc compiler/aarch64/src/rules.gr \
    "        mach.C_MI," "        mach.C_LT," -- \
    scripts/encodetest.sh "on doubles, slt maps to lt" - \
    scripts/fuzz_s3.py "FAILED program" --no-rule-check

control fp-class compiler/aarch64/src/isel.gr \
    "    if vcode.cls_of(vc, d) == mach.CLS_FP {" "    if false {" -- \
    scripts/s3subset.sh "register allocation check failed" -

control capture-rc compiler/mir/src/lower.gr \
    "            x = retain_copy(v, w)" "            x = w" -- \
    scripts/s3rc.sh "use after free" -

control mono compiler/mir/src/mono.gr \
    "    let iname = inst_name(m.t, name, targs)" "    let iname = name + \"[]\"" -- \
    scripts/s3subset.sh "FAILED:" -

control ref-cell compiler/mir/src/lower.gr \
    '        rt(v, "grasm_ref_set", [raw, fb], R_VOID)' \
    '        rt(v, "grasm_ref_set", [op2(v, ir.M_ADD, raw, konst(v, 4294967296)), fb], R_VOID)' -- \
    scripts/s3subset.sh "MISMATCH:" -

control atomic-plain compiler/aarch64/src/rules.gr \
    '        rs.push(r("aadd" + widths[k], ir.M_AADD8 + k, G_ANY, [t(mach.A_LDADDALB + k, S_RES, S_A, S_B)]))' \
    '        rs.push(r("aadd" + widths[k], ir.M_AADD8 + k, G_ANY, [ti(mach.A_LDR, S_RES, S_A, 0, 0), t(mach.A_ADDR, S_T0, S_RES, S_B), ti(mach.A_STR, S_T0, S_A, 0, 0)]))' -- \
    scripts/s3subset.sh "MISMATCH:" -

control handler compiler/mir/src/lower.gr \
    '    call_value(v, f, args, rk)' \
    $'    let got = call_value(v, f, args, rk)\n    if ir.ty_int(rk) {\n        return op2(v, ir.M_ADD, got, konst(v, 1))\n    }\n    got' -- \
    scripts/s3subset.sh "MISMATCH:" -

control range-bound compiler/mir/src/lower.gr \
    '    rt(v, "grasm_range_new", [lo, hi, kc], R_BOX)' \
    '    rt(v, "grasm_range_new", [lo, op2(v, ir.M_ADD, hi, konst(v, 1)), kc], R_BOX)' -- \
    scripts/s3subset.sh "MISMATCH:" -

control task-result compiler/mir/src/lower.gr \
    $'    } else {\n        bits = r\n    }\n    do_return(v, bits)' \
    $'    } else {\n        bits = konst(v, 0)\n    }\n    do_return(v, bits)' -- \
    scripts/s3subset.sh "MISMATCH:" -

# Part five: the line table, and the generators' third engine.
control dwarf-line compiler/s3/src/main.gr \
    "    (k + 1, lo + 1)" "    (k + 1, lo + 2)" -- \
    scripts/objtest.sh "maps to line" -

control backtrack compiler/aarch64/src/backtrack.gr \
    " or busy(occ[r], sg.seg_s[k], sg.seg_e[k])) {" ") {" -- \
    scripts/fuzz_s3.py "register allocation check failed" -

# script_control NAME FILE OLD NEW GATE SYMPTOM
# The fault is planted in a copy of `scripts/` -- the generators' shared
# engine (`scripts/lib/s3engine.py`) or a generator -- and the copied
# generator is run against the unmutated backend: an engine that stops
# comparing must fail the generator, not leave it green.
script_control() {
    local name=$1 file=$2 old=$3 new=$4 gate=$5 symptom=$6
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src"
    cp -R compiler scripts "$WORK/src/"
    controls=$((controls + 1))
    if ! python3 scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    local grasm_abs rt_abs log="$WORK/$name.log"
    grasm_abs=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
    rt_abs=${S3_RT:-$PWD/target/release/libgrasm_rt_abi.a}
    case "$rt_abs" in /*) ;; *) rt_abs="$PWD/$rt_abs" ;; esac
    GRASM="$grasm_abs" S3_RT="$rt_abs" python3 "$WORK/src/$gate" >"$log" 2>&1
    local code=$?
    if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name by $(basename "$gate" .py) (exit $code; \"$symptom\")"
    elif [ "$code" != 0 ]; then
        MISSED+=("$name: $(basename "$gate" .py) failed (exit $code) but not with \"$symptom\"; see its last lines below")
        tail -5 "$log" | sed 's/^/    /'
    else
        MISSED+=("$name: $(basename "$gate" .py) passed with the fault in place")
    fi
}

# The engine declines every program it builds: nothing is compared.
script_control engine-skip scripts/lib/s3engine.py \
    "        if c[0] == 3:" "        if c[0] in (0, 3):" \
    scripts/fuzz_operators.py "the Grasm backend compared nothing"

# A generator that never hands its programs to the engine.
script_control engine-unused scripts/fuzz_operators.py \
    "                s3.check(path, exe, label, interp=(rc, out))" "                pass" \
    scripts/fuzz_operators.py "the Grasm backend compared nothing"

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# Twenty-nine gate runs over twenty-four faults, and every one must have
# failed.
if [ "$controls" -ne 29 ] || [ "$caught" -ne 29 ]; then
    echo "expected 29 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
