#!/usr/bin/env bash
# The playground built by the Grasm backend (docs/self-hosting.md §6.25, S8
# part eight): `tools/playground` -- the frontend and the TIR interpreter in
# Grasm -- compiled by `s3 --target wasm32 --exports` into a module with the
# Rust playground's ABI (`crates/grasm_wasm`), held to that module and to
# `grasm run`, through `scripts/wasm/run.mjs`, the glue both share.
#
# Two modes, both the same module: with JS promise integration
# (`GRASM_WASM_STACKS` unset; the engine has it), stacks are switched -- the
# interpreter hops to fresh stacks and runs a program's tasks; without it
# (`GRASM_WASM_STACKS=0`, an engine with none), nothing is switched.
#
# Sections (`PLAYTEST_ONLY` names a subset, for the controls):
#
#   build    the module built twice, the same bytes; V8 and
#            `scripts/wasm_validate.py` validate it (bodies counted); its
#            imports exactly the seven named below, each supplied by
#            `run.mjs`; its exports the Rust module's five, of the same
#            arities, with `memory` and `_fiber`
#   frame    `scripts/fixtures/play/`'s programs and the frame's edges --
#            no source, invalid UTF-8, a bundle, standard input with and
#            without a trailing newline, a journal that does not install --
#            each answered as the Rust module answers it, status, output and
#            text, in both modes; where the Grasm module answers otherwise
#            by decision (an `eprint` kept, tasks run, `\r\n` read as
#            `grasm run` reads it), held to `grasm run` instead and counted
#   session  the same programs run one after another in one instance
#            (`run.mjs --session`), each answer the one a fresh instance
#            gives (decision (6), the reset)
#   stack    four shapes of recursion at 10,000 calls (as `grasm run`),
#            99,998 (run) and 99,999 (the count's panic) under JSPI, and at
#            10,000 without it (the open defect: the panic, never a trap);
#            `scripts/fixtures/stack/` as `grasm run`, with the two
#            differences §6.23 allows counted as `clidiff`'s `stack` section
#            counts them; out of memory, in a module capped at 256 MiB:
#            the panic "memory allocation of N bytes failed", 101; and a
#            program's module mapping a fiber-sized block, unmapping it dirty
#            and mapping it again, zero both times (`mapzero.gr`)
#   corpus   every corpus program `wasmtest` compares, under JSPI: output,
#            status and standard error (the panic's stack trace stripped
#            from `grasm run`'s, `scripts/lib/trace.sh`, as `clidiff` strips
#            it) as `grasm run`'s, and output and status as the Rust
#            module's; the ones `wasmtest` sets aside only for threads held
#            as `wasmsubset` holds them; the ones it sets aside for stack
#            switching held to `grasm run` and counted apart; the set aside
#            are exactly `wasmtest`'s, for its reasons. Without JSPI, the
#            sample named in `scripts/lib/playsample.txt`, as `grasm run`
#            but for a recursion deeper than one engine stack, which must end
#            in the panic, never a trap, and is counted
#   replay   `scripts/replaytest.sh` with this module: a native recording
#            replayed alike
#   speed    size, and three interleaved rounds of whole runs against the
#            Rust module, as ratios: printed, not held
#
# Limits: `PLAYTEST_LIMIT` seconds a run (600), `PLAYTEST_LONG` (3600) for
# the two programs that take minutes under every engine, named below, and
# for them alone. `S3=`, `WASM_LIBC=` (the C library's package) and
# `PLAYGROUND=` (the root holding `tools/playground` and its dependencies)
# name the code under test, for `scripts/wasmcontrols.sh`;
# `PLAYTEST_STOP=1` stops at the first failure.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require
. "$(dirname "$0")/lib/trace.sh"
. "$(dirname "$0")/lib/wasmskip.sh"
RUST_WASM=${WASM:-./target/wasm32-unknown-unknown/release/grasm_wasm.wasm}
ONLY=${PLAYTEST_ONLY:-"build frame session stack corpus replay speed"}
STOP=${PLAYTEST_STOP:-}
LIMIT=${PLAYTEST_LIMIT:-600}
LONG=${PLAYTEST_LONG:-3600}
LONG_PROGRAMS="packages/gc-demo/src/main.gr examples/parallel_map.gr"
LIBC=${WASM_LIBC:-compiler/rtwasm}
ROOT=${PLAYGROUND:-.}
DEEP_MSG="panic: stack overflow: recursion too deep"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver
command -v node >/dev/null 2>&1 || { echo "error: node is needed to run a wasm module"; exit 1; }
[ -f "$RUST_WASM" ] || { echo "error: the Rust module $RUST_WASM is missing (WASM= names another)"; exit 1; }
t_start=$(date +%s)

fail=0
declare -a FAILED=()
failed() {
    fail=$((fail + 1))
    FAILED+=("$1")
    echo "FAILED: $1"
    if [ -n "$STOP" ]; then
        summary
        exit 1
    fi
}
want() {
    case " $ONLY " in
    *" $1 "*) return 0 ;;
    esac
    return 1
}
summary() {
    echo
    echo "playtest: $(( ($(date +%s) - t_start) / 60 )) minutes   failed: $fail"
    if [ ${#FAILED[@]} -gt 0 ]; then
        printf '  %s\n' "${FAILED[@]}"
    fi
}

# ----- build --------------------------------------------------------------------

# The C library as the playground links it: `play/streams.gr` in
# `src/streams.gr`'s place, as the runtime's Linux object is built from a
# copy with `os/linux.gr` as `src/os.gr` (docs/self-hosting.md §6.18).
mkdir -p "$WORK/libc"
cp -R "$LIBC/." "$WORK/libc/"
cp "$LIBC/play/streams.gr" "$WORK/libc/src/streams.gr"
rm -rf "$WORK/libc/play"
for t in "rt compiler/rt/src/main.gr" "libc $WORK/libc/src/main.gr" "prog $LIBC/src/main.gr" "pg $ROOT/tools/playground/src/main.gr"; do
    set -- $t
    if ! "$GRASM" tir "$2" >"$WORK/$1.tir" 2>"$WORK/$1.err"; then
        echo "error: the TIR of $2:"
        head -5 "$WORK/$1.err"
        exit 1
    fi
done
build_pg() { # OUT [s3 option ...]
    local out=$1
    shift
    "$S3" "$WORK/pg.tir" -o "$out" --target wasm32 --exports --export-prefix play_:grasm_ \
        --runtime-tir "$WORK/rt.tir" --libc-tir "$WORK/libc.tir" "$@"
}
build_prog() { # TIR OUT
    "$S3" "$1" -o "$2" --target wasm32 --runtime-tir "$WORK/rt.tir" --libc-tir "$WORK/prog.tir"
}
PGW="$WORK/playground.wasm"
t0=$(date +%s)
if ! build_pg "$PGW" >"$WORK/s3.log" 2>&1; then
    echo "error: the playground does not build:"
    head -5 "$WORK/s3.log"
    exit 1
fi
link_s=$(( $(date +%s) - t0 ))
built_same=0
if build_pg "$WORK/again.wasm" >/dev/null 2>&1 && cmp -s "$PGW" "$WORK/again.wasm"; then
    built_same=1
else
    failed "the module built twice is not the same bytes"
fi
v8=$(node -e 'try { new WebAssembly.Module(require("fs").readFileSync(process.argv[1])); console.log("valid") } catch (e) { console.log("REFUSES " + e.message) }' "$PGW")
[ "$v8" = valid ] || failed "V8 $v8"
python3 -B scripts/wasm_validate.py "$PGW" >"$WORK/pv.txt" 2>&1
pv_bodies=$(awk '/^ok /{sub("bodies=","",$3); print $3}' "$WORK/pv.txt")
if [ -z "$pv_bodies" ] || [ "$pv_bodies" -lt 1000 ]; then
    failed "wasm_validate.py: $(head -1 "$WORK/pv.txt")"
    pv_bodies=0
fi
imports=$(node scripts/wasm/imports.mjs "$PGW" | sort | tr '\n' ' ')
want_imports="env.grasm_entropy env.grasm_fiber env.grasm_monotonic_millis env.grasm_now_millis env.grasm_sleep_millis env.grasm_stacks env.grasm_switch "
[ "$imports" = "$want_imports" ] || failed "the module imports $imports, not $want_imports"
for sym in $imports; do
    grep -cF "${sym#env.}:" scripts/wasm/run.mjs >/dev/null || failed "run.mjs does not supply $sym"
done
arities() {
    node -e '
const m = new WebAssembly.Module(require("fs").readFileSync(process.argv[1]));
const i = new WebAssembly.Instance(m, { env: new Proxy({}, { get: () => () => 0 }) });
const names = WebAssembly.Module.exports(m).map((e) => e.name).filter((n) => n.startsWith("grasm_")).sort();
console.log(names.map((n) => n + "/" + i.exports[n].length).join(" "));' "$1"
}
pg_ar=$(arities "$PGW")
rw_ar=$(arities "$RUST_WASM")
[ "$pg_ar" = "$rw_ar" ] || failed "the exports $pg_ar are not the Rust module's $rw_ar"
exports=$(node -e 'console.log(WebAssembly.Module.exports(new WebAssembly.Module(require("fs").readFileSync(process.argv[1]))).map((e) => e.name).sort().join(" "))' "$PGW")
[ "$exports" = "_fiber grasm_alloc grasm_free grasm_run grasm_run_with_journal grasm_run_with_stdin memory" ] || failed "the module exports $exports"
pg_bytes=$(wc -c <"$PGW" | tr -d ' ')
rw_bytes=$(wc -c <"$RUST_WASM" | tr -d ' ')
pg_gz=$(gzip -9c "$PGW" | wc -c | tr -d ' ')
rw_gz=$(gzip -9c "$RUST_WASM" | wc -c | tr -d ' ')
echo "build: built twice the same: $built_same   V8: $v8   wasm_validate.py: $pv_bodies bodies   link: ${link_s}s"
echo "  exports: $pg_ar (the Rust module's: $rw_ar)"
echo "  imports: $imports"
echo "  size: $pg_bytes bytes ($pg_gz under gzip -9), the Rust module's $rw_bytes ($rw_gz)"

# Runs `$2` through module `$1` in mode `$3` (1 JSPI, 0 none), optional
# standard input `$4`; output, text and status into `$WORK/<$5>.{out,err,rc}`.
play() {
    local mod=$1 src=$2 mode=$3 stdin=${4:-} tag=$5 lim=$LIMIT
    case " $LONG_PROGRAMS " in *" ${6:-} "*) lim=$LONG ;; esac
    if [ "$mode" = 0 ]; then
        GRASM_WASM_STACKS=0 timeout "$lim" node scripts/wasm/run.mjs "$mod" "$src" $stdin >"$WORK/$tag.out" 2>"$WORK/$tag.err" </dev/null
    else
        timeout "$lim" node scripts/wasm/run.mjs "$mod" "$src" $stdin >"$WORK/$tag.out" 2>"$WORK/$tag.err" </dev/null
    fi
    echo $? >"$WORK/$tag.rc"
}
same3() { # TAG TAG: output, text and status
    cmp -s "$WORK/$1.out" "$WORK/$2.out" && cmp -s "$WORK/$1.err" "$WORK/$2.err" && cmp -s "$WORK/$1.rc" "$WORK/$2.rc"
}
same2() { # TAG TAG: output and status
    cmp -s "$WORK/$1.out" "$WORK/$2.out" && cmp -s "$WORK/$1.rc" "$WORK/$2.rc"
}
grun() { # SRC TAG [STDIN]: `grasm run`, its trace stripped
    local lim=$LIMIT
    case " $LONG_PROGRAMS " in *" $1 "*) lim=$LONG ;; esac
    if [ -n "${3:-}" ]; then
        timeout "$lim" "$GRASM" run "$1" >"$WORK/$2.out" 2>"$WORK/$2.raw" <"$3"
    else
        timeout "$lim" "$GRASM" run "$1" >"$WORK/$2.out" 2>"$WORK/$2.raw" </dev/null
    fi
    echo $? >"$WORK/$2.rc"
    strip_trace "$(cat "$WORK/$2.raw")" >"$WORK/$2.err"
    # `strip_trace` reads through a here-string, which ends what it gives
    # back with a newline; an empty text stays empty.
    [ -s "$WORK/$2.raw" ] || : >"$WORK/$2.err"
}
show() { # TAG TAG
    echo "    $1: exit $(cat "$WORK/$1.rc"); $2: exit $(cat "$WORK/$2.rc")"
    diff "$WORK/$1.out" "$WORK/$2.out" | head -4 | sed 's/^/    out /'
    diff "$WORK/$1.err" "$WORK/$2.err" | head -4 | sed 's/^/    err /'
}

# ----- frame --------------------------------------------------------------------

F=scripts/fixtures/play
fr_n=0
fr_rust=0
fr_run=0
if want frame; then
    printf '' >"$WORK/empty.gr"
    printf 'fn main() {\n    print("a\377b")\n}\n' >"$WORK/badutf8.gr"
    "$GRASM" build examples/multi/app.gr --source -o "$WORK/bundle.gr" >/dev/null 2>&1 ||
        failed "examples/multi/app.gr does not bundle"
    printf 'one\ntwo\nthree\n' >"$WORK/in-nl.txt"
    printf 'one\n\ntwo' >"$WORK/in-plain.txt"
    printf 'a\r\nb\n' >"$WORK/in-crlf.txt"
    printf 'not a journal' >"$WORK/bad.jrnl"
    printf '' >"$WORK/empty.jrnl"
    # Each case: the source, then what run.mjs takes after it.
    cases="$F/hello.gr|
$F/typeerr.gr|
$F/nomain.gr|
$F/exitneg.gr|
$F/mainerr.gr|
$F/eprint.gr|
$F/tasks.gr|
$F/input.gr|
$F/input.gr|$WORK/in-nl.txt
$F/input.gr|$WORK/in-plain.txt
$F/input.gr|$WORK/in-crlf.txt
$WORK/empty.gr|
$WORK/badutf8.gr|
$WORK/bundle.gr|
$F/hello.gr|--journal $WORK/bad.jrnl
$F/hello.gr|--journal $WORK/empty.jrnl"
    while IFS='|' read -r src rest; do
        fr_n=$((fr_n + 1))
        node scripts/wasm/run.mjs "$RUST_WASM" "$src" $rest >"$WORK/rw.out" 2>"$WORK/rw.err" </dev/null
        echo $? >"$WORK/rw.rc"
        for mode in 1 0; do
            play "$PGW" "$src" $mode "$rest" pg
            # Where the Grasm module answers otherwise by decision, `grasm
            # run` is the reference: an `eprint` kept (4), tasks run (2),
            # `\r\n` read as `grasm run` reads a line.
            case "$src|$rest|$mode" in
            "$F/eprint.gr||"* | "$F/tasks.gr||1" | "$F/input.gr|$WORK/in-crlf.txt|"*)
                if [ "$rest" != "" ]; then
                    grun "$src" gr "${rest}"
                else
                    grun "$src" gr
                fi
                if same3 pg gr; then
                    fr_run=$((fr_run + 1))
                else
                    failed "$src $rest (mode $mode) -- not as grasm run"
                    show gr pg
                fi
                ;;
            *)
                if same3 pg rw; then
                    fr_rust=$((fr_rust + 1))
                else
                    failed "$src $rest (mode $mode) -- not as the Rust module answers it"
                    show rw pg
                fi
                ;;
            esac
        done
    done <<<"$cases"
    echo "frame: $fr_n cases, 2 modes each   as the Rust module: $fr_rust   as grasm run, by decision: $fr_run"
    [ $((fr_rust + fr_run)) = $((2 * fr_n)) ] || failed "frame: $((2 * fr_n)) answers, $((fr_rust + fr_run)) accounted for"
fi

# ----- session ------------------------------------------------------------------

se_n=0
se_same=0
if want session; then
    # A journal for `probe.gr`, recorded natively: replayed, then the same
    # program run plainly, which must not answer from it.
    if ! "$GRASM" build --native "$F/probe.gr" -o "$WORK/probe" >/dev/null 2>&1; then
        failed "$F/probe.gr does not build natively"
    fi
    GRASM_RECORD="$WORK/probe.jrnl" "$WORK/probe" >/dev/null 2>&1
    printf '%s\n' \
        "$F/eprint.gr	-	-" \
        "$F/input.gr	$WORK/in-nl.txt	-" \
        "$F/input.gr	-	-" \
        "$F/probe.gr	-	$WORK/probe.jrnl" \
        "$F/probe.gr	-	-" \
        "$F/tasks.gr	-	-" \
        "$F/typeerr.gr	-	-" \
        "$F/mainerr.gr	-	-" \
        "$F/hello.gr	-	-" >"$WORK/session.txt"
    for mode in 1 0; do
        if [ "$mode" = 0 ]; then
            GRASM_WASM_STACKS=0 node scripts/wasm/run.mjs "$PGW" --session "$WORK/session.txt" >"$WORK/one.jsonl" 2>"$WORK/one.err"
        else
            node scripts/wasm/run.mjs "$PGW" --session "$WORK/session.txt" >"$WORK/one.jsonl" 2>"$WORK/one.err"
        fi
        : >"$WORK/fresh.jsonl"
        while IFS= read -r line; do
            printf '%s\n' "$line" >"$WORK/single.txt"
            if [ "$mode" = 0 ]; then
                GRASM_WASM_STACKS=0 node scripts/wasm/run.mjs "$PGW" --session "$WORK/single.txt" >>"$WORK/fresh.jsonl" 2>/dev/null
            else
                node scripts/wasm/run.mjs "$PGW" --session "$WORK/single.txt" >>"$WORK/fresh.jsonl" 2>/dev/null
            fi
        done <"$WORK/session.txt"
        k=0
        while IFS= read -r a && IFS= read -r b <&3; do
            k=$((k + 1))
            se_n=$((se_n + 1))
            if [ "$a" = "$b" ]; then
                se_same=$((se_same + 1))
            else
                failed "session (mode $mode): run $k in one instance is not what a fresh instance answers -- state carried: $(echo "$a" | head -c 200) vs $(echo "$b" | head -c 200)"
            fi
        done <"$WORK/one.jsonl" 3<"$WORK/fresh.jsonl"
    done
    want_runs=$((2 * $(wc -l <"$WORK/session.txt")))
    echo "session: $se_n runs in one instance   as a fresh instance: $se_same"
    [ "$se_n" = "$want_runs" ] || failed "session: $want_runs runs expected, $se_n answered"
fi

# ----- stack --------------------------------------------------------------------

sk_runs=0
sk_ok=0
sk_window=0
sk_task=0
if want stack; then
    shape() { # SHAPE N OUT
        sed -e "s/    let shape = \"plain\"/    let shape = \"$1\"/" -e "s/    let n = 10\$/    let n = $2/" "$F/shapes.gr" >"$3"
    }
    for s in plain stmt mapped through; do
        shape $s 10000 "$WORK/s.gr"
        grun "$WORK/s.gr" gr
        play "$PGW" "$WORK/s.gr" 1 "" pg
        sk_runs=$((sk_runs + 1))
        if same3 pg gr && [ "$(cat "$WORK/pg.rc")" = 0 ]; then
            sk_ok=$((sk_ok + 1))
        else
            failed "$s 10,000 deep under JSPI -- not as grasm run"
            show gr pg
        fi
        # Past the Rust interpreter's stack: held to the count both Grasm
        # engines keep (§6.23), the output the depth, then the count's panic.
        shape $s 99998 "$WORK/s.gr"
        play "$PGW" "$WORK/s.gr" 1 "" pg
        sk_runs=$((sk_runs + 1))
        if [ "$(cat "$WORK/pg.rc")" = 0 ] && [ "$(cat "$WORK/pg.out")" = "$([ $s = stmt ] && echo done || echo 99998)" ]; then
            sk_ok=$((sk_ok + 1))
        else
            failed "$s 99,998 deep under JSPI -- exit $(cat "$WORK/pg.rc"): $(head -c 200 "$WORK/pg.err")"
        fi
        shape $s 99999 "$WORK/s.gr"
        play "$PGW" "$WORK/s.gr" 1 "" pg
        sk_runs=$((sk_runs + 1))
        if [ "$(cat "$WORK/pg.rc")" = 101 ] && [ "$(cat "$WORK/pg.err")" = "$DEEP_MSG" ] && [ ! -s "$WORK/pg.out" ]; then
            sk_ok=$((sk_ok + 1))
        else
            failed "$s 99,999 deep under JSPI -- not the count's panic: exit $(cat "$WORK/pg.rc"): $(head -c 200 "$WORK/pg.err")"
        fi
        # Without JSPI: the open defect (docs/known-issues.md) -- the
        # recursion ends in the panic, never a trap.
        shape $s 10000 "$WORK/s.gr"
        play "$PGW" "$WORK/s.gr" 0 "" pg
        sk_runs=$((sk_runs + 1))
        if [ "$(cat "$WORK/pg.rc")" = 101 ] && [ "$(cat "$WORK/pg.err")" = "$DEEP_MSG" ]; then
            sk_ok=$((sk_ok + 1))
        else
            failed "$s 10,000 deep without JSPI -- not the panic: exit $(cat "$WORK/pg.rc"): $(head -c 200 "$WORK/pg.err")"
        fi
    done
    for src in scripts/fixtures/stack/*.gr; do
        name=$(basename "$src" .gr)
        grun "$src" gr
        play "$PGW" "$src" 1 "" pg
        sk_runs=$((sk_runs + 1))
        if same3 pg gr; then
            sk_ok=$((sk_ok + 1))
        elif [ "${name%%-*}" = window ] && [ "$(cat "$WORK/gr.rc")" = 101 ] && [ "$(head -1 "$WORK/gr.err")" = "$DEEP_MSG" ] && [ "$(cat "$WORK/pg.rc")" = 0 ] && [ -s "$WORK/pg.out" ]; then
            sk_window=$((sk_window + 1))
            echo "  past the Rust interpreter's stack, within the count: $src"
        elif [ "$name" = task-past ] && [ "$(cat "$WORK/gr.rc")" = 0 ] && [ "$(cat "$WORK/pg.rc")" = 101 ] && [ "$(cat "$WORK/pg.err")" = "$DEEP_MSG" ]; then
            sk_task=$((sk_task + 1))
            echo "  past a task's spare stacks, within the Rust interpreter's: $src"
        else
            failed "$src under JSPI -- not as grasm run"
            show gr pg
        fi
    done
    # Out of memory: a module whose memory may not pass 256 MiB, a recursion
    # that needs more.
    if build_pg "$WORK/capped.wasm" --max-memory 256 >/dev/null 2>&1; then
        shape mapped 99998 "$WORK/s.gr"
        play "$WORK/capped.wasm" "$WORK/s.gr" 1 "" pg
        sk_runs=$((sk_runs + 1))
        if [ "$(cat "$WORK/pg.rc")" = 101 ] && grep -cxE 'panic: memory allocation of [0-9]+ bytes failed' "$WORK/pg.err" >/dev/null && [ "$(wc -l <"$WORK/pg.err" | tr -d " ")" = 1 ]; then
            sk_ok=$((sk_ok + 1))
            echo "  out of memory at 256 MiB: $(cat "$WORK/pg.err")"
        else
            failed "out of memory at 256 MiB -- not the panic: exit $(cat "$WORK/pg.rc"): $(head -c 200 "$WORK/pg.err")"
        fi
    else
        failed "the playground capped at 256 MiB does not build"
    fi
    # A fiber-sized mapping, unmapped dirty and mapped again: zero.
    if "$GRASM" tir "$F/mapzero.gr" >"$WORK/mz.tir" 2>/dev/null && build_prog "$WORK/mz.tir" "$WORK/mz.wasm" >/dev/null 2>&1; then
        node scripts/wasm/runprog.mjs "$WORK/mz.wasm" >"$WORK/mz.out" 2>&1
        sk_runs=$((sk_runs + 1))
        if [ "$(cat "$WORK/mz.out")" = "first zero: true
again zero: true" ]; then
            sk_ok=$((sk_ok + 1))
        else
            failed "$F/mapzero.gr -- a mapping not zero: $(tr '\n' ' ' <"$WORK/mz.out")"
        fi
    else
        failed "$F/mapzero.gr does not build for wasm32"
    fi
    echo "stack: $sk_runs runs   as held: $sk_ok   past the Rust interpreter's stack, within the count: $sk_window   past a task's spare stacks: $sk_task"
    [ $((sk_ok + sk_window + sk_task)) = "$sk_runs" ] || failed "stack: $sk_runs runs, $((sk_ok + sk_window + sk_task)) accounted for"
    [ "$sk_runs" -ge 26 ] || failed "stack: $sk_runs runs, fewer than the 26 written"
fi

# ----- corpus -------------------------------------------------------------------

co_compared=0
co_run=0
co_rust=0
co_err=0
th_held=0
th_rust=0
sw_held=0
nj_n=0
nj_same=0
nj_deep=0
skipped=0
declare -A REASONS=()
if want corpus; then
    declare -A SAMPLE=()
    while IFS= read -r p; do
        case "$p" in "" | "#"*) continue ;; esac
        SAMPLE["$p"]=1
    done <scripts/lib/playsample.txt
    for src in $(corpus_programs); do
        grep -qE '^\s*(pub )?fn main\(' "$src" || continue
        grep -q '^//! expect: error$' "$src" && continue
        subject=$src
        run_src=$src
        if grep -qE '^\s*import ' "$src"; then
            bundle="$WORK/b.bundle.gr"
            if ! "$GRASM" build "$src" --source -o "$bundle" >/dev/null 2>&1; then
                skipped=$((skipped + 1))
                REASONS["its modules could not be bundled"]=$(( ${REASONS["its modules could not be bundled"]:-0} + 1 ))
                continue
            fi
            subject=$bundle
            run_src=$bundle
        fi
        why=$(wasm_skip_reason "$src" "$subject")
        kind=main
        if [[ "$why" == needs\ threads* ]]; then
            why=$(WASM_SKIP_NO_THREADS=1 wasm_skip_reason "$src" "$subject")
            [ -z "$why" ] && kind=threads
        elif [[ "$why" == needs\ stack-switching* ]]; then
            kind=switching
            why=""
        fi
        if [ -n "$why" ]; then
            skipped=$((skipped + 1))
            REASONS["$why"]=$(( ${REASONS["$why"]:-0} + 1 ))
            continue
        fi
        grun "$src" gr
        play "$PGW" "$run_src" 1 "" pg "$src"
        case $kind in
        main)
            co_compared=$((co_compared + 1))
            if same2 pg gr; then
                co_run=$((co_run + 1))
            else
                failed "$src -- not as grasm run"
                show gr pg
            fi
            if cmp -s "$WORK/pg.err" "$WORK/gr.err"; then
                co_err=$((co_err + 1))
            else
                failed "$src -- standard error not grasm run's (its trace stripped)"
                show gr pg
            fi
            play "$RUST_WASM" "$run_src" 1 "" rw "$src"
            if same2 pg rw; then
                co_rust=$((co_rust + 1))
            else
                failed "$src -- not as the Rust module"
                show rw pg
            fi
            ;;
        threads)
            th_held=$((th_held + 1))
            if same2 pg gr; then
                play "$RUST_WASM" "$run_src" 1 "" rw "$src"
                if same2 rw gr; then
                    th_rust=$((th_rust + 1))
                fi
            else
                failed "$src (threads) -- not as grasm run"
                show gr pg
            fi
            ;;
        switching)
            sw_held=$((sw_held + 1))
            if ! same3 pg gr; then
                failed "$src (stack switching, under JSPI) -- not as grasm run"
                show gr pg
            fi
            ;;
        esac
        if [ -n "${SAMPLE[$src]:-}" ]; then
            nj_n=$((nj_n + 1))
            play "$PGW" "$run_src" 0 "" nj "$src"
            if [ $kind = switching ]; then
                # Refused as the Rust module refuses a `concurrent` block.
                play "$RUST_WASM" "$run_src" 1 "" rw "$src"
                if same3 nj rw; then
                    nj_same=$((nj_same + 1))
                else
                    failed "$src without JSPI -- not refused as the Rust module refuses it"
                    show rw nj
                fi
            elif same3 nj pg; then
                nj_same=$((nj_same + 1))
            elif [ "$(cat "$WORK/nj.rc")" = 101 ] && [ "$(tail -1 "$WORK/nj.err")" = "$DEEP_MSG" ]; then
                nj_deep=$((nj_deep + 1))
                echo "  deeper than one engine stack, without JSPI: $src"
            else
                failed "$src without JSPI -- not as under JSPI, and not the stack's panic"
                show pg nj
            fi
        fi
    done
    echo "corpus: compared $co_compared   as grasm run: $co_run (standard error too: $co_err)   as the Rust module: $co_rust   set aside: $skipped"
    for r in "${!REASONS[@]}"; do
        echo "  set aside, ${REASONS[$r]}: $r"
    done | sort
    echo "  named par_map, par_filter or spawn and held: $th_held, the Rust module running them as grasm run does: $th_rust"
    echo "  stack switching, held under JSPI to grasm run: $sw_held"
    echo "  without JSPI, the sample (scripts/lib/playsample.txt): $nj_n   as under JSPI (or refused as the Rust module refuses): $nj_same   deeper than one engine stack, the panic: $nj_deep"
    floor=${PLAYTEST_FLOOR:-508}
    [ "$co_compared" -ge "$floor" ] || failed "corpus: $co_compared compared, below the floor of $floor"
    sample_n=$(grep -cvE '^(#|$)' scripts/lib/playsample.txt)
    [ "$nj_n" = "$sample_n" ] || [ "${DIRS:-}" != "" ] || failed "the sample names $sample_n programs and $nj_n ran"
fi

# ----- replay -------------------------------------------------------------------

if want replay; then
    WASM="$PGW" GRASM_NO_BUILD=1 scripts/replaytest.sh >"$WORK/replay.log" 2>&1
    rp=$(grep -F 'a native recording replayed elsewhere:' "$WORK/replay.log")
    echo "replay: ${rp#*: }"
    [ "$rp" = "a native recording replayed elsewhere: identical on every engine" ] ||
        failed "a native recording not replayed alike by the module: $(tail -5 "$WORK/replay.log" | tr '\n' ' ')"
fi

# ----- speed --------------------------------------------------------------------

if want speed; then
    progs="benchmarks/grasm/fib.gr benchmarks/grasm/sieve.gr benchmarks/grasm/matmul.gr benchmarks/grasm/sort.gr benchmarks/grasm/tree.gr benchmarks/grasm/wordfreq.gr examples/hello.gr examples/effects.gr"
    secs() {
        local s e
        s=$(python3 -c 'import time; print(time.time())')
        "$@" >/dev/null 2>&1 </dev/null
        e=$(python3 -c 'import time; print(time.time())')
        python3 -c "print($e - $s)"
    }
    : >"$WORK/speed.tsv"
    for r in 1 2 3; do
        for p in $progs; do
            if [ $((r % 2)) = 1 ]; then
                a=$(secs node scripts/wasm/run.mjs "$RUST_WASM" "$p")
                b=$(secs node scripts/wasm/run.mjs "$PGW" "$p")
            else
                b=$(secs node scripts/wasm/run.mjs "$PGW" "$p")
                a=$(secs node scripts/wasm/run.mjs "$RUST_WASM" "$p")
            fi
            printf '%s\t%s\t%s\n' "$p" "$a" "$b" >>"$WORK/speed.tsv"
        done
    done
    python3 - "$WORK/speed.tsv" <<'PY'
import collections, math, statistics, sys
d = collections.defaultdict(lambda: ([], []))
for line in open(sys.argv[1]):
    p, a, b = line.split("\t")
    d[p][0].append(float(a))
    d[p][1].append(float(b))
rs = []
for p, (a, b) in d.items():
    r = statistics.median(b) / statistics.median(a)
    rs.append(r)
    print(f"  {p}: the Rust module {statistics.median(a):.3f}s, this one {statistics.median(b):.3f}s, {r:.3f}")
print(f"speed: geometric mean of this module's time over the Rust module's, three interleaved rounds: {math.exp(sum(map(math.log, rs)) / len(rs)):.3f}")
PY
fi

summary
[ "$fail" = 0 ]
