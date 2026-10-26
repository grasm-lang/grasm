#!/usr/bin/env bash
# The Grasm linker (docs/self-hosting.md, stage S4) must link every corpus
# program into one that prints and exits exactly as today's `cc`-linked build
# does, from either backend's objects.
#
# `compiler/link` takes the objects a native build makes and the runtime
# archive and writes a signed executable itself. For each corpus program with
# a `main`:
#
#   * the Cranelift backend's objects -- `grasm build --native` with
#     GRASM_LINKER=cc and `scripts/lib/linkcc/cc` first on PATH, which keeps
#     the objects and the link line it was handed. That build is the
#     reference; the same objects are linked again by `compiler/link`, with
#     the same libraries and exports and `-strip`, as the native build links
#     them, and both are run with stdin closed. stdout, the exit status and
#     stderr must be identical. A program that links a system library (SDL)
#     is built and run here too when the library is present -- with
#     SDL_VIDEODRIVER=dummy -- and set aside, counted, when it is not;
#   * the Grasm backend's objects (`compiler/s3`, the programs in its subset,
#     `scripts/s3_subset.py`), linked with `cc` as `scripts/lib/s3.sh` links
#     them and by `compiler/link`, run and compared the same way;
#   * thread-local variables, which neither backend nor the runtime emits:
#     `scripts/fixtures/link/tlv_*.c`, built with `cc -c`, linked both ways,
#     run and compared.
#
# Every executable `compiler/link` writes must pass `codesign -v --strict`, and
# a second link of the same inputs must give the same bytes. The counts are
# derived and asserted: every program with a `main` is linked from the
# Cranelift objects or set aside as external, and every program in the
# subset from the Grasm backend's, with the subset held to its floor.
#
# `LINK=<binary>` tests that linker instead of building one; `LINK_EXTRA`
# adds arguments to every link (the controls, `scripts/linkcontrols.sh`,
# plant faults with it); `LINK_PROGRAMS=<regex>` keeps only the programs
# whose path matches (the controls again); `S3=` and `DIRS=` as for the S3
# gates.
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
link_driver
s3_driver
s3_runtime
s3_collect
EXTRA=${LINK_EXTRA:-}
SDL=$(link_sdl_present)

considered=0
cl_pass=0
cl_fail=0
external=0
s3_pass=0
s3_fail=0
in_subset=0
timedout=0
linked=0
declare -a FAILED=()
declare -a TIMEDOUT=()
declare -a EXTERNAL=()

# Writes `$1` through the existing file `$2`, so the run reuses its inode and
# macOS's verdict on it (lib/native.sh).
through() {
    if [ -e "$2" ]; then
        cat "$1" >"$2"
    else
        cp "$1" "$2"
    fi
}

# Links "$@" twice with compiler/link into $WORK/a/p and $WORK/b/p; checks
# the signature and that the two are the same bytes. Fails naming why.
link_twice() {
    rm -rf "$WORK/a" "$WORK/b"
    mkdir -p "$WORK/a" "$WORK/b"
    # shellcheck disable=SC2086
    if ! "$LINK" "$@" $EXTRA -o "$WORK/a/p" >"$WORK/link.log" 2>&1; then
        echo "compiler/link failed: $(head -3 "$WORK/link.log" | tr '\n' ' ')"
        return 1
    fi
    # shellcheck disable=SC2086
    if ! "$LINK" "$@" $EXTRA -o "$WORK/b/p" >"$WORK/link2.log" 2>&1; then
        echo "the second link failed: $(head -3 "$WORK/link2.log" | tr '\n' ' ')"
        return 1
    fi
    if ! cmp -s "$WORK/a/p" "$WORK/b/p"; then
        echo "two links of the same inputs differ"
        return 1
    fi
    if ! codesign -v --strict "$WORK/a/p" >"$WORK/cs.log" 2>&1; then
        echo "codesign -v --strict refuses it: $(head -2 "$WORK/cs.log" | tr '\n' ' ')"
        return 1
    fi
}

# Runs the reference `$1` and ours ($WORK/a/p) and compares them; extra
# environment in `$2`. Prints a reason and fails when they differ.
compare_runs() {
    local ref=$1 envs=$2
    through "$WORK/a/p" "$WORK/run_ours"
    # shellcheck disable=SC2086
    env $envs $S3_RUN "$ref" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
    local rc=$?
    # shellcheck disable=SC2086
    env $envs $S3_RUN "$WORK/run_ours" >"$WORK/o.out" 2>"$WORK/o.err" </dev/null
    local oc=$?
    if [ "$rc" = 124 ] || [ "$oc" = 124 ]; then
        echo "TIMEOUT cc $rc ours $oc"
        return 2
    fi
    if cmp -s "$WORK/r.out" "$WORK/o.out" && [ "$rc" = "$oc" ] && cmp -s "$WORK/r.err" "$WORK/o.err"; then
        return 0
    fi
    echo "exit $rc (cc) vs $oc (compiler/link)"
    diff "$WORK/r.out" "$WORK/o.out" | head -6 | sed 's/^/    /'
    diff "$WORK/r.err" "$WORK/o.err" | head -4 | sed 's/^/    /'
    return 1
}

note_failure() {
    local why=$1 src=$2 half=$3
    if [ "${why%% *}" = TIMEOUT ]; then
        timedout=$((timedout + 1))
        TIMEDOUT+=("$src ($half) — $why")
    else
        FAILED+=("$src ($half) — $why")
        echo "FAIL: $src ($half): $why"
    fi
}

for k in "${!S3_SRC[@]}"; do
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    rule=${S3_RULE[$k]}
    if [ -n "${LINK_PROGRAMS:-}" ] && ! [[ "$src" =~ $LINK_PROGRAMS ]]; then
        continue
    fi
    considered=$((considered + 1))
    envs=""
    case "$rule" in
    external:*)
        if [ "$SDL" != yes ]; then
            external=$((external + 1))
            EXTERNAL+=("$src")
            continue
        fi
        envs="SDL_VIDEODRIVER=dummy"
        ;;
    esac
    # The Cranelift backend's objects.
    if ! link_capture "$src" "$WORK/cl"; then
        cl_fail=$((cl_fail + 1))
        FAILED+=("$src (Cranelift) — the native build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    if ! args=$(link_args "$WORK/cl.cap"); then
        cl_fail=$((cl_fail + 1))
        FAILED+=("$src (Cranelift) — its link line has a flag compiler/link does not take")
        continue
    fi
    IFS=$'\n' read -r -d '' -a argv <<<"$args"
    if ! why=$(link_twice "${argv[@]}" -strip); then
        cl_fail=$((cl_fail + 1))
        note_failure "$why" "$src" Cranelift
    elif linked=$((linked + 2)) && why=$(compare_runs "$WORK/cl" "$envs"); then
        cl_pass=$((cl_pass + 1))
    else
        cl_fail=$((cl_fail + 1))
        note_failure "$why" "$src" Cranelift
    fi
    # The Grasm backend's objects, for programs in its subset.
    [ "$rule" = in ] || continue
    in_subset=$((in_subset + 1))
    # shellcheck disable=SC2086
    if ! "$S3" "$tir" -o "$WORK/p.o" >"$WORK/s3.out" 2>"$WORK/s3.err"; then
        s3_fail=$((s3_fail + 1))
        FAILED+=("$src (s3) — the backend fails: $(head -2 "$WORK/s3.err" | tr '\n' ' ')")
        continue
    fi
    if ! s3_link "$WORK/p.o" "$WORK/s3cc" "$WORK/cclink.log"; then
        s3_fail=$((s3_fail + 1))
        FAILED+=("$src (s3) — cc does not link it: $(head -2 "$WORK/cclink.log" | tr '\n' ' ')")
        continue
    fi
    if ! why=$(link_twice "$WORK/p.o" "$S3_RT" -dead_strip); then
        s3_fail=$((s3_fail + 1))
        note_failure "$why" "$src" s3
    elif linked=$((linked + 2)) && why=$(compare_runs "$WORK/s3cc" ""); then
        s3_pass=$((s3_pass + 1))
    else
        s3_fail=$((s3_fail + 1))
        note_failure "$why" "$src" s3
    fi
done

# The C fixtures: thread-local variables, an export reached only by name.
fx_pass=0
for entry in "${LINK_FIXTURES[@]}"; do
    name=${entry%%|*}
    if ! link_fixture_build "$name" >"$WORK/fx.log" 2>&1; then
        FAILED+=("fixture $name — cc does not build it: $(head -2 "$WORK/fx.log" | tr '\n' ' ')")
        continue
    fi
    if ! why=$(link_twice "${FX_OBJS[@]}" "${FX_FLAGS[@]}"); then
        note_failure "$why" "fixture $name" C
    elif linked=$((linked + 2)) && why=$(compare_runs "$WORK/fx/$name/ref" ""); then
        fx_pass=$((fx_pass + 1))
    else
        note_failure "$why" "fixture $name" C
    fi
done

echo
total=$considered
echo "programs with a main: $total   external set aside: $external (SDL2 present: $SDL)"
echo "Cranelift objects: identical: $cl_pass   failed: $cl_fail"
echo "Grasm backend objects: in the subset: $in_subset   identical: $s3_pass   failed: $s3_fail"
echo "C fixtures: identical: $fx_pass of ${#LINK_FIXTURES[@]}"
echo "links by compiler/link (each made twice, compared): $linked   timed out: $timedout   no TIR: ${#S3_NOTIR[@]}"
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#S3_NOTIR[@]} -gt 0 ]; then
    echo
    echo "emit no TIR:"
    printf '  %s\n' "${S3_NOTIR[@]}"
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
# Every program was linked from the Cranelift objects or set aside; every
# program in the subset from the Grasm backend's too.
if [ $((cl_pass + external)) -ne "$total" ]; then
    echo "the counts do not add up: $cl_pass + $external is not $total"
    exit 1
fi
if [ "$s3_pass" -ne "$in_subset" ] || [ "$fx_pass" -ne ${#LINK_FIXTURES[@]} ]; then
    exit 1
fi
if [ "$timedout" -ne 0 ] || [ ${#S3_NOTIR[@]} -ne 0 ]; then
    exit 1
fi
if [ -z "${DIRS:-}" ] && [ -z "${LINK_PROGRAMS:-}" ] && [ "$in_subset" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $in_subset programs are in the subset; the floor is $S3_SUBSET_FLOOR"
    exit 1
fi
if [ "$cl_pass" -lt 1 ] || [ "$s3_pass" -lt 1 ]; then
    echo "nothing was compared"
    exit 1
fi
exit 0
