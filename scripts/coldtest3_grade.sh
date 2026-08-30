#!/usr/bin/env bash
# Grades cold-model run 3. Same strictness as run 2's grader: a program must
# compile, run, AND print exactly what was asked.
#
# Run 3 adds two multi-file tasks — a module pair and a two-package workspace —
# because `import` and the capability manifest were each mentioned twice in the
# language pack and demonstrated nowhere, which is where gaps hide.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
DIR="${1:?usage: coldtest3_grade.sh <programs-dir>}"
pass=0; fail=0; declare -a FAILED=()

grade() {
    local file="$1" want="$2" cmd="${3:-run}" name; name=$(basename "$file")
    if [ ! -f "$file" ]; then
        printf '  %-22s MISSING\n' "$name"; fail=$((fail+1)); FAILED+=("$name: not written"); return
    fi
    local got; got=$("$GRASM" "$cmd" "$file" 2>&1); local rc=$?
    if [ $rc -ne 0 ] && [ "$want" != "@fails" ]; then
        printf '  %-22s DID NOT RUN\n' "$name"; echo "$got" | head -2 | sed 's/^/      /'
        fail=$((fail+1)); FAILED+=("$name: did not run"); return
    fi
    case "$want" in
        @any)   printf '  %-22s CORRECT\n' "$name"; pass=$((pass+1)) ;;
        @fails)
            # A property test that must fail, and must name a counterexample.
            if [ $rc -ne 0 ] && echo "$got" | grep -c "counterexample" >/dev/null; then
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

echo "== part A: carry-over =="
grade "$DIR/t2_wordcount.gr"   "$(printf 'end=1\nlazy=1\nquick=1\nthe=3')"
grade "$DIR/t5_errors.gr"      "@any"
grade "$DIR/t10_concurrency.gr" "first second"
grade "$DIR/t12_closures.gr"   "12 8"

echo "== part B: new surface =="
grade "$DIR/n1_modules/main.gr" "HI!"
grade "$DIR/n2_structs.gr"     "$(printf '3 4\n7')"
grade "$DIR/n3_nested.gr"      "$(printf '6\n[a, b]')"
grade "$DIR/n4_own.gr"         "5"
grade "$DIR/n5_effect2.gr"     "$(printf 'I: up\nW: down')"
grade "$DIR/n6_recursion.gr"   "55"
grade "$DIR/n7_doctest.gr"     "ok"
grade "$DIR/n9_unicode.gr"     "$(printf '6\n6\né')"
grade "$DIR/n10_property.gr"   "@fails" test

# The doc example inside n7 must actually run and pass under `grasm test`.
if [ -f "$DIR/n7_doctest.gr" ]; then
    if "$GRASM" test "$DIR/n7_doctest.gr" 2>&1 | grep -c "doc example.*ok" >/dev/null; then
        printf '  %-22s CORRECT (doc example runs)\n' "n7 doc example"; pass=$((pass+1))
    else
        printf '  %-22s WRONG (doc example did not run)\n' "n7 doc example"; fail=$((fail+1)); FAILED+=("n7 doc example")
    fi
fi

# The workspace must resolve with no capability violation, and app must run.
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

echo
echo "  passed $pass / $((pass+fail))"
[ ${#FAILED[@]} -gt 0 ] && printf '  failed: %s\n' "${FAILED[@]}"
exit 0
