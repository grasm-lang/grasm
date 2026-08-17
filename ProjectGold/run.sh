#!/usr/bin/env bash
# Every ProjectGold program, under all three engines, against its expected output.
#
# A program that works under `grasm run` and not under `--native` is a defect
# with a number, not a program — so all three are run and compared, exactly as
# scripts/difftest.sh does for the repository's own corpus. The difference here
# is the corpus: these were written to somebody else's list, so they probe the
# places nobody working on the language thought to probe.
set -uo pipefail
cd "$(dirname "$0")"

GRASM=${GRASM:-../target/release/grasm}
if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")

WORK="${TMPDIR:-/tmp}/projectgold_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; blocked=0
declare -a FAILED=() BLOCKED=()

for dir in [0-9]*/; do
    name=${dir%/}
    [ -f "$dir/main.gr" ] || continue

    # A project may declare itself blocked on a missing language feature. It
    # still gets a directory and a README saying what it needed, because a gap
    # nobody wrote down is a gap nobody fixes.
    if [ -f "$dir/BLOCKED" ]; then
        blocked=$((blocked + 1))
        BLOCKED+=("$name — $(head -1 "$dir/BLOCKED")")
        continue
    fi

    stdin_file="$dir/input.txt"
    [ -f "$stdin_file" ] || stdin_file=/dev/null
    expected="$dir/expected.txt"

    # 1. the interpreter, which is normative
    got_run=$("$GRASM" run "$dir/main.gr" < "$stdin_file" 2>"$WORK/run.err"); run_code=$?

    # 2. the bundle, which is a self-contained executable and is *run*, not
    #    handed back to `grasm run` — it is not source any more.
    if "$GRASM" build "$dir/main.gr" -o "$WORK/$name.bundle" >/dev/null 2>&1; then
        got_bundle=$("$WORK/$name.bundle" < "$stdin_file" 2>/dev/null); bundle_code=$?
    else
        got_bundle="<did not build>"; bundle_code=-1
    fi

    # 3. the native build
    nat_log=$("$GRASM" build "$dir/main.gr" --native -o "$WORK/$name.native" 2>&1)
    if [ -x "$WORK/$name.native" ]; then
        got_native=$("$WORK/$name.native" < "$stdin_file" 2>/dev/null); native_code=$?
    else
        got_native="<did not build>"; native_code=-1
    fi

    problems=""
    if [ -f "$expected" ]; then
        want=$(cat "$expected")
        [ "$got_run" = "$want" ] || problems="$problems interpreter-output"
    fi
    [ "$got_run" = "$got_bundle" ] || problems="$problems bundle-differs"
    [ "$got_run" = "$got_native" ] || problems="$problems native-differs"
    [ "$run_code" = "$bundle_code" ] || problems="$problems bundle-exit"
    [ "$run_code" = "$native_code" ] || problems="$problems native-exit"

    if [ -n "$problems" ]; then
        fail=$((fail + 1))
        FAILED+=("$name —$problems")
        if [ "$got_native" = "<did not build>" ]; then
            FAILED+=("    native: $(echo "$nat_log" | head -1)")
        fi
        if [ -s "$WORK/run.err" ] && [ "$got_run" != "$(cat "$expected" 2>/dev/null)" ]; then
            FAILED+=("    run: $(head -1 "$WORK/run.err")")
        fi
    else
        pass=$((pass + 1))
    fi
done

echo "agreeing: $pass   failing: $fail   blocked: $blocked"
if [ ${#BLOCKED[@]} -gt 0 ]; then
    echo
    echo "blocked on a missing language feature:"
    printf '  %s\n' "${BLOCKED[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILING:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
