# What the S6 gates share (docs/self-hosting.md §6.14): the Grasm compiler
# built in stages, each stage by the one before it.
#
# "The compiler" at this stage is the pipeline the gates already drive from a
# source file to a running program -- four programs, and what a stage holds:
#
#   front  compiler/front  source to TIR (`front tir`), and every other
#                          frontend command the S2 gates take
#   s3     compiler/s3     TIR to a Mach-O object; with `--runtime`, the
#                          runtime's object
#   link   compiler/link   objects to a signed executable
#   rt     compiler/rt     the runtime every program links, the object
#                          `s3 --runtime` makes -- with its poisoned build,
#                          `rt.poison`, which the use-after-free runs link
#
# and `tirinterp` (tools/tirinterp), which runs TIR rather than compiling it,
# built beside them because `frontsmoke` takes it. Not the `grasm` command:
# its subcommands are S8.
#
#   stage 1    front, s3, link and tirinterp, each built by the Rust
#              `grasm build --native --no-incremental` with GRASM_LINKER=cc:
#              Cranelift, the Rust runtime, the system linker. It has no
#              `rt`: its programs run on the Rust runtime.
#   stage N+1  every program built by stage N alone: stage N's `front`
#              makes its TIR, stage N's `s3` its object and the runtime's,
#              and stage N's `link` the executable, linked with that runtime
#              and nothing else (`-interpose`, no runtime archive on the
#              line) -- no Rust runtime, no `cc`, no `grasm`.
#
# A stage is a directory: `<name>` each executable, with the `<name>.tir`
# and `<name>.o` it was made from; `rt.tir`, `rt.o`, `rt.poison.tir` and
# `rt.poison.o`; a log per step (`<name>.<step>.log`); and `cost.tsv`, each
# step's wall time and peak memory. An object carries `$PWD`, DWARF's
# compile directory, and TIR the paths it was handed, so stages compare only
# when built from one directory, as every gate builds them: the checkout's
# root. The poisoned runtime is built from one copy of `compiler/rt`
# (`stage_poison_src`) that every stage of a run shares, for the same reason.
#
# Sourced from the checkout's root, with `GRASM` set.

# The executables, in the order a stage builds and compares them.
STAGE_EXES=(
    "front|compiler/front/src/main.gr"
    "s3|compiler/s3/src/main.gr"
    "link|compiler/link/src/main.gr"
    "tirinterp|tools/tirinterp/src/main.gr"
)
# The runtimes a stage after the first builds, before its executables, which
# link the first of them.
STAGE_RTS=(rt rt.poison)

# Every file a stage after the first must hold, in the order the gates
# compare them: each runtime's TIR and object, then each executable's TIR,
# object and executable.
stage_artifacts() {
    local r e n
    for r in "${STAGE_RTS[@]}"; do
        echo "$r.tir"
        echo "$r.o"
    done
    for e in "${STAGE_EXES[@]}"; do
        n=${e%%|*}
        echo "$n.tir"
        echo "$n.o"
        echo "$n"
    done
}

# A copy of compiler/rt in `$1` whose `build.gr` says POISON is true, as
# `scripts/lib/rtswap.sh` makes it; `STAGE_POISON` names it. Every stage of
# a run builds its `rt.poison` from this one copy.
stage_poison_src() {
    local dir=$1
    rm -rf "${dir:?}"
    cp -R compiler/rt "$dir" || return 1
    if [ "$(grep -c '^pub let POISON = false$' "$dir/src/build.gr")" != 1 ]; then
        echo "compiler/rt/src/build.gr does not say \`pub let POISON = false\` once"
        return 1
    fi
    sed -i '' 's/^pub let POISON = false$/pub let POISON = true/' "$dir/src/build.gr"
    STAGE_POISON=$dir
}

# Runs one step of a build -- `$1` the stage directory, `$2` the step's name,
# the rest the command, its output to `<dir>/<step>.log` -- and appends its
# wall time and peak memory to `<dir>/cost.tsv`. With `STAGE_OUT=<file>` set
# the command's stdout goes there instead. Fails as the command does.
stage_step() {
    local dir=$1 step=$2 rc
    shift 2
    if [ -n "${STAGE_OUT:-}" ]; then
        /usr/bin/time -l -o "$dir/$step.time" "$@" >"$STAGE_OUT" 2>"$dir/$step.log"
    else
        /usr/bin/time -l -o "$dir/$step.time" "$@" >"$dir/$step.log" 2>&1
    fi
    rc=$?
    awk -v s="$step" '/ real / { t = $1 } /maximum resident set size/ { m = $1 } END { printf "%s\t%s\t%d\n", s, t, m }' "$dir/$step.time" >>"$dir/cost.tsv"
    rm -f "$dir/$step.time"
    return $rc
}

# Says why step `$2` of stage directory `$1` failed, from its log.
stage_failed() {
    echo "error: $3"
    sed 's/^/  /' "$1/$2.log" | head -20
}

# Stage 1 into directory `$1`. Fails, saying why, when a program does not
# build.
stage_one() {
    local dir=$1 e n src
    rm -rf "${dir:?}"
    mkdir -p "$dir"
    for e in "${STAGE_EXES[@]}"; do
        n=${e%%|*}
        src=${e#*|}
        if ! GRASM_LINKER=cc stage_step "$dir" "$n.build" "$GRASM" build "$src" --native --no-incremental -o "$dir/$n" ||
            [ ! -x "$dir/$n" ]; then
            stage_failed "$dir" "$n.build" "stage 1's $n does not build: grasm build --native $src"
            return 1
        fi
    done
}

# One program through stage directory `$1`'s compiler into stage directory
# `$2`: `$3` the name, `$4` the entry, `$5` "runtime" for a runtime (an
# object, no executable). Fails, saying which step and why.
stage_program() {
    local from=$1 dir=$2 n=$3 src=$4 kind=$5 flag=""
    [ "$kind" = runtime ] && flag=--runtime
    if ! STAGE_OUT="$dir/$n.tir" stage_step "$dir" "$n.front" "$from/front" tir "$src"; then
        stage_failed "$dir" "$n.front" "$n: $from/front tir $src fails"
        return 1
    fi
    # shellcheck disable=SC2086
    if ! stage_step "$dir" "$n.s3" "$from/s3" "$dir/$n.tir" -o "$dir/$n.o" $flag; then
        stage_failed "$dir" "$n.s3" "$n: $from/s3 $flag does not compile its TIR"
        return 1
    fi
    [ "$kind" = runtime ] && return 0
    if ! stage_step "$dir" "$n.link" "$from/link" -interpose "$dir/rt.o" "$dir/$n.o" -dead_strip -o "$dir/$n" ||
        [ ! -x "$dir/$n" ]; then
        stage_failed "$dir" "$n.link" "$n: $from/link does not link its object with the runtime"
        return 1
    fi
}

# The stage after the one in directory `$1`, into directory `$2`: the
# runtimes first, then the executables, which link the runtime. Needs
# `STAGE_POISON` (`stage_poison_src`).
stage_next() {
    local from=$1 dir=$2 e
    if [ -z "${STAGE_POISON:-}" ] || [ ! -f "$STAGE_POISON/src/main.gr" ]; then
        echo "error: stage_next needs STAGE_POISON, the poisoned runtime's source (stage_poison_src)"
        return 1
    fi
    rm -rf "${dir:?}"
    mkdir -p "$dir"
    stage_program "$from" "$dir" rt compiler/rt/src/main.gr runtime || return 1
    stage_program "$from" "$dir" rt.poison "$STAGE_POISON/src/main.gr" runtime || return 1
    for e in "${STAGE_EXES[@]}"; do
        stage_program "$from" "$dir" "${e%%|*}" "${e#*|}" exe || return 1
    done
}

# A stage's cost from its `cost.tsv`: the wall time of its steps added up, in
# seconds, and the largest peak memory of any of them, in MB.
stage_cost() {
    awk -F'\t' '{ t += $2; if ($3 > m) m = $3 } END { printf "%.1f s, peak %.0f MB", t, m / 1048576 }' "$1/cost.tsv"
}
