#!/usr/bin/env bash
# The `grasm` command in Grasm against the Rust one (docs/self-hosting.md
# §6.17 and §6.19, S8 parts one and two): the same stdout, stderr, exit
# status and files.
#
# `compiler/grasm` runs `version`, `tokens`, `ast`, `toml`, `check`, `tir`,
# `run`, `build --native`, `links`, `fmt`, `lint`, `test` and `bench` with the
# Grasm toolchain's own code, and reads every command line as the Rust command
# reads it. Ten sections, each both commands on the same input from the same
# directory -- six of part one:
#
#   lines   every command line in scripts/lib/clicases.txt, in an empty
#           directory of each command's own that links to the checkout's
#           entries, so the files a command writes are compared too; the
#           lines tagged there are the places §6.17 says the two differ, each
#           held to what the Grasm command must say instead
#   corpus  `tokens`, `ast`, `check`, `tir` and `links` of every corpus
#           program, the conformance negatives among them
#   toml    `toml` of every `.toml` file git tracks
#   run     `run` of every corpus program with a `main`, stdin closed; the
#           panic's stack trace is the one difference allowed
#           ([E-PANIC-TRACE]), and is stripped from the Rust side
#   build   `build --native --no-incremental` of every corpus program with a
#           `main` that links no system library: the command's output (the
#           size and the backend's name aside), then the two executables run
#           and compared as `linktest` compares them, then the Grasm build
#           made again under the same name, the same bytes
#   target  `build --native --target <triple>` of every tenth of those, for
#           x86_64-apple-darwin, x86_64-unknown-linux-gnu and
#           aarch64-unknown-linux-gnu: the same output, an object of the
#           triple's format (Mach-O, ELF) and machine, made twice the same
#
# and four for the subcommands of S8 part two (§6.19):
#
#   fmt     every `.gr` file git tracks and four rewrites of it
#           (scripts/lib/fmtmangle.py), each formatted in a directory of each
#           command's own: `fmt --check`, `fmt`, and the file left behind
#   lint    every `.gr` file git tracks, plain and `--json`, every tenth with
#           `--only` for each rule and for none
#   test    every corpus program, with a seed, a number of cases and a filter
#           where it has a property test, `--contracts` where it has a
#           contract, `--coverage` where it has a test and on every tenth;
#           then every program with a `main`, from a copy of the corpus,
#           with `test "main" { main() }` appended, so a test fails wherever
#           `run` panics or exits. A program with tasks or channels may end
#           on the host's panic where Rust reports the test failed -- the one
#           difference §6.19 allows (decision 3) -- and is counted as such
#   bench   every corpus program, each timing line with its time and count
#           taken out and held to the format (scripts/lib/benchnorm.py); one
#           with a benchmark filtered to its first and to none
#
# A section's floors hold only on a whole run.
#
# The Grasm command never starts another program: nothing in its package
# calls `proc_run` or `proc_pipeline` (counted here), and every command but
# `run` runs under a sandbox profile that lets it execute nothing but itself;
# `run`, whose programs may start others, may execute nothing from the Rust
# toolchain's directory. So a subcommand routed to the Rust `grasm` fails
# here even when its output would be the same.
#
# `CLI=<dir>` names a Grasm command to test, `grasm` with the runtime object
# `rt.o` beside it (a stage, docs/self-hosting.md §6.17); without it one is
# built here: by `$GRASM build --native`, and its `rt.o` by itself.
# `ONLY=<section>[,<section>]` runs those sections, `FILES=<list>` takes the
# corpus programs from a file; the floors hold only on a whole run.
# `CLI_SRC=<dir>` is the command's source to count `proc_run` in (a
# control's planted copy); `CLIDIFF_FAULT=nogrep` skips the count, for a
# control.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
. "$(dirname "$0")/lib/corpus.sh"
. "$(dirname "$0")/lib/trace.sh"
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
ONLY=${ONLY:-lines,corpus,toml,run,fmt,lint,test,bench,build,target}
want() { [[ ",$ONLY," == *",$1,"* ]]; }

if ! command -v sandbox-exec >/dev/null 2>&1; then
    echo "error: sandbox-exec is missing, so whether the Grasm command starts another program cannot be checked"
    exit 1
fi

# The Grasm command.
if [ -n "${CLI:-}" ]; then
    CLI=$(cd "$CLI" && pwd) || exit 1
else
    CLI="$WORK/cli"
    mkdir -p "$CLI"
    if ! "$GRASM" build compiler/grasm/src/main.gr --native --no-incremental -o "$CLI/grasm" >"$WORK/cli.log" 2>&1 ||
        ! "$CLI/grasm" build compiler/rt/src/main.gr --native -o "$CLI/rt.o" >>"$WORK/cli.log" 2>&1; then
        echo "error: compiler/grasm and its runtime object do not build:"
        sed 's/^/  /' "$WORK/cli.log"
        exit 1
    fi
fi
G="$CLI/grasm"
if [ ! -x "$G" ] || [ ! -f "$CLI/rt.o" ]; then
    echo "error: $CLI holds no grasm with rt.o beside it"
    exit 1
fi
G_REAL=$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$G")
RUST_DIR=$(dirname "$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$GRASM")")

# Nothing in the command's own package starts a process.
started=0
if [ "${CLIDIFF_FAULT:-}" != nogrep ]; then
    CLI_SRC=${CLI_SRC:-compiler/grasm/src}
    sources=$(find "$CLI_SRC" -name '*.gr' | grep -c .)
    started=$(cat "$CLI_SRC"/*.gr | grep -cE '\b(proc_run|proc_pipeline)\b')
    if [ "$sources" -lt 2 ]; then
        echo "error: $CLI_SRC holds $sources source files, which cannot be all of it"
        exit 1
    fi
fi

# The sandboxes: every command but `run` may execute itself alone; `run` may
# execute nothing in the Rust toolchain's directory. Both must be in effect.
NOEXEC="(version 1)(allow default)(deny process-exec)(allow process-exec (literal \"$G_REAL\"))"
NORUST="(version 1)(allow default)(deny process-exec (subpath \"$RUST_DIR\"))"
if sandbox-exec -p "$NOEXEC" /bin/echo >/dev/null 2>&1 ||
    sandbox-exec -p "$NORUST" "$GRASM" version >/dev/null 2>&1 ||
    ! sandbox-exec -p "$NOEXEC" "$G" version >/dev/null 2>&1; then
    echo "error: the sandbox is not in effect, so a command that started another program would go unseen"
    exit 1
fi
g() { sandbox-exec -p "$NOEXEC" "$G" "$@"; }
g_run() { sandbox-exec -p "$NORUST" "$G" "$@"; }

declare -a FAILED=()
fail() {
    FAILED+=("$1")
    echo "DIFF: $1"
}

# The `build --native` success line with the size and the backend taken out:
# the two commands' code is two compilers', so neither can be the other's.
# Only the backend the command is: a Grasm command that names Cranelift has
# been routed elsewhere, and its line stays as it is.
norm_built() { # file backend
    sed -E "s/^(built \`.*\` natively \\()[0-9]+ bytes, $2\\)\$/\\1N bytes)/" "$1"
}

# Every file and directory under `$1` but the links to the checkout and the
# Rust command's module cache, with whether each is executable.
listing() {
    (cd "$1" && find . -mindepth 1 ! -type l ! -path './.grasm-cache' ! -path './.grasm-cache/*' | sort | while IFS= read -r f; do
        if [ -d "$f" ]; then
            echo "d $f"
        elif [ -x "$f" ]; then
            echo "x $f"
        else
            echo "f $f"
        fi
    done)
}

# -------------------------------------------------------------------------
# lines
# -------------------------------------------------------------------------
lines=0
lines_same=0
lines_elsewhere=0
lines_runtime=0
lines_target=0
if want lines; then
    while IFS= read -r line; do
        case "$line" in
        '' | '#'*) continue ;;
        esac
        lines=$((lines + 1))
        tag=
        rest=$line
        case "$line" in
        'elsewhere '* | 'runtime '* | 'target '*)
            tag=${line%% *}
            rest=${line#* }
            ;;
        esac
        if [ "$rest" = "(no arguments)" ]; then
            args=()
        else
            eval "args=($rest)"
        fi
        for side in r g; do
            rm -rf "$WORK/l/$side"
            mkdir -p "$WORK/l/$side"
            for e in "$ROOT"/*; do
                ln -s "$e" "$WORK/l/$side/$(basename "$e")"
            done
        done
        (cd "$WORK/l/r" && $RUN_LIMIT "$GRASM" ${args[@]+"${args[@]}"} >"$WORK/l/r.out" 2>"$WORK/l/r.err" </dev/null)
        r_rc=$?
        (cd "$WORK/l/g" && $RUN_LIMIT sandbox-exec -p "$NOEXEC" "$G" ${args[@]+"${args[@]}"} >"$WORK/l/g.out" 2>"$WORK/l/g.err" </dev/null)
        g_rc=$?
        listing "$WORK/l/r" >"$WORK/l/r.files"
        listing "$WORK/l/g" >"$WORK/l/g.files"
        case "$tag" in
        '')
            if cmp -s <(norm_built "$WORK/l/r.out" Cranelift) <(norm_built "$WORK/l/g.out" Grasm) &&
                cmp -s "$WORK/l/r.err" "$WORK/l/g.err" && [ "$r_rc" = "$g_rc" ] &&
                cmp -s "$WORK/l/r.files" "$WORK/l/g.files"; then
                lines_same=$((lines_same + 1))
            else
                fail "grasm $rest -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/l/r.out" "$WORK/l/g.out" | head -6 | sed 's/^/  /'
                diff "$WORK/l/r.err" "$WORK/l/g.err" | head -6 | sed 's/^/  /'
                diff "$WORK/l/r.files" "$WORK/l/g.files" | head -6 | sed 's/^/  /'
            fi
            ;;
        elsewhere)
            first=$(head -1 "$WORK/l/g.err")
            if [ "$g_rc" = 1 ] && [ ! -s "$WORK/l/g.out" ] && [ ! -s "$WORK/l/g.files" ] &&
                [[ "$first" == "error: \`grasm "* ]] && [[ "$first" == *" is not in the Grasm toolchain yet" ]]; then
                lines_elsewhere=$((lines_elsewhere + 1))
            else
                fail "grasm $rest -- not said to be elsewhere (exit $g_rc): $first"
            fi
            ;;
        runtime)
            out=$(cat "$WORK/l/g.out")
            obj=${args[${#args[@]} - 1]}
            if [ "$g_rc" = 0 ] && [ "$r_rc" = 1 ] && [ "$(cat "$WORK/l/r.err")" = "error: no \`main\` function" ] &&
                [[ "$out" == "built \`$obj\`, the runtime \`rt\` ($(wc -c <"$WORK/l/g/$obj" 2>/dev/null | tr -d ' ') bytes, Grasm)" ]]; then
                lines_runtime=$((lines_runtime + 1))
            else
                fail "grasm $rest -- the runtime's object not built (exit $g_rc): $out $(head -1 "$WORK/l/g.err")"
            fi
            ;;
        target)
            if [ "$g_rc" = 1 ] && [ ! -s "$WORK/l/g.files" ] &&
                { [[ "$(cat "$WORK/l/g.err")" == "error: unsupported target \`"*"\`: the Grasm backend writes objects for x86_64 and aarch64 macOS and Linux (glibc) only" ]] ||
                    [[ "$(cat "$WORK/l/g.err")" == "error: unsupported target \`"*"-linux-gnu\` for a runtime package: its Linux object is built from a copy with \`os/linux.gr\` as \`src/os.gr\` (docs/self-hosting.md §6.18)" ]]; }; then
                lines_target=$((lines_target + 1))
            else
                fail "grasm $rest -- the target not refused (exit $g_rc): $(head -1 "$WORK/l/g.err")"
            fi
            ;;
        esac
    done <scripts/lib/clicases.txt
    echo "lines: $lines   the same: $lines_same   not here, as §6.17 says: $lines_elsewhere   the runtime built: $lines_runtime   a target refused: $lines_target"
fi

# The corpus programs, or the ones `FILES=` names.
if [ -n "${FILES:-}" ]; then
    cp "$FILES" "$WORK/programs"
else
    corpus_programs >"$WORK/programs"
fi
listed=$(grep -c . "$WORK/programs")

# -------------------------------------------------------------------------
# corpus
# -------------------------------------------------------------------------
corpus_runs=0
corpus_same=0
corpus_refused=0
if want corpus; then
    while IFS= read -r src; do
        for cmd in tokens ast check tir links; do
            corpus_runs=$((corpus_runs + 1))
            "$GRASM" "$cmd" "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            g "$cmd" "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err" && [ "$r_rc" = "$g_rc" ]; then
                corpus_same=$((corpus_same + 1))
                [ "$r_rc" != 0 ] && corpus_refused=$((corpus_refused + 1))
            else
                fail "grasm $cmd $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
            fi
        done
    done <"$WORK/programs"
    echo "corpus: $listed programs, $corpus_runs commands (tokens, ast, check, tir, links)   the same: $corpus_same, $corpus_refused of them refusals"
fi

# -------------------------------------------------------------------------
# toml
# -------------------------------------------------------------------------
tomls=0
toml_same=0
toml_refused=0
if want toml; then
    while IFS= read -r f; do
        tomls=$((tomls + 1))
        "$GRASM" toml "$f" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        g toml "$f" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err" && [ "$r_rc" = "$g_rc" ]; then
            toml_same=$((toml_same + 1))
            grep -c '^refused ' "$WORK/r.out" >/dev/null && toml_refused=$((toml_refused + 1))
        else
            fail "grasm toml $f -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
        fi
    done < <(git ls-files '*.toml')
    echo "toml: $tomls files   the same: $toml_same, $toml_refused of them refused by both"
fi

# The programs with a `main` that are not meant to be refused.
: >"$WORK/mains"
while IFS= read -r src; do
    grep -c '^//! expect: error$' "$src" >/dev/null && continue
    grep -cE '^[[:space:]]*(pub )?fn main\(' "$src" >/dev/null || continue
    echo "$src" >>"$WORK/mains"
done <"$WORK/programs"
mains=$(grep -c . "$WORK/mains")

# -------------------------------------------------------------------------
# run
# -------------------------------------------------------------------------
runs=0
run_same=0
run_timed_out=0
if want run; then
    while IFS= read -r src; do
        runs=$((runs + 1))
        $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        $RUN_LIMIT sandbox-exec -p "$NORUST" "$G" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if [ "$r_rc" = 124 ] || [ "$g_rc" = 124 ]; then
            run_timed_out=$((run_timed_out + 1))
            fail "grasm run $src -- timed out after ${RUN_LIMIT_SECONDS}s (Rust $r_rc, Grasm $g_rc)"
            continue
        fi
        if cmp -s "$WORK/r.out" "$WORK/g.out" && [ "$r_rc" = "$g_rc" ] &&
            [ "$(strip_trace "$(cat "$WORK/r.err")")" == "$(cat "$WORK/g.err")" ]; then
            run_same=$((run_same + 1))
        else
            fail "grasm run $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
            diff <(strip_trace "$(cat "$WORK/r.err")") "$WORK/g.err" | head -4 | sed 's/^/  /'
        fi
    done <"$WORK/mains"
    echo "run: $runs programs with a main   the same: $run_same   timed out: $run_timed_out"
fi

# -------------------------------------------------------------------------
# fmt, lint, test, bench (docs/self-hosting.md §6.19)
# -------------------------------------------------------------------------

# Whether the last two runs, $WORK/r.* and $WORK/g.* with the statuses
# given, are the same bytes.
same3() { # r_rc g_rc
    [ "$1" = "$2" ] && cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err"
}
show3() {
    diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
    diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
}

# The `.gr` files git tracks, or the programs `FILES=` names.
tracked_gr() {
    if [ -n "${FILES:-}" ]; then
        cat "$WORK/programs"
    else
        git ls-files '*.gr'
    fi
}

# fmt: every `.gr` file git tracks and four rewrites of it
# (scripts/lib/fmtmangle.py), each copied into a directory of each
# command's own as `x.gr`: `fmt --check`, then `fmt`, then the file left.
fmt_files=0
fmt_runs=0
fmt_same=0
fmt_rewritten=0
fmt_refused=0
if want fmt; then
    while IFS= read -r f; do
        fmt_files=$((fmt_files + 1))
        rm -rf "$WORK/fm"
        python3 scripts/lib/fmtmangle.py "$f" "$WORK/fm" || { fail "fmtmangle could not rewrite $f"; continue; }
        cp "$f" "$WORK/fm/orig.gr"
        for v in orig dedent spaced tight crlf; do
            for side in r g; do
                rm -rf "$WORK/f$side"
                mkdir -p "$WORK/f$side"
                cp "$WORK/fm/$v.gr" "$WORK/f$side/x.gr"
            done
            ok=1
            for step in check write; do
                fmt_runs=$((fmt_runs + 1))
                if [ "$step" = check ]; then set -- fmt --check x.gr; else set -- fmt x.gr; fi
                (cd "$WORK/fr" && "$GRASM" "$@" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null)
                r_rc=$?
                (cd "$WORK/fg" && g "$@" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null)
                g_rc=$?
                if same3 "$r_rc" "$g_rc"; then
                    fmt_same=$((fmt_same + 1))
                else
                    ok=0
                    fail "grasm $* -- $f ($v) -- exit $r_rc (Rust) and $g_rc (Grasm)"
                    show3
                fi
            done
            if ! cmp -s "$WORK/fr/x.gr" "$WORK/fg/x.gr"; then
                fail "grasm fmt -- $f ($v) -- the files left differ"
                diff "$WORK/fr/x.gr" "$WORK/fg/x.gr" | head -4 | sed 's/^/  /'
            elif [ "$ok" = 1 ]; then
                grep -c '^formatted `x.gr`$' "$WORK/r.out" >/dev/null && fmt_rewritten=$((fmt_rewritten + 1))
                grep -c 'does not parse' "$WORK/r.err" >/dev/null && fmt_refused=$((fmt_refused + 1))
            fi
        done
    done < <(tracked_gr)
    echo "fmt: $fmt_files files, each and four rewrites, $fmt_runs commands   the same: $fmt_same   rewritten alike: $fmt_rewritten   refused alike: $fmt_refused"
fi

# lint: every `.gr` file git tracks, plain and `--json`; every tenth with
# `--only` for each rule and one that is none.
lint_files=0
lint_runs=0
lint_same=0
lint_findings=0
lint_only=0
if want lint; then
    k=0
    while IFS= read -r f; do
        lint_files=$((lint_files + 1))
        k=$((k + 1))
        lines_of=("$f" "$f --json")
        if [ $((k % 10)) = 0 ]; then
            for rule in unused-binding unused-parameter unused-import shadowing long-function shift-count no-such-rule; do
                lines_of+=("$f --only $rule")
            done
            lint_only=$((lint_only + 7))
        fi
        for l in "${lines_of[@]}"; do
            lint_runs=$((lint_runs + 1))
            # shellcheck disable=SC2086
            "$GRASM" lint $l >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            # shellcheck disable=SC2086
            g lint $l >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if same3 "$r_rc" "$g_rc"; then
                lint_same=$((lint_same + 1))
                if [ "$l" = "$f" ]; then
                    n=$(sed -n 's/^findings: //p' "$WORK/r.out")
                    lint_findings=$((lint_findings + ${n:-0}))
                fi
            else
                fail "grasm lint $l -- exit $r_rc (Rust) and $g_rc (Grasm)"
                show3
            fi
        done
    done < <(tracked_gr)
    echo "lint: $lint_files files, $lint_runs commands ($lint_only with --only)   the same: $lint_same   findings: $lint_findings"
fi

# A difference §6.19 allows (decision 3): the Grasm command ended by the
# host's panic where the Rust runner reported the test failed -- only in a
# program that has tasks or channels, and with everything before it the
# same.
host_panic() { # src
    local m
    [ "$g_rc" = 101 ] || return 1
    [ "$(grep -c . "$WORK/g.err")" = 1 ] || return 1
    m=$(sed -n 's/^panic: //p' "$WORK/g.err")
    [ -n "$m" ] || return 1
    grep -F -- "... FAILED: $m" "$WORK/r.out" >/dev/null || return 1
    cmp -s "$WORK/g.out" <(head -c "$(wc -c <"$WORK/g.out")" "$WORK/r.out") || return 1
    grep -lE '\bconcurrent\b|\bchan_' "$1" >/dev/null
}

# test: every corpus program; with a property test, again with a seed, a
# number of cases and a filter; with a contract, `--contracts`; with a test,
# and every tenth, `--coverage`. Then every program with a `main`, copied
# with `test "main" { main() }` appended, from a copy of the corpus.
test_runs=0
test_same=0
test_failed=0
test_props=0
test_cov=0
test_contracts=0
test_timed_out=0
test_host=0
tm_runs=0
tm_same=0
tm_failed=0
tm_host=0
if want test; then
    k=0
    while IFS= read -r src; do
        k=$((k + 1))
        lines_of=("$src")
        if grep -cE '^[[:space:]]*test "[^"]*" *\(' "$src" >/dev/null; then
            lines_of+=("$src --seed 7 --cases 37" "--filter a $src")
            test_props=$((test_props + 1))
        fi
        if grep -cE '^[[:space:]]*(requires|ensures) ' "$src" >/dev/null; then
            lines_of+=("--contracts $src")
            test_contracts=$((test_contracts + 1))
        fi
        if [ $((k % 10)) = 0 ] || grep -cE '^[[:space:]]*test "' "$src" >/dev/null; then
            lines_of+=("--coverage $src")
            test_cov=$((test_cov + 1))
        fi
        for l in "${lines_of[@]}"; do
            test_runs=$((test_runs + 1))
            # shellcheck disable=SC2086
            $RUN_LIMIT "$GRASM" test $l >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            # shellcheck disable=SC2086
            $RUN_LIMIT sandbox-exec -p "$NORUST" "$G" test $l >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if [ "$r_rc" = 124 ] && [ "$g_rc" = 124 ] && [[ "$l" == --contracts* ]]; then
                # `T-CONTRACT-ENSURES`'s `good_sqrt` counts to the root of
                # `i64::MAX` for one input, under both (§6.19).
                test_timed_out=$((test_timed_out + 1))
            elif same3 "$r_rc" "$g_rc"; then
                test_same=$((test_same + 1))
                [ "$r_rc" = 1 ] && grep -c ' FAILED' "$WORK/r.out" >/dev/null && test_failed=$((test_failed + 1))
            elif host_panic "$src"; then
                test_host=$((test_host + 1))
                echo "  the host's panic, as §6.19 allows: grasm test $l"
            else
                fail "grasm test $l -- exit $r_rc (Rust) and $g_rc (Grasm)"
                show3
            fi
        done
    done <"$WORK/programs"
    echo "test: $listed programs, $test_runs commands ($test_props with a property test, $test_contracts with a contract, $test_cov with --coverage)   the same: $test_same, $test_failed of them with a failing test   the host's panic: $test_host   both timed out, --contracts: $test_timed_out"

    mkdir -p "$WORK/tm"
    for d in $CORPUS_DIRS; do
        mkdir -p "$WORK/tm/$(dirname "$d")"
        cp -R "$d" "$WORK/tm/$d"
    done
    while IFS= read -r src; do
        printf '\ntest "main" {\n    main()\n}\n' >>"$WORK/tm/$src"
        tm_runs=$((tm_runs + 1))
        (cd "$WORK/tm" && $RUN_LIMIT "$GRASM" test "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null)
        r_rc=$?
        (cd "$WORK/tm" && $RUN_LIMIT sandbox-exec -p "$NORUST" "$G" test "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null)
        g_rc=$?
        if same3 "$r_rc" "$g_rc"; then
            tm_same=$((tm_same + 1))
            grep -c '^test "main" \.\.\. FAILED' "$WORK/r.out" >/dev/null && tm_failed=$((tm_failed + 1))
        elif host_panic "$WORK/tm/$src"; then
            tm_host=$((tm_host + 1))
            echo "  the host's panic, as §6.19 allows: grasm test $src, main as a test"
        else
            fail "grasm test $src, main as a test -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
        fi
    done <"$WORK/mains"
    rm -rf "$WORK/tm"
    echo "test, main as a test: $tm_runs programs   the same: $tm_same, $tm_failed of them failing   the host's panic: $tm_host"
fi

# bench: every corpus program, its timing lines compared with the time and
# the count taken out and each held to the format (scripts/lib/benchnorm.py);
# a program with a benchmark again filtered to its first and to none.
bench_runs=0
bench_same=0
bench_timed=0
bench_files=0
if want bench; then
    while IFS= read -r src; do
        lines_of=("$src")
        first=$(sed -nE 's/^[[:space:]]*bench "([^"]*)".*/\1/p' "$src" | head -1)
        if [ -n "$first" ]; then
            bench_files=$((bench_files + 1))
            lines_of+=("$src --filter $first" "$src --filter no-such-benchmark")
        fi
        for l in "${lines_of[@]}"; do
            bench_runs=$((bench_runs + 1))
            # shellcheck disable=SC2086
            $RUN_LIMIT "$GRASM" bench $l >"$WORK/r.raw" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            # shellcheck disable=SC2086
            $RUN_LIMIT sandbox-exec -p "$NORUST" "$G" bench $l >"$WORK/g.raw" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            python3 scripts/lib/benchnorm.py "$WORK/r.raw" >"$WORK/r.out" 2>/dev/null
            if ! python3 scripts/lib/benchnorm.py "$WORK/g.raw" >"$WORK/g.out" 2>"$WORK/g.bad"; then
                fail "grasm bench $l -- a timing line not in the format: $(head -1 "$WORK/g.bad")"
                continue
            fi
            if same3 "$r_rc" "$g_rc"; then
                bench_same=$((bench_same + 1))
                bench_timed=$((bench_timed + $(grep -c ' iterations)$' "$WORK/g.raw")))
            else
                fail "grasm bench $l -- exit $r_rc (Rust) and $g_rc (Grasm)"
                show3
            fi
        done
    done <"$WORK/programs"
    echo "bench: $listed programs, $bench_runs commands ($bench_files with a benchmark, each filtered twice)   the same: $bench_same   timing lines in the format: $bench_timed"
fi

# -------------------------------------------------------------------------
# build, target
# -------------------------------------------------------------------------
builds=0
built_same=0
build_refused=0
ran_same=0
repro=0
external=0
targets=0
target_same=0
if want build || want target; then
    mkdir -p "$WORK/b/r" "$WORK/b/g" "$WORK/b/g2" "$WORK/t/r" "$WORK/t/g" "$WORK/t/g2"
    k=0
    while IFS= read -r src; do
        if corpus_external "$src"; then
            external=$((external + 1))
            continue
        fi
        k=$((k + 1))
        if want build; then
            builds=$((builds + 1))
            rm -f "$WORK/b/r/prog" "$WORK/b/g/prog" "$WORK/b/g2/prog"
            "$GRASM" build "$src" --native --no-incremental -o "$WORK/b/r/prog" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            g build "$src" --native --no-incremental -o "$WORK/b/g/prog" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            size=$(wc -c <"$WORK/b/g/prog" 2>/dev/null | tr -d ' ')
            # The two outputs' paths differ in their directory alone.
            sed "s|$WORK/b/r/|$WORK/b/g/|g" "$WORK/r.out" >"$WORK/r.norm"
            sed "s|$WORK/b/r/|$WORK/b/g/|g" "$WORK/r.err" >"$WORK/r.enorm"
            if ! cmp -s <(norm_built "$WORK/r.norm" Cranelift) <(norm_built "$WORK/g.out" Grasm) ||
                ! cmp -s "$WORK/r.enorm" "$WORK/g.err" || [ "$r_rc" != "$g_rc" ] ||
                { [ "$g_rc" = 0 ] && [ "$(cat "$WORK/g.out")" != "built \`$WORK/b/g/prog\` natively ($size bytes, Grasm)" ]; }; then
                fail "grasm build --native $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
                continue
            fi
            built_same=$((built_same + 1))
            if [ "$r_rc" != 0 ]; then
                build_refused=$((build_refused + 1))
                continue
            fi
            $RUN_LIMIT "$WORK/b/r/prog" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            $RUN_LIMIT "$WORK/b/g/prog" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err" && [ "$r_rc" = "$g_rc" ]; then
                ran_same=$((ran_same + 1))
            else
                fail "$src built by each runs otherwise -- exit $r_rc (Rust's build) and $g_rc (Grasm's)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
            fi
            if g build "$src" --native --no-incremental -o "$WORK/b/g2/prog" >/dev/null 2>&1 </dev/null &&
                cmp -s "$WORK/b/g/prog" "$WORK/b/g2/prog"; then
                repro=$((repro + 1))
            else
                fail "grasm build --native $src -- built twice, two executables"
            fi
        fi
        if want target && [ $((k % 10)) = 1 ]; then
            targets=$((targets + 1))
            # Each triple with the start of the object it must write: the
            # format's magic, then its class and machine (Mach-O's cputype;
            # ELF's class, data and version, and e_machine at offset 18).
            for tt in x86_64-apple-darwin:cffaedfe07000001: x86_64-unknown-linux-gnu:7f454c4602010100:3e00 \
                aarch64-unknown-linux-gnu:7f454c4602010100:b700; do
                IFS=: read -r triple want_header want_machine <<<"$tt"
                rm -f "$WORK/t/r/p.o" "$WORK/t/g/p.o" "$WORK/t/g2/p.o"
                "$GRASM" build "$src" --native --target "$triple" -o "$WORK/t/r/p.o" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
                r_rc=$?
                g build "$src" --native --target "$triple" -o "$WORK/t/g/p.o" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
                g_rc=$?
                g build "$src" --native --target "$triple" -o "$WORK/t/g2/p.o" >/dev/null 2>&1 </dev/null
                sed "s|$WORK/t/r/|$WORK/t/g/|g" "$WORK/r.out" >"$WORK/r.norm"
                header=$(head -c 8 "$WORK/t/g/p.o" 2>/dev/null | od -An -tx1 | tr -d ' \n')
                machine=$(dd if="$WORK/t/g/p.o" bs=1 skip=18 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')
                if cmp -s "$WORK/r.norm" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err" && [ "$r_rc" = "$g_rc" ] &&
                    { [ "$g_rc" != 0 ] || { [ "$header" = "$want_header" ] && { [ -z "$want_machine" ] || [ "$machine" = "$want_machine" ]; } &&
                        cmp -s "$WORK/t/g/p.o" "$WORK/t/g2/p.o"; }; }; then
                    target_same=$((target_same + 1))
                else
                    fail "grasm build --native --target $triple $src -- exit $r_rc (Rust) and $g_rc (Grasm), object header $header, machine $machine"
                    diff "$WORK/r.norm" "$WORK/g.out" | head -4 | sed 's/^/  /'
                fi
            done
        fi
    done <"$WORK/mains"
    if want build; then
        echo "build: $builds programs with a main ($external link a system library)   the same output: $built_same, $build_refused of them refusals   run alike: $ran_same   built twice the same: $repro"
    fi
    if want target; then
        echo "target: $targets programs, each for x86_64-apple-darwin, x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu   the same output, the triple's object made twice the same: $target_same of $((3 * targets))"
    fi
fi

echo "the Grasm command's package calls proc_run or proc_pipeline: $started time(s)"
if [ "$started" != 0 ]; then
    FAILED+=("compiler/grasm calls proc_run or proc_pipeline $started time(s): the command must start no process")
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED: ${#FAILED[@]}"
    printf '  %s\n' "${FAILED[@]}" | head -60
    exit 1
fi

# The counts add up, and a whole run compared enough to mean something.
whole=$([ -z "${FILES:-}" ] && [ "$ONLY" = lines,corpus,toml,run,fmt,lint,test,bench,build,target ] && echo 1)
if want lines && [ $((lines_same + lines_elsewhere + lines_runtime + lines_target)) -ne "$lines" ]; then
    echo "lines: the counts do not add up to $lines"
    exit 1
fi
if want lines && { [ "$lines" -lt 140 ] || [ "$lines_elsewhere" -lt 10 ] || [ "$lines_runtime" != 1 ] || [ "$lines_target" != 4 ]; }; then
    echo "lines: too few compared ($lines, $lines_elsewhere not here, $lines_runtime runtime, $lines_target target)"
    exit 1
fi
if want corpus && [ "$corpus_same" -ne $((5 * listed)) ]; then
    echo "corpus: $corpus_same of $((5 * listed)) commands compared"
    exit 1
fi
if want run && [ "$run_same" -ne "$mains" ]; then
    echo "run: $run_same of $mains programs compared"
    exit 1
fi
if want fmt && { [ "$fmt_same" -ne "$fmt_runs" ] || [ "$fmt_runs" -ne $((10 * fmt_files)) ]; }; then
    echo "fmt: the counts do not add up ($fmt_same of $fmt_runs, $fmt_files files)"
    exit 1
fi
if want lint && { [ "$lint_same" -ne "$lint_runs" ] || [ "$lint_runs" -ne $((2 * lint_files + lint_only)) ]; }; then
    echo "lint: the counts do not add up ($lint_same of $lint_runs, $lint_files files, $lint_only with --only)"
    exit 1
fi
if want test && { [ $((test_same + test_host + test_timed_out)) -ne "$test_runs" ] || [ $((tm_same + tm_host)) -ne "$tm_runs" ] || [ "$tm_runs" -ne "$mains" ]; }; then
    echo "test: the counts do not add up ($test_same + $test_host + $test_timed_out of $test_runs; $tm_same + $tm_host of $tm_runs, $mains with a main)"
    exit 1
fi
if want bench && [ "$bench_same" -ne "$bench_runs" ]; then
    echo "bench: $bench_same of $bench_runs commands compared"
    exit 1
fi
if want build && { [ "$built_same" -ne "$builds" ] || [ "$ran_same" -ne $((builds - build_refused)) ] || [ "$repro" -ne "$ran_same" ] ||
    [ $((builds + external)) -ne "$mains" ]; }; then
    echo "build: the counts do not add up ($built_same, $ran_same, $repro of $builds; $external external of $mains)"
    exit 1
fi
if [ -n "$whole" ]; then
    if [ "$listed" -lt 1700 ] || [ "$corpus_refused" -lt 120 ] || [ "$tomls" -lt 600 ] ||
        [ "$run_same" -lt 640 ] || [ "$ran_same" -lt 640 ] || [ "$targets" -lt 60 ] || [ "$target_same" -ne $((3 * targets)) ]; then
        echo "too little compared for this to mean anything: $listed programs, $corpus_refused refusals, $tomls manifests, $run_same runs, $ran_same builds run, $target_same of $targets targets"
        exit 1
    fi
    # The floors of §6.19's sections, below what the corpus holds on
    # 2026-11-05: 1,737 programs, 43 with a test, 6 with a property test, 3
    # with a contract, 3 with a benchmark; about 2,000 `.gr` files tracked.
    if [ "$fmt_files" -lt 1900 ] || [ "$fmt_rewritten" -lt 1900 ] || [ "$fmt_refused" -lt 1 ] ||
        [ "$lint_files" -lt 1900 ] || [ "$lint_findings" -lt 100 ] ||
        [ "$test_props" -lt 6 ] || [ "$test_contracts" -lt 3 ] || [ "$test_cov" -lt 200 ] || [ "$test_failed" -lt 3 ] ||
        [ "$tm_runs" -lt 640 ] || [ "$tm_failed" -lt 10 ] || [ "$tm_host" -gt 10 ] ||
        [ "$bench_files" -lt 3 ] || [ "$bench_timed" -lt 3 ]; then
        echo "too little compared in fmt, lint, test or bench: $fmt_files files, $fmt_rewritten rewritten, $fmt_refused refused; $lint_files linted, $lint_findings findings; $test_props, $test_contracts, $test_cov, $test_failed failing; $tm_runs mains as tests, $tm_failed failing, $tm_host the host's; $bench_files with a benchmark, $bench_timed timing lines"
        exit 1
    fi
fi
exit 0
