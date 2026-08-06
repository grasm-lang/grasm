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
        grep -v -e '^stack trace (innermost first):$' -e '^  [A-Za-z_][A-Za-z0-9_:]* at ' <<<"$1"
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

echo
echo "=========================================="
echo "agreeing: $pass   disagreeing: $fail"
echo "=========================================="

if [ "$fail" -gt 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
exit 0
