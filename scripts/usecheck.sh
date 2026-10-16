#!/usr/bin/env bash
# Every program in the corpus, run against a runtime that never reuses a freed
# box and refuses to touch one.
#
# `leakcheck.sh` cannot see a premature free. It counts what is still live at
# exit, and a value freed too early is *less* live, not more — so a
# use-after-free reads as a clean run. One went undetected through all 497
# programs while a borrow optimisation was being written, and only crashed when
# the program was run by hand.
#
# Under `--features grasm_rt/poison` a freed box is marked and kept out of
# circulation, so the next thing to touch it says so. The cost is that nothing
# is ever reclaimed: a program that allocates heavily will exhaust memory here
# and abort, which is reported as EXHAUSTED rather than as a defect.
set -uo pipefail

cd "$(dirname "$0")/.."
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}

echo "building the runtime with poisoning on"
if ! cargo build --release --workspace --features grasm_rt/poison >/dev/null 2>&1; then
    echo "error: the poisoned build failed"
    exit 1
fi
GRASM=./target/release/grasm
# Straight into `/tmp`, not a directory under it, and not under `$TMPDIR`.
#
# The one use-after-free this has found so far — in `packages/aes-demo` — shows
# itself only when the binary sits directly in `/tmp`. One directory deeper
# hides it, and `$TMPDIR` on macOS is sixty characters of `/var/folders/...`,
# which hides it too. That is exactly why `difftest.sh` has never seen it: it
# builds into a working directory under `$TMPDIR`.
#
# Why the depth matters is not understood. What is measured is that it does,
# so the shallowest path available is the one to use.
WORK=/tmp
PREFIX="gu$$"

# Restore the ordinary runtime whatever happens: leaving a poisoned archive in
# `target` would make every later build leak by design.
trap 'rm -f "$WORK/$PREFIX"_*; cargo build --release --workspace >/dev/null 2>&1' EXIT

n=0
clean=0
caught=0
external=0
declined=0
timedout=0
exhausted=0
declare -a CAUGHT=()
declare -a EXTERNAL=()
declare -a DECLINED=()
declare -a TIMEDOUT=()

# Every program below runs with stdin closed. A program that reads stdin
# otherwise inherits this script's, and `packages/rmcp-demo` -- an MCP server
# whose loop runs until end of input -- hung the whole suite for an hour with
# nothing printed, because this gate buffers its output until the end. Closed
# stdin is also the only comparable choice: `input()` answers "" at end of
# input, so both engines see the same thing instead of whatever a terminal
# happened to supply.
for src in $(corpus_programs); do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q 'expect: error' "$src" && continue
    # Numbered, not named after the source. The whole path has to stay short:
    # see the note on `WORK` above. A name mangled from the source path is
    # sixty characters on its own and hides the bug again.
    n=$((n + 1))
    bin="$WORK/${PREFIX}_$n"
    # A program that does not build is a program this gate did not check.
    # `|| continue` counted it as nothing at all, so it vanished rather than
    # failing -- which is why this said "clean: 521" while three programs in
    # the corpus could not be linked. Classified instead: needing a system
    # library is a fact about the machine, anything else is a defect.
    if ! "$GRASM" build "$src" --native -o "$bin" >/dev/null 2>&1; then
        if corpus_external "$src"; then
            external=$((external + 1))
            EXTERNAL+=("$src")
        else
            declined=$((declined + 1))
            DECLINED+=("$src")
        fi
        continue
    fi
    err=$($RUN_LIMIT "$bin" 2>&1 >/dev/null </dev/null)
    code=$?
    # A program stopped at its wall clock did not finish, so nothing was
    # learned about whether it frees what it allocates. Counting it as clean
    # is the same mistake as counting a build failure as nothing.
    if [ "$code" -eq 124 ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$src")
        echo "TIMED OUT: $src after ${GRASM_RUN_LIMIT:-120}s"
        continue
    fi
    # `grep -c … >/dev/null` rather than `grep -q`: under `pipefail` a `-q`
    # that matches exits early, the writer takes SIGPIPE, and the pipeline
    # reports failure for a pattern that was found. `gatecheck.sh` watches for
    # exactly this.
    if echo "$err" | grep -c "use after free" >/dev/null; then
        caught=$((caught + 1))
        CAUGHT+=("$src")
        echo "USE AFTER FREE: $src"
        echo "$err" | grep "use after free" | head -1
    elif [ "$code" -eq 134 ] || echo "$err" | grep -ci "memory allocation of" >/dev/null; then
        exhausted=$((exhausted + 1))
    else
        clean=$((clean + 1))
    fi
done

echo
echo "=========================================="
echo "clean: $clean   use-after-free: $caught   out of memory: $exhausted   \
external: $external   declined: $declined   timed out: $timedout"
echo "=========================================="

if [ "$declined" -gt 0 ]; then
    echo
    echo "the native backend declined these, so they were never run:"
    printf '  %s\n' "${DECLINED[@]}"
fi
if [ "$timedout" -gt 0 ]; then
    echo
    echo "these did not finish, so nothing was learned about them:"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
if [ "$external" -gt 0 ]; then
    echo
    echo "needs a system library, so checked by sdltest instead:"
    printf '  %s\n' "${EXTERNAL[@]}"
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
[ "$caught" -eq 0 ] && [ "$declined" -eq 0 ] && [ "$timedout" -eq 0 ]
