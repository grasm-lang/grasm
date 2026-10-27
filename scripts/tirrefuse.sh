#!/usr/bin/env bash
# What the Rust frontend refuses, the Grasm one refuses (docs/self-hosting.md,
# S2 part two).
#
# The programs: the conformance programs marked `//! expect: error`, and the
# programs written inline in the Rust test suites -- the checker's check
# tests, the parser's, the module, stress and CLI tests -- read out of their
# string literals (scripts/lib/rust_sources.py). Most are negatives: a type
# error, an effect a row does not allow, a module that does not load. For
# each one `grasm tir` and `front tir` must both refuse it, or both accept it
# with the same TIR, byte for byte.
#
# Which diagnostic, at which span, is part three's question; a program one
# side accepts and the other refuses is this gate's. A Rust panic is a Rust
# bug and fails the gate. The counts must add up, and too few refusals
# compared fails.
#
# `FRONT=` names a prebuilt frontend (the controls use it).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

FRONT=${FRONT:-}
if [ -z "$FRONT" ]; then
    FRONT="$WORK/front"
    if ! "$GRASM" build compiler/front/src/main.gr --native -o "$FRONT" >"$WORK/build.log" 2>&1; then
        echo "error: compiler/front does not build:"
        sed 's/^/  /' "$WORK/build.log"
        exit 1
    fi
fi

: >"$WORK/files"
git grep -l '^//! expect: error$' -- 'spec/conformance/*.gr' >>"$WORK/files"
conf=$(wc -l <"$WORK/files" | tr -d ' ')
mkdir -p "$WORK/rust"
python3 scripts/lib/rust_sources.py "$WORK/rust" crates/grasm_sema/tests/check_tests.rs \
    crates/grasm_syntax/tests/parser_tests.rs crates/grasm_sema/tests/module_tests.rs \
    crates/grasm_sema/tests/stress_tests.rs crates/grasm/tests/cli_tests.rs >/dev/null
find "$WORK/rust" -name '*.gr' | sort >>"$WORK/files"
listed=$(wc -l <"$WORK/files" | tr -d ' ')

refused=0
conf_refused=0
accepted=0
differ=0
panics=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" tir "$f" >"$WORK/ref.tir" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" tir "$f" >"$WORK/got.tir" 2>"$WORK/got.err"
    gc=$?
    if grep -q 'panicked at' "$WORK/ref.err"; then
        panics=$((panics + 1))
        FAILED+=("$f — the Rust frontend panics: $(grep -o 'panicked at [^ ]*' "$WORK/ref.err" | head -1)")
        continue
    fi
    if [ "$rc" != 0 ] && [ "$gc" != 0 ]; then
        refused=$((refused + 1))
        case "$f" in
            spec/*) conf_refused=$((conf_refused + 1)) ;;
        esac
    elif [ "$rc" = 0 ] && [ "$gc" = 0 ] && cmp -s "$WORK/ref.tir" "$WORK/got.tir"; then
        accepted=$((accepted + 1))
    else
        differ=$((differ + 1))
        if [ "$rc" = 0 ] && [ "$gc" = 0 ]; then
            FAILED+=("$f — both accept it and the TIR differs")
        elif [ "$rc" = 0 ]; then
            FAILED+=("$f — the Rust frontend accepts it and the Grasm one refuses it: $(head -1 "$WORK/got.err" | cut -c1-160)")
        else
            FAILED+=("$f — the Rust frontend refuses it ($(grep -m1 'error' "$WORK/ref.err" | cut -c1-160)) and the Grasm one accepts it")
        fi
    fi
done <"$WORK/files"

echo "programs: $listed ($conf conformance negatives, $((listed - conf)) from the Rust test suites)"
echo "refused by both: $refused (conformance: $conf_refused)   accepted by both, same TIR: $accepted   disagree: $differ   Rust panics: $panics"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -60
fi
if [ $((refused + accepted + differ + panics)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed programs listed"
    exit 1
fi
if [ "$differ" -ne 0 ] || [ "$panics" -ne 0 ]; then
    exit 1
fi
if [ "$conf_refused" -ne "$conf" ] || [ "$refused" -lt 500 ] || [ "$accepted" -lt 250 ]; then
    echo "too few programs compared: $refused refused ($conf_refused of $conf conformance negatives), $accepted accepted"
    exit 1
fi
exit 0
