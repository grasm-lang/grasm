#!/usr/bin/env bash
# `scripts/clidiff.sh` must fail on a planted fault, each for its reason
# (docs/self-hosting.md §6.17).
#
# A control copies the Grasm command's sources -- every directory its TIR
# names: compiler/, tools/tirinterp and the packages they use -- changes one
# thing in compiler/grasm there, builds that command with the Rust `grasm`,
# and runs clidiff on it (CLI=, CLI_SRC=), on the sections and programs that
# can show the fault. clidiff must fail with the line the control names.
#
#   reworded       "takes one file" says something else          a line differs
#   exit-status    the usage after an unknown command exits 2     a line differs
#   flag-misread   `--link` read as a flag with no value          a line differs
#   routed         `check` handed to the Rust `grasm` by proc_run the count of proc_run
#   routed-sandbox the same, the count skipped (CLIDIFF_FAULT=nogrep): the
#                  sandbox stops the Rust `grasm` from running     a line differs
#   backend-name   the success line says Cranelift                 a line differs
#   wrong-target   `build --native` compiles for x86-64            a build differs
#   linux-refused  the Linux triples refused as before §6.18        a line differs
#   linux-macho    `--target <linux>` writes a Mach-O object        a target differs
#   fmt-spacing    the formatter puts a space before a comma        a format differs
#   lint-dropped   the linter drops its shadowing finding           a lint differs
#   test-inverted  a unit test that holds reported failed, and back a test differs
#   test-no-trap   a test's panic ends the command (no `trap`)      a test differs
#   bench-filter   `bench --filter` ignored                         a bench differs
#   doc-dropped    each doc comment's last line dropped             a doc differs
#   api-missed     a changed signature not reported                 an api differs
#   bundle-entry   the bundle names its entry with its directory    a bundle differs
#   source-order   `--source` writes the modules last first          a source differs
#   lock-order     `deps` writes the lockfile's entries last first    a lockfile differs
#   dep-dropped    `deps` leaves the first package out of the lock    a lockfile differs
#   publish-where  `publish` writes into <registry>/<version>/<name>  a registry differs
#   toml-first     the TOML reader keeps the first of a key set twice
#                  in an entry of an array of tables, not the last    a deps differs
#   journal-lost   the interpreter's journal drops every `read_bytes` record
#                                                                    the journals differ
#   exempt-gone    the runtime looks for another name than `unjournaled_host`,
#                  so the command journals its own reads             a recording differs
#   dap-field      `stopped` without `allThreadsStopped`             a session differs
#   dap-line       every stop a line further on                      a session differs
#   lsp-diag       a diagnostic at the end of the buffer kept         a session differs
#   lsp-hover      a function's signature written `->` with no spaces a session differs
#   dap-hang       the adapter never ends a session of `testing_tour`,
#                  even given the program's run time               a session differs
#   net-handle     every `tcp_connect` leaks a listener first, so each
#                  socket's handle is one higher before any `net_close`,
#                  outside the journal's renumbering exception     the journals differ
#   sched-seed     the scheduler ignores the schedule seed         a schedule differs
#   task-panic-pass a task's panic taken for the task ending well  a test differs
#   stack-frame    the count of calls one higher                   a recursion differs
#   task-stack     no spare host stacks for a task                 a recursion differs
#   recursion-seen the analysis does not see a call through an
#                  interface's method, and finds no recursion      a recursion differs
#   iofailed-old   an `IoFailed` worded as the runtime words it    an IoFailed differs
#
# `CONTROLS=a,b` runs only the controls named, for trying one; the floor of
# 36 holds only on a whole run. `PLANT_RT=1` in a control's variables builds
# its runtime object from the planted sources too.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
export GRASM
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The directories the command's sources are in, from its TIR.
if ! "$GRASM" tir compiler/grasm/src/main.gr >"$WORK/list.tir" 2>"$WORK/list.err"; then
    echo "error: grasm tir compiler/grasm/src/main.gr fails: $(head -1 "$WORK/list.err")"
    exit 1
fi
grep '^(file ' "$WORK/list.tir" | awk '{ print $4 }' | tr -d '"' | awk -F/ '{ print $1 "/" $2 }' | sort -u >"$WORK/dirs"
if [ "$(grep -c . "$WORK/dirs")" -lt 8 ] || ! grep -qx compiler/grasm "$WORK/dirs"; then
    echo "error: the command's sources are in $(grep -c . "$WORK/dirs") directories, which cannot be all of them"
    exit 1
fi

# The runtime object every planted command is given: the clean command's.
mkdir -p "$WORK/clean"
if ! "$GRASM" build compiler/grasm/src/main.gr --native --no-incremental -o "$WORK/clean/grasm" >"$WORK/clean.log" 2>&1 ||
    ! "$WORK/clean/grasm" build compiler/rt/src/main.gr --native -o "$WORK/clean/rt.o" >>"$WORK/clean.log" 2>&1; then
    echo "error: compiler/grasm does not build:"
    sed 's/^/  /' "$WORK/clean.log"
    exit 1
fi

caught=0
missed=0
ran=0
declare -a MISSED=()

# plant NAME FILE OLD NEW: the sources copied to $WORK/NAME, with OLD in FILE
# (a path in the tree) replaced by NEW, exactly once; then the command built
# from them into $WORK/NAME/cli, the clean runtime object beside it, or with
# PLANT_RT set, compiler/rt copied too and its object built from the copy.
plant() {
    local name=$1 file=$2 d
    shift 2
    rm -rf "${WORK:?}/$name"
    while IFS= read -r d; do
        mkdir -p "$WORK/$name/$(dirname "$d")"
        cp -R "$d" "$WORK/$name/$d" || return 1
    done <"$WORK/dirs"
    # The runtime is not one of the command's sources; a control that
    # plants into it has it copied too.
    if [ -n "${PLANT_RT:-}" ] && [ ! -d "$WORK/$name/compiler/rt" ]; then
        cp -R compiler/rt "$WORK/$name/compiler/rt" || return 1
    fi
    python3 - "$WORK/$name/$file" "$@" <<'EOF' || return 1
import sys
path = sys.argv[1]
pairs = sys.argv[2:]
text = open(path).read()
for old, new in zip(pairs[::2], pairs[1::2]):
    if text.count(old) != 1:
        print(f"the control's text occurs {text.count(old)} times in {path}: {old!r}")
        sys.exit(1)
    text = text.replace(old, new)
open(path, "w").write(text)
EOF
    mkdir -p "$WORK/$name/cli"
    "$GRASM" build "$WORK/$name/compiler/grasm/src/main.gr" --native --no-incremental -o "$WORK/$name/cli/grasm" || return 1
    if [ -n "${PLANT_RT:-}" ]; then
        "$WORK/clean/grasm" build "$WORK/$name/compiler/rt/src/main.gr" --native -o "$WORK/$name/cli/rt.o" || return 1
    else
        cp "$WORK/clean/rt.o" "$WORK/$name/cli/rt.o"
    fi
}

# control NAME WHY ENV FILE OLD NEW: plant, then clidiff, with the
# variables ENV sets, must fail with a line matching WHY.
control() {
    local name=$1 why=$2 envs=$3 file=$4 rc
    shift 4
    if [ -n "${CONTROLS:-}" ] && [[ ",$CONTROLS," != *",$name,"* ]]; then
        return 0
    fi
    ran=$((ran + 1))
    local plant_rt=
    case " $envs " in *" PLANT_RT=1 "*) plant_rt=1 ;; esac
    if ! PLANT_RT=$plant_rt plant "$name" "$file" "$@" >"$WORK/$name.plant" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: could not be planted: $(head -3 "$WORK/$name.plant" | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return 1
    fi
    # shellcheck disable=SC2086
    env $envs CLI="$WORK/$name/cli" CLI_SRC="$WORK/$name/compiler/grasm/src" GRASM_NO_BUILD=1 \
        scripts/clidiff.sh >"$WORK/$name.out" 2>&1
    rc=$?
    if [ "$rc" = 0 ]; then
        missed=$((missed + 1))
        MISSED+=("$name: clidiff passed")
        echo "MISSED: $name (clidiff passed)"
    elif grep -E "$why" "$WORK/$name.out" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-200)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: clidiff failed, but not for its reason: $(grep -m3 -E 'DIFF|error|FAILED' "$WORK/$name.out" | tr '\n' ' ' | cut -c1-300)")
        echo "MISSED: $name failed for another reason"
    fi
}

MAIN=compiler/grasm/src/main.gr
BUILD=compiler/grasm/src/build.gr
printf '%s\n' examples/hello.gr benchmarks/grasm/fib.gr >"$WORK/two"

control reworded '^DIFF: grasm check examples/hello.gr extra ' ONLY=lines "$MAIN" \
    'eprint("error: `grasm {cmd}` takes one file, and `{x}` is a second")' \
    'eprint("error: `grasm {cmd}` takes one file, and `{x}` is a second file")'

control exit-status '^DIFF: grasm bogus -- exit 1 \(Rust\) and 2 \(Grasm\)' ONLY=lines "$MAIN" \
    '    if not asked {
        exit(1)' \
    '    if not asked {
        exit(2)'

control flag-misread '^DIFF: grasm build examples/hello.gr --link ' ONLY=lines "$MAIN" \
    'return (["--native", "--no-incremental", "--source"], ["-o", "--target", "--link"])' \
    'return (["--native", "--no-incremental", "--source", "--link"], ["-o", "--target"])'

ROUTED='        match proc_run(env("GRASM") ?? "grasm", a) {
            Ok(r) -> {
                let (code, out, err) = r
                if out != "" {
                    print(out.substr(0, out.len() - 1))
                }
                if err != "" {
                    eprint(err.substr(0, err.len() - 1))
                }
                exit(code)
            }
            Err(e) -> {
                eprint("{e}")
                exit(1)
            }
        }'
control routed '^the Grasm command.s package calls proc_run or proc_pipeline: 1 time' ONLY=lines "$MAIN" \
    '        front.cmd_check(operand(a, "usage: grasm check <file.gr>"))' "$ROUTED"

control routed-sandbox '^DIFF: grasm check examples/hello.gr ' "ONLY=lines CLIDIFF_FAULT=nogrep" "$MAIN" \
    '        front.cmd_check(operand(a, "usage: grasm check <file.gr>"))' "$ROUTED"

control backend-name '^DIFF: grasm build examples/hello.gr --native -o h ' ONLY=lines "$BUILD" \
    'print("built `{out}` natively ({size} bytes, Grasm)")' \
    'print("built `{out}` natively ({size} bytes, Cranelift)")'

control wrong-target '^DIFF: grasm build --native examples/hello.gr ' "ONLY=build FILES=$WORK/two" "$BUILD" \
    '    let bytes = compile(tir_of(f), path, false, false, false)' \
    '    let bytes = compile(tir_of(f), path, false, true, false)'

control linux-refused '^DIFF: grasm build examples/hello.gr --native --target x86_64-unknown-linux-gnu ' ONLY=lines "$BUILD" \
    '    if gnu and parts[0] == "x86_64" {' \
    '    if false and gnu and parts[0] == "x86_64" {'

control linux-macho '^DIFF: grasm build --native --target aarch64-unknown-linux-gnu ' "ONLY=target FILES=$WORK/two" "$BUILD" \
    '        let bytes = compile(tir_of(f), path, false, x64, linux)' \
    '        let bytes = compile(tir_of(f), path, false, x64, false)'

# S8 part two (§6.19): one fault in each of the four subcommands, and the
# runner's trap taken away.
printf '%s\n' examples/testing_tour.gr >"$WORK/tour"
printf '%s\n' spec/conformance/E-LOOP-RANGE.gr >"$WORK/shadow"
printf '%s\n' spec/conformance/E-TEST-SHRINK.gr >"$WORK/shrink"
printf '%s\n' spec/conformance/E-BENCH-WARMUP.gr >"$WORK/warm"
FMT=tools/gfmt/src/fmt.gr
LINT=compiler/grasm/src/lint.gr
TESTING=compiler/grasm/src/testing.gr

control fmt-spacing '^DIFF: grasm fmt ' "ONLY=fmt FILES=$WORK/tour" "$FMT" \
    'if n_op and [",", ";", ")",' \
    'if n_op and [";", ")",'

control lint-dropped '^DIFF: grasm lint spec/conformance/E-LOOP-RANGE.gr ' "ONLY=lint FILES=$WORK/shadow" "$LINT" \
    '            report(l, l.a.s[id], "shadowing", "`{name}` hides the one bound on line {line}")' \
    ''

control test-inverted '^DIFF: grasm test examples/testing_tour.gr ' "ONLY=test FILES=$WORK/tour" "$TESTING" \
    '            } else if m == "\0" {
                print("test \"{label}\" ... ok")' \
    '            } else if m != "\0" {
                print("test \"{label}\" ... ok")'

control test-no-trap '^DIFF: grasm test spec/conformance/E-TEST-SHRINK.gr ' "ONLY=test FILES=$WORK/shrink" "$TESTING" \
    '                cx.trap = true' \
    '                cx.trap = false'

control bench-filter '^DIFF: grasm bench spec/conformance/E-BENCH-WARMUP.gr --filter no-such-benchmark ' "ONLY=bench FILES=$WORK/warm" "$TESTING" \
    '        if filter != "" and not p.bench_name[b].contains(filter) {' \
    '        if false and filter != "" and not p.bench_name[b].contains(filter) {'

# S8 part three (§6.20): one fault in each of `doc`, `api`, the bundle and
# `--source`.
printf '%s\n' examples/documented.gr >"$WORK/documented"
printf '%s\n' packages/b39-demo/src/main.gr >"$WORK/b39"
DOCGEN=compiler/grasm/src/docgen.gr
API=compiler/grasm/src/api.gr
BUNDLE=compiler/grasm/src/bundle.gr

control doc-dropped '^DIFF: grasm doc examples/documented.gr ' "ONLY=doc FILES=$WORK/documented" "$DOCGEN" \
    '    out.join("\n")
}

/// `render_markdown`.' \
    '    if out.len() > 1 {
        let _gone = out.pop()
    }
    out.join("\n")
}

/// `render_markdown`.'

control api-missed '^DIFF: grasm api examples/documented.gr --against, an item changed ' "ONLY=api FILES=$WORK/documented" "$API" \
    '                if now != was {' \
    '                if false and now != was {'

control bundle-entry '^DIFF: grasm build examples/hello.gr \(a bundle\) -- not the command followed by' "ONLY=bundle FILES=$WORK/two" "$BUNDLE" \
    '    let entry = if l.paths.len() > 0 { file_name(l.paths[0]) } else { "" }' \
    '    let entry = if l.paths.len() > 0 { l.paths[0] } else { "" }'

control source-order '^DIFF: grasm build packages/b39-demo/src/main.gr --source ' "ONLY=source FILES=$WORK/b39" "$BUNDLE" \
    '    for i in 0..l.names.len() {
        text.push_str("//!module ")' \
    '    for j in 0..l.names.len() {
        let i = l.names.len() - 1 - j
        text.push_str("//!module ")'

# S8 part four (§6.21): the lockfile's order, a package dropped from it, a
# registry written in the wrong place, and the TOML reader.
PKG=compiler/grasm/src/pkg.gr
TOML=packages/toml/src/main.gr

control lock-order '^DIFF: registry [a-z-]+: grasm deps app .*the files differ' ONLY=registry "$PKG" \
    '    for e in entries {
        out.push_str("\n[[package]]\n")' \
    '    for j in 0..entries.len() {
        let e = entries[entries.len() - 1 - j]
        out.push_str("\n[[package]]\n")'

control dep-dropped '^DIFF: registry [a-z-]+: grasm deps' ONLY=registry "$PKG" \
    '    var entries: [LockEntry] = []
    for i in 0..n {' \
    '    var entries: [LockEntry] = []
    for i in 1..n {'

control publish-where '^DIFF: registry publish: grasm publish pkg --registry reg ' ONLY=registry "$PKG" \
    '    let dest = loader.join(loader.join(reg, m.name), version)' \
    '    let dest = loader.join(loader.join(reg, version), m.name)'

control toml-first '^DIFF: registry lockfile: grasm deps app' ONLY=registry "$TOML" \
    '            if at_key >= 0 {
                vals[at_key] = vid' \
    '            if at_key >= 0 {
                let _first = vid'

# S8 part five (§6.22): a journal entry lost, the command's exemption from
# its own runtime's journal gone, a response field dropped, a step on the
# wrong line, a diagnostic kept that Rust drops, a hover reworded, a socket
# handle off by one before any `net_close`, a session that never ends.
printf '%s\n' packages/certdiff/src/main.gr >"$WORK/certdiff"
printf '%s\n' examples/hello.gr >"$WORK/hello"
printf '%s\n' spec/conformance/L-LIB-NET-WORKERS.gr >"$WORK/networkers"
BUILTINS=tools/tirinterp/src/builtins.gr
JOURNAL=tools/tirinterp/src/journal.gr
DEBUG=tools/tirinterp/src/debug.gr
DAPCMD=compiler/grasm/src/dap.gr
LSP=compiler/grasm/src/lsp.gr
RTJ=compiler/rt/src/journal.gr

control journal-lost '^DIFF: GRASM_RECORD grasm run packages/certdiff/src/main.gr -- the journals differ' "ONLY=journal FILES=$WORK/certdiff CLI_JOURNAL_BUILD=1" "$JOURNAL" \
    'pub fn put(cx: Cx, name: str, v: Value) {' \
    'pub fn put(cx: Cx, name: str, v: Value) {
    if name == "read_bytes" {
        return
    }'

control exempt-gone '^DIFF: GRASM_RECORD grasm run examples/hello.gr -- a journal written by one command and not the other' "ONLY=journal FILES=$WORK/hello CLI_JOURNAL_BUILD=1 PLANT_RT=1" "$RTJ" \
    'mem_static("unjournaled_host")' \
    'mem_static("unjournaled_hosx")'

# Socket handles numbered otherwise only after a `net_close`, as one
# renumbering, are the journal comparison's counted exception (the Rust
# interpreter never gives a handle out twice; §6.22): a handle that differs
# before any `net_close` is still a difference.
control net-handle '^DIFF: GRASM_RECORD grasm run spec/conformance/L-LIB-NET-WORKERS.gr -- the journals differ: record 3 \(tcp_connect\) differs' "ONLY=journal FILES=$WORK/networkers CLI_JOURNAL_BUILD=1" "$BUILTINS" \
    '        22 -> int_result(p, tcp_connect(str_of(p, args[0]), int_of(args[1])))' \
    '        22 -> {
            let _ = tcp_listen(0) ?? 0
            int_result(p, tcp_connect(str_of(p, args[0]), int_of(args[1])))
        }'

# A session the Grasm adapter does not end in its limit is given the
# program's own run time once more (§6.22), and no longer: a hang still
# fails. A short limit keeps the control short.
control dap-hang '^DIFF: grasm dap step examples/testing_tour.gr -- exit 0 \(Rust\) and 124 \(Grasm\)' "ONLY=dap FILES=$WORK/tour DAP_LIMIT=10" "$DAPCMD" \
    '    cx.trap = true
    cx.dbg = true' \
    '    cx.trap = true
    if l.paths[0].ends_with("testing_tour.gr") {
        while true {
            cx.d_seq += 0
        }
    }
    cx.dbg = true'

control dap-field '^DIFF: grasm dap step examples/hello.gr ' "ONLY=dap FILES=$WORK/hello" "$DEBUG" \
    '("threadId", json.jint(THREAD_ID)), ("allThreadsStopped", JBool(true))' \
    '("threadId", json.jint(THREAD_ID))'

control dap-line '^DIFF: grasm dap step examples/hello.gr ' "ONLY=dap FILES=$WORK/hello" "$DEBUG" \
    '    (cx.d_paths[f], lo + 1, off - starts[lo] + 1)' \
    '    (cx.d_paths[f], lo + 2, off - starts[lo] + 1)'

control lsp-diag '^DIFF: grasm lsp on examples/hello.gr ' "ONLY=lsp FILES=$WORK/hello" "$LSP" \
    '                if ss[i] < end {' \
    '                if ss[i] <= end {'

control lsp-hover '^DIFF: grasm lsp on examples/hello.gr ' "ONLY=lsp FILES=$WORK/hello" "$LSP" \
    '        s.push_str(" -> " + type_text(a, ret))' \
    '        s.push_str("->" + type_text(a, ret))'

# §6.23: the scheduler, the interpreter's stack and the IoFailed text.
TASKS=tools/tirinterp/src/tasks.gr
EVAL=tools/tirinterp/src/eval.gr
LOAD=tools/tirinterp/src/load.gr

control sched-seed '^DIFF: grasm test fuzz[0-9]+\.gr --schedules 20 \(schedules\)' "ONLY=schedules FILES=$WORK/hello" "$TASKS" \
    '    if sc.sched_on {' \
    '    if false and sc.sched_on {'

control task-panic-pass '^DIFF: grasm test scripts/fixtures/sched/panics\.gr \(schedules\)' "ONLY=schedules FILES=$WORK/hello" "$TASKS" \
    '    if ctl != 0 and ctl != eval.CTL_RETURN {' \
    '    if ctl != 0 and ctl != eval.CTL_RETURN and ctl != eval.CTL_PANIC {'

control stack-frame '^DIFF: grasm (run|test) scripts/fixtures/stack/count\.gr \(stack\)' ONLY=stack "$EVAL" \
    'pub let MAX_DEPTH = 100000' \
    'pub let MAX_DEPTH = 100001'

control task-stack '^DIFF: grasm (run|test) scripts/fixtures/stack/task-limit\.gr \(stack\)' ONLY=stack "$TASKS" \
    '    let nsp = spares(cx)' \
    '    let nsp = 0'

control recursion-seen '^DIFF: grasm (run|test) scripts/fixtures/stack/iface\.gr \(stack\)' ONLY=stack "$LOAD" \
    ' or o == OP_BOUNDCALL' \
    ''

control iofailed-old '^DIFF: grasm (run|test) scripts/fixtures/io/iofailed\.gr \(stack\)' ONLY=stack "$BUILTINS" \
    '[VStr(os_words(msg))]' \
    '[VStr(msg)]'

echo
echo "controls: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# `CONTROLS=a,b` runs those alone, for trying one; the floor holds only on
# a whole run.
if [ -n "${CONTROLS:-}" ]; then
    [ "$caught" = "$ran" ] && [ "$ran" -gt 0 ]
    exit
fi
[ "$ran" -ge 36 ] && [ "$caught" = "$ran" ]
