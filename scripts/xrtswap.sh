#!/usr/bin/env bash
# `rtswap` for x86-64 (docs/self-hosting.md §6.16, stage S7): every corpus
# program in the subset, linked for x86-64 with the runtime written in Grasm
# alone and by the linker written in Grasm, prints and exits exactly as it
# does linked by the system linker with the Rust runtime built for
# x86_64-apple-darwin, all run under Rosetta.
#
# The Grasm runtime (`compiler/rt`) is compiled by `s3 --runtime --target
# x86_64`, with its poisoned build, and held to `compiler/rt/families.txt` as
# `rtswap` holds the arm64 one (`scripts/lib/rtswap.sh`: every moved name
# exported, nothing else, no Rust runtime name called). Then, for each
# program in the subset (`scripts/s3_subset.py`; the SDL programs set aside):
#
#   * the reference: the Cranelift x86-64 object (`grasm build --native
#     --target x86_64-apple-darwin`) linked by `cc -arch x86_64` with the Rust
#     runtime archive;
#   * the Cranelift object linked by `compiler/link -interpose rt.o` with no
#     runtime archive on the line, so every runtime name binds to the Grasm
#     runtime or the link fails; and again with the poisoned build, which
#     must report no use after free;
#   * the Grasm backend's object (`s3 --target x86_64`) linked the same way,
#     with the Grasm runtime alone: no Rust, no Cranelift, no `cc` -- the
#     Grasm toolchain end to end for x86-64 -- and linked a second time, under
#     the same name in another directory, to the same bytes.
#
# Each runs with stdin closed under `arch -x86_64`; stdout, the exit status
# and stderr must be the reference's. Every swapped link's `-interpose_log`
# says which runtime names it took from Grasm, and none may come from
# anywhere else. The counts are derived and asserted.
#
# `S3=`, `LINK=`, `RT_OBJ=` (with `RT_OBJ_POISON=`) name what to test instead
# of building it; `S3_RT_X64=` the reference runtime; `S3_STOP_AT_FIRST=1`
# stops at the first failure (`scripts/x64controls.sh`).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver

TARGET=x86_64-apple-darwin
if [ -z "${S3_RT_X64:-}" ]; then
    if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build"
        exit 1
    fi
    S3_RT_X64=./target/$TARGET/release/libgrasm_rt_abi.a
fi
[ -f "$S3_RT_X64" ] || { echo "error: $S3_RT_X64 is missing"; exit 1; }

# The Grasm runtime for x86-64, and its poisoned build.
if [ -z "${RT_OBJ:-}" ]; then
    RT_OBJ="$WORK/rt.o"
    RT_OBJ_POISON="$WORK/rt.poison.o"
    "$GRASM" tir "$RT_SRC" >"$WORK/rt.tir" 2>"$WORK/rt.log" &&
        "$S3" "$WORK/rt.tir" -o "$RT_OBJ" --runtime --target x86_64 >>"$WORK/rt.log" 2>&1 ||
        { echo "error: the Grasm runtime does not build for x86-64:"; sed 's/^/  /' "$WORK/rt.log" | head -20; exit 1; }
    rm -rf "$WORK/rtp"
    cp -R compiler/rt "$WORK/rtp"
    if [ "$(grep -c '^pub let POISON = false$' "$WORK/rtp/src/build.gr")" != 1 ]; then
        echo "error: compiler/rt/src/build.gr does not say \`pub let POISON = false\` once"
        exit 1
    fi
    sed -i '' 's/^pub let POISON = false$/pub let POISON = true/' "$WORK/rtp/src/build.gr"
    "$GRASM" tir "$WORK/rtp/src/main.gr" >"$WORK/rtp.tir" 2>"$WORK/rtp.log" &&
        "$S3" "$WORK/rtp.tir" -o "$RT_OBJ_POISON" --runtime --target x86_64 >>"$WORK/rtp.log" 2>&1 ||
        { echo "error: the poisoned Grasm runtime does not build for x86-64:"; sed 's/^/  /' "$WORK/rtp.log" | head -20; exit 1; }
fi
RT_OBJ_POISON=${RT_OBJ_POISON:-${RT_OBJ%.o}.poison.o}
[ -f "$RT_OBJ_POISON" ] || { echo "error: RT_OBJ_POISON=$RT_OBJ_POISON is missing"; exit 1; }
if ! otool -hv "$RT_OBJ" | grep -c X86_64 >/dev/null; then
    echo "error: $RT_OBJ is not an x86-64 object"
    exit 1
fi
rtswap_families

s3_collect
STOP=${S3_STOP_AT_FIRST:-}
stopped=0
programs=0
cl_same=0
p_same=0
s3_same=0
relinked=0
failed=0
timedout=0
external=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a EXTERNAL=()
: >"$WORK/taken.txt"

# Runs `$2` under Rosetta and compares it with the reference's run in
# `$WORK/ref.*`. Prints why and fails when they differ; answers 2 when it ran
# out of time.
run_cmp() {
    local exe=$1
    $S3_RUN arch -x86_64 "$exe" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    local bc=$?
    if [ "$bc" = 124 ]; then
        echo "TIMEOUT"
        return 2
    fi
    if cmp -s "$WORK/ref.out" "$WORK/b.out" && [ "$(cat "$WORK/ref.code")" = "$bc" ] && cmp -s "$WORK/ref.err" "$WORK/b.err"; then
        return 0
    fi
    echo "exit $(cat "$WORK/ref.code") (Rust runtime, ld) vs $bc"
    diff "$WORK/ref.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
    diff "$WORK/ref.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
    return 1
}

# A swapped link's log: every name taken from Grasm, none from anywhere else.
read_log() {
    local log=$1 src=$2
    if grep -v '^grasm ' "$log" | grep -c . >/dev/null; then
        echo "took runtime names from elsewhere: $(grep -v '^grasm ' "$log" | head -3 | tr '\n' ' ')"
        return 1
    fi
    awk -v s="$src" '{ print s, $2 }' "$log" >>"$WORK/taken.txt"
    return 0
}

note() { # why src half
    if [ "$1" = TIMEOUT ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$2 ($3)")
    else
        failed=$((failed + 1))
        FAILED+=("$2 ($3) — $1")
        echo "FAIL: $2 ($3): $1"
    fi
}

for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ $((failed + timedout)) -gt 0 ]; then
        stopped=1
        break
    fi
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    case "${S3_RULE[$k]}" in
        external:*) external=$((external + 1)); EXTERNAL+=("$src"); continue ;;
        in) ;;
        *) continue ;;
    esac
    programs=$((programs + 1))
    rm -f "$WORK/cl.o" "$WORK/ref" "$WORK/swap" "$WORK/swapp" "$WORK/s3swap" "$WORK/p.o"
    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/cl" >"$WORK/build.log" 2>&1 ||
        ! cc -arch x86_64 "$WORK/cl.o" "$S3_RT_X64" -o "$WORK/ref" >>"$WORK/build.log" 2>&1; then
        note "the reference does not build: $(head -1 "$WORK/build.log")" "$src" reference
        continue
    fi
    $S3_RUN arch -x86_64 "$WORK/ref" >"$WORK/ref.out" 2>"$WORK/ref.err" </dev/null
    echo $? >"$WORK/ref.code"
    if [ "$(cat "$WORK/ref.code")" = 124 ]; then
        note TIMEOUT "$src" reference
        continue
    fi
    # The Cranelift object, the Grasm runtime alone, the Grasm linker.
    if ! "$LINK" -interpose "$RT_OBJ" -interpose_log "$WORK/swap.log" "$WORK/cl.o" -dead_strip -o "$WORK/swap" >"$WORK/link.log" 2>&1; then
        note "compiler/link does not link it with the Grasm runtime: $(head -2 "$WORK/link.log" | tr '\n' ' ')" "$src" Cranelift
        continue
    fi
    if ! why=$(read_log "$WORK/swap.log" "$src"); then
        note "$why" "$src" Cranelift
        continue
    fi
    if why=$(run_cmp "$WORK/swap"); then
        cl_same=$((cl_same + 1))
    else
        note "$why" "$src" Cranelift
        continue
    fi
    # The same with the poisoned build: the same output, no use after free.
    if ! "$LINK" -interpose "$RT_OBJ_POISON" "$WORK/cl.o" -dead_strip -o "$WORK/swapp" >"$WORK/link.log" 2>&1; then
        note "compiler/link does not link it with the poisoned runtime: $(head -2 "$WORK/link.log" | tr '\n' ' ')" "$src" poisoned
        continue
    fi
    if why=$(run_cmp "$WORK/swapp"); then
        if grep -c 'use after free' "$WORK/b.err" >/dev/null; then
            note "a use after free under the poisoned runtime" "$src" poisoned
            continue
        fi
        p_same=$((p_same + 1))
    else
        note "$why" "$src" poisoned
        continue
    fi
    # The Grasm backend's object: the Grasm toolchain end to end.
    if ! "$S3" "$tir" -o "$WORK/p.o" --target x86_64 >"$WORK/s3.log" 2>&1; then
        note "s3 --target x86_64 does not compile it: $(head -2 "$WORK/s3.log" | tr '\n' ' ')" "$src" s3
        continue
    fi
    if ! "$LINK" -interpose "$RT_OBJ" -interpose_log "$WORK/s3swap.log" "$WORK/p.o" -dead_strip -o "$WORK/s3swap" >"$WORK/link.log" 2>&1; then
        note "compiler/link does not link it: $(head -2 "$WORK/link.log" | tr '\n' ' ')" "$src" s3
        continue
    fi
    if ! why=$(read_log "$WORK/s3swap.log" "$src"); then
        note "$why" "$src" s3
        continue
    fi
    mkdir -p "$WORK/again"
    if ! "$LINK" -interpose "$RT_OBJ" "$WORK/p.o" -dead_strip -o "$WORK/again/s3swap" >"$WORK/link.log" 2>&1 ||
        ! cmp -s "$WORK/s3swap" "$WORK/again/s3swap"; then
        note "two links of the same objects differ" "$src" s3
        continue
    fi
    relinked=$((relinked + 1))
    if why=$(run_cmp "$WORK/s3swap"); then
        s3_same=$((s3_same + 1))
    else
        note "$why" "$src" s3
    fi
done

names=$(awk '{ print $2 }' "$WORK/taken.txt" | LC_ALL=C sort -u | grep -c .)
refs=$(grep -c . "$WORK/taken.txt")
echo
echo "target: $TARGET, run with arch -x86_64; linked by compiler/link with the Grasm runtime alone"
echo "programs: $programs   external set aside: $external"
echo "Cranelift objects: identical: $cl_same   poisoned runtime, agreeing with no use after free: $p_same"
echo "Grasm backend objects: identical: $s3_same   linked twice, the same bytes: $relinked"
echo "failed: $failed   timed out: $timedout"
echo "runtime names the programs reference: $names distinct, all taken from Grasm; per link, $refs references"
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#TIMEDOUT[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${S3_RUN_SECONDS}s (not agreement):"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
    exit 1
fi
[ "$failed" -eq 0 ] && [ "$timedout" -eq 0 ] || exit 1
if [ "$cl_same" -ne "$programs" ] || [ "$p_same" -ne "$programs" ] || [ "$s3_same" -ne "$programs" ] || [ "$relinked" -ne "$programs" ]; then
    echo "of $programs programs, $cl_same, $p_same and $s3_same agreed, and $relinked linked twice the same"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$programs" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $programs programs in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
if [ "$names" -lt 1 ] || [ "$programs" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
exit 0
