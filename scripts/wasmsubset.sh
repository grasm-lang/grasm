#!/usr/bin/env bash
# The Grasm backend's wasm32 target (docs/self-hosting.md §6.24, S8 part
# seven): every corpus program the Rust playground's gate compares, compiled
# to a WebAssembly module by `s3 --target wasm32` and run by node, must print
# and exit as `grasm run` does.
#
# For each corpus program with a `main`:
#
#   * what is set aside: exactly what `wasmtest` sets aside and for its
#     reasons (`scripts/lib/wasmskip.sh`, which both gates source), with one
#     difference the coordinator asked for (§6.24): a program set aside there
#     only because it names `par_map`, `par_filter` or `spawn` -- which the
#     Rust module runs, on one core -- is held here, to `grasm run` and,
#     where the Rust module runs it as `grasm run` does, to that module too,
#     and counted apart;
#   * the module: compiled from `grasm tir`'s TIR with the runtime's and the
#     wasm C library's (`compiler/rtwasm`), twice, and the two must be the
#     same bytes;
#   * the run: by `scripts/wasm/runprog.mjs`, stdin closed: stdout and the
#     exit status must be `grasm run`'s, and stdout, stderr and the status the
#     program's native build's (`grasm build --native`) -- a panic's text is
#     the runtime's, and only the interpreter adds a trace;
#   * the checkers: every module validated by V8 (`new WebAssembly.Module`,
#     which validates every function) and by `scripts/wasm_validate.py`,
#     independent of V8, whose count of bodies checked is asserted;
#   * the refusals (decisions (4) and (6)): `scripts/fixtures/wasmrefuse/`'s
#     programs -- a `concurrent` block, files, a socket, another program --
#     must print, write to stderr and exit as the Rust module does on them;
#   * the stack (decision (5), and §6.26, S8 part nine):
#     `scripts/fixtures/wasmstack/`'s recursions, each of a different shape
#     (two of them inside `par_map` and `spawn`), run past any depth and
#     bisected for the deepest call that prints, under node's default stack
#     and no flag. With JS promise integration a recursion hops to fresh
#     engine stacks: each shape must reach at least the 10,000 calls
#     [E-RECURSION-DEPTH] guarantees. Without it (`GRASM_WASM_STACKS=0`)
#     nothing can hop: each must reach at least what it reached when this
#     was written, less 1% -- below 10,000 for every shape, the open S3 entry
#     of docs/known-issues.md. Either way, past the bound the runtime's panic
#     and exit 101, never the engine's stack overflow -- a trap the clause
#     rules out. Every depth is from runs with no flag. Then the tiers: each
#     shape run past any depth again under node's `--liftoff-only` and
#     `--no-liftoff` (TurboFan from the start), which change only which of
#     V8's compilers makes the code and so the size of its frames, never the
#     stack's: the panic and 101 under each, never a `RangeError`. `scripts/fixtures/wasmdeep/trap.gr` traps on purpose, on the first
#     engine stack and on one a hop made: each must end as a trap, never as
#     an exit, a panic or output. And `wasmdeep/split.gr` prints the hot
#     split's cost -- a call made across a segment's edge against one made
#     beside it -- measured, not asserted;
#   * the three programs `wasmtest` sets aside for their depth, which its
#     Rust module cannot run (`WASM_SKIP_NO_DEPTH=1`): held here as any
#     other, and counted apart;
#
# Counts asserted; the floor is the 508 programs `wasmtest` compared when
# this was written (2026-11-22).
#
# `WASM_SUBSET_ONLY=stack` runs the stack and the refusals alone, with no
# corpus (for the controls). `S3=<binary>` tests that driver; `WASM=<module>` names the Rust module
# (default the build `wasmtest` makes); `WASM_LIBC=<dir>` the wasm C
# library's package (default `compiler/rtwasm`); `WASM_SUBSET_STOP=1` stops
# at the first failure, for the controls (`scripts/wasmcontrols.sh`).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require
FLOOR=${WASM_SUBSET_FLOOR:-508}
RUST_WASM=${WASM:-./target/wasm32-unknown-unknown/release/grasm_wasm.wasm}
STOP=${WASM_SUBSET_STOP:-}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/native.sh"
. "$(dirname "$0")/lib/wasmskip.sh"
s3_driver
command -v node >/dev/null 2>&1 || { echo "error: node is needed to run a wasm module"; exit 1; }
[ -f "$RUST_WASM" ] || { echo "error: the Rust module $RUST_WASM is missing (WASM= names another)"; exit 1; }

if ! "$GRASM" tir compiler/rt/src/main.gr >"$WORK/rt.tir" 2>"$WORK/rt.err" ||
    ! "$GRASM" tir "${WASM_LIBC:-compiler/rtwasm}/src/main.gr" >"$WORK/rtwasm.tir" 2>"$WORK/rtwasm.err"; then
    echo "error: the runtime's or the C library's TIR:"
    cat "$WORK/rt.err" "$WORK/rtwasm.err" | head -5
    exit 1
fi
build_wasm() { # TIR OUT
    "$S3" "$1" -o "$2" --target wasm32 --runtime-tir "$WORK/rt.tir" --libc-tir "$WORK/rtwasm.tir"
}

# `WASM_SUBSET_ONLY=stack` runs the stack alone (the controls of §6.26):
# no corpus, so the floors are not met, and `WASM_SUBSET_STOP=1` with it.
if [ "${WASM_SUBSET_ONLY:-}" = stack ]; then
    S3_SRC=()
    S3_TIR=()
else
    s3_collect
fi
mkdir -p "$WORK/mods"
compared=0
same_run=0
same_native=0
threads_held=0
threads_rust=0
threads_rust_differs=0
depth_held=0
skipped=0
reproducible=0
timedout=0
fail=0
nmods=0
declare -a FAILED=()
declare -a SKIPPED=()
declare -A REASONS=()

failed() {
    fail=$((fail + 1))
    FAILED+=("$1")
    echo "FAILED: $1"
}

for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ "$fail" -gt 0 ]; then
        break
    fi
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    subject=$src
    if grep -qE '^\s*import ' "$src"; then
        bundle="$WORK/b.bundle.gr"
        if ! "$GRASM" build "$src" --source -o "$bundle" >/dev/null 2>&1; then
            skipped=$((skipped + 1))
            SKIPPED+=("$src — its modules could not be bundled")
            REASONS["its modules could not be bundled"]=$(( ${REASONS["its modules could not be bundled"]:-0} + 1 ))
            continue
        fi
        subject=$bundle
    fi
    why=$(wasm_skip_reason "$src" "$subject")
    held_threads=0
    if [[ "$why" == needs\ threads* ]]; then
        why=$(WASM_SKIP_NO_THREADS=1 wasm_skip_reason "$src" "$subject")
        [ -z "$why" ] && held_threads=1
    fi
    held_depth=0
    if [[ "$why" == recurses\ past* ]]; then
        why=$(WASM_SKIP_NO_DEPTH=1 wasm_skip_reason "$src" "$subject")
        [ -z "$why" ] && held_depth=1
    fi
    if [ -n "$why" ]; then
        skipped=$((skipped + 1))
        SKIPPED+=("$src — $why")
        REASONS["$why"]=$(( ${REASONS["$why"]:-0} + 1 ))
        continue
    fi
    mod="$WORK/mods/$k.wasm"
    if ! build_wasm "$tir" "$mod" >"$WORK/s3.log" 2>&1; then
        failed "$src — s3 --target wasm32: $(head -2 "$WORK/s3.log" | tr '\n' ' ')"
        continue
    fi
    nmods=$((nmods + 1))
    if build_wasm "$tir" "$WORK/again.wasm" >/dev/null 2>&1 && cmp -s "$mod" "$WORK/again.wasm"; then
        reproducible=$((reproducible + 1))
    else
        failed "$src — the module built twice is not the same bytes"
    fi
    # shellcheck disable=SC2086
    $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
    r_code=$?
    # shellcheck disable=SC2086
    $RUN_LIMIT node scripts/wasm/runprog.mjs "$mod" >"$WORK/w.out" 2>"$WORK/w.err" </dev/null
    w_code=$?
    if [ "$r_code" = 124 ] || [ "$w_code" = 124 ]; then
        timedout=$((timedout + 1))
        failed "$src — timed out after ${RUN_LIMIT_SECONDS}s (grasm run $r_code, wasm $w_code)"
        continue
    fi
    compared=$((compared + 1))
    if cmp -s "$WORK/r.out" "$WORK/w.out" && [ "$r_code" = "$w_code" ]; then
        same_run=$((same_run + 1))
    else
        failed "$src — not as grasm run: exit $r_code (grasm run) vs $w_code (wasm); $(diff "$WORK/r.out" "$WORK/w.out" | head -4 | tr '\n' ' ') $(head -c 300 "$WORK/w.err")"
        continue
    fi
    if grasm_build_native "$src" "$WORK/nat" >"$WORK/nat.log" 2>&1; then
        "$WORK/nat" >"$WORK/n.out" 2>"$WORK/n.err" </dev/null
        n_code=$?
        if cmp -s "$WORK/n.out" "$WORK/w.out" && cmp -s "$WORK/n.err" "$WORK/w.err" && [ "$n_code" = "$w_code" ]; then
            same_native=$((same_native + 1))
        else
            failed "$src — not as its native build: exit $n_code vs $w_code; stderr $(diff "$WORK/n.err" "$WORK/w.err" | head -4 | tr '\n' ' ')"
            continue
        fi
    else
        failed "$src — its native build fails: $(head -1 "$WORK/nat.log")"
        continue
    fi
    if [ "$held_depth" = 1 ]; then
        depth_held=$((depth_held + 1))
    fi
    if [ "$held_threads" = 1 ]; then
        threads_held=$((threads_held + 1))
        # The Rust module, where it runs the program as `grasm run` does.
        node scripts/wasm/run.mjs "$RUST_WASM" "$subject" >"$WORK/rw.out" 2>/dev/null </dev/null
        rw_code=$?
        if cmp -s "$WORK/rw.out" "$WORK/r.out" && [ "$rw_code" = "$r_code" ]; then
            threads_rust=$((threads_rust + 1))
        else
            threads_rust_differs=$((threads_rust_differs + 1))
            echo "  (the Rust module does not run $src as grasm run does: exit $rw_code)"
        fi
    fi
done

# The checkers, over every module built.
v8=$(node -e '
const fs = require("fs");
let ok = 0, bad = 0;
for (const f of fs.readdirSync(process.argv[1])) {
  try { new WebAssembly.Module(fs.readFileSync(process.argv[1] + "/" + f)); ok++; }
  catch (e) { bad++; console.log("V8 REFUSES " + f + ": " + e.message); }
}
console.log("v8 " + ok + " " + bad);
' "$WORK/mods")
echo "$v8" | grep -v '^v8 '
v8_ok=$(echo "$v8" | awk '/^v8 /{print $2}')
v8_bad=$(echo "$v8" | awk '/^v8 /{print $3}')
python3 -B scripts/wasm_validate.py "$WORK"/mods/*.wasm >"$WORK/pv.txt" 2>&1
pv_ok=$(grep -c '^ok ' "$WORK/pv.txt")
pv_bad=$(grep -c '^INVALID ' "$WORK/pv.txt")
pv_bodies=$(awk '/^ok /{sub("bodies=","",$3); s += $3} END {print s + 0}' "$WORK/pv.txt")
grep '^INVALID ' "$WORK/pv.txt" | head -10

# The refusals: as the Rust module refuses.
refuse_same=0
refuse_n=0
for fx in scripts/fixtures/wasmrefuse/*.gr; do
    refuse_n=$((refuse_n + 1))
    "$GRASM" tir "$fx" >"$WORK/fx.tir" 2>/dev/null && build_wasm "$WORK/fx.tir" "$WORK/fx.wasm" >/dev/null 2>&1 || {
        failed "$fx does not build"; continue; }
    node scripts/wasm/runprog.mjs "$WORK/fx.wasm" >"$WORK/fo1" 2>"$WORK/fe1" </dev/null
    c1=$?
    node scripts/wasm/run.mjs "$RUST_WASM" "$fx" >"$WORK/fo2" 2>"$WORK/fe2" </dev/null
    c2=$?
    if cmp -s "$WORK/fo1" "$WORK/fo2" && cmp -s "$WORK/fe1" "$WORK/fe2" && [ "$c1" = "$c2" ]; then
        refuse_same=$((refuse_same + 1))
    else
        failed "$fx — not refused as the Rust module refuses it: exit $c1 vs $c2; $(diff "$WORK/fo1" "$WORK/fo2" | head -3 | tr '\n' ' ') $(diff "$WORK/fe1" "$WORK/fe2" | head -3 | tr '\n' ' ')"
    fi
done

# The stack (§6.26). Each shape's depth without JS promise integration as
# measured on 2026-11-25 at the scale 1.08, the floor (less 1%, below).
declare -A BEFORE=([closure]=3117 [doubles]=7487 [locals]=5104 [mutual]=9359 [plain]=8639 [tree]=6239 [parmap]=8632 [spawn]=8638)
GUARANTEED=10000
DEEP_MSG='panic: stack overflow: recursion too deep'
stack_ok=0
stack_bad=0
stack_n=0
tier_ok=0
tier_n=0
declare -a DEPTHS=()
declare -a DEPTHS_NJ=()
# deepest MODULE [ENV]: the deepest call that prints, by bisection.
deepest() {
    local m=$1 lo=1 hi=400000 mid
    shift
    while [ "$lo" -lt "$hi" ]; do
        mid=$(( (lo + hi + 1) / 2 ))
        if env "$@" node scripts/wasm/runprog.mjs "$m" "$mid" >/dev/null 2>&1 </dev/null; then lo=$mid; else hi=$((mid - 1)); fi
    done
    echo "$lo"
}
for fx in scripts/fixtures/wasmstack/*.gr; do
    name=$(basename "$fx" .gr)
    stack_n=$((stack_n + 1))
    "$GRASM" tir "$fx" >"$WORK/fx.tir" 2>/dev/null && build_wasm "$WORK/fx.tir" "$WORK/fx.wasm" >/dev/null 2>&1 || {
        stack_bad=$((stack_bad + 1)); echo "FAILED: $fx does not build"; continue; }
    ok=1
    for mode in jspi nojspi; do
        if [ "$mode" = jspi ]; then e=(GRASM_WASM_STACKS=1); say=""; else e=(GRASM_WASM_STACKS=0); say=" without JSPI,"; fi
        env "${e[@]}" node scripts/wasm/runprog.mjs "$WORK/fx.wasm" 100000000 >"$WORK/fx.out" 2>"$WORK/fx.err" </dev/null
        c=$?
        if [ "$c" != 101 ] || ! grep -qx "$DEEP_MSG" "$WORK/fx.err" || [ -s "$WORK/fx.out" ]; then
            ok=0
            echo "FAILED: $fx$say past any depth: exit $c, $(grep -v '^ *at ' "$WORK/fx.err" | head -c 200 | tr '\n' ' ')"
        fi
        d=$(deepest "$WORK/fx.wasm" "${e[@]}")
        if [ "$mode" = jspi ]; then
            DEPTHS+=("$name $d")
            if [ "$d" -lt "$GUARANTEED" ]; then
                ok=0
                echo "FAILED: $fx reaches only $d calls, short of the $GUARANTEED guaranteed"
            fi
        else
            DEPTHS_NJ+=("$name $d")
            want=$(( ${BEFORE[$name]:-$GUARANTEED} * 99 / 100 ))
            if [ "$d" -lt "$want" ]; then
                ok=0
                echo "FAILED: $fx without JSPI reaches only $d calls, below the $want held (${BEFORE[$name]:-?} measured, less 1%)"
            fi
        fi
    done
    # The tiers: a frame's size is the compiler's, so each must leave the
    # floor first.
    for tier in --liftoff-only --no-liftoff; do
        node "$tier" scripts/wasm/runprog.mjs "$WORK/fx.wasm" 100000000 >"$WORK/fx.out" 2>"$WORK/fx.err" </dev/null
        c=$?
        if [ "$c" = 101 ] && grep -qx "$DEEP_MSG" "$WORK/fx.err" && [ ! -s "$WORK/fx.out" ]; then
            tier_ok=$((tier_ok + 1))
        else
            ok=0
            echo "FAILED: $fx under $tier, past any depth: exit $c, $(grep -v '^ *at ' "$WORK/fx.err" | head -c 200 | tr '\n' ' ')"
        fi
        tier_n=$((tier_n + 1))
    done
    if [ "$ok" = 1 ]; then stack_ok=$((stack_ok + 1)); else stack_bad=$((stack_bad + 1)); fi
done
# A trap on the first engine stack and on one a hop made: a trap, never an
# exit, a panic or output.
trap_ok=0
trap_n=0
"$GRASM" tir scripts/fixtures/wasmdeep/trap.gr >"$WORK/trap.tir" 2>/dev/null && build_wasm "$WORK/trap.tir" "$WORK/trap.wasm" >/dev/null 2>&1 || {
    stack_bad=$((stack_bad + 1)); echo "FAILED: scripts/fixtures/wasmdeep/trap.gr does not build"; }
if [ -f "$WORK/trap.wasm" ]; then
    for depth in 100 30000; do
        for how in 1 2; do
            trap_n=$((trap_n + 1))
            node scripts/wasm/runprog.mjs "$WORK/trap.wasm" "$depth" "$how" >"$WORK/t.out" 2>"$WORK/t.err" </dev/null
            c=$?
            if [ "$c" = 1 ] && [ ! -s "$WORK/t.out" ] && grep -c 'RuntimeError' "$WORK/t.err" >/dev/null && ! grep -c '^panic: ' "$WORK/t.err" >/dev/null; then
                trap_ok=$((trap_ok + 1))
            else
                echo "FAILED: a trap in a deeper stack (wasmdeep/trap.gr $depth $how) not ended as a trap: exit $c, stdout $(head -c 60 "$WORK/t.out" | tr '\n' ' '), $(grep -v '^ *at ' "$WORK/t.err" | head -c 200 | tr '\n' ' ')"
            fi
        done
    done
fi
[ "$trap_ok" = 4 ] || stack_bad=$((stack_bad + 1))
# The hot split: `split.gr`'s `leaf` called beside the first engine stack's
# edge (the deepest `walk` at which it fits without a hop) and across it.
split_line="not measured"
"$GRASM" tir scripts/fixtures/wasmdeep/split.gr >"$WORK/split.tir" 2>/dev/null && build_wasm "$WORK/split.tir" "$WORK/split.wasm" >/dev/null 2>&1 || {
    stack_bad=$((stack_bad + 1)); echo "FAILED: scripts/fixtures/wasmdeep/split.gr does not build"; }
if [ -f "$WORK/split.wasm" ]; then
    lo=1
    hi=100000
    while [ "$lo" -lt "$hi" ]; do
        mid=$(( (lo + hi + 1) / 2 ))
        if GRASM_WASM_STACKS=0 node scripts/wasm/runprog.mjs "$WORK/split.wasm" "$mid" 1 >/dev/null 2>&1 </dev/null; then lo=$mid; else hi=$((mid - 1)); fi
    done
    split_line=$(perl -MTime::HiRes=time -e '
        my ($m, $edge) = @ARGV;
        sub run { my $t = time; system("node scripts/wasm/runprog.mjs $m @_ >/dev/null 2>&1 </dev/null") == 0 or die "split.gr @_ failed\n"; time - $t }
        my (@z, @b, @a);
        for (1 .. 3) { push @z, run($edge, 0); push @b, run($edge, 20000000); push @a, run($edge + 1, 1000000) }
        sub med { my @s = sort { $a <=> $b } @_; $s[1] }
        my ($tz, $tb, $ta) = (med(@z), med(@b), med(@a));
        my $beside = ($tb - $tz) / 20000000; my $across = ($ta - $tz) / 1000000;
        printf "%.0f ns a call across an edge, %.1f ns beside it: %.0f times (medians of 3, walk at %d)", $across * 1e9, $beside * 1e9, $across / $beside, $edge;
    ' "$WORK/split.wasm" "$lo" 2>&1 || true)
fi

echo
echo "skipped, by reason:"
for r in "${!REASONS[@]}"; do echo "  ${REASONS[$r]}  $r"; done | sort -k2
echo
echo "wasmsubset: ${#S3_SRC[@]} programs with a main, $skipped set aside, $compared compared"
echo "as grasm run: $same_run   as the native build (stdout, stderr, status): $same_native   timed out: $timedout"
echo "named par_map, par_filter or spawn and held: $threads_held, the Rust module running them as grasm run does: $threads_rust, otherwise: $threads_rust_differs"
echo "set aside by wasmtest for their depth and held here: $depth_held"
echo "modules: $nmods   built twice the same: $reproducible   V8 validates: ${v8_ok:-0} (refuses ${v8_bad:-0})   wasm_validate.py: $pv_ok valid, $pv_bad invalid, $pv_bodies bodies checked"
echo "refusals: $refuse_same of $refuse_n as the Rust module's"
echo "stack: $stack_ok of $stack_n shapes held (at least $GUARANTEED calls with JSPI, past any depth the panic, with and without)"
echo "stack, deepest with JSPI: ${DEPTHS[*]}"
echo "stack, deepest without JSPI (open S3): ${DEPTHS_NJ[*]}"
echo "stack, past any depth under --liftoff-only and --no-liftoff, the panic: $tier_ok of $tier_n"
echo "stack, a trap in a deeper stack ended as a trap: $trap_ok of $trap_n"
echo "stack, the hot split: $split_line"
status=0
[ "$fail" -gt 0 ] && status=1
[ "$stack_bad" -gt 0 ] && status=1
if [ -z "$STOP" ]; then
    if [ "$stack_n" -lt 8 ] || [ "$stack_ok" -ne "$stack_n" ]; then
        echo "the stack: $stack_ok of $stack_n shapes held, of at least 8"
        status=1
    fi
    if [ "$depth_held" -lt 3 ]; then
        echo "the programs set aside for their depth: $depth_held held, of 3"
        status=1
    fi
    if [ "$refuse_n" -lt 4 ] || [ "$refuse_same" -ne "$refuse_n" ]; then
        echo "the refusals: $refuse_same of $refuse_n alike, of at least 4"
        status=1
    fi
    if [ "$compared" -lt "$FLOOR" ] || [ "$same_run" -lt "$FLOOR" ]; then
        echo "fewer than the floor of $FLOOR programs compared alike"
        status=1
    fi
    if [ "$nmods" -ne "${v8_ok:-0}" ] || [ "$nmods" -ne "$pv_ok" ] || [ "$pv_bodies" -lt "$nmods" ] || [ "$nmods" -ne "$reproducible" ]; then
        echo "the checkers did not check every module: $nmods built, $v8_ok by V8, $pv_ok by wasm_validate.py ($pv_bodies bodies), $reproducible reproducible"
        status=1
    fi
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
exit $status
