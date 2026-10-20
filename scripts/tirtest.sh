#!/usr/bin/env bash
# TIR for the whole corpus (docs/tir.md, docs/self-hosting.md stage S0).
#
# Every program that checks must emit TIR with no holes: a hole is a decision
# the checker made and did not write down, and a consumer of TIR would have to
# make it again -- the duplicated inference TIR exists to end. The programs
# that do not check must be exactly the conformance files that say they
# should not (`//! expect: error`), so a compiler that stopped checking half
# the corpus cannot pass by emitting nothing for it.
#
# Every fifth program is emitted twice and the two must be the same bytes:
# TIR is compared between two frontends, so the same input has to give the
# same text every time.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "$GRASM"
. "$(dirname "$0")/lib/corpus.sh"
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

emitted=0
holed=0
unchecked=0
twice=0
unstable=0
i=0
: > "$WORK/unchecked"
for src in $(corpus_programs); do
    i=$((i + 1))
    if "$GRASM" tir "$src" > "$WORK/a.tir" 2> "$WORK/a.err"; then
        emitted=$((emitted + 1))
        if [ $((i % 5)) = 0 ]; then
            twice=$((twice + 1))
            "$GRASM" tir "$src" > "$WORK/b.tir" 2>/dev/null
            if ! cmp -s "$WORK/a.tir" "$WORK/b.tir"; then
                unstable=$((unstable + 1))
                echo "NOT DETERMINISTIC: $src"
            fi
        fi
    elif grep -q 'hole(s)$' "$WORK/a.err"; then
        holed=$((holed + 1))
        echo "HOLES: $src"
        grep '^hole ' "$WORK/a.err" | head -5 | sed 's/^/  /'
    else
        unchecked=$((unchecked + 1))
        echo "$src" >> "$WORK/unchecked"
    fi
done

grep -l '^//! expect: error' spec/conformance/*.gr | sort > "$WORK/expected"
sort "$WORK/unchecked" > "$WORK/unchecked.sorted"
mismatch=$(comm -3 "$WORK/expected" "$WORK/unchecked.sorted")
if [ -n "$mismatch" ]; then
    echo "the programs that do not check are not the ones that say they should not:"
    echo "$mismatch" | sed 's/^/  /'
fi

echo "emitted: $emitted   holes: $holed   not checking: $unchecked   emitted twice: $twice   unstable: $unstable"
if [ "$holed" -ne 0 ] || [ "$unstable" -ne 0 ] || [ -n "$mismatch" ]; then
    exit 1
fi
# A gate that emitted nothing has checked nothing.
if [ "$emitted" -lt 1000 ] || [ "$twice" -lt 200 ]; then
    echo "too few programs emitted ($emitted) or compared ($twice) for this to mean anything"
    exit 1
fi
exit 0
