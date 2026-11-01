#!/usr/bin/env bash
# A gate, with every binary of the Grasm toolchain it takes being one stage
# of the Grasm compiler built by itself (docs/self-hosting.md §6.14, S6).
#
#     scripts/withstage.sh scripts/tirdiff.sh
#     scripts/withstage.sh python3 scripts/fuzz_tir.py
#     STAGE=<dir> scripts/withstage.sh scripts/s3subset.sh
#
# Builds stage 1 and, by it, stage 2 (`scripts/lib/stages.sh`) -- or takes
# the stage directory `STAGE=` names, as `scripts/fixpoint.sh` keeps them
# (FIXPOINT_KEEP) -- and runs the command with
#
#   FRONT, TIRINTERP, S3, LINK   that stage's front, tirinterp, s3 and link;
#                                FRONT also makes the S3 gates' TIR
#                                (`scripts/s3_collect.py`)
#   RT_OBJ, RT_OBJ_POISON        that stage's runtime and its poisoned build
#   S3_LINKER, LINKER            `scripts/lib/stage_link`: what the Grasm
#                                backend writes is linked by that stage's
#                                link with that stage's runtime alone
#
# What a gate compares against -- `grasm tir`, `grasm check`, `grasm run`,
# the Cranelift build as it ships, the system's tools -- is left as it is,
# so what the gate reports is about the stage. `grasm links`, `grasm build`
# and the rest of the `grasm` command are not the stage's: they are S8.
# `GRASM=` must name the compiler, as for `scripts/rtwith.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
. "$(dirname "$0")/lib/stages.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
if [ $# -lt 1 ]; then
    echo "usage: scripts/withstage.sh <command> [arguments]"
    exit 2
fi

if [ -z "${STAGE:-}" ]; then
    if ! stage_poison_src "$WORK/rt.poison.src" >"$WORK/poison.log" 2>&1 ||
        ! stage_one "$WORK/1" >"$WORK/build.log" 2>&1 ||
        ! stage_next "$WORK/1" "$WORK/2" >>"$WORK/build.log" 2>&1; then
        echo "error: the stages do not build:"
        cat "$WORK/poison.log" "$WORK/build.log" | sed 's/^/  /'
        exit 1
    fi
    STAGE="$WORK/2"
fi
STAGE=$(cd "$STAGE" && pwd) || exit 1
while IFS= read -r f; do
    if [ ! -f "$STAGE/$f" ]; then
        echo "error: $STAGE holds no $f, so it is not a stage after the first"
        exit 1
    fi
done < <(stage_artifacts)

export GRASM
export FRONT="$STAGE/front" TIRINTERP="$STAGE/tirinterp" S3="$STAGE/s3" LINK="$STAGE/link"
export RT_OBJ="$STAGE/rt.o" RT_OBJ_POISON="$STAGE/rt.poison.o"
export STAGE_LINK="$STAGE/link" STAGE_RT="$STAGE/rt.o"
export S3_LINKER="$PWD/scripts/lib/stage_link" LINKER="$PWD/scripts/lib/stage_link"

# That the stage is in effect: a program through its front, s3 and link, on
# its runtime alone, runs as `grasm run` runs it, and took its runtime names
# from that runtime and from nothing else.
canary=examples/hello.gr
"$GRASM" run "$canary" >"$WORK/want.out" 2>&1
if ! "$FRONT" tir "$canary" >"$WORK/c.tir" 2>"$WORK/c.log" ||
    ! "$S3" "$WORK/c.tir" -o "$WORK/c.o" >>"$WORK/c.log" 2>&1 ||
    ! STAGE_LINK_LOG="$WORK/c.names" "$S3_LINKER" "$WORK/c.o" "$(dirname "$GRASM")/libgrasm_rt_abi.a" -dead_strip -o "$WORK/c" >>"$WORK/c.log" 2>&1 ||
    ! "$WORK/c" >"$WORK/got.out" 2>&1 || ! cmp -s "$WORK/want.out" "$WORK/got.out"; then
    echo "error: $canary does not go through the stage in $STAGE as grasm run runs it:"
    sed 's/^/  /' "$WORK/c.log" | head -10
    exit 1
fi
if ! grep -q '^grasm ' "$WORK/c.names" || grep -q '^rust ' "$WORK/c.names"; then
    echo "error: $canary, linked by the stage, did not take its runtime names from the stage's runtime alone"
    exit 1
fi
echo "the stage in $STAGE is in effect: $canary through its front, s3 and link took $(grep -c '^grasm ' "$WORK/c.names") runtime names from its runtime and none from Rust, and runs as grasm run runs it"
"$@"
