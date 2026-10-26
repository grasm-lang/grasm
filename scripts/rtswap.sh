#!/usr/bin/env bash
# Every corpus program, linked with the runtime written in Grasm in front of
# the Rust one, must print and exit exactly as it does today
# (docs/self-hosting.md §6.7, stage S5).
#
# The Grasm runtime (`compiler/rt`) is built by the Grasm backend (`grasm
# tir`, `compiler/s3 --runtime`) and held to `compiler/rt/families.txt`
# (`scripts/lib/rtswap.sh`). Then, for each corpus program with a `main`:
#
#   * the Cranelift backend's objects: `grasm build --native` with
#     `scripts/lib/rtswap_link` as its linker, which links the program as it
#     ships and again with `compiler/link -interpose`, so the two differ in
#     the runtime and in nothing else;
#   * the Grasm backend's objects, for the programs in its subset, linked
#     both ways the same (`RT_S3=0` leaves them out).
#
# Both binaries run with stdin closed, and stdout, the exit status and stderr
# must be identical. Each swapped link says which runtime names the
# program's objects took from each side (`-interpose_log`); every name that
# families.txt says has moved must have been taken from Grasm, in every
# program, and a program that took nothing from Grasm compared nothing and is
# counted as such. The counts are derived and asserted.
#
# Then the poisoned runtime's run (`scripts/s3rc.sh`'s method): both links
# again against the runtime built with `grasm_rt/poison`, run, and required
# to agree, with no use after free reported. That runtime is built as
# `s3rc` builds it, into the same directory, or named by
# `RT_POISON=<archive>`; `RT_POISON=` empty leaves the run out.
# `RT_PROGRAMS=<regex>` keeps only the programs whose path matches;
# `RT_OBJ=`, `S3=`, `LINK=` and `DIRS=` name what to test instead of building
# it (the controls, `scripts/rtcontrols.sh`, use them).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/native.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver
s3_runtime
rtswap_runtime
rtswap_families
rtswap_linker
if [ -z "${RT_POISON+x}" ]; then
    if ! cargo build --release -q -p grasm_rt_abi --features grasm_rt/poison --target-dir target/s3rc-poison; then
        echo "error: the poisoned runtime does not build"
        exit 1
    fi
    RT_POISON=target/s3rc-poison/release/libgrasm_rt_abi.a
fi
if [ -n "${RT_POISON:-}" ]; then
    if [ ! -f "$RT_POISON" ]; then
        echo "error: RT_POISON=$RT_POISON is missing"
        exit 1
    fi
    export RTSWAP_POISON="$RT_POISON"
fi

considered=0
identical=0
failed=0
external=0
timedout=0
nothing=0
p_same=0
p_skipped=0
s3_in=0
s3_same=0
s3_timedout=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a EXTERNAL=()
declare -a NOTHING=()
: >"$WORK/taken.txt"

through() {
    if [ -e "$2" ]; then
        cat "$1" >"$2"
    else
        cp "$1" "$2"
    fi
}

# Runs `$1` and `$2` with stdin closed and compares them. Prints why and
# fails when they differ; answers 2 when either ran out of time.
compare() {
    through "$1" "$WORK/run_a"
    through "$2" "$WORK/run_b"
    # shellcheck disable=SC2086
    $S3_RUN "$WORK/run_a" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    local ac=$?
    # shellcheck disable=SC2086
    $S3_RUN "$WORK/run_b" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    local bc=$?
    if [ "$ac" = 124 ] || [ "$bc" = 124 ]; then
        echo "TIMEOUT rust $ac grasm $bc"
        return 2
    fi
    if cmp -s "$WORK/a.out" "$WORK/b.out" && [ "$ac" = "$bc" ] && cmp -s "$WORK/a.err" "$WORK/b.err"; then
        return 0
    fi
    echo "exit $ac (Rust runtime) vs $bc (Grasm runtime in front)"
    diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
    diff "$WORK/a.err" "$WORK/b.err" | head -4 | sed 's/^/    /'
    return 1
}

# Reads a swapped link's log: counts, and every moved name taken from Rust.
# Prints `<grasm> <rust>`; fails naming a moved name the link took from Rust.
read_log() {
    local log=$1 src=$2 g r fell
    g=$(grep -c '^grasm ' "$log")
    r=$(grep -c '^rust ' "$log")
    fell=$(awk '$1 == "rust" { print $2 }' "$log" | LC_ALL=C sort | LC_ALL=C comm -12 - "$WORK/moved.txt" | tr '\n' ' ')
    awk -v s="$src" '{ print s, $1, $2 }' "$log" >>"$WORK/taken.txt"
    if [ -n "$fell" ]; then
        echo "took moved names from the Rust runtime: $fell"
        return 1
    fi
    echo "$g $r"
}

note() { # why src half
    if [ "${1%% *}" = TIMEOUT ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$2 ($3) — $1")
    else
        failed=$((failed + 1))
        FAILED+=("$2 ($3) — $1")
        echo "FAIL: $2 ($3): $1"
    fi
}

for src in $(corpus_programs); do
    grep -qE '^\s*(pub )?fn main\(' "$src" || continue
    grep -q '^//! expect: error$' "$src" && continue
    if [ -n "${RT_PROGRAMS:-}" ] && ! [[ "$src" =~ $RT_PROGRAMS ]]; then
        continue
    fi
    considered=$((considered + 1))
    if corpus_external "$src"; then
        external=$((external + 1))
        EXTERNAL+=("$src")
        continue
    fi
    rm -rf "$WORK/b"
    mkdir -p "$WORK/b"
    if ! "$GRASM" build "$src" --native -o "$WORK/b/p" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/b/p.swap" ]; then
        note "the native build fails: $(head -1 "$WORK/build.log")" "$src" Cranelift
        continue
    fi
    if ! counts=$(read_log "$WORK/b/p.swaplog" "$src"); then
        note "$counts" "$src" Cranelift
        continue
    fi
    if [ "${counts%% *}" = 0 ]; then
        nothing=$((nothing + 1))
        NOTHING+=("$src")
    fi
    if why=$(compare "$WORK/b/p" "$WORK/b/p.swap"); then
        identical=$((identical + 1))
    else
        note "$why" "$src" Cranelift
        continue
    fi
    if [ -n "${RTSWAP_POISON:-}" ]; then
        if why=$(compare "$WORK/b/p.poison" "$WORK/b/p.swap.poison"); then
            if grep -q 'use after free' "$WORK/a.err" "$WORK/b.err"; then
                note "a use after free under the poisoned runtime: $(grep -h 'use after free' "$WORK/a.err" "$WORK/b.err" | head -1)" "$src" poisoned
            else
                p_same=$((p_same + 1))
            fi
        elif [ "${why%% *}" = TIMEOUT ]; then
            p_skipped=$((p_skipped + 1))
        else
            note "$why" "$src" poisoned
        fi
    fi
done

# The Grasm backend's objects, for the programs in its subset.
if [ "${RT_S3:-1}" = 1 ]; then
    s3_collect
    for k in "${!S3_SRC[@]}"; do
        src=${S3_SRC[$k]}
        [ "${S3_RULE[$k]}" = in ] || continue
        if [ -n "${RT_PROGRAMS:-}" ] && ! [[ "$src" =~ $RT_PROGRAMS ]]; then
            continue
        fi
        s3_in=$((s3_in + 1))
        if ! "$S3" "${S3_TIR[$k]}" -o "$WORK/p.o" >"$WORK/s3.log" 2>&1; then
            note "the backend fails: $(head -1 "$WORK/s3.log")" "$src" s3
            continue
        fi
        if ! "$LINK" "$WORK/p.o" "$S3_RT" -dead_strip -o "$WORK/s3ref" >"$WORK/l1.log" 2>&1 ||
            ! "$LINK" -interpose "$RT_OBJ" -interpose_log "$WORK/s3.swaplog" "$WORK/p.o" "$S3_RT" -dead_strip -o "$WORK/s3swap" >"$WORK/l2.log" 2>&1; then
            note "a link fails: $(cat "$WORK/l1.log" "$WORK/l2.log" | head -1)" "$src" s3
            continue
        fi
        if ! counts=$(read_log "$WORK/s3.swaplog" "$src(s3)"); then
            note "$counts" "$src" s3
            continue
        fi
        if why=$(compare "$WORK/s3ref" "$WORK/s3swap"); then
            s3_same=$((s3_same + 1))
        elif [ "${why%% *}" = TIMEOUT ]; then
            s3_timedout=$((s3_timedout + 1))
            TIMEDOUT+=("$src (s3) — $why")
        else
            note "$why" "$src" s3
        fi
    done
fi

# What the corpus took from each side, by name, over the Cranelift objects.
grep -v '(s3) ' "$WORK/taken.txt" | awk '{ print $2, $3 }' | sort | uniq -c | sort -k3 >"$WORK/names.txt"
names_grasm=$(awk '$2 == "grasm"' "$WORK/names.txt" | grep -c .)
names_rust=$(awk '$2 == "rust"' "$WORK/names.txt" | grep -c .)
refs_grasm=$(grep -v '(s3) ' "$WORK/taken.txt" | grep -c ' grasm ')
refs_rust=$(grep -v '(s3) ' "$WORK/taken.txt" | grep -c ' rust ')
if [ -n "${RT_NAMES:-}" ]; then
    cp "$WORK/names.txt" "$RT_NAMES"
fi

echo
echo "programs with a main: $considered   external set aside: $external"
echo "Cranelift objects: identical: $identical   failed: $failed   timed out: $timedout"
echo "runtime names the programs reference: $((names_grasm + names_rust)) distinct, $names_grasm taken from Grasm and $names_rust from Rust; per program, $refs_grasm references bound to Grasm and $refs_rust to Rust"
echo "programs that took nothing from Grasm (compared nothing): $nothing"
if [ -n "${RTSWAP_POISON:-}" ]; then
    echo "poisoned runtime: agreeing with no use after free: $p_same   timed out (not compared): $p_skipped"
fi
if [ "${RT_S3:-1}" = 1 ]; then
    echo "Grasm backend objects: in the subset: $s3_in   identical: $s3_same   timed out: $s3_timedout"
fi
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#NOTHING[@]} -gt 0 ]; then
    echo
    echo "took nothing from Grasm:"
    printf '  %s\n' "${NOTHING[@]}"
fi
if [ ${#TIMEDOUT[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${S3_RUN_SECONDS}s (not agreement):"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ $((identical + external + timedout)) -ne "$considered" ]; then
    echo "the counts do not add up: $identical + $external + $timedout is not $considered"
    exit 1
fi
if [ "$timedout" -ne 0 ] || [ "$s3_timedout" -ne 0 ]; then
    exit 1
fi
if [ $((identical - nothing)) -lt 1 ] || [ "$names_grasm" -lt 1 ]; then
    echo "nothing was compared: no program took anything from the Grasm runtime"
    exit 1
fi
if [ -n "${RTSWAP_POISON:-}" ] && [ $((p_same + p_skipped)) -ne "$identical" ]; then
    echo "the poisoned counts do not add up: $p_same + $p_skipped is not $identical"
    exit 1
fi
if [ "${RT_S3:-1}" = 1 ] && [ "$s3_same" -ne "$s3_in" ]; then
    exit 1
fi
if [ "${RT_S3:-1}" = 1 ] && [ -z "${DIRS:-}" ] && [ -z "${RT_PROGRAMS:-}" ] && [ "$s3_in" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $s3_in programs are in the Grasm backend's subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
exit 0
