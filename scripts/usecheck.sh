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
exhausted=0
declare -a CAUGHT=()

for src in $(corpus_programs); do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q 'expect: error' "$src" && continue
    # Numbered, not named after the source. The whole path has to stay short:
    # see the note on `WORK` above. A name mangled from the source path is
    # sixty characters on its own and hides the bug again.
    n=$((n + 1))
    bin="$WORK/${PREFIX}_$n"
    "$GRASM" build "$src" --native -o "$bin" >/dev/null 2>&1 || continue
    err=$("$bin" 2>&1 >/dev/null)
    code=$?
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
echo "clean: $clean   use-after-free: $caught   out of memory: $exhausted"
echo "=========================================="
[ "$caught" -eq 0 ]
