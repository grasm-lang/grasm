#!/usr/bin/env bash
# The toolchain's one TOML reader against the Rust one and Python's `tomllib`
# (docs/self-hosting.md §6.21, the gate S8 leaves the reader under).
#
# `packages/toml` is the toolchain's only TOML reader: the Rust toolchain's
# own, `grasm_pkg::toml`, line for line, read here through `front toml`. The
# Rust reader is `grasm toml`; `tomllib` the third (`scripts/lib/tomldump.py`).
# Each prints every key as a dotted path with its value's kind and value.
#
#   manifests  every manifest git tracks and `scripts/lib/tomlcases`: the
#              reader the Rust reader's bytes, its refusals' words included,
#              and the Rust reader and `tomllib` alike -- or both refusing
#   fuzzed     the manifests `scripts/lib/tomlfuzz.py` writes: the reader the
#              Rust reader's bytes on every one; `tomllib` the same wherever
#              the Rust reader agrees with it; and every file where it does
#              not counted by how (`refuses-valid`, `accepts-invalid`,
#              `reads-otherwise`) and by a recorded cause
#              (`scripts/lib/tomlclass.py`: the dialect the Rust reader
#              documents, or its misreadings in docs/known-issues.md). A
#              file no recorded cause explains fails the gate, and at the
#              default size and seed every count is asserted, so a new way to
#              disagree shows.
#
# The entries of an array of tables are numbered and kept in the file's
# order, so a key read into the wrong entry shows (`tircontrols` plants one).
#
# The counts must add up to the files listed. `FRONT=` names a prebuilt
# frontend; `FILES=` a list of manifests; `FUZZ=N` fuzzes N files (0: none;
# default 1500), `FUZZ_SEED=S` with seed S (default 1).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if ! "$GRASM" toml /dev/null >/dev/null 2>&1; then
    echo "error: $GRASM has no \`toml\` command, so there is nothing to compare against"
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

# No manifest writes an array of tables, so `scripts/lib/tomlcases` does.
if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/files"
else
    git ls-files '*grasm.toml' 'scripts/lib/tomlcases/*.toml' >"$WORK/files"
fi
listed=$(wc -l <"$WORK/files" | tr -d ' ')

# Between the Rust reader and `tomllib` a refusal is compared as the word:
# each says why in its own words. The reader and the Rust one: the bytes.
norm() {
    sed -E 's/^refused.*/refused/' "$1"
}

same=0
refused=0
keys=0
differ=0
declare -a FAILED=()
while IFS= read -r f; do
    "$GRASM" toml "$f" >"$WORK/r.out" 2>&1
    "$FRONT" toml "$f" >"$WORK/g.out" 2>&1
    python3 scripts/lib/tomldump.py "$f" >"$WORK/p.out" 2>&1
    norm "$WORK/r.out" >"$WORK/r.txt"
    norm "$WORK/p.out" >"$WORK/p.txt"
    if cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.txt" "$WORK/p.txt"; then
        if [ "$(cat "$WORK/r.txt")" = refused ]; then
            refused=$((refused + 1))
        else
            same=$((same + 1))
            keys=$((keys + $(wc -l <"$WORK/r.txt")))
        fi
    else
        differ=$((differ + 1))
        if ! cmp -s "$WORK/r.out" "$WORK/g.out"; then
            FAILED+=("$f — packages/toml and the Rust reader differ: $(diff "$WORK/r.out" "$WORK/g.out" | sed -n 2p | cut -c1-120)")
        else
            FAILED+=("$f — tomllib differs: $(diff "$WORK/r.txt" "$WORK/p.txt" | sed -n 2p | cut -c1-120)")
        fi
    fi
done <"$WORK/files"

echo "manifests: $listed   read alike by all three: $same ($keys keys)   refused by all three: $refused   disagree: $differ"

# The fuzzed manifests.
FUZZ=${FUZZ:-1500}
FUZZ_SEED=${FUZZ_SEED:-1}
fuzzed=0
f_rust=0
f_agree=0
f_refused=0
declare -i f_rv=0 f_ai=0 f_ro=0
: >"$WORK/disagree.list"
if [ "$FUZZ" -gt 0 ]; then
    python3 scripts/lib/tomlfuzz.py "$WORK/fuzz" "$FUZZ" "$FUZZ_SEED"
    for f in "$WORK"/fuzz/*.toml; do
        fuzzed=$((fuzzed + 1))
        "$GRASM" toml "$f" >"$WORK/r.out" 2>&1
        "$FRONT" toml "$f" >"$WORK/g.out" 2>&1
        python3 scripts/lib/tomldump.py "$f" >"$WORK/p.out" 2>&1
        if ! cmp -s "$WORK/r.out" "$WORK/g.out"; then
            FAILED+=("fuzzed $(basename "$f") (seed $FUZZ_SEED) — packages/toml and the Rust reader differ: $(diff "$WORK/r.out" "$WORK/g.out" | sed -n 2p | cut -c1-120)")
            continue
        fi
        f_rust=$((f_rust + 1))
        norm "$WORK/r.out" >"$WORK/r.txt"
        norm "$WORK/p.out" >"$WORK/p.txt"
        r_ref=$(grep -qx refused "$WORK/r.txt" && echo 1 || echo 0)
        p_ref=$(grep -qx refused "$WORK/p.txt" && echo 1 || echo 0)
        if cmp -s "$WORK/r.txt" "$WORK/p.txt"; then
            f_agree=$((f_agree + 1))
            [ "$r_ref" = 1 ] && f_refused=$((f_refused + 1))
        elif [ "$r_ref" = 1 ]; then
            f_rv+=1
            printf 'refuses-valid\t%s\n' "$f" >>"$WORK/disagree.list"
        elif [ "$p_ref" = 1 ]; then
            f_ai+=1
            printf 'accepts-invalid\t%s\n' "$f" >>"$WORK/disagree.list"
        else
            f_ro+=1
            printf 'reads-otherwise\t%s\n' "$f" >>"$WORK/disagree.list"
        fi
    done
    python3 scripts/lib/tomlclass.py "$WORK/disagree.list" >"$WORK/causes.txt"
    unexplained=$(grep -c $'\tUNEXPLAINED\t' "$WORK/causes.txt")
    echo "fuzzed: $fuzzed (seed $FUZZ_SEED)   the Rust reader's bytes: $f_rust   read as tomllib reads them: $f_agree, $f_refused of them refused by both"
    echo "  where the Rust reader is not tomllib: refuses-valid $f_rv   accepts-invalid $f_ai   reads-otherwise $f_ro   unexplained: $unexplained"
    tail -1 "$WORK/causes.txt" | sed 's/^/  /'
    if [ "$unexplained" -ne 0 ]; then
        while IFS=$'\t' read -r cls why path; do
            [ "$why" = UNEXPLAINED ] && FAILED+=("fuzzed $(basename "$path") (seed $FUZZ_SEED) — $cls, and no recorded cause explains it: $(head -c 300 "$path" | tr '\n' '|')")
        done <"$WORK/causes.txt"
    fi
    if [ $((f_agree + f_rv + f_ai + f_ro)) -ne "$f_rust" ] || [ "$fuzzed" -ne "$FUZZ" ]; then
        FAILED+=("the fuzzed counts do not add up: $fuzzed written, $f_rust the Rust reader's bytes, $f_agree + $f_rv + $f_ai + $f_ro classed")
    fi
    # At the default size and seed the counts are what docs/self-hosting.md
    # §6.21 records (python3 3.14's tomllib): one that moves is a reader, a
    # generator or an oracle that changed, and a new way to disagree.
    if [ "$FUZZ" = 1500 ] && [ "$FUZZ_SEED" = 1 ]; then
        want="1051 216 156 77"
        got="$f_agree $f_rv $f_ai $f_ro"
        [ "$got" = "$want" ] || FAILED+=("fuzzed: agree, refuses-valid, accepts-invalid, reads-otherwise are $got, where they were $want")
        want_causes="$(cat scripts/lib/tomlclass.expected)"
        got_causes="$(tail -1 "$WORK/causes.txt")"
        [ "$got_causes" = "$want_causes" ] || FAILED+=("fuzzed: the causes moved: $got_causes")
    fi
fi

if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -40
fi
if [ $((same + refused + differ)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed manifests listed"
    exit 1
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    exit 1
fi
if [ -z "${FILES:-}" ] && { [ "$same" -lt 1400 ] || [ "$keys" -lt 5000 ]; }; then
    echo "too few manifests ($same) or keys ($keys) compared for this to mean anything"
    exit 1
fi
exit 0
