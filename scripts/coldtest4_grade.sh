#!/usr/bin/env bash
# Grades cold-model run 4.
#
# Parts A and B are run 3's tasks unchanged, so the score is comparable across
# runs rather than being a fresh exam each time. Part C covers what the
# language grew afterwards — effect-proven parallelism, registry dependencies,
# and the list-element inference a fuzzer found a hole in — because new surface
# is exactly where a pack that documents nothing hides it.
#
# Strictness is run 3's: a program must compile, run, AND print exactly what
# was asked. "Close enough" is not a measurement.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
DIR="${1:?usage: coldtest4_grade.sh <programs-dir>}"
pass=0; fail=0; declare -a FAILED=()

grade() {
    local file="$1" want="$2" cmd="${3:-run}" name; name=$(basename "$file")
    if [ ! -f "$file" ]; then
        printf '  %-22s MISSING\n' "$name"; fail=$((fail+1)); FAILED+=("$name: not written"); return
    fi
    local got; got=$("$GRASM" "$cmd" "$file" 2>&1); local rc=$?
    if [ $rc -ne 0 ] && [ "$want" != "@fails" ] && [ "${want#@rejects:}" = "$want" ]; then
        printf '  %-22s DID NOT RUN\n' "$name"; echo "$got" | head -2 | sed 's/^/      /'
        fail=$((fail+1)); FAILED+=("$name: did not run"); return
    fi
    case "$want" in
        @any)   printf '  %-22s CORRECT\n' "$name"; pass=$((pass+1)) ;;
        @rejects:*)
            # A program that must NOT compile, and must say why. Accepting it
            # would be the failure; so would rejecting it for a different
            # reason, which is why the message is matched too.
            local needle="${want#@rejects:}"
            if [ $rc -ne 0 ] && echo "$got" | grep -q "$needle"; then
                printf '  %-22s CORRECT (rejected: %s)\n' "$name" "$needle"; pass=$((pass+1))
            else
                printf '  %-22s WRONG (expected rejection mentioning %s)\n' "$name" "$needle"
                echo "$got" | head -2 | sed 's/^/      /'; fail=$((fail+1)); FAILED+=("$name")
            fi ;;
        @fails)
            if [ $rc -ne 0 ] && echo "$got" | grep -q "counterexample"; then
                printf '  %-22s CORRECT (fails with counterexample)\n' "$name"; pass=$((pass+1))
            else
                printf '  %-22s WRONG (expected a reported counterexample)\n' "$name"
                echo "$got" | head -3 | sed 's/^/      /'; fail=$((fail+1)); FAILED+=("$name")
            fi ;;
        *)
            if [ "$got" = "$want" ]; then printf '  %-22s CORRECT\n' "$name"; pass=$((pass+1))
            else
                printf '  %-22s WRONG OUTPUT\n' "$name"
                echo "      got:  $(echo "$got" | tr '\n' '|')"
                echo "      want: $(echo "$want" | tr '\n' '|')"
                fail=$((fail+1)); FAILED+=("$name: wrong output")
            fi ;;
    esac
}

echo "== part A: carry-over from run 3 =="
grade "$DIR/t2_wordcount.gr"   "$(printf 'end=1\nlazy=1\nquick=1\nthe=3')"
grade "$DIR/t5_errors.gr"      "@any"
grade "$DIR/t10_concurrency.gr" "first second"
grade "$DIR/t12_closures.gr"   "12 8"

echo "== part B: run-3 surface =="
grade "$DIR/n1_modules/main.gr" "HI!"
grade "$DIR/n2_structs.gr"     "$(printf '3 4\n7')"
grade "$DIR/n3_nested.gr"      "$(printf '6\n[a, b]')"
grade "$DIR/n4_own.gr"         "5"
grade "$DIR/n5_effect2.gr"     "$(printf 'I: up\nW: down')"
grade "$DIR/n6_recursion.gr"   "55"
grade "$DIR/n7_doctest.gr"     "ok"
grade "$DIR/n9_unicode.gr"     "$(printf '6\n6\né')"
grade "$DIR/n10_property.gr"   "@fails" test

if [ -f "$DIR/n7_doctest.gr" ]; then
    if "$GRASM" test "$DIR/n7_doctest.gr" 2>&1 | grep -q "doc example.*ok"; then
        printf '  %-22s CORRECT (doc example runs)\n' "n7 doc example"; pass=$((pass+1))
    else
        printf '  %-22s WRONG (doc example did not run)\n' "n7 doc example"; fail=$((fail+1)); FAILED+=("n7 doc example")
    fi
fi

if [ -d "$DIR/n8_workspace/app" ]; then
    if "$GRASM" deps "$DIR/n8_workspace/app" >/dev/null 2>&1; then
        out=$("$GRASM" run "$DIR/n8_workspace/app/src/main.gr" 2>&1)
        if [ "$out" = "10" ]; then
            printf '  %-22s CORRECT (resolves, prints 10)\n' "n8_workspace"; pass=$((pass+1))
        else
            printf '  %-22s WRONG OUTPUT: %s\n' "n8_workspace" "$out"; fail=$((fail+1)); FAILED+=("n8_workspace")
        fi
    else
        printf '  %-22s DEPS FAILED\n' "n8_workspace"
        "$GRASM" deps "$DIR/n8_workspace/app" 2>&1 | head -3 | sed 's/^/      /'
        fail=$((fail+1)); FAILED+=("n8_workspace: deps")
    fi
else
    printf '  %-22s MISSING\n' "n8_workspace"; fail=$((fail+1)); FAILED+=("n8_workspace: not written")
fi

echo "== part C: surface added since run 3 =="
grade "$DIR/r1_parallel.gr"    "$(printf '[1, 4, 9, 16, 25]\n[2, 4]')"
grade "$DIR/r2_inference.gr"   "$(printf '[3]\n[12]\n[6]')"
grade "$DIR/r4_effectfail.gr"  "@rejects:effect-free function" check

# A registry dependency must resolve and run, which needs the manifest written
# with a `version` and no `path`, and the package placed under the registry.
if [ -d "$DIR/r3_registry/app" ]; then
    if GRASM_REGISTRY="$DIR/r3_registry/registry" "$GRASM" deps "$DIR/r3_registry/app" >/dev/null 2>&1; then
        out=$(GRASM_REGISTRY="$DIR/r3_registry/registry" "$GRASM" run "$DIR/r3_registry/app/src/main.gr" 2>&1)
        if [ "$out" = "HELLO" ]; then
            printf '  %-22s CORRECT (registry dep resolves)\n' "r3_registry"; pass=$((pass+1))
        else
            printf '  %-22s WRONG OUTPUT: %s\n' "r3_registry" "$out"; fail=$((fail+1)); FAILED+=("r3_registry")
        fi
    else
        printf '  %-22s DEPS FAILED\n' "r3_registry"
        GRASM_REGISTRY="$DIR/r3_registry/registry" "$GRASM" deps "$DIR/r3_registry/app" 2>&1 | head -3 | sed 's/^/      /'
        fail=$((fail+1)); FAILED+=("r3_registry: deps")
    fi
else
    printf '  %-22s MISSING\n' "r3_registry"; fail=$((fail+1)); FAILED+=("r3_registry: not written")
fi

echo
echo "  passed $pass / $((pass+fail))"
[ ${#FAILED[@]} -gt 0 ] && printf '  failed: %s\n' "${FAILED[@]}"
exit 0
