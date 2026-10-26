#!/usr/bin/env bash
# The Grasm frontend's first diagnostic against the Rust parser's
# (docs/self-hosting.md, S2).
#
# Every program the Rust parser rejects must be rejected by the Grasm one
# with the same first diagnostic: message, span and help. The programs:
#
#   * the conformance programs marked `//! expect: error`;
#   * the programs written inline in the Rust test suites -- the checker's
#     check tests, the parser's, the module and stress tests, the CLI's --
#     read out of their string literals (scripts/lib/rust_sources.py);
#   * 600 programs `scripts/fuzz_front.py` makes from a fixed seed.
#
# Most of them are rejected by the checker, not the parser; those the Rust
# parser accepts must be accepted by the Grasm one too, and are counted
# apart. A Rust parser panic is a Rust bug and is listed, never counted as
# agreement. The counts must add up, and a run that found too few parser
# negatives fails.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if ! "$GRASM" ast /dev/null >/dev/null 2>&1; then
    echo "error: $GRASM has no \`ast\` command, so there is nothing to compare against"
    exit 1
fi
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
mkdir -p "$WORK/rust" "$WORK/fuzz"
inline=$(python3 scripts/lib/rust_sources.py "$WORK/rust" crates/grasm_sema/tests/check_tests.rs \
    crates/grasm_syntax/tests/parser_tests.rs crates/grasm_sema/tests/module_tests.rs \
    crates/grasm_sema/tests/stress_tests.rs crates/grasm/tests/cli_tests.rs)
find "$WORK/rust" -name '*.gr' | sort >>"$WORK/files"
if ! GRASM="$GRASM" GRASM_NO_BUILD=1 python3 scripts/fuzz_front.py --emit "$WORK/fuzz" --programs 600 --seed 7 >/dev/null; then
    echo "error: scripts/fuzz_front.py --emit failed"
    exit 1
fi
find "$WORK/fuzz" -name '*.gr' | sort >>"$WORK/files"
listed=$(wc -l <"$WORK/files" | tr -d ' ')

negatives=0
same=0
differ=0
accepted=0
rust_panics=0
declare -a FAILED=()
declare -a PANICS=()
while IFS= read -r f; do
    "$GRASM" ast "$f" >"$WORK/ref.out" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" ast "$f" >"$WORK/got.out" 2>"$WORK/got.err"
    gc=$?
    if grep -q 'panicked at' "$WORK/ref.err"; then
        rust_panics=$((rust_panics + 1))
        PANICS+=("$f — $(grep -o 'panicked at [^ ]*' "$WORK/ref.err" | head -1)")
        continue
    fi
    if [ "$rc" = 0 ]; then
        if [ "$gc" = 0 ]; then
            accepted=$((accepted + 1))
        else
            differ=$((differ + 1))
            FAILED+=("$f — the Rust parser accepts it, the Grasm one does not: $(sed -n 2p "$WORK/got.out" | cut -c1-200)")
        fi
        continue
    fi
    negatives=$((negatives + 1))
    want=$(sed -n 2p "$WORK/ref.out")
    got=$(sed -n 2p "$WORK/got.out")
    if [ "$rc" = 1 ] && [ "$gc" = 1 ] && [ -n "$want" ] && [ "$want" == "$got" ]; then
        same=$((same + 1))
    else
        differ=$((differ + 1))
        FAILED+=("$f — exit $rc vs $gc
      rust:  $(echo "$want" | cut -c1-240)
      front: $(echo "$got" | cut -c1-240)")
    fi
done <"$WORK/files"

echo "programs: $listed (conformance $conf, inline in Rust tests $inline, fuzz 600)"
echo "rejected by the Rust parser: $negatives   same first diagnostic: $same   differ: $differ"
echo "accepted by both: $accepted   rust panics: $rust_panics"
if [ ${#PANICS[@]} -gt 0 ]; then
    echo
    echo "THE RUST PARSER PANICKED:"
    printf '  %s\n' "${PANICS[@]}" | head -20
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -60
fi
if [ $((same + accepted + rust_panics + differ)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed programs"
    exit 1
fi
if [ "$differ" -ne 0 ]; then
    exit 1
fi
# A Rust parser panic is a Rust bug, and none is known: the one there was,
# an unknown escape before a multi-byte character, is fixed.
if [ "$rust_panics" -ne 0 ]; then
    echo "the Rust parser panicked"
    exit 1
fi
if [ "$negatives" -lt 200 ]; then
    echo "too few parser negatives ($negatives) for this to mean anything"
    exit 1
fi
exit 0
