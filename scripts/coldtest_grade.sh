#!/usr/bin/env bash
# Grades a cold-model run: programs written from the language pack alone, by a
# model with no prior exposure to Grasm (it exists in no training data).
#
# Two questions. Does each program compile and run? And does it print exactly
# what was asked? A program that compiles but prints the wrong thing is not a
# pass — the point is whether the specification teaches the language, not
# whether it teaches plausible-looking syntax.
#
# Usage: scripts/coldtest_grade.sh <programs-dir>
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
DIR="${1:?usage: coldtest_grade.sh <programs-dir>}"

pass=0; fail=0
declare -a FAILED=()

grade() {
    local file="$1" want="$2" cmd="${3:-run}"
    local name; name=$(basename "$file")
    if [ ! -f "$file" ]; then
        printf '  %-22s MISSING\n' "$name"; fail=$((fail+1)); FAILED+=("$name: not written"); return
    fi
    local got; got=$("$GRASM" "$cmd" "$file" 2>&1); local rc=$?
    if [ $rc -ne 0 ]; then
        printf '  %-22s DID NOT COMPILE/RUN\n' "$name"
        echo "$got" | head -2 | sed 's/^/      /'
        fail=$((fail+1)); FAILED+=("$name: did not run"); return
    fi
    if [ "$want" = "@any" ] || [ "$got" = "$want" ]; then
        printf '  %-22s CORRECT\n' "$name"; pass=$((pass+1))
    else
        printf '  %-22s WRONG OUTPUT\n' "$name"
        echo "      got:  $(echo "$got" | tr '\n' '|')"
        echo "      want: $(echo "$want" | tr '\n' '|')"
        fail=$((fail+1)); FAILED+=("$name: wrong output")
    fi
}

echo "== original eight =="
grade "$DIR/t1_fizzbuzz.gr"   "$(printf '1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz')"
grade "$DIR/t2_wordcount.gr"  "$(printf 'end=1\nlazy=1\nquick=1\nthe=3')"
grade "$DIR/t3_binsearch.gr"  "$(printf '3\n-1')"
grade "$DIR/t4_shapes.gr"     "$(printf '12.56\n12.0')"
grade "$DIR/t5_errors.gr"     "@any"
grade "$DIR/t6_effect.gr"     "42"
grade "$DIR/t7_property.gr"   "@any" test
grade "$DIR/t8_parmap.gr"     "332833500"

echo "== harder set =="
grade "$DIR/t9_generic.gr"     "$(printf '10\nnone')"
grade "$DIR/t10_concurrency.gr" "first second"
grade "$DIR/t11_inout.gr"      "2"
grade "$DIR/t12_closures.gr"   "@any"
grade "$DIR/t13_guards.gr"     "$(printf 'neg\nzero\nsmall\nbig')"
grade "$DIR/t14_bench.gr"      "ready"

echo
echo "  passed $pass / $((pass+fail))"
if [ ${#FAILED[@]} -gt 0 ]; then
    printf '  failed: %s\n' "${FAILED[@]}"
fi
