#!/usr/bin/env bash
# The Grasm frontend's diagnostics against the Rust frontend's
# (docs/self-hosting.md, S2 part three).
#
# `front check P` must print what `grasm check P` prints -- stdout, stderr and
# exit status, byte for byte: every diagnostic of the loader and the parser,
# or of the checker and the effect pass, in Rust's order, each with its
# file:line:column, its source line, its carets and its help; the count line;
# the capability ceilings' breaches with their help; or the `ok` line. The
# programs:
#
#   * the conformance programs marked `//! expect: error`;
#   * the programs written inline in the Rust test suites -- the checker's
#     check tests, the parser's, the module, stress and CLI tests -- read out
#     of their string literals (scripts/lib/rust_sources.py);
#   * the package graphs `scripts/lib/diagcases.py` writes, whose manifests,
#     dependencies, imports or effects are wrong in one way each -- nothing
#     in the repository reaches the manifest's or the resolver's words -- and
#     a few right in an unusual way (a dependency through a symbolic link);
#   * a `^` chain and an `else if` chain of 6,000, 20,000 and 50,000 terms,
#     and every shape fuzz_front's deep generator makes at every depth it
#     makes it (scripts/lib/chains.py): the nesting cap counts neither chain,
#     so every pass has to walk them without recursing per term -- and, for
#     the 6,000-term chains, `front tir` against `grasm tir` too, compared by
#     digest, since their TIR is 72 and 361 MB;
#   * every corpus program and every other `.gr` file git tracks, as tirdiff
#     takes them;
#   * 700 programs `scripts/fuzz_tir.py` makes from a fixed seed, near-misses
#     and package graphs among them.
#
# A Rust panic is a Rust bug and is listed, never counted as agreement. The
# counts must add up, every conformance negative must be refused with the
# same output, and a run that compared too few refusals or too few
# diagnostics fails: a gate that compared nothing has checked nothing.
#
# `FRONT=` names a prebuilt frontend (the controls use it); `FILES=` a file
# listing the paths to compare instead.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
corpus_require

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
conf=0
inline=0
cases=0
chains=0
fuzz=0
if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    git grep -l '^//! expect: error$' -- 'spec/conformance/*.gr' >>"$WORK/files"
    conf=$(wc -l <"$WORK/files" | tr -d ' ')
    mkdir -p "$WORK/rust"
    inline=$(python3 scripts/lib/rust_sources.py "$WORK/rust" crates/grasm_sema/tests/check_tests.rs \
        crates/grasm_syntax/tests/parser_tests.rs crates/grasm_sema/tests/module_tests.rs \
        crates/grasm_sema/tests/stress_tests.rs crates/grasm/tests/cli_tests.rs)
    find "$WORK/rust" -name '*.gr' | sort >>"$WORK/files"
    python3 scripts/lib/diagcases.py "$WORK/cases" >"$WORK/cases.list"
    cases=$(wc -l <"$WORK/cases.list" | tr -d ' ')
    cat "$WORK/cases.list" >>"$WORK/files"
    python3 scripts/lib/chains.py "$WORK/chains" >"$WORK/chains.list"
    chains=$(wc -l <"$WORK/chains.list" | tr -d ' ')
    cat "$WORK/chains.list" >>"$WORK/files"
    if ! GRASM="$GRASM" python3 scripts/fuzz_tir.py --emit "$WORK/fuzz" --programs 700 --seed 1 >"$WORK/fuzz.log" 2>&1; then
        echo "error: scripts/fuzz_tir.py --emit failed:"
        sed 's/^/  /' "$WORK/fuzz.log"
        exit 1
    fi
    fuzz=$(wc -l <"$WORK/fuzz/entries" | tr -d ' ')
    cat "$WORK/fuzz/entries" >>"$WORK/files"
    # The corpus last, without the negatives already listed.
    { corpus_programs; git ls-files '*.gr'; } | sort -u | grep -vxF -f "$WORK/files" >>"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

ok=0
refused=0
conf_same=0
cases_same=0
chains_same=0
plain=0
diags=0
helps=0
ceilings=0
differ=0
panics=0
declare -a FAILED=()
declare -a PANICS=()
i=0
while IFS= read -r f; do
    i=$((i + 1))
    "$GRASM" check "$f" >"$WORK/ref.out" 2>"$WORK/ref.err"
    rc=$?
    "$FRONT" check "$f" >"$WORK/got.out" 2>"$WORK/got.err"
    gc=$?
    if [ "$rc" -gt 1 ] || grep -c 'panicked at' "$WORK/ref.err" >/dev/null; then
        panics=$((panics + 1))
        PANICS+=("$f — exit $rc: $(grep -m1 -o 'panicked at [^ ]*' "$WORK/ref.err")")
        continue
    fi
    if [ "$rc" = "$gc" ] && cmp -s "$WORK/ref.out" "$WORK/got.out" && cmp -s "$WORK/ref.err" "$WORK/got.err"; then
        if [ "$rc" = 0 ]; then
            ok=$((ok + 1))
            case "$f" in
                "$WORK/cases/"*) cases_same=$((cases_same + 1)) ;;
                "$WORK/chains/"*) chains_same=$((chains_same + 1)) ;;
            esac
        else
            refused=$((refused + 1))
            [ "$i" -le "$conf" ] && conf_same=$((conf_same + 1))
            case "$f" in
                "$WORK/cases/"*) cases_same=$((cases_same + 1)) ;;
                "$WORK/chains/"*) chains_same=$((chains_same + 1)) ;;
            esac
            # A refusal before any module is read: the manifest, the
            # resolver or the ceilings, said on a line of its own.
            plain=$((plain + $(grep -c '^error: ' "$WORK/ref.err")))
            diags=$((diags + $(grep -cE '^.+:[0-9]+:[0-9]+: error: ' "$WORK/ref.err")))
            helps=$((helps + $(grep -c '^  help: ' "$WORK/ref.err")))
            ceilings=$((ceilings + $(grep -c '^error: package `[^`]*` performs ' "$WORK/ref.err")))
        fi
        continue
    fi
    differ=$((differ + 1))
    if [ "$rc" != "$gc" ]; then
        why="exit $rc against $gc"
    elif ! cmp -s "$WORK/ref.out" "$WORK/got.out"; then
        why="stdout differs"
    else
        line=$(cmp "$WORK/ref.err" "$WORK/got.err" 2>&1 | head -1 | sed 's/.* line //')
        why="stderr differs from line $line
      rust:  $(sed -n "${line}p" "$WORK/ref.err" | cut -c1-200)
      front: $(sed -n "${line}p" "$WORK/got.err" | cut -c1-200)"
    fi
    FAILED+=("$f — $why")
done <"$WORK/files"

# The long chains' TIR, by digest: each is too large to keep twice.
tir_same=0
tir_chains=0
for n in pow6000 elseif6000; do
    [ -f "$WORK/chains/$n.gr" ] || continue
    tir_chains=$((tir_chains + 1))
    want=$("$GRASM" tir "$WORK/chains/$n.gr" 2>&1 | shasum -a 256)
    got=$("$FRONT" tir "$WORK/chains/$n.gr" 2>&1 | shasum -a 256)
    if [ "$want" = "$got" ]; then
        tir_same=$((tir_same + 1))
    else
        FAILED+=("$WORK/chains/$n.gr — front tir and grasm tir differ")
    fi
done

echo "programs: $listed ($conf conformance negatives, $inline from the Rust test suites, $cases package cases, $chains long chains, $fuzz from fuzz_tir, the rest the corpus)"
echo "refused with the same output: $refused (conformance: $conf_same)   accepted by both: $ok   differ: $differ   Rust panics: $panics"
echo "diagnostics compared: $diags   help lines: $helps   unrendered errors: $plain   ceiling breaches: $ceilings   package cases the same: $cases_same   long chains the same: $chains_same (TIR: $tir_same of $tir_chains)"
if [ ${#PANICS[@]} -gt 0 ]; then
    echo
    echo "THE RUST FRONTEND PANICKED (a Rust bug, never agreement):"
    printf '  %s\n' "${PANICS[@]}" | head -20
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -80
fi
if [ $((ok + refused + differ + panics)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed programs listed"
    exit 1
fi
if [ "$differ" -ne 0 ] || [ "$panics" -ne 0 ] || [ "$tir_same" -ne "$tir_chains" ]; then
    exit 1
fi
if [ "$refused" -lt 1 ] || [ "$diags" -lt 1 ]; then
    echo "nothing refused was compared"
    exit 1
fi
# The repository's programs, less a margin: a corpus that shrank, a suite
# whose programs stopped being read, or a frontend that stopped refusing.
if [ -z "${FILES:-}" ] && { [ "$conf_same" -ne "$conf" ] || [ "$conf" -lt 60 ] || [ "$cases_same" -ne "$cases" ] ||
    [ "$cases" -lt 60 ] || [ "$chains_same" -ne "$chains" ] || [ "$chains" -lt 600 ] ||
    [ "$tir_same" -ne 2 ] || [ "$refused" -lt 850 ] || [ "$diags" -lt 1000 ] || [ "$helps" -lt 400 ] ||
    [ "$plain" -lt 60 ] || [ "$ceilings" -lt 20 ] || [ "$ok" -lt 2000 ]; }; then
    echo "too few compared: $conf_same of $conf conformance negatives, $cases_same of $cases package cases, $chains_same of $chains long chains ($tir_same TIR), $refused refused, $diags diagnostics, $helps help lines, $plain unrendered errors, $ceilings ceiling breaches, $ok accepted"
    exit 1
fi
exit 0
