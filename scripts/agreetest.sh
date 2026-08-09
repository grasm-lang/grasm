#!/usr/bin/env bash
# Do the three ways of running a program agree?
#
# `difftest.sh` runs every program with no arguments and compares stdout and
# the exit code. That leaves three things it cannot see, and a cold model
# found two of them by writing an ordinary command-line tool:
#
#   - `args()`, because difftest never passes any
#   - the exit code and message of a panic, in a *compiled* binary
#   - `grasm build` without `--native`, the bundled executable, which difftest
#     does not exercise at all
#
# So this compares all three modes — interpreter, native, bundled — on
# programs written to depend on exactly those.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
WORK="${TMPDIR:-/tmp}/grasm_agree_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

pass=0
fail=0
declare -a FAILED=()

# check <name> <source> <args...>
check() {
    local name="$1" src="$2"
    shift 2
    local f="$WORK/$name.gr"
    printf '%s\n' "$src" >"$f"

    if ! "$GRASM" build "$f" --native -o "$WORK/${name}_native" >/dev/null 2>&1; then
        echo "  BUILD FAILED (native): $name"
        fail=$((fail + 1))
        FAILED+=("$name: native build")
        return
    fi
    if ! "$GRASM" build "$f" -o "$WORK/${name}_bundle" >/dev/null 2>&1; then
        echo "  BUILD FAILED (bundle): $name"
        fail=$((fail + 1))
        FAILED+=("$name: bundle build")
        return
    fi

    # The interpreter takes the program's arguments after `--`; a compiled
    # binary takes them directly. That is the same program input, spelled the
    # way each mode spells it.
    local i_out i_code n_out n_code b_out b_code
    i_out=$("$GRASM" run "$f" -- "$@" 2>&1)
    i_code=$?
    n_out=$("$WORK/${name}_native" "$@" 2>&1)
    n_code=$?
    b_out=$("$WORK/${name}_bundle" "$@" 2>&1)
    b_code=$?

    # A panic's stack trace comes from the interpreter, and `grasm build`
    # without `--native` *is* the interpreter with the program attached — so
    # both of those print one and the compiled binary does not. That is a
    # documented difference in what the two build modes are, not a
    # disagreement about what the program does, so the trace block is excluded
    # from both. Everything else, including the panic message and the exit
    # code, has to match exactly.
    strip_trace() {
        grep -v \
            -e '^stack trace (innermost first):$' \
            -e '^  [A-Za-z_][A-Za-z0-9_:]* at ' \
            -e '^  \.\.\. [0-9]* more frame(s)$' <<<"$1"
    }
    i_out=$(strip_trace "$i_out")
    b_out=$(strip_trace "$b_out")

    if [ "$i_out" == "$n_out" ] && [ "$i_out" == "$b_out" ] \
        && [ "$i_code" == "$n_code" ] && [ "$i_code" == "$b_code" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("$name")
        echo "  DISAGREE: $name"
        echo "    interp ($i_code): $(tr '\n' '|' <<<"$i_out")"
        echo "    native ($n_code): $(tr '\n' '|' <<<"$n_out")"
        echo "    bundle ($b_code): $(tr '\n' '|' <<<"$b_out")"
    fi
}

echo "comparing interpreter / native / bundled ..."

check args_none 'fn main() {
    print("{args()}")
}'

check args_two 'fn main() {
    print("{args()}")
}' one two

check args_literal_dashes 'fn main() {
    print("{args()}")
}' -- x

check args_used 'fn main() {
    var total = 0
    for a in args() {
        total = total + a.len()
    }
    print("{args().len()} {total}")
}' alpha beta gamma

check args_empty_string 'fn main() {
    print("{args()}")
}' "" x

check panic_index 'fn main() {
    let xs = [1, 2]
    print("{xs[9]}")
}'

check panic_message 'fn main() {
    let n = 7
    panic("value {n} is out of range")
}'

check panic_divide 'fn main() {
    var z = 0
    print("{10 / z}")
}'

# A panic partway along an interpolated line. Emitting pieces as they were
# computed left the first half of the line on stdout, which [E-PANIC-EXIT]
# says never happens. difftest cannot see it: it compares stdout only for
# programs that finish.
check panic_mid_line 'fn boom(n: i64) -> i64 {
    var z = 0
    n / z
}

fn main() {
    print("before")
    print("a=1 b={boom(2)} c=3")
    print("after")
}'

# The panic *line* is identical in all three modes even where the trace after
# it is not, and even where the engines stop at different depths. The
# interpreter used to name its own bound here, so one failure read as two.
check panic_stack_overflow 'fn down(n: i64) -> i64 {
    down(n + 1)
}

fn main() {
    print("{down(0)}")
}'

# Infinite recursion inside a `par_map` worker. Natively this was SIGBUS,
# exit 138, with nothing on stderr — the worker's stack was smaller than the
# budget the guard assumed, so the guard never fired.
check panic_parallel_overflow 'fn down(n: i64) -> i64 {
    down(n + 1)
}

fn main() {
    var xs: [i64] = []
    for i in 0..600 {
        xs.push(i)
    }
    print("{xs.par_map(|x| down(x)).len()}")
}'

check exit_code 'fn main() {
    print("before")
    exit(3)
}'

check main_error 'type E = { msg: str }

fn boom() -> i64 ! E {
    fail(E{ msg: "no" })
}

fn main() {
    let v = try boom()
    print("{v}")
}'

# A *package* with a dependency, run all three ways.
#
# `check` above writes one file, and this class of bug needs two packages: a
# dependency whose own `import util` has to resolve inside the dependency. It
# did not, in the bundler — the bundle reloads its sources at startup and had
# no record of which modules were packages, so the library's `util` was
# resolved as a bare name and found the *application's*. The bundled program
# printed `3000 3000` where every other way of running it printed `6 3000`.
# Third time for this class: the main loader, then the doc-example harness,
# now the bundler.
check_package() {
    local root="$WORK/pkgcase"
    rm -rf "$root"
    mkdir -p "$root/dep/src" "$root/app/src"
    cat >"$root/dep/grasm.toml" <<'EOF'
[package]
name = "dep"
version = "0.1.0"
entry = "src/main.gr"
capabilities = []
EOF
    printf 'import util

pub fn twice(n: i64) -> i64 {
    util.scale(n)
}
'         >"$root/dep/src/main.gr"
    printf 'pub fn scale(n: i64) -> i64 {
    n * 2
}
' >"$root/dep/src/util.gr"
    cat >"$root/app/grasm.toml" <<'EOF'
[package]
name = "app"
version = "0.1.0"
entry = "src/main.gr"
capabilities = ["io"]

[dependencies.dep]
path = "../dep"
version = "^0.1.0"
EOF
    # Same module name as the dependency's private one, different answer.
    printf 'pub fn scale(n: i64) -> i64 {
    n * 1000
}
' >"$root/app/src/util.gr"
    printf 'import dep
import util

fn main() {
    print("{dep.twice(3)} {util.scale(3)}")
}
'         >"$root/app/src/main.gr"

    local entry="$root/app/src/main.gr"
    if ! "$GRASM" build "$entry" --native -o "$root/native" >/dev/null 2>&1         || ! "$GRASM" build "$entry" -o "$root/bundle" >/dev/null 2>&1; then
        echo "  BUILD FAILED: package_dependency"
        fail=$((fail + 1))
        FAILED+=("package_dependency: build")
        return
    fi
    local i n b
    i=$("$GRASM" run "$entry" 2>&1)
    n=$("$root/native" 2>&1)
    b=$("$root/bundle" 2>&1)
    if [ "$i" == "$n" ] && [ "$i" == "$b" ] && [ "$i" == "6 3000" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("package_dependency")
        echo "  DISAGREE: package_dependency (want '6 3000')"
        echo "    interp: $i"
        echo "    native: $n"
        echo "    bundle: $b"
    fi
}

check_package

echo
echo "=========================================="
echo "agreeing: $pass   disagreeing: $fail"
echo "=========================================="

if [ "$fail" -gt 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
exit 0
