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
#   * the stack (decision (5)): `scripts/fixtures/wasmstack/`'s recursions,
#     each of a different shape, run past any depth: each must end in the
#     runtime's panic, exit 101, never the engine's stack overflow -- a trap
#     the clause rules out; and the depth each reaches, under node's default
#     stack, printed.
#
# Counts asserted; the floor is the 508 programs `wasmtest` compared when
# this was written (2026-11-22).
#
# `S3=<binary>` tests that driver; `WASM=<module>` names the Rust module
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

s3_collect
mkdir -p "$WORK/mods"
compared=0
same_run=0
same_native=0
threads_held=0
threads_rust=0
threads_rust_differs=0
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

# The stack.
stack_ok=0
stack_bad=0
declare -a DEPTHS=()
for fx in scripts/fixtures/wasmstack/*.gr; do
    "$GRASM" tir "$fx" >"$WORK/fx.tir" 2>/dev/null && build_wasm "$WORK/fx.tir" "$WORK/fx.wasm" >/dev/null 2>&1 || {
        stack_bad=$((stack_bad + 1)); echo "FAILED: $fx does not build"; continue; }
    node scripts/wasm/runprog.mjs "$WORK/fx.wasm" 100000000 >/dev/null 2>"$WORK/fx.err" </dev/null
    c=$?
    if [ "$c" = 101 ] && grep -qx 'panic: stack overflow: recursion too deep' "$WORK/fx.err"; then
        stack_ok=$((stack_ok + 1))
    else
        stack_bad=$((stack_bad + 1))
        echo "FAILED: $fx past any depth: exit $c, $(head -c 200 "$WORK/fx.err" | tr '\n' ' ')"
    fi
    # The deepest it reaches, by bisection on its argument.
    lo=1
    hi=200000
    while [ "$lo" -lt "$hi" ]; do
        mid=$(( (lo + hi + 1) / 2 ))
        if node scripts/wasm/runprog.mjs "$WORK/fx.wasm" "$mid" >/dev/null 2>&1 </dev/null; then lo=$mid; else hi=$((mid - 1)); fi
    done
    DEPTHS+=("$(basename "$fx" .gr) $lo")
done

echo
echo "skipped, by reason:"
for r in "${!REASONS[@]}"; do echo "  ${REASONS[$r]}  $r"; done | sort -k2
echo
echo "wasmsubset: ${#S3_SRC[@]} programs with a main, $skipped set aside, $compared compared"
echo "as grasm run: $same_run   as the native build (stdout, stderr, status): $same_native   timed out: $timedout"
echo "named par_map, par_filter or spawn and held: $threads_held, the Rust module running them as grasm run does: $threads_rust, otherwise: $threads_rust_differs"
echo "modules: $nmods   built twice the same: $reproducible   V8 validates: ${v8_ok:-0} (refuses ${v8_bad:-0})   wasm_validate.py: $pv_ok valid, $pv_bad invalid, $pv_bodies bodies checked"
echo "refusals: $refuse_same of $refuse_n as the Rust module's"
echo "stack: $stack_ok of $((stack_ok + stack_bad)) recursions past any depth end in the panic; deepest reached: ${DEPTHS[*]}"
status=0
[ "$fail" -gt 0 ] && status=1
[ "$stack_bad" -gt 0 ] && status=1
if [ -z "$STOP" ]; then
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
