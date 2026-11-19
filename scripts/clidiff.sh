#!/usr/bin/env bash
# The `grasm` command in Grasm against the Rust one (docs/self-hosting.md
# §6.17, §6.19, §6.20 and §6.21, S8 parts one to four): the same stdout,
# stderr, exit status and files.
#
# `compiler/grasm` runs `version`, `tokens`, `ast`, `toml`, `check`, `tir`,
# `run`, `build`, `links`, `fmt`, `lint`, `test`, `bench`, `doc`, `api`,
# `deps`, `add` and `publish` with the Grasm toolchain's own code, and reads
# every command line as the Rust command reads it. Eighteen sections, each
# both commands on the same input from the same directory -- six of part
# one:
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
# and four for the subcommands of S8 part three (§6.20):
#
#   doc     every corpus program, `doc P`; every tenth and every package's
#           entry also `doc P -o F`, the file compared
#   api     every corpus program, `api P`; where it has a surface, `api P
#           --against` that surface and four baselines rewritten from it (CR
#           LF, no tabs, empty, not UTF-8); then, in a copy of the corpus, an
#           added, a removed and a changed public item (`api_probe`
#           appended), each against a baseline the Rust command made
#   bundle  every program with a `main`, `build P -o prog` without
#           `--native`: the output (the size aside), the Grasm bundle the
#           command's own executable followed by exactly the bytes that follow
#           the Rust command's executable in the Rust bundle, both bundles
#           run from the checkout with no argument and from empty
#           directories with arguments (the Rust trace stripped, the files
#           they leave compared; the `IoFailed` wording of
#           docs/known-issues.md counted), the Grasm one made twice the same
#   source  every corpus program, `build P --source -o F`: the output and
#           the file; every tenth with a `main` also run from that file by
#           both commands
#
# and four for the package commands of S8 part four (§6.21), each side in a
# copy of its own, the copies' paths taken out:
#
#   registry  the package graphs and registries scripts/lib/regcases.py
#             writes, its `deps`, `add` and `publish` lines in order, with
#             lockfile edits (scripts/lib/lockedit.py) between them: the
#             output, and every path, mode and byte of the case after each
#   deps      every package git tracks a manifest of, in a copy of the
#             checkout: `deps --locked`, `deps`, `deps --locked`, the
#             lockfile compared after each
#   add       every package: a registry dependency added, the manifest
#             compared
#   publish   every package, into a registry of each side's own: the output
#             and the registry, every path, mode and byte
#
# A registry is a directory: nothing here reaches the network.
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
# corpus programs from a file, `PACKAGES=<list>` the package directories of
# `deps`, `add` and `publish`; the floors hold only on a whole run.
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
ALL=lines,corpus,toml,run,fmt,lint,test,bench,doc,api,bundle,source,build,target,registry,deps,add,publish,journal,dap,lsp
ONLY=${ONLY:-$ALL}
want() { [[ ",$ONLY," == *",$1,"* ]]; }

if ! command -v sandbox-exec >/dev/null 2>&1; then
    echo "error: sandbox-exec is missing, so whether the Grasm command starts another program cannot be checked"
    exit 1
fi

# The Grasm command.
CLI_GIVEN=${CLI:-}
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
# The command the `journal` section records and replays with: one whose
# runtime is the Grasm one, which leaves the command's own reads out of the
# journal (`unjournaled_host`, docs/self-hosting.md §6.22, decision (2)).
# The Rust runtime cannot, so a command built here by the Rust `grasm` is
# built again for this section as `scripts/rtswap.sh` builds a program on
# the Grasm runtime: the Cranelift object, linked by `compiler/link` with
# `rt.o` interposed. A `CLI=` command (a stage, built by the Grasm one) is
# on the Grasm runtime already, unless `CLI_JOURNAL_BUILD=1` says to build
# one so from `CLI_SRC` (the controls' planted copies);
# `CLI_JOURNAL=<file>` names one.
GJ=${CLI_JOURNAL:-}
if want journal && [ -z "$GJ" ]; then
    if [ -n "$CLI_GIVEN" ] && [ -z "${CLI_JOURNAL_BUILD:-}" ]; then
        GJ=$G
    else
        GJ="$WORK/gj/grasm"
        mkdir -p "$WORK/gj"
        if ! "$GRASM" build "${CLI_SRC:-compiler/grasm/src}/main.gr" --native --no-incremental --target aarch64-apple-darwin -o "$WORK/gj/grasm.o" >"$WORK/gj.log" 2>&1 ||
            ! "$GRASM" build compiler/link/src/main.gr --native -o "$WORK/gj/link" >>"$WORK/gj.log" 2>&1 ||
            ! "$WORK/gj/link" -interpose "$CLI/rt.o" "$WORK/gj/grasm.o" -dead_strip -exported_symbol _unjournaled_host -o "$GJ" >>"$WORK/gj.log" 2>&1; then
            echo "error: the command does not build on the Grasm runtime:"
            sed 's/^/  /' "$WORK/gj.log"
            exit 1
        fi
    fi
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
# been routed elsewhere, and its line stays as it is. And a bundle's line
# with its size taken out: each bundle starts with its own command.
norm_built() { # file backend
    sed -E -e "s/^(built \`.*\` natively \\()[0-9]+ bytes, $2\\)\$/\\1N bytes)/" \
        -e "s/^(built \`.*\` \\()[0-9]+ bytes, ([0-9]+ module\\(s\\))\\)\$/\\1N bytes, \\2)/" "$1"
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
            # `--contracts` runs every generated input to the end, and one
            # program's runs for hours under both commands: two minutes is
            # ample for the others (each takes under a second).
            limit=$RUN_LIMIT
            [[ "$l" == --contracts* ]] && [ -n "$RUN_LIMIT" ] && limit="timeout 120"
            # shellcheck disable=SC2086
            $limit "$GRASM" test $l >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            # shellcheck disable=SC2086
            $limit sandbox-exec -p "$NORUST" "$G" test $l >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
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
# doc, api, bundle, source (docs/self-hosting.md §6.20)
# -------------------------------------------------------------------------

# doc: every corpus program, `doc P`; every tenth and every package's
# entry also with `-o`, into a directory of each command's own.
doc_runs=0
doc_same=0
doc_refused=0
doc_written=0
doc_items=0
if want doc; then
    mkdir -p "$WORK/d/r" "$WORK/d/g"
    k=0
    while IFS= read -r src; do
        k=$((k + 1))
        doc_runs=$((doc_runs + 1))
        "$GRASM" doc "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        g doc "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if same3 "$r_rc" "$g_rc"; then
            doc_same=$((doc_same + 1))
            [ "$r_rc" != 0 ] && doc_refused=$((doc_refused + 1))
        else
            fail "grasm doc $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
        fi
        if [ $((k % 10)) = 0 ] || [[ "$src" == packages/*/src/main.gr ]]; then
            doc_runs=$((doc_runs + 1))
            rm -f "$WORK/d/r/out.md" "$WORK/d/g/out.md"
            "$GRASM" doc "$src" -o "$WORK/d/r/out.md" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            g doc "$src" -o "$WORK/d/g/out.md" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            sed -i '' "s|$WORK/d/r/|$WORK/d/g/|g" "$WORK/r.out" "$WORK/r.err"
            if same3 "$r_rc" "$g_rc" &&
                { { [ ! -e "$WORK/d/r/out.md" ] && [ ! -e "$WORK/d/g/out.md" ]; } || cmp -s "$WORK/d/r/out.md" "$WORK/d/g/out.md"; } &&
                { [ "$g_rc" != 0 ] || [ -s "$WORK/d/g/out.md" ]; }; then
                doc_same=$((doc_same + 1))
                if [ "$r_rc" = 0 ]; then
                    doc_written=$((doc_written + 1))
                    n=$(sed -nE 's/^wrote .* \(([0-9]+) item\(s\)\)$/\1/p' "$WORK/g.out")
                    doc_items=$((doc_items + ${n:-0}))
                else
                    doc_refused=$((doc_refused + 1))
                fi
            else
                fail "grasm doc $src -o F -- exit $r_rc (Rust) and $g_rc (Grasm)"
                show3
                diff "$WORK/d/r/out.md" "$WORK/d/g/out.md" | head -4 | sed 's/^/  /'
            fi
        fi
    done <"$WORK/programs"
    echo "doc: $listed programs, $doc_runs commands ($doc_written written with -o, $doc_items items)   the same: $doc_same, $doc_refused of them refusals"
fi

# api: every corpus program; where it has a surface, against that surface
# and four baselines rewritten from it; then an added, a removed and a
# changed public item in a copy of the corpus.
api_runs=0
api_same=0
api_refused=0
api_surfaces=0
api_patch=0
api_baselines=0
api_added=0
api_removed=0
api_changed=0
if want api; then
    : >"$WORK/with_surface"
    while IFS= read -r src; do
        api_runs=$((api_runs + 1))
        "$GRASM" api "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        g api "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if ! same3 "$r_rc" "$g_rc"; then
            fail "grasm api $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
            continue
        fi
        api_same=$((api_same + 1))
        if [ "$r_rc" != 0 ]; then
            api_refused=$((api_refused + 1))
            continue
        fi
        [ -s "$WORK/r.out" ] || continue
        api_surfaces=$((api_surfaces + 1))
        echo "$src" >>"$WORK/with_surface"
        cp "$WORK/r.out" "$WORK/base.api"
        # The surface itself, then CR LF line ends, every tab a space, no
        # line at all, and a byte that is not UTF-8.
        sed 's/$/\r/' "$WORK/base.api" >"$WORK/crlf.api"
        tr '\t' ' ' <"$WORK/base.api" >"$WORK/notabs.api"
        : >"$WORK/empty.api"
        { printf '\377'; cat "$WORK/base.api"; } >"$WORK/binary.api"
        for b in base crlf notabs empty binary; do
            api_runs=$((api_runs + 1))
            "$GRASM" api "$src" --against "$WORK/$b.api" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            g api "$src" --against "$WORK/$b.api" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if same3 "$r_rc" "$g_rc"; then
                api_same=$((api_same + 1))
                if [ "$b" = base ]; then
                    grep -c 'this is a PATCH release$' "$WORK/r.out" >/dev/null && api_patch=$((api_patch + 1))
                else
                    api_baselines=$((api_baselines + 1))
                fi
            else
                fail "grasm api $src --against ($b) -- exit $r_rc (Rust) and $g_rc (Grasm)"
                show3
            fi
        done
    done <"$WORK/programs"

    mkdir -p "$WORK/ac"
    for d in $CORPUS_DIRS; do
        mkdir -p "$WORK/ac/$(dirname "$d")"
        cp -R "$d" "$WORK/ac/$d"
    done
    # api_against SRC BASELINE KIND: both commands, from the copy.
    api_against() {
        api_runs=$((api_runs + 1))
        (cd "$WORK/ac" && "$GRASM" api "$1" --against "$2" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null)
        r_rc=$?
        (cd "$WORK/ac" && g api "$1" --against "$2" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null)
        g_rc=$?
        if same3 "$r_rc" "$g_rc"; then
            api_same=$((api_same + 1))
            case "$3" in
            added) grep -c '^ADDED    .*::api_probe$' "$WORK/r.out" >/dev/null && api_added=$((api_added + 1)) ;;
            removed) grep -c '^REMOVED  .*::api_probe$' "$WORK/r.out" >/dev/null && api_removed=$((api_removed + 1)) ;;
            changed) grep -c '^CHANGED  .*::api_probe$' "$WORK/r.out" >/dev/null && api_changed=$((api_changed + 1)) ;;
            esac
        else
            fail "grasm api $1 --against, an item $3 -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
        fi
    }
    while IFS= read -r src; do
        f="$WORK/ac/$src"
        cp "$f" "$WORK/orig.gr"
        (cd "$WORK/ac" && "$GRASM" api "$src" >"$WORK/b0.api" 2>/dev/null </dev/null)
        printf '\npub fn api_probe() -> i64 {\n    1\n}\n' >>"$f"
        (cd "$WORK/ac" && "$GRASM" api "$src" >"$WORK/b1.api" 2>/dev/null </dev/null)
        api_against "$src" "$WORK/b0.api" added
        cp "$WORK/orig.gr" "$f"
        api_against "$src" "$WORK/b1.api" removed
        printf '\npub fn api_probe(n: i64) -> i64 {\n    n\n}\n' >>"$f"
        api_against "$src" "$WORK/b1.api" changed
        cp "$WORK/orig.gr" "$f"
    done <"$WORK/with_surface"
    rm -rf "$WORK/ac"
    echo "api: $listed programs, $api_runs commands ($api_surfaces with a surface)   the same: $api_same, $api_refused of them refusals   against its own surface, a patch: $api_patch   against a rewritten baseline: $api_baselines   an item added: $api_added, removed: $api_removed, changed: $api_changed"
fi

# bundle: every program with a `main`, built without `--native`; the Grasm
# bundle is the command followed by the Rust bundle's payload, byte for
# byte; both run; the Grasm one made twice.
bundles=0
bundle_same=0
bundle_refused=0
bundle_payload=0
bundle_ran=0
bundle_iofailed=0
bundle_repro=0
if want bundle; then
    mkdir -p "$WORK/u/r" "$WORK/u/g" "$WORK/u/g2"
    r_exe=$(wc -c <"$GRASM" | tr -d ' ')
    g_exe=$(wc -c <"$G" | tr -d ' ')
    while IFS= read -r src; do
        bundles=$((bundles + 1))
        rm -f "$WORK/u/r/prog" "$WORK/u/g/prog" "$WORK/u/g2/prog"
        "$GRASM" build "$src" -o "$WORK/u/r/prog" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        g build "$src" -o "$WORK/u/g/prog" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        sed "s|$WORK/u/r/|$WORK/u/g/|g" "$WORK/r.out" >"$WORK/r.norm"
        sed "s|$WORK/u/r/|$WORK/u/g/|g" "$WORK/r.err" >"$WORK/r.enorm"
        size=$(wc -c <"$WORK/u/g/prog" 2>/dev/null | tr -d ' ')
        if ! cmp -s <(norm_built "$WORK/r.norm" Cranelift) <(norm_built "$WORK/g.out" Grasm) ||
            ! cmp -s "$WORK/r.enorm" "$WORK/g.err" || [ "$r_rc" != "$g_rc" ] ||
            { [ "$g_rc" = 0 ] && ! grep -c "^built \`$WORK/u/g/prog\` ($size bytes, [0-9]* module(s))\$" "$WORK/g.out" >/dev/null; }; then
            fail "grasm build $src (a bundle) -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.norm" "$WORK/g.out" | head -4 | sed 's/^/  /'
            diff "$WORK/r.enorm" "$WORK/g.err" | head -4 | sed 's/^/  /'
            continue
        fi
        bundle_same=$((bundle_same + 1))
        if [ "$r_rc" != 0 ]; then
            bundle_refused=$((bundle_refused + 1))
            continue
        fi
        r_size=$(wc -c <"$WORK/u/r/prog" | tr -d ' ')
        if [ -x "$WORK/u/g/prog" ] && [ $((size - g_exe)) = $((r_size - r_exe)) ] &&
            cmp -s <(head -c "$g_exe" "$WORK/u/g/prog") "$G" &&
            cmp -s <(tail -c "+$((r_exe + 1))" "$WORK/u/r/prog") <(tail -c "+$((g_exe + 1))" "$WORK/u/g/prog"); then
            bundle_payload=$((bundle_payload + 1))
        else
            fail "grasm build $src (a bundle) -- not the command followed by the Rust bundle's payload ($size and $r_size bytes)"
            continue
        fi
        # Run as `run` runs them, from the checkout with no argument; then
        # with arguments, each from an empty directory of its own, whose
        # files are compared too -- a program may take an argument for a
        # path to write, which from the checkout would be litter.
        ran_ok=1
        for how in plain args; do
            rm -rf "$WORK/u/rd" "$WORK/u/gd"
            mkdir -p "$WORK/u/rd" "$WORK/u/gd"
            if [ "$how" = plain ]; then
                $RUN_LIMIT "$WORK/u/r/prog" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
                r_rc=$?
                $RUN_LIMIT sandbox-exec -p "$NORUST" "$WORK/u/g/prog" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
                g_rc=$?
            else
                (cd "$WORK/u/rd" && $RUN_LIMIT "$WORK/u/r/prog" a "b c" -- -d >"$WORK/r.out" 2>"$WORK/r.err" </dev/null)
                r_rc=$?
                (cd "$WORK/u/gd" && $RUN_LIMIT sandbox-exec -p "$NORUST" "$WORK/u/g/prog" a "b c" -- -d >"$WORK/g.out" 2>"$WORK/g.err" </dev/null)
                g_rc=$?
            fi
            strip_trace "$(cat "$WORK/r.err")" >"$WORK/r.strip"
            if cmp -s "$WORK/r.out" "$WORK/g.out" && [ "$r_rc" = "$g_rc" ] && [ "$r_rc" != 124 ] &&
                [ "$(cat "$WORK/r.strip")" == "$(cat "$WORK/g.err")" ] &&
                [ "$(listing "$WORK/u/rd")" = "$(listing "$WORK/u/gd")" ]; then
                :
            elif cmp -s "$WORK/r.out" "$WORK/g.out" && [ "$r_rc" = "$g_rc" ] &&
                [ "$(listing "$WORK/u/rd")" = "$(listing "$WORK/u/gd")" ] &&
                [ "$(sed -E 's/(IoFailed\(.*) \(os error [0-9]+\)\)/\1)/' "$WORK/r.strip")" == "$(cat "$WORK/g.err")" ]; then
                # The `IoFailed` text the engines word differently, an open
                # S1 in docs/known-issues.md (§6.17): counted, not hidden.
                bundle_iofailed=$((bundle_iofailed + 1))
                echo "  the IoFailed wording, docs/known-issues.md: $src ($how)"
            else
                ran_ok=0
                fail "$src bundled by each runs otherwise ($how) -- exit $r_rc (Rust's bundle) and $g_rc (Grasm's)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                diff "$WORK/r.strip" "$WORK/g.err" | head -4 | sed 's/^/  /'
                diff <(listing "$WORK/u/rd") <(listing "$WORK/u/gd") | head -4 | sed 's/^/  /'
            fi
        done
        rm -rf "$WORK/u/rd" "$WORK/u/gd"
        [ "$ran_ok" = 1 ] && bundle_ran=$((bundle_ran + 1))
        if g build "$src" -o "$WORK/u/g2/prog" >/dev/null 2>&1 </dev/null && cmp -s "$WORK/u/g/prog" "$WORK/u/g2/prog"; then
            bundle_repro=$((bundle_repro + 1))
        else
            fail "grasm build $src (a bundle) -- built twice, two bundles"
        fi
    done <"$WORK/mains"
    rm -rf "$WORK/u"
    echo "bundle: $bundles programs with a main   the same output: $bundle_same, $bundle_refused of them refusals   the command and Rust's payload: $bundle_payload   run alike, plain and with arguments: $bundle_ran ($bundle_iofailed run(s) differing only in the IoFailed wording)   built twice the same: $bundle_repro"
fi

# source: every corpus program, `--source`; every tenth with a `main` run
# from the file the Grasm command wrote, by both.
sources=0
sources_beside=0
source_beside=0
source_same=0
source_refused=0
source_ran=0
if want source; then
    mkdir -p "$WORK/s/r" "$WORK/s/g"
    # Without `-o` the file is the source's stem, here; with an empty one it
    # goes beside the source, every trailing `.gr` taken off: tried on a
    # copy in each directory, every file left compared.
    for o in none empty; do
        sources_beside=$((sources_beside + 1))
        for side in r g; do
            rm -rf "$WORK/s/$side"/*
            cp examples/hello.gr "$WORK/s/$side/x.gr.gr"
        done
        if [ "$o" = none ]; then set -- build x.gr.gr --source; else set -- build x.gr.gr --source -o ""; fi
        (cd "$WORK/s/r" && "$GRASM" "$@" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null)
        r_rc=$?
        (cd "$WORK/s/g" && g "$@" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null)
        g_rc=$?
        if same3 "$r_rc" "$g_rc" && [ "$r_rc" = 0 ] && [ "$(listing "$WORK/s/r")" = "$(listing "$WORK/s/g")" ] &&
            [ "$(cd "$WORK/s/r" && find . -type f | sort | xargs cat | cksum)" = "$(cd "$WORK/s/g" && find . -type f | sort | xargs cat | cksum)" ] &&
            [ "$(find "$WORK/s/g" -type f | grep -c .)" = 2 ]; then
            source_beside=$((source_beside + 1))
        else
            fail "grasm $* -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
        fi
    done
    rm -rf "$WORK/s/r"/* "$WORK/s/g"/*
    k=0
    while IFS= read -r src; do
        sources=$((sources + 1))
        rm -f "$WORK/s/r/x.bundle.gr" "$WORK/s/g/x.bundle.gr"
        "$GRASM" build "$src" --source -o "$WORK/s/r/x.bundle.gr" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        g build "$src" --source -o "$WORK/s/g/x.bundle.gr" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        sed -i '' "s|$WORK/s/r/|$WORK/s/g/|g" "$WORK/r.out" "$WORK/r.err"
        if ! same3 "$r_rc" "$g_rc" ||
            { { [ -e "$WORK/s/r/x.bundle.gr" ] || [ -e "$WORK/s/g/x.bundle.gr" ]; } &&
                ! cmp -s "$WORK/s/r/x.bundle.gr" "$WORK/s/g/x.bundle.gr"; } ||
            { [ "$g_rc" = 0 ] && [ ! -s "$WORK/s/g/x.bundle.gr" ]; }; then
            fail "grasm build $src --source -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
            diff "$WORK/s/r/x.bundle.gr" "$WORK/s/g/x.bundle.gr" | head -4 | sed 's/^/  /'
            continue
        fi
        source_same=$((source_same + 1))
        if [ "$r_rc" != 0 ]; then
            source_refused=$((source_refused + 1))
            continue
        fi
        grep -qxF "$src" "$WORK/mains" || continue
        k=$((k + 1))
        [ $((k % 10)) = 1 ] || continue
        $RUN_LIMIT "$GRASM" run "$WORK/s/g/x.bundle.gr" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        $RUN_LIMIT sandbox-exec -p "$NORUST" "$G" run "$WORK/s/g/x.bundle.gr" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if cmp -s "$WORK/r.out" "$WORK/g.out" && [ "$r_rc" = "$g_rc" ] && [ "$r_rc" != 124 ] &&
            [ "$(strip_trace "$(cat "$WORK/r.err")")" == "$(cat "$WORK/g.err")" ]; then
            source_ran=$((source_ran + 1))
        else
            fail "grasm run on $src's --source file -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
            diff <(strip_trace "$(cat "$WORK/r.err")") "$WORK/g.err" | head -4 | sed 's/^/  /'
        fi
    done <"$WORK/programs"
    rm -rf "$WORK/s"
    echo "source: $sources programs   the same output and file: $source_same, $source_refused of them refusals   run from the file alike: $source_ran   written beside the source alike: $source_beside of $sources_beside"
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

# -------------------------------------------------------------------------
# The package commands (§6.21). Each side works in a copy of its own -- the
# checkout's tracked files, or `regcases.py`'s trees -- because `deps`
# writes a lockfile into the package, `add` its manifest, `publish` a
# registry; the copies' own paths are taken out of the output (`@`). A
# registry is a directory in the copy: nothing reaches the network, and no
# `.grasm/registry` above the work directory, or `GRASM_REGISTRY` in the
# environment, can stand in for the one a case names.
# -------------------------------------------------------------------------
unset GRASM_REGISTRY
PK="$(cd "$WORK" && pwd -P)/pk"
if want registry || want deps || want add || want publish; then
    up=$(dirname "$PK")
    while [ "$up" != / ]; do
        if [ -e "$up/.grasm/registry" ]; then
            echo "error: $up/.grasm/registry exists, and a package here without a registry of its own would find it"
            exit 1
        fi
        up=$(dirname "$up")
    done
fi

# Every entry under `$1`, with its mode, and each file's digest.
pk_tree() {
    (cd "$1" && find . -mindepth 1 | LC_ALL=C sort | while IFS= read -r f; do
        if [ -L "$f" ]; then
            echo "l $f"
        elif [ -d "$f" ]; then
            echo "d $(stat -f %Lp "$f") $f"
        else
            echo "f $(stat -f %Lp "$f") $f $(shasum -a 256 <"$f" | cut -c1-16)"
        fi
    done)
}

# pk_pair DIR ENV ARGS...: the Rust command in $PK/r/DIR and the Grasm one
# in $PK/g/DIR, ENV (`-` for none) set for both; their output with the
# copies' paths taken out, in $WORK/pk.{r,g}.{out,err,rc}.
pk_pair() {
    local dir=$1 env=$2
    shift 2
    local s
    for s in r g; do
        local -a run=("$GRASM")
        [ "$s" = g ] && run=(sandbox-exec -p "$NOEXEC" "$G")
        if [ "$env" = - ]; then
            (cd "$PK/$s/$dir" && $RUN_LIMIT "${run[@]}" "$@" >"$WORK/pk.$s.out" 2>"$WORK/pk.$s.err" </dev/null)
        else
            (cd "$PK/$s/$dir" && env "$env" $RUN_LIMIT "${run[@]}" "$@" >"$WORK/pk.$s.out" 2>"$WORK/pk.$s.err" </dev/null)
        fi
        echo $? >"$WORK/pk.$s.rc"
        sed -i '' -e "s|$PK/$s|@|g" "$WORK/pk.$s.out" "$WORK/pk.$s.err"
    done
}

# Whether the last pair answered alike; reported as $1 if not.
pk_same() {
    if cmp -s "$WORK/pk.r.out" "$WORK/pk.g.out" && cmp -s "$WORK/pk.r.err" "$WORK/pk.g.err" &&
        cmp -s "$WORK/pk.r.rc" "$WORK/pk.g.rc"; then
        return 0
    fi
    fail "$1 -- exit $(cat "$WORK/pk.r.rc") (Rust) and $(cat "$WORK/pk.g.rc") (Grasm)"
    diff "$WORK/pk.r.out" "$WORK/pk.g.out" | head -6 | sed 's/^/  /'
    diff "$WORK/pk.r.err" "$WORK/pk.g.err" | head -6 | sed 's/^/  /'
    return 1
}

# Whether file $1 (under each copy) is the same on both sides, or absent
# from both.
pk_file_same() {
    if [ -e "$PK/r/$1" ] || [ -e "$PK/g/$1" ]; then
        cmp -s "$PK/r/$1" "$PK/g/$1"
    fi
}

# A copy of the checkout's tracked files for each side.
pk_checkout() {
    rm -rf "$PK"
    mkdir -p "$PK/r" "$PK/g"
    git ls-files -z | (cd "$ROOT" && cpio -0pdm "$PK/r" 2>/dev/null)
    git ls-files -z | (cd "$ROOT" && cpio -0pdm "$PK/g" 2>/dev/null)
}

# -------------------------------------------------------------------------
# registry: `regcases.py`'s package graphs and registries, its commands in
# order on each side's copy, the output and every file of the case compared
# after each one
# -------------------------------------------------------------------------
reg_runs=0
reg_same=0
reg_deps=0
reg_add=0
reg_publish=0
reg_edits=0
reg_refused=0
if want registry; then
    rm -rf "$PK"
    python3 scripts/lib/regcases.py "$PK/r" >"$WORK/regcmds"
    python3 scripts/lib/regcases.py "$PK/g" >/dev/null
    while IFS=$'\t' read -r case_name cwd env line; do
        read -r -a args <<<"$line"
        if [ "${args[0]}" = @edit ]; then
            (cd "$PK/r/$case_name/$cwd" && python3 "$ROOT/scripts/lib/lockedit.py" "${args[1]}" "${args[2]}") &&
                (cd "$PK/g/$case_name/$cwd" && python3 "$ROOT/scripts/lib/lockedit.py" "${args[1]}" "${args[2]}") ||
                fail "registry $case_name: the edit ${args[1]} could not be made"
            reg_edits=$((reg_edits + 1))
            continue
        fi
        reg_runs=$((reg_runs + 1))
        case "${args[0]}" in
        deps) reg_deps=$((reg_deps + 1)) ;;
        add) reg_add=$((reg_add + 1)) ;;
        publish) reg_publish=$((reg_publish + 1)) ;;
        esac
        pk_pair "$case_name/$cwd" "$env" "${args[@]}"
        pk_tree "$PK/r/$case_name" >"$WORK/pk.r.tree"
        pk_tree "$PK/g/$case_name" >"$WORK/pk.g.tree"
        if pk_same "registry $case_name: grasm $line (in $cwd, $env)"; then
            if cmp -s "$WORK/pk.r.tree" "$WORK/pk.g.tree"; then
                reg_same=$((reg_same + 1))
                [ "$(cat "$WORK/pk.r.rc")" != 0 ] && reg_refused=$((reg_refused + 1))
            else
                fail "registry $case_name: grasm $line (in $cwd, $env) -- the files differ"
                diff "$WORK/pk.r.tree" "$WORK/pk.g.tree" | head -6 | sed 's/^/  /'
            fi
        fi
    done <"$WORK/regcmds"
    echo "registry: $reg_runs commands ($reg_deps deps, $reg_add add, $reg_publish publish; $reg_edits lockfile edits between them)   the same, files and all: $reg_same, $reg_refused of them refusals"
fi

# Every package in the checkout: the directories of the manifests git tracks.
if [ -n "${PACKAGES:-}" ]; then
    cp "$PACKAGES" "$WORK/packages"
else
    git ls-files '*grasm.toml' | sed -e 's|/grasm.toml$||' -e 's|^grasm.toml$|.|' >"$WORK/packages"
fi
packages=$(grep -c . "$WORK/packages")

# -------------------------------------------------------------------------
# deps: every package, `deps --locked` (the tracked lockfile or none), `deps`
# (the lockfile written), `deps --locked` over it; the lockfile compared
# after each
# -------------------------------------------------------------------------
deps_runs=0
deps_same=0
deps_written=0
deps_refused=0
if want deps; then
    pk_checkout
    while IFS= read -r p; do
        for flags in --locked "" --locked; do
            deps_runs=$((deps_runs + 1))
            pk_pair . - deps "$p" $flags
            if pk_same "grasm deps $p $flags"; then
                if pk_file_same "$p/grasm.lock"; then
                    deps_same=$((deps_same + 1))
                    [ "$(cat "$WORK/pk.r.rc")" != 0 ] && deps_refused=$((deps_refused + 1))
                    [ -z "$flags" ] && grep -q '^wrote ' "$WORK/pk.r.out" && deps_written=$((deps_written + 1))
                else
                    fail "grasm deps $p $flags -- the lockfiles differ"
                    diff "$PK/r/$p/grasm.lock" "$PK/g/$p/grasm.lock" | head -6 | sed 's/^/  /'
                fi
            fi
        done
    done <"$WORK/packages"
    echo "deps: $packages packages, $deps_runs commands   the same, lockfiles and all: $deps_same, $deps_refused of them refusals   lockfiles written: $deps_written"
fi

# -------------------------------------------------------------------------
# add: every package, a registry dependency added to its manifest
# -------------------------------------------------------------------------
add_runs=0
add_same=0
add_refused=0
if want add; then
    pk_checkout
    while IFS= read -r p; do
        add_runs=$((add_runs + 1))
        pk_pair . - add clidiff_probe ^1.2.3 "$p"
        if pk_same "grasm add clidiff_probe ^1.2.3 $p"; then
            if pk_file_same "$p/grasm.toml"; then
                add_same=$((add_same + 1))
                [ "$(cat "$WORK/pk.r.rc")" != 0 ] && add_refused=$((add_refused + 1))
            else
                fail "grasm add clidiff_probe ^1.2.3 $p -- the manifests differ"
            fi
        fi
    done <"$WORK/packages"
    echo "add: $packages packages   the same, manifests and all: $add_same, $add_refused of them refusals"
fi

# -------------------------------------------------------------------------
# publish: every package into a registry of each side's own; after each, the
# registry's every path and mode, and the bytes of what it just wrote; at
# the end every byte of the whole registry
# -------------------------------------------------------------------------
pub_runs=0
pub_same=0
pub_done=0
# Every path under `$1` and its mode, one `stat` for the lot: comparing
# every byte after each of 1,448 publishes would be quadratic.
pk_names() {
    (cd "$1" && find . -mindepth 1 -exec stat -f '%Lp %N' {} + | LC_ALL=C sort)
}
if want publish; then
    pk_checkout
    mkdir -p "$PK/r/clidiff-registry" "$PK/g/clidiff-registry"
    while IFS= read -r p; do
        pub_runs=$((pub_runs + 1))
        pk_pair . - publish "$p" --registry clidiff-registry
        if pk_same "grasm publish $p --registry clidiff-registry"; then
            dest=$(sed -n 's/^published .* to \(clidiff-registry\/.*\)$/\1/p' "$WORK/pk.r.out")
            if ! cmp -s <(pk_names "$PK/r/clidiff-registry") <(pk_names "$PK/g/clidiff-registry"); then
                fail "grasm publish $p --registry clidiff-registry -- the registries' paths differ"
                diff <(pk_names "$PK/r/clidiff-registry") <(pk_names "$PK/g/clidiff-registry") | head -6 | sed 's/^/  /'
            elif [ -n "$dest" ] && ! cmp -s <(pk_tree "$PK/r/$dest") <(pk_tree "$PK/g/$dest"); then
                fail "grasm publish $p --registry clidiff-registry -- what it wrote differs"
                diff <(pk_tree "$PK/r/$dest") <(pk_tree "$PK/g/$dest") | head -6 | sed 's/^/  /'
            else
                pub_same=$((pub_same + 1))
                [ "$(cat "$WORK/pk.r.rc")" = 0 ] && pub_done=$((pub_done + 1))
            fi
        fi
    done <"$WORK/packages"
    if ! cmp -s <(pk_tree "$PK/r/clidiff-registry") <(pk_tree "$PK/g/clidiff-registry"); then
        fail "publish: the two registries differ at the end"
        diff <(pk_tree "$PK/r/clidiff-registry") <(pk_tree "$PK/g/clidiff-registry") | head -6 | sed 's/^/  /'
    fi
    echo "publish: $packages packages   the same, registries and all: $pub_same, $pub_done of them published"
fi
rm -rf "$PK"

# -------------------------------------------------------------------------
# journal, dap, lsp (docs/self-hosting.md §6.22)
# -------------------------------------------------------------------------

# Whether the last two runs, $WORK/r.* and $WORK/g.*, said the same, a
# panic's stack trace taken out of the Rust side's stderr as in `run`.
same_run() { # r_rc g_rc
    [ "$1" = "$2" ] && cmp -s "$WORK/r.out" "$WORK/g.out" &&
        [ "$(strip_trace "$(cat "$WORK/r.err")")" == "$(strip_trace "$(cat "$WORK/g.err")")" ]
}

# journal: every corpus program with a `main`, run by each command under
# `GRASM_RECORD`, each into a file of its own: the same output and status,
# the same file or none, and the two the same bytes but for the values of
# the built-ins that answer differently each time (scripts/lib/jnorm.py).
# Every recording is then replayed by both commands, the Grasm command's
# replayed by Rust too, and Rust's printed by both with and without
# `--full`; and from every one that holds more than `args`, and every tenth
# other, the journals scripts/lib/jmangle.py makes, each replayed and
# printed by both (every one at first: five hours, most of it on the 140
# that hold `args` alone); and the same for
# scripts/fixtures/journal/, which journal from tasks. Then each program whose
# recording holds more than `args` is built natively by the Grasm command,
# recorded by the executable, and replayed by both commands' `run`.
j_runs=0
j_same=0
j_files=0
j_replayed=0
j_shown=0
j_mangled=0
j_iofailed=0
j_native=0
j_native_same=0
mkdir -p "$WORK/jrec"
: >"$WORK/jrich"
if want journal; then
    n=0
    while IFS= read -r src; do
        n=$((n + 1))
        j_runs=$((j_runs + 1))
        rm -f "$WORK/jr.jrnl" "$WORK/jg.jrnl"
        GRASM_RECORD="$WORK/jr.jrnl" $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        GRASM_RECORD="$WORK/jg.jrnl" $RUN_LIMIT sandbox-exec -p "$NORUST" "$GJ" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if ! same_run "$r_rc" "$g_rc"; then
            fail "GRASM_RECORD grasm run $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
            diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
            continue
        fi
        if { [ -f "$WORK/jr.jrnl" ] && [ ! -f "$WORK/jg.jrnl" ]; } || { [ ! -f "$WORK/jr.jrnl" ] && [ -f "$WORK/jg.jrnl" ]; }; then
            fail "GRASM_RECORD grasm run $src -- a journal written by one command and not the other"
            continue
        fi
        if [ ! -f "$WORK/jr.jrnl" ]; then
            j_same=$((j_same + 1))
            continue
        fi
        j_files=$((j_files + 1))
        why=$(python3 scripts/lib/jnorm.py "$WORK/jr.jrnl" "$WORK/jg.jrnl")
        case $? in
        0) ;;
        2) j_iofailed=$((j_iofailed + 1)) ;;
        *)
            fail "GRASM_RECORD grasm run $src -- the journals differ: $why"
            continue
            ;;
        esac
        cp "$WORK/jr.jrnl" "$WORK/jrec/$n.jrnl"
        ok=1
        # Replayed: Rust's recording by both, the Grasm command's by Rust.
        for pair in "jr.jrnl:G" "jg.jrnl:R"; do
            j=${pair%%:*}
            GRASM_REPLAY="$WORK/$j" $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            if [ "${pair#*:}" = G ]; then
                GRASM_REPLAY="$WORK/$j" $RUN_LIMIT sandbox-exec -p "$NORUST" "$GJ" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            else
                GRASM_REPLAY="$WORK/jr.jrnl" $RUN_LIMIT sandbox-exec -p "$NORUST" "$GJ" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            fi
            g_rc=$?
            if same_run "$r_rc" "$g_rc"; then
                j_replayed=$((j_replayed + 1))
            else
                fail "GRASM_REPLAY=$j grasm run $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
                ok=0
            fi
        done
        # Printed, as the file is, by both.
        for j in jr.jrnl; do
            for full in "" --full; do
                (cd "$WORK" && "$GRASM" journal "$j" $full >r.out 2>r.err)
                r_rc=$?
                (cd "$WORK" && g journal "$j" $full >g.out 2>g.err)
                g_rc=$?
                if [ "$r_rc" = "$g_rc" ] && cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err"; then
                    j_shown=$((j_shown + 1))
                else
                    fail "grasm journal $j $full (of $src) -- exit $r_rc (Rust) and $g_rc (Grasm)"
                    diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                    ok=0
                fi
            done
        done
        # More than `args`: built natively below, and mangled.
        rich=0
        if "$GRASM" journal "$WORK/jr.jrnl" | tail -1 | grep -cvE 'builtin\(s\): args [0-9]+$' >/dev/null; then
            echo "$n $src" >>"$WORK/jrich"
            rich=1
        fi
        # The mangled journals, replayed and printed by both: of every
        # recording that holds more than `args`, and of every tenth other.
        rm -rf "$WORK/jm"
        if [ "$rich" = 0 ] && [ $((j_files % 10)) != 1 ]; then
            [ "$ok" = 1 ] && j_same=$((j_same + 1))
            continue
        fi
        for k in $(python3 scripts/lib/jmangle.py "$WORK/jr.jrnl" "$WORK/jm"); do
            GRASM_REPLAY="$WORK/jm/$k.jrnl" $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            GRASM_REPLAY="$WORK/jm/$k.jrnl" $RUN_LIMIT sandbox-exec -p "$NORUST" "$GJ" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            if ! same_run "$r_rc" "$g_rc"; then
                fail "GRASM_REPLAY=$k.jrnl grasm run $src -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
                ok=0
                continue
            fi
            (cd "$WORK/jm" && "$GRASM" journal "$k.jrnl" --full >../r.out 2>../r.err)
            r_rc=$?
            (cd "$WORK/jm" && g journal "$k.jrnl" --full >../g.out 2>../g.err)
            g_rc=$?
            if [ "$r_rc" = "$g_rc" ] && cmp -s "$WORK/r.out" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err"; then
                j_mangled=$((j_mangled + 1))
            else
                fail "grasm journal $k.jrnl --full (of $src) -- exit $r_rc (Rust) and $g_rc (Grasm)"
                diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
                ok=0
            fi
        done
        [ "$ok" = 1 ] && j_same=$((j_same + 1))
    done < <(cat "$WORK/mains"; ls scripts/fixtures/journal/*.gr)
    # A native recording replayed under `grasm run`, which is what a
    # recording is for (docs/replay.md).
    while read -r n src; do
        j_native=$((j_native + 1))
        rm -rf "$WORK/jn"
        mkdir -p "$WORK/jn"
        if ! "$G" build "$src" --native -o "$WORK/jn/p" >"$WORK/jn/build.log" 2>&1; then
            fail "grasm build $src --native -- does not build, to record it"
            continue
        fi
        GRASM_RECORD="$WORK/jn/p.jrnl" $RUN_LIMIT "$WORK/jn/p" >"$WORK/jn/live.out" 2>/dev/null </dev/null
        GRASM_REPLAY="$WORK/jn/p.jrnl" $RUN_LIMIT "$GRASM" run "$src" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
        r_rc=$?
        GRASM_REPLAY="$WORK/jn/p.jrnl" $RUN_LIMIT sandbox-exec -p "$NORUST" "$GJ" run "$src" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
        g_rc=$?
        if same_run "$r_rc" "$g_rc"; then
            j_native_same=$((j_native_same + 1))
        else
            fail "a native recording of $src replayed -- exit $r_rc (Rust) and $g_rc (Grasm)"
            diff "$WORK/r.out" "$WORK/g.out" | head -4 | sed 's/^/  /'
            diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
        fi
    done <"$WORK/jrich"
    echo "journal: $j_runs programs recorded   the same: $j_same, $j_files of them with a journal ($j_iofailed differing only in the IoFailed wording)   replays alike: $j_replayed   printed alike: $j_shown   mangled journals alike: $j_mangled   native recordings replayed alike: $j_native_same of $j_native"
fi

# dap: the sessions scripts/lib/dapscript.py writes -- every corpus program
# with a `main` stepped into 200 times, every fourth also stepped over and
# out and run to breakpoints -- and its fixed sessions; each answer stream
# compared as it arrives by length and SHA-256 (scripts/lib/streamsum.py),
# with the status and stderr. A program that reads stdin is left out (the
# protocol is its stdin, decision (5)); a session the Rust adapter does not
# finish in 60 s, or ends by overflowing its own stack (exit 134), is counted
# and not compared; a program whose stepping took Rust more than 10 s gets
# no other session.
d_runs=0
d_same=0
d_reads_stdin=0
d_rust_long=0
d_rust_abort=0
d_fixed=0
d_skipped=0
d_rust_otherwise=0
d_pane_only=0
d_replayed=0
d_host=0
declare -a D_REPLAYED=()
declare -a D_HOST=()
declare -a D_OTHERWISE=()
declare -a D_PANE=()
DAP_LIMIT=${DAP_LIMIT:-60}
dap_compare() { # label input
    local t0 t1
    t0=$(date +%s)
    timeout "$DAP_LIMIT" "$GRASM" dap <"$2" 2>"$WORK/r.err" | python3 scripts/lib/streamsum.py >"$WORK/r.sum"
    local r_rc=${PIPESTATUS[0]}
    t1=$(date +%s)
    D_RUST_SECONDS=$((t1 - t0))
    if [ "$r_rc" = 124 ]; then
        d_rust_long=$((d_rust_long + 1))
        return 0
    fi
    if [ "$r_rc" = 134 ]; then
        d_rust_abort=$((d_rust_abort + 1))
        return 0
    fi
    d_runs=$((d_runs + 1))
    timeout $((DAP_LIMIT * 4)) sandbox-exec -p "$NORUST" "$G" dap <"$2" 2>"$WORK/g.err" | python3 scripts/lib/streamsum.py >"$WORK/g.sum"
    local g_rc=${PIPESTATUS[0]}
    if [ "$r_rc" = "$g_rc" ] && cmp -s "$WORK/r.sum" "$WORK/g.sum" && cmp -s "$WORK/r.err" "$WORK/g.err"; then
        d_same=$((d_same + 1))
        return 0
    fi
    # A panic the host's channel operations raise ends the Grasm command
    # there, under `dap` as under `test` (§6.19, decision (3): the
    # interpreter's channels are the host's): counted, as `test` counts it.
    if [ "$g_rc" = 101 ] && grep -qE '^panic: (send on a closed channel|close of a closed channel|deadlock: )' "$WORK/g.err"; then
        d_host=$((d_host + 1))
        D_HOST+=("$1")
        return 0
    fi
    # The Rust adapter runs a program without the checker's facts -- the
    # literal a `[f64]` stores, a `u64`'s or a `u8`'s width, raw memory,
    # `call_c`, interface calls (docs/known-issues.md) -- so where it prints
    # or ends otherwise than `grasm run`, its session is not the one to hold
    # the Grasm adapter to: counted, and the Grasm adapter held to `grasm
    # run` instead.
    if [ -n "${3:-}" ] && ! ran_as_run "$GRASM" "$3"; then
        if ran_as_run "$G" "$3"; then
            d_rust_otherwise=$((d_rust_otherwise + 1))
            D_OTHERWISE+=("$3")
            return 0
        fi
        fail "grasm dap $1 -- the Grasm adapter runs it otherwise than grasm run"
        return 1
    fi
    # A program that asks outside itself -- random bytes, the clock, a port
    # -- may take another path through itself from one run to the next
    # (L-LIB-RANDOM's rejection sampling stops at a breakpoint as many times
    # as it draws again): both adapters once more, launched with one
    # recording of it the Rust command made, so both see the same answers.
    # Counted.
    if [ -n "${3:-}" ] && [[ " step next bp " == *" ${1%% *} "* ]]; then
        rm -f "$WORK/once.jrnl"
        GRASM_RECORD="$WORK/once.jrnl" timeout $((DAP_LIMIT * 4)) "$GRASM" run "$3" >/dev/null 2>&1 </dev/null
        if [ -s "$WORK/once.jrnl" ] && ! "$GRASM" journal "$WORK/once.jrnl" 2>/dev/null | head -1 | grep -q ' 0 record(s)'; then
            python3 scripts/lib/dapscript.py "${1%% *}" "$3" "$WORK/once.in" "$WORK/once.jrnl"
            timeout "$DAP_LIMIT" "$GRASM" dap <"$WORK/once.in" 2>"$WORK/r.err" | python3 scripts/lib/streamsum.py >"$WORK/r.sum"
            r_rc=${PIPESTATUS[0]}
            timeout $((DAP_LIMIT * 4)) sandbox-exec -p "$NORUST" "$G" dap <"$WORK/once.in" 2>"$WORK/g.err" | python3 scripts/lib/streamsum.py >"$WORK/g.sum"
            g_rc=${PIPESTATUS[0]}
            if [ "$r_rc" = "$g_rc" ] && cmp -s "$WORK/r.sum" "$WORK/g.sum" && cmp -s "$WORK/r.err" "$WORK/g.err"; then
                d_replayed=$((d_replayed + 1))
                D_REPLAYED+=("$1")
                return 0
            fi
            fail "grasm dap $1 -- under one recording, exit $r_rc (Rust) and $g_rc (Grasm), $(cat "$WORK/r.sum") and $(cat "$WORK/g.sum")"
            diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
            return 1
        fi
    fi
    # A difference in the variables pane alone -- a `u64` the Rust adapter
    # shows as an `i64`, for the same reason -- is a counted, printed
    # exception (the coordinator's decision, §6.22); the answers are kept
    # for this, up to 100 MB each.
    if [ "$r_rc" = "$g_rc" ] && cmp -s "$WORK/r.err" "$WORK/g.err"; then
        timeout "$DAP_LIMIT" "$GRASM" dap <"$2" 2>/dev/null | head -c 100000000 >"$WORK/r.ans"
        timeout $((DAP_LIMIT * 4)) sandbox-exec -p "$NORUST" "$G" dap <"$2" 2>/dev/null | head -c 100000000 >"$WORK/g.ans"
        if why=$(python3 scripts/lib/dapmask.py "$WORK/r.ans" "$WORK/g.ans"); then
            d_pane_only=$((d_pane_only + 1))
            D_PANE+=("$1 -- $why")
            rm -f "$WORK/r.ans" "$WORK/g.ans"
            return 0
        fi
        rm -f "$WORK/r.ans" "$WORK/g.ans"
    fi
    fail "grasm dap $1 -- exit $r_rc (Rust) and $g_rc (Grasm), $(cat "$WORK/r.sum") and $(cat "$WORK/g.sum")"
    diff "$WORK/r.err" "$WORK/g.err" | head -4 | sed 's/^/  /'
    return 1
}

# Whether adapter $1, launched on $2 with no stop, prints and ends as `grasm
# run` does.
ran_as_run() { # adapter program
    python3 scripts/lib/dapscript.py fixed launch "$2" "$WORK/plain.in"
    if [ "$1" = "$GRASM" ]; then
        timeout $((DAP_LIMIT * 4)) "$1" dap <"$WORK/plain.in" 2>/dev/null | python3 scripts/lib/dapran.py >"$WORK/plain.dap"
    else
        timeout $((DAP_LIMIT * 4)) sandbox-exec -p "$NORUST" "$1" dap <"$WORK/plain.in" 2>/dev/null | python3 scripts/lib/dapran.py >"$WORK/plain.dap"
    fi
    timeout $((DAP_LIMIT * 4)) "$GRASM" run "$2" >"$WORK/plain.run" 2>/dev/null </dev/null
    echo "exit $?" >>"$WORK/plain.run"
    cmp -s "$WORK/plain.dap" "$WORK/plain.run"
}
if want dap; then
    n=0
    while IFS= read -r src; do
        n=$((n + 1))
        if grep -q 'input()' "$src"; then
            d_reads_stdin=$((d_reads_stdin + 1))
            continue
        fi
        python3 scripts/lib/dapscript.py step "$src" "$WORK/d.in"
        dap_compare "step $src" "$WORK/d.in" "$src"
        [ $((n % 4)) = 0 ] || continue
        if [ "$D_RUST_SECONDS" -gt 10 ]; then
            d_skipped=$((d_skipped + 2))
            continue
        fi
        for kind in next bp; do
            python3 scripts/lib/dapscript.py "$kind" "$src" "$WORK/d.in"
            dap_compare "$kind $src" "$WORK/d.in" "$src"
        done
    done <"$WORK/mains"
    # The fixed sessions, on `examples/hello.gr`; launches that fail, on
    # every tenth conformance program that must not compile and the
    # registry app; a journal given to launch, on the recordings above.
    for f in $(python3 scripts/lib/dapscript.py list-fixed); do
        python3 scripts/lib/dapscript.py fixed "$f" examples/hello.gr "$WORK/d.in"
        dap_compare "fixed $f" "$WORK/d.in" && d_fixed=$((d_fixed + 1))
    done
    k=0
    for src in $(grep -l '^//! expect: error$' spec/conformance/*.gr) examples/registry/app/src/main.gr; do
        k=$((k + 1))
        [ $((k % 10)) = 1 ] || [ "$src" = examples/registry/app/src/main.gr ] || continue
        python3 scripts/lib/dapscript.py fixed launch "$src" "$WORK/d.in"
        dap_compare "launch $src" "$WORK/d.in" && d_fixed=$((d_fixed + 1))
    done
    printf 'GRASMJRNL1' >"$WORK/notajournal.jrnl"
    for j in "$WORK/nosuch.jrnl" "$WORK/notajournal.jrnl"; do
        python3 scripts/lib/dapscript.py fixed journal examples/hello.gr "$WORK/d.in" "$j"
        dap_compare "journal $j" "$WORK/d.in" && d_fixed=$((d_fixed + 1))
    done
    while read -r jn src; do
        python3 scripts/lib/dapscript.py fixed journal "$src" "$WORK/d.in" "$WORK/jrec/$jn.jrnl"
        dap_compare "journal $src" "$WORK/d.in" "$src" && d_fixed=$((d_fixed + 1))
    done <"$WORK/jrich"
    echo "dap: $d_runs sessions compared   the same: $d_same, $d_fixed of them fixed   the Rust adapter running it otherwise than grasm run: $d_rust_otherwise   differing in the variables pane alone: $d_pane_only   the same under one recording: $d_replayed   the host's channel panic: $d_host   reading stdin, left out: $d_reads_stdin   past ${DAP_LIMIT}s for Rust: $d_rust_long   the Rust adapter's own stack: $d_rust_abort   long, not stepped again: $d_skipped"
    if [ ${#D_OTHERWISE[@]} -gt 0 ]; then
        printf '  run otherwise by the Rust adapter: %s\n' $(printf '%s\n' "${D_OTHERWISE[@]}" | sort -u)
    fi
    if [ ${#D_HOST[@]} -gt 0 ]; then
        printf '  the host channel panic: %s\n' "${D_HOST[@]}"
    fi
    if [ ${#D_REPLAYED[@]} -gt 0 ]; then
        printf '  the same under one recording: %s\n' "${D_REPLAYED[@]}"
    fi
    if [ ${#D_PANE[@]} -gt 0 ]; then
        printf '  the variables pane alone: %s\n' "${D_PANE[@]}"
    fi
fi

# lsp: a scripted session per tracked `.gr` file (scripts/lib/lspscript.py)
# and its fixed sessions, every byte of the answers compared.
l_runs=0
l_same=0
l_fixed=0
if want lsp; then
    while IFS= read -r f; do
        l_runs=$((l_runs + 1))
        python3 scripts/lib/lspscript.py file "$f" "$WORK/l.in"
        "$GRASM" lsp <"$WORK/l.in" >"$WORK/r.out" 2>"$WORK/r.err"
        r_rc=$?
        g lsp <"$WORK/l.in" >"$WORK/g.out" 2>"$WORK/g.err"
        g_rc=$?
        if same3 "$r_rc" "$g_rc"; then
            l_same=$((l_same + 1))
        else
            fail "grasm lsp on $f -- exit $r_rc (Rust) and $g_rc (Grasm)"
            python3 - "$WORK/r.out" "$WORK/g.out" <<'PY' | head -6 | sed 's/^/  /'
import sys
a, b = open(sys.argv[1], 'rb').read(), open(sys.argv[2], 'rb').read()
i = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
print("first difference at byte %d:" % i)
print(" Rust:  %r" % a[max(0, i - 80):i + 120])
print(" Grasm: %r" % b[max(0, i - 80):i + 120])
PY
        fi
    done < <(tracked_gr)
    for f in $(python3 scripts/lib/lspscript.py list-fixed); do
        l_runs=$((l_runs + 1))
        python3 scripts/lib/lspscript.py fixed "$f" "$WORK/l.in"
        "$GRASM" lsp <"$WORK/l.in" >"$WORK/r.out" 2>"$WORK/r.err"
        r_rc=$?
        g lsp <"$WORK/l.in" >"$WORK/g.out" 2>"$WORK/g.err"
        g_rc=$?
        if same3 "$r_rc" "$g_rc"; then
            l_same=$((l_same + 1))
            l_fixed=$((l_fixed + 1))
        else
            fail "grasm lsp, fixed session $f -- exit $r_rc (Rust) and $g_rc (Grasm)"
            show3
        fi
    done
    echo "lsp: $l_runs sessions   the same: $l_same, $l_fixed of them fixed"
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
whole=$([ -z "${FILES:-}" ] && [ -z "${PACKAGES:-}" ] && [ "$ONLY" = "$ALL" ] && echo 1)
if want lines && [ $((lines_same + lines_elsewhere + lines_runtime + lines_target)) -ne "$lines" ]; then
    echo "lines: the counts do not add up to $lines"
    exit 1
fi
if want lines && { [ "$lines" -lt 140 ] || [ "$lines_elsewhere" != 0 ] || [ "$lines_runtime" != 1 ] || [ "$lines_target" != 4 ]; }; then
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
if want doc && [ "$doc_same" -ne "$doc_runs" ]; then
    echo "doc: $doc_same of $doc_runs commands compared"
    exit 1
fi
if want api && { [ "$api_same" -ne "$api_runs" ] || [ "$api_patch" -ne "$api_surfaces" ] ||
    [ "$api_baselines" -ne $((4 * api_surfaces)) ] || [ "$api_runs" -ne $((listed + 8 * api_surfaces)) ]; }; then
    echo "api: the counts do not add up ($api_same of $api_runs, $listed programs, $api_surfaces with a surface, $api_patch patches, $api_baselines rewritten baselines)"
    exit 1
fi
if want bundle && { [ "$bundle_same" -ne "$bundles" ] || [ "$bundle_payload" -ne $((bundles - bundle_refused)) ] ||
    [ "$bundle_ran" -ne "$bundle_payload" ] || [ "$bundle_repro" -ne "$bundle_payload" ] || [ "$bundles" -ne "$mains" ]; }; then
    echo "bundle: the counts do not add up ($bundle_same, $bundle_payload, $bundle_ran, $bundle_repro of $bundles; $bundle_refused refused; $mains with a main)"
    exit 1
fi
if want source && { [ "$source_same" -ne "$sources" ] || [ "$sources" -ne "$listed" ] || [ "$source_beside" != 2 ]; }; then
    echo "source: $source_same of $sources programs compared ($listed listed)"
    exit 1
fi
if want build && { [ "$built_same" -ne "$builds" ] || [ "$ran_same" -ne $((builds - build_refused)) ] || [ "$repro" -ne "$ran_same" ] ||
    [ $((builds + external)) -ne "$mains" ]; }; then
    echo "build: the counts do not add up ($built_same, $ran_same, $repro of $builds; $external external of $mains)"
    exit 1
fi
if want registry && { [ "$reg_same" -ne "$reg_runs" ] || [ $((reg_deps + reg_add + reg_publish)) -ne "$reg_runs" ]; }; then
    echo "registry: the counts do not add up ($reg_same of $reg_runs; $reg_deps deps, $reg_add add, $reg_publish publish)"
    exit 1
fi
if want deps && { [ "$deps_same" -ne "$deps_runs" ] || [ "$deps_runs" -ne $((3 * packages)) ]; }; then
    echo "deps: the counts do not add up ($deps_same of $deps_runs, $packages packages)"
    exit 1
fi
if want add && { [ "$add_same" -ne "$add_runs" ] || [ "$add_runs" -ne "$packages" ]; }; then
    echo "add: the counts do not add up ($add_same of $add_runs, $packages packages)"
    exit 1
fi
if want publish && { [ "$pub_same" -ne "$pub_runs" ] || [ "$pub_runs" -ne "$packages" ]; }; then
    echo "publish: the counts do not add up ($pub_same of $pub_runs, $packages packages)"
    exit 1
fi
if want journal && [ "$j_same" -ne "$j_runs" ]; then
    echo "journal: $j_same of $j_runs programs compared"
    exit 1
fi
if want dap && [ $((d_same + d_rust_otherwise + d_pane_only + d_replayed + d_host)) -ne "$d_runs" ]; then
    echo "dap: $d_same of $d_runs sessions compared"
    exit 1
fi
if want lsp && [ "$l_same" -ne "$l_runs" ]; then
    echo "lsp: $l_same of $l_runs sessions compared"
    exit 1
fi
if [ -n "$whole" ]; then
    if [ "$listed" -lt 1700 ] || [ "$corpus_refused" -lt 120 ] || [ "$tomls" -lt 600 ] ||
        [ "$run_same" -lt 640 ] || [ "$ran_same" -lt 640 ] || [ "$targets" -lt 60 ] || [ "$target_same" -ne $((3 * targets)) ]; then
        echo "too little compared for this to mean anything: $listed programs, $corpus_refused refusals, $tomls manifests, $run_same runs, $ran_same builds run, $target_same of $targets targets"
        exit 1
    fi
    # The floors of §6.19's sections, a margin below the first whole run
    # (2026-11-06): 2,007 files tracked, 7,638 rewrites formatted alike and
    # 385 refused alike; 1,001 findings; 6 programs with a property test, 3
    # with a contract, 209 with --coverage, 12 with a failing test; 651 mains
    # as tests, 13 failing and 2 ended by the host's panic (E-CHANNEL,
    # E-DEADLOCK); 3 programs with a benchmark, 6 timing lines.
    if [ "$fmt_files" -lt 1900 ] || [ "$fmt_rewritten" -lt 7000 ] || [ "$fmt_refused" -lt 300 ] ||
        [ "$lint_files" -lt 1900 ] || [ "$lint_findings" -lt 900 ] ||
        [ "$test_props" -lt 6 ] || [ "$test_contracts" -lt 3 ] || [ "$test_cov" -lt 200 ] || [ "$test_failed" -lt 10 ] ||
        [ "$tm_runs" -lt 640 ] || [ "$tm_failed" -lt 11 ] || [ "$tm_host" -gt 4 ] ||
        [ "$bench_files" -lt 3 ] || [ "$bench_timed" -lt 6 ]; then
        echo "too little compared in fmt, lint, test or bench: $fmt_files files, $fmt_rewritten rewritten, $fmt_refused refused; $lint_files linted, $lint_findings findings; $test_props, $test_contracts, $test_cov, $test_failed failing; $tm_runs mains as tests, $tm_failed failing, $tm_host the host's; $bench_files with a benchmark, $bench_timed timing lines"
        exit 1
    fi
    # The floors of §6.20's sections, a margin below the first runs of the
    # whole corpus (2026-11-13 and 14, the stage-1 command): `doc` 1,426
    # documents written and 66,515 items, 71 refusals; `api` 1,132 programs
    # with a surface, each an item added, removed and changed, 66 refusals;
    # `bundle` 651 run alike, none differing in the `IoFailed` wording;
    # `source` 1,086 refusals (no `main`) and 66 run from the file.
    if [ "$doc_written" -lt 1300 ] || [ "$doc_items" -lt 60000 ] || [ "$doc_refused" -lt 60 ] ||
        [ "$api_surfaces" -lt 1050 ] || [ "$api_added" -lt 1050 ] || [ "$api_removed" -lt 1050 ] ||
        [ "$api_changed" -lt 1050 ] || [ "$api_refused" -lt 55 ] ||
        [ "$bundle_ran" -lt 640 ] || [ "$bundle_iofailed" -gt 0 ] ||
        [ "$source_refused" -lt 1000 ] || [ "$source_ran" -lt 60 ]; then
        echo "too little compared in doc, api, bundle or source: $doc_written documents, $doc_items items, $doc_refused refused; $api_surfaces surfaces, $api_added added, $api_removed removed, $api_changed changed, $api_refused refused; $bundle_ran bundles run alike, $bundle_iofailed in the IoFailed wording; $source_refused refused, $source_ran run"
        exit 1
    fi
    # The floors of §6.21's sections, a margin below the first whole run
    # (2026-11-15, the stage-1 command): `registry` 129 commands, 75 of them
    # refusals; `deps` 1,448 packages, 1,448 lockfiles written, 1,447
    # refusals (the first `--locked`, where no lockfile was); `add` 1,448;
    # `publish` 635 published.
    if [ "$reg_runs" -lt 120 ] || [ "$reg_refused" -lt 60 ] ||
        [ "$packages" -lt 1400 ] || [ "$deps_written" -lt 1400 ] || [ "$deps_refused" -lt 1400 ] ||
        [ "$add_same" -lt 1400 ] || [ "$pub_done" -lt 600 ]; then
        echo "too little compared in registry, deps, add or publish: $reg_runs registry commands, $reg_refused refused; $packages packages, $deps_written lockfiles written, $deps_refused refused; $add_same added; $pub_done published"
        exit 1
    fi
fi
exit 0
