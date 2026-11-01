#!/usr/bin/env bash
# Two stages of the Grasm compiler against each other on the corpus, every
# artifact each makes byte for byte (docs/self-hosting.md §6.14, stage S6).
#
# By default stage 1 -- the compiler as Cranelift builds it -- against stage
# 2, the compiler as stage 1 builds it (`scripts/lib/stages.sh`), both built
# here. `STAGE_A=<dir> STAGE_B=<dir>` names two stage directories instead
# (`scripts/fixpoint.sh` keeps them with FIXPOINT_KEEP): stage 2 against
# stage 3 is `STAGE_A=<kept>/2 STAGE_B=<kept>/3`.
#
# A compiler's output, for each:
#
#   * the runtime it makes -- its `front tir` of compiler/rt, compiled by its
#     `s3 --runtime` -- and the poisoned runtime the same way; the two
#     compilers' TIR and objects must be the same bytes;
#   * for every corpus program and every other `.gr` file git tracks, as
#     `tirdiff` lists them, its `front tir`: the same exit status, TIR and
#     diagnostics;
#   * for every program with a `main` whose TIR both accept, its `s3`'s
#     answer and object, and the executable its `link` makes of that object
#     with its own runtime alone: the same bytes (`scripts/stagediff.py`;
#     a program that links a system library is set aside, counted).
#
# Nothing is run: whether the programs are right is the other gates'
# question (`scripts/withstage.sh`). The counts are derived and must add up;
# over the whole corpus at least 1,800 files' TIR and the subset's floor of
# executables (`scripts/lib/s3.sh`) must be compared. `FILES=` names a list
# of files instead; `STAGEDIFF_JOBS` how many programs at a time (4).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
. "$(dirname "$0")/lib/corpus.sh"
corpus_require
. "$(dirname "$0")/lib/stages.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"

if ! stage_poison_src "$WORK/rt.poison.src" >"$WORK/poison.log" 2>&1; then
    echo "error: $(cat "$WORK/poison.log")"
    exit 1
fi
if [ -z "${STAGE_A:-}" ] && [ -z "${STAGE_B:-}" ]; then
    if ! stage_one "$WORK/1" >"$WORK/build.log" 2>&1 || ! stage_next "$WORK/1" "$WORK/2" >>"$WORK/build.log" 2>&1; then
        echo "error: the stages do not build:"
        sed 's/^/  /' "$WORK/build.log"
        exit 1
    fi
    STAGE_A="$WORK/1"
    STAGE_B="$WORK/2"
    echo "A: stage 1, by grasm build --native; B: stage 2, by stage 1"
elif [ -z "${STAGE_A:-}" ] || [ -z "${STAGE_B:-}" ]; then
    echo "error: STAGE_A and STAGE_B name the two stages together"
    exit 1
else
    echo "A: $STAGE_A; B: $STAGE_B"
fi

# Each compiler, with the runtime it makes, in $WORK/A and $WORK/B.
rt_same=0
rt_listed=0
declare -a FAILED=()
for side in A B; do
    var=STAGE_$side
    dir=${!var}
    dir=$(cd "$dir" && pwd) || exit 1
    mkdir -p "$WORK/$side"
    for t in front s3 link; do
        if [ ! -x "$dir/$t" ]; then
            echo "error: $dir holds no $t"
            exit 1
        fi
        ln -s "$dir/$t" "$WORK/$side/$t"
    done
    for r in rt rt.poison; do
        src=compiler/rt/src/main.gr
        [ "$r" = rt.poison ] && src="$STAGE_POISON/src/main.gr"
        if ! "$dir/front" tir "$src" >"$WORK/$side/$r.tir" 2>"$WORK/$side/$r.log" ||
            ! "$dir/s3" "$WORK/$side/$r.tir" -o "$WORK/$side/$r.o" --runtime >>"$WORK/$side/$r.log" 2>&1; then
            echo "error: $side's compiler does not build $r:"
            sed 's/^/  /' "$WORK/$side/$r.log" | head -10
            exit 1
        fi
    done
done
for f in rt.tir rt.o rt.poison.tir rt.poison.o; do
    rt_listed=$((rt_listed + 1))
    if cmp -s "$WORK/A/$f" "$WORK/B/$f"; then
        rt_same=$((rt_same + 1))
    else
        FAILED+=("$f — the runtime each compiler makes differs ($(cmp "$WORK/A/$f" "$WORK/B/$f" 2>&1 | sed 's/.*differ: //'))")
    fi
done
echo "the runtime each makes, its TIR and object, and the poisoned one's: the same: $rt_same of $rt_listed"

if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    { corpus_programs; git ls-files '*.gr'; } | sort -u >"$WORK/files"
    export STAGEDIFF_FLOORS="1800 $S3_SUBSET_FLOOR"
fi
python3 scripts/stagediff.py "$GRASM" "$WORK/A" "$WORK/B" "$WORK/files" "$WORK/out"
rc=$?
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$rc" != 0 ] || [ "$rt_same" -ne "$rt_listed" ] || [ "$rt_listed" -ne 4 ]; then
    exit 1
fi
exit 0
