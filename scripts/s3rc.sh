#!/usr/bin/env bash
# The Grasm backend's reference counting, checked the way `leakcheck.sh` and
# `usecheck.sh` check the Cranelift build's (docs/self-hosting.md, stage S3).
#
# `compiler/mir` makes every `inc` and `dec` an instruction of its own, placed
# by the ownership rules in `compiler/mir/src/lower.gr`. Output alone cannot
# show they are right: a leaking program prints exactly what a clean one does,
# and a value freed too early usually still holds its bytes. So every corpus
# program in the subset (`scripts/s3_subset.py`) is compiled by the backend
# and run twice:
#
#   * leaks -- linked with the ordinary runtime and run with
#     `GRASM_LEAK_CHECK=1`, so the runtime counts what is still live when
#     `main` returns: the line `leak check: nothing live at exit` is clean,
#     exit 7 with `leak:` is a leak. A program that ends before `main` returns
#     -- a panic (101), an error out of `main` (1), its own `exit` -- never
#     reaches the check and is counted as that, by name; a death by signal
#     fails the gate.
#   * use after free -- linked with the runtime built with
#     `--features grasm_rt/poison`, which never reuses a freed box and stops
#     the program at the first touch of one ("use after free").
#
# Leaks 0, uses after free 0, no crash, no timeout -- and enough programs
# reaching the leak check that the count means something.
#
# `S3_POISON_RT=<libgrasm_rt_abi.a>` names a poisoned runtime instead of
# building one (into `target/s3rc-poison`, a staticlib of a few megabytes).
# `S3=`, `S3_EXTRA` and `S3_STOP_AT_FIRST` as in `scripts/s3subset.sh`, for
# the controls.
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
. "$(dirname "$0")/lib/native.sh"
s3_driver
s3_runtime
EXTRA=${S3_EXTRA:-}
STOP=${S3_STOP_AT_FIRST:-}
stopped=0

POISON_RT=${S3_POISON_RT:-}
if [ -z "$POISON_RT" ]; then
    if ! cargo build --release -q -p grasm_rt_abi --features grasm_rt/poison --target-dir target/s3rc-poison; then
        echo "error: the poisoned runtime does not build"
        exit 1
    fi
    POISON_RT=target/s3rc-poison/release/libgrasm_rt_abi.a
fi
if [ ! -f "$POISON_RT" ]; then
    echo "error: $POISON_RT is missing"
    exit 1
fi

s3_collect

programs=0
clean=0
leaked=0
early=0
uaf=0
uaf_clean=0
exhausted=0
failed=0
declare -a LEAKS=()
declare -a EARLY=()
declare -a UAF=()
declare -a FAILED=()

for k in "${!S3_SRC[@]}"; do
    [ "${S3_RULE[$k]}" = "in" ] || continue
    if [ -n "$STOP" ] && [ $((leaked + uaf + failed)) -gt 0 ]; then
        stopped=1
        break
    fi
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    programs=$((programs + 1))
    rm -f "$WORK/p.o"
    # shellcheck disable=SC2086
    if ! "$S3" "$tir" -o "$WORK/p.o" $EXTRA >"$WORK/s3.log" 2>&1; then
        failed=$((failed + 1))
        FAILED+=("$src — the backend does not compile it: $(head -1 "$WORK/s3.log")")
        continue
    fi
    # Leaks, against the ordinary runtime.
    if ! s3_link "$WORK/p.o" "$WORK/p" "$WORK/link.log"; then
        failed=$((failed + 1))
        FAILED+=("$src — the object does not link: $(head -1 "$WORK/link.log")")
        continue
    fi
    ended=$(GRASM_LEAK_CHECK=1 python3 scripts/lib/exitkind.py "$S3_RUN_SECONDS" "$WORK/p" 2>"$WORK/err" </dev/null)
    code=${ended#exit }
    # A crash here is a failure, and the poisoned run below still happens:
    # it is what says whether the crash was a use after free.
    if [ "$ended" = timeout ]; then
        failed=$((failed + 1))
        FAILED+=("$src — timed out after ${S3_RUN_SECONDS}s")
    elif [ "${ended%% *}" = signal ]; then
        failed=$((failed + 1))
        FAILED+=("$src — killed by ${ended}")
    elif [ "$code" = 7 ] && grep -c '^leak: ' "$WORK/err" >/dev/null; then
        leaked=$((leaked + 1))
        LEAKS+=("$src — $(grep '^leak: ' "$WORK/err" | head -1)")
    elif [ "$code" = 0 ] && grep -c '^leak check: nothing live at exit$' "$WORK/err" >/dev/null; then
        clean=$((clean + 1))
    else
        # A panic (101), an error out of `main` (1), or the program's own
        # `exit`: deliberate, and the same as the Cranelift build's, which
        # `s3subset` holds it to. Named, so a program that stopped reaching
        # the check is seen.
        early=$((early + 1))
        EARLY+=("$src — exit $code: $(tail -1 "$WORK/err" | cut -c1-80)")
    fi
    # Uses after free, against the poisoned runtime.
    if ! s3_link "$WORK/p.o" "$WORK/q" "$WORK/plink.log" "$POISON_RT"; then
        failed=$((failed + 1))
        FAILED+=("$src — the object does not link with the poisoned runtime: $(head -1 "$WORK/plink.log")")
        continue
    fi
    # How it ended, told apart as `exitkind.py` tells them: a program's own
    # `exit(-1)` is 255, not a signal.
    pended=$(python3 scripts/lib/exitkind.py "$S3_RUN_SECONDS" "$WORK/q" 2>"$WORK/perr" </dev/null)
    if grep -c "use after free" "$WORK/perr" >/dev/null; then
        uaf=$((uaf + 1))
        UAF+=("$src — $(grep 'use after free' "$WORK/perr" | head -1)")
        echo "USE AFTER FREE: $src"
    elif [ "$pended" = timeout ]; then
        failed=$((failed + 1))
        FAILED+=("$src — timed out under the poisoned runtime")
    elif [ "$pended" = "signal 6" ] || grep -ci "memory allocation of" "$WORK/perr" >/dev/null; then
        exhausted=$((exhausted + 1))
    elif [ "${pended%% *}" = signal ]; then
        failed=$((failed + 1))
        FAILED+=("$src — killed by ${pended} under the poisoned runtime")
    else
        uaf_clean=$((uaf_clean + 1))
    fi
done

echo "programs: $programs   leak-checked: $clean   leaking: $leaked   ended before the check: $early"
echo "use-after-free: $uaf   clean under poison: $uaf_clean   out of memory under poison: $exhausted   failed: $failed"
if [ ${#EARLY[@]} -gt 0 ]; then
    echo
    echo "ended before main returned, so the leak check never ran:"
    printf '  %s\n' "${EARLY[@]}"
fi
if [ ${#LEAKS[@]} -gt 0 ]; then
    echo
    echo "LEAKS:"
    printf '  %s\n' "${LEAKS[@]}"
fi
if [ ${#UAF[@]} -gt 0 ]; then
    echo
    echo "USE AFTER FREE:"
    printf '  %s\n' "${UAF[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
fi
[ "$leaked" -eq 0 ] && [ "$uaf" -eq 0 ] && [ "$failed" -eq 0 ] || exit 1
# Every program was accounted for, twice.
if [ $((clean + leaked + early)) -ne "$programs" ] || [ $((uaf_clean + exhausted)) -ne "$programs" ]; then
    echo "the counts do not add up over $programs programs"
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ "$clean" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $clean programs reached the leak check; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
[ "$clean" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
