#!/usr/bin/env bash
# The `grasm` command in Grasm against the Rust one (docs/self-hosting.md
# §6.17, S8 part one): the same stdout, stderr, exit status and files.
#
# `compiler/grasm` runs `version`, `tokens`, `ast`, `toml`, `check`, `tir`,
# `run`, `build --native` and `links` with the Grasm toolchain's own code, and
# reads every command line as the Rust command reads it. Six sections, each
# both commands on the same input from the same directory:
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
#   target  `build --native --target x86_64-apple-darwin` of every tenth of
#           those: the same output, an x86-64 Mach-O object, made twice the
#           same
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
ONLY=${ONLY:-lines,corpus,toml,run,build,target}
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
                [[ "$(cat "$WORK/l/g.err")" == "error: unsupported target \`"*"\`: the Grasm backend writes Mach-O objects for x86_64 and aarch64 macOS only" ]]; then
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
            rm -f "$WORK/t/r/p.o" "$WORK/t/g/p.o" "$WORK/t/g2/p.o"
            "$GRASM" build "$src" --native --target x86_64-apple-darwin -o "$WORK/t/r/p.o" >"$WORK/r.out" 2>"$WORK/r.err" </dev/null
            r_rc=$?
            g build "$src" --native --target x86_64-apple-darwin -o "$WORK/t/g/p.o" >"$WORK/g.out" 2>"$WORK/g.err" </dev/null
            g_rc=$?
            g build "$src" --native --target x86_64-apple-darwin -o "$WORK/t/g2/p.o" >/dev/null 2>&1 </dev/null
            sed "s|$WORK/t/r/|$WORK/t/g/|g" "$WORK/r.out" >"$WORK/r.norm"
            header=$(head -c 8 "$WORK/t/g/p.o" 2>/dev/null | od -An -tx1 | tr -d ' \n')
            if cmp -s "$WORK/r.norm" "$WORK/g.out" && cmp -s "$WORK/r.err" "$WORK/g.err" && [ "$r_rc" = "$g_rc" ] &&
                { [ "$g_rc" != 0 ] || { [ "$header" = cffaedfe07000001 ] && cmp -s "$WORK/t/g/p.o" "$WORK/t/g2/p.o"; }; }; then
                target_same=$((target_same + 1))
            else
                fail "grasm build --native --target x86_64-apple-darwin $src -- exit $r_rc (Rust) and $g_rc (Grasm), object header $header"
                diff "$WORK/r.norm" "$WORK/g.out" | head -4 | sed 's/^/  /'
            fi
        fi
    done <"$WORK/mains"
    if want build; then
        echo "build: $builds programs with a main ($external link a system library)   the same output: $built_same, $build_refused of them refusals   run alike: $ran_same   built twice the same: $repro"
    fi
    if want target; then
        echo "target: $targets programs for x86_64-apple-darwin   the same output, an x86-64 object made twice the same: $target_same"
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
whole=$([ -z "${FILES:-}" ] && [ "$ONLY" = lines,corpus,toml,run,build,target ] && echo 1)
if want lines && [ $((lines_same + lines_elsewhere + lines_runtime + lines_target)) -ne "$lines" ]; then
    echo "lines: the counts do not add up to $lines"
    exit 1
fi
if want lines && { [ "$lines" -lt 140 ] || [ "$lines_elsewhere" -lt 15 ] || [ "$lines_runtime" != 1 ] || [ "$lines_target" != 3 ]; }; then
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
if want build && { [ "$built_same" -ne "$builds" ] || [ "$ran_same" -ne $((builds - build_refused)) ] || [ "$repro" -ne "$ran_same" ] ||
    [ $((builds + external)) -ne "$mains" ]; }; then
    echo "build: the counts do not add up ($built_same, $ran_same, $repro of $builds; $external external of $mains)"
    exit 1
fi
if [ -n "$whole" ]; then
    if [ "$listed" -lt 1700 ] || [ "$corpus_refused" -lt 120 ] || [ "$tomls" -lt 600 ] ||
        [ "$run_same" -lt 640 ] || [ "$ran_same" -lt 640 ] || [ "$targets" -lt 60 ] || [ "$target_same" -ne "$targets" ]; then
        echo "too little compared for this to mean anything: $listed programs, $corpus_refused refusals, $tomls manifests, $run_same runs, $ran_same builds run, $target_same of $targets targets"
        exit 1
    fi
fi
exit 0
