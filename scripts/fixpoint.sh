#!/usr/bin/env bash
# The fixed point (docs/self-hosting.md §6.14, stage S6): the Grasm compiler
# built by the Rust one (stage 1) builds itself (stage 2), which builds stage
# 3, and stage 3 is stage 2, byte for byte.
#
# The compiler is `compiler/front`, `compiler/s3`, `compiler/link` and the
# runtime `compiler/rt`, with `tools/tirinterp` built beside them; what a
# stage is and how one builds the next is `scripts/lib/stages.sh`. Five
# builds: stage 1; stage 2 twice, by stage 1; stage 3 twice, by stage 2. Then
# every artifact a stage holds (`stage_artifacts`: each runtime's TIR and
# object, each executable's TIR, object and executable) is compared:
#
#   * the two stage 2s with each other, and the two stage 3s: a compiler run
#     twice on the same input must say the same. One that does not is
#     NONDETERMINISTIC -- hash order, an address, a clock or uninitialised
#     bytes in what it writes -- and which stage's compiler it is says
#     whether Cranelift's build of it or our own is;
#   * stage 3 with stage 2: the fixed point. A stage 3 that differs from a
#     deterministic stage 2 is NOT A FIXED POINT: the compiler compiled by
#     itself behaves otherwise than compiled by Cranelift, a miscompile that
#     only compiling the compiler shows.
#
# The objects are compared and not only the executables: an object holds a
# DWARF line table the executable does not, so a miscompile that moves only
# the lines is seen there and nowhere else. And stage 2's TIR, which stage 1's
# frontend made, must be the TIR `grasm tir` makes of the same source.
#
# The count of artifacts compared is derived from the stages' lists and
# asserted, and the four programs that are the compiler must be among them,
# so a comparison that was skipped or a program that dropped out fails.
# Prints each build's cost, and stage 3's sizes and SHA-256s.
#
# Then the `grasm` command in Grasm (compiler/grasm, §6.17), staged by
# itself (`driver_one`, `driver_next`): its stage 1 by the Rust `grasm`, with
# stage 2's runtime object; stages 2 and 3 each built twice, the command and
# its `rt.o`, by the stage before's command alone -- held the same three
# ways. Two more checks: the `rt.o` stage 1's command makes is the one stage
# 1's `front` and `s3` make, and stage 1's command, symbols kept, builds the
# compiler's four programs into stage 2's very executables -- so `grasm build
# --native` is the stage pipeline.
#
# `FIXPOINT_ROOT=<dir>` builds the stages from that tree instead of this
# checkout (the controls, `scripts/fixpointcontrols.sh`, plant faults in a
# copy); `FIXPOINT_KEEP=<dir>` copies the stages there; `FIXPOINT_FAULT=skip`
# leaves one comparison out, for a control.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
. "$(dirname "$0")/lib/stages.sh"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [ -n "${FIXPOINT_ROOT:-}" ]; then
    cd "$FIXPOINT_ROOT" || exit 1
fi

if ! stage_poison_src "$WORK/rt.poison.src" >"$WORK/poison.log" 2>&1; then
    echo "error: $(cat "$WORK/poison.log")"
    exit 1
fi

# Each build, timed; the last line of a failed build's output says why.
build() { # what label
    local what=$1 label=$2 start=$SECONDS
    shift 2
    if ! "$@" >"$WORK/build.out" 2>&1; then
        echo "FAILED: $label"
        sed 's/^/  /' "$WORK/build.out"
        exit 1
    fi
    printf '%-34s %s (wall %d s)\n' "$label:" "$(stage_cost "$what")" $((SECONDS - start))
}
build "$WORK/1" "stage 1, by grasm build --native" stage_one "$WORK/1"
build "$WORK/2" "stage 2, by stage 1" stage_next "$WORK/1" "$WORK/2"
build "$WORK/2b" "stage 2 again, by stage 1" stage_next "$WORK/1" "$WORK/2b"
build "$WORK/3" "stage 3, by stage 2" stage_next "$WORK/2" "$WORK/3"
build "$WORK/3b" "stage 3 again, by stage 2" stage_next "$WORK/2" "$WORK/3b"
build "$WORK/d1" "the command's stage 1, by grasm" driver_one "$WORK/d1" "$WORK/2/rt.o"
build "$WORK/d2" "its stage 2, by its stage 1" driver_next "$WORK/d1" "$WORK/d2"
build "$WORK/d2b" "its stage 2 again" driver_next "$WORK/d1" "$WORK/d2b"
build "$WORK/d3" "its stage 3, by its stage 2" driver_next "$WORK/d2" "$WORK/d3"
build "$WORK/d3b" "its stage 3 again" driver_next "$WORK/d2" "$WORK/d3b"

stage_artifacts >"$WORK/artifacts"
listed=$(grep -c . "$WORK/artifacts")
for need in front.tir front.o front s3.tir s3.o s3 link.tir link.o link rt.tir rt.o; do
    if ! grep -qx "$need" "$WORK/artifacts"; then
        echo "the stages do not hold $need, one of the compiler's own artifacts"
        exit 1
    fi
done

compared=0
same=0
declare -a FAILED=()
printf '%s\n' "${DRIVER_ARTIFACTS[@]}" >"$WORK/driver_artifacts"
driver_listed=$(grep -c . "$WORK/driver_artifacts")

# compare A B VERDICT [LIST]: one artifact of two stages, every one listed
# (the compiler's, or the command's).
compare() {
    local a=$1 b=$2 verdict=$3 list=${4:-$WORK/artifacts} f k=0
    while IFS= read -r f; do
        k=$((k + 1))
        if [ "${FIXPOINT_FAULT:-}" = skip ] && [ "$verdict" = "NOT A FIXED POINT" ] && [ "$k" = "$listed" ]; then
            continue
        fi
        compared=$((compared + 1))
        if [ ! -f "$WORK/$a/$f" ] || [ ! -f "$WORK/$b/$f" ]; then
            FAILED+=("$f — missing from stage $a or stage $b")
        elif cmp -s "$WORK/$a/$f" "$WORK/$b/$f"; then
            same=$((same + 1))
        else
            FAILED+=("$verdict: stage $b's $f is not stage $a's ($(cmp "$WORK/$a/$f" "$WORK/$b/$f" 2>&1 | sed 's/.*differ: //'))")
        fi
    done <"$list"
}
compare 2 2b "NONDETERMINISTIC (stage 1's compiler, run twice)"
compare 3 3b "NONDETERMINISTIC (stage 2's compiler, run twice)"
compare 2 3 "NOT A FIXED POINT"
compare d2 d2b "NONDETERMINISTIC (the command's stage 1, run twice)" "$WORK/driver_artifacts"
compare d3 d3b "NONDETERMINISTIC (the command's stage 2, run twice)" "$WORK/driver_artifacts"
compare d2 d3 "NOT A FIXED POINT (the command)" "$WORK/driver_artifacts"

# The command's runtime object is the compiler's, and its build is the
# stage pipeline: stage 1's command, symbols kept as a stage keeps them,
# builds each of the compiler's programs into stage 2's executable.
pipeline=0
if cmp -s "$WORK/d2/rt.o" "$WORK/2/rt.o"; then
    pipeline=$((pipeline + 1))
else
    FAILED+=("the rt.o the command's stage 1 makes is not the one stage 1's front and s3 make")
fi
mkdir -p "$WORK/via"
for e in "${STAGE_EXES[@]}"; do
    n=${e%%|*}
    if GRASM_KEEP_SYMBOLS=1 "$WORK/d1/grasm" build "${e#*|}" --native --no-incremental -o "$WORK/via/$n" >"$WORK/via.log" 2>&1 &&
        cmp -s "$WORK/via/$n" "$WORK/2/$n"; then
        pipeline=$((pipeline + 1))
    else
        FAILED+=("$n built by the command's stage 1 is not stage 2's $n: $(head -2 "$WORK/via.log" | tr '\n' ' ')")
    fi
done

# Stage 1's frontend on the compiler's own source, against the Rust one's.
tir_same=0
tir_listed=0
while IFS='|' read -r n src; do
    tir_listed=$((tir_listed + 1))
    if "$GRASM" tir "$src" >"$WORK/ref.tir" 2>"$WORK/ref.err" && cmp -s "$WORK/ref.tir" "$WORK/2/$n.tir"; then
        tir_same=$((tir_same + 1))
    else
        FAILED+=("$n.tir — stage 1's front tir $src is not grasm tir's")
    fi
done < <(printf '%s\n' "rt|compiler/rt/src/main.gr" "rt.poison|$STAGE_POISON/src/main.gr" "${STAGE_EXES[@]}")

echo
echo "stage 3, which is stage 2 when every comparison below holds:"
while IFS= read -r f; do
    [ -f "$WORK/3/$f" ] || continue
    case "$f" in
    *.tir) continue ;;
    esac
    printf '  %-14s %9d  %s\n' "$f" "$(wc -c <"$WORK/3/$f")" "$(shasum -a 256 "$WORK/3/$f" | cut -d' ' -f1)"
done <"$WORK/artifacts"
for f in "${DRIVER_ARTIFACTS[@]}"; do
    [ -f "$WORK/d3/$f" ] || continue
    printf '  %-14s %9d  %s\n' "command $f" "$(wc -c <"$WORK/d3/$f")" "$(shasum -a 256 "$WORK/d3/$f" | cut -d' ' -f1)"
done
if [ -n "${FIXPOINT_KEEP:-}" ]; then
    rm -rf "${FIXPOINT_KEEP:?}"
    mkdir -p "$FIXPOINT_KEEP"
    cp -R "$WORK/1" "$WORK/2" "$WORK/3" "$WORK/d1" "$WORK/d2" "$WORK/d3" "$FIXPOINT_KEEP/"
    echo "the stages are kept in $FIXPOINT_KEEP"
fi

echo
echo "programs: $((${#STAGE_EXES[@]} + ${#STAGE_RTS[@]})) and the grasm command   artifacts per stage: $listed and $driver_listed   compared: $compared (stage 2 twice, stage 3 twice, stage 3 against stage 2)   identical: $same   differ: $((compared - same))"
echo "stage 2's TIR the same as grasm tir's: $tir_same of $tir_listed"
echo "the command's build the stage pipeline's (its rt.o and the ${#STAGE_EXES[@]} programs): $pipeline of $((${#STAGE_EXES[@]} + 1))"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$compared" -ne $((3 * listed + 3 * driver_listed)) ]; then
    echo "compared $compared of the $((3 * listed + 3 * driver_listed)) artifacts the three pairs of stages hold"
    exit 1
fi
if [ "$pipeline" -ne $((${#STAGE_EXES[@]} + 1)) ]; then
    exit 1
fi
if [ "$same" -ne "$compared" ] || [ "$tir_same" -ne "$tir_listed" ] || [ "$tir_listed" -lt 6 ]; then
    exit 1
fi
exit 0
