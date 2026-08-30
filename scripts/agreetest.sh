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
# Overridable so a gate can be run against a debug build, where Cranelift
# keeps its own assertions on and the runtime checks integer overflow.
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
. "$(dirname "$0")/lib/trace.sh"
fresh_or_stop "./target/release/grasm"
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

# A package graph, which `check` cannot express: three packages in a chain,
# where the middle one imports both a sibling module of its own and its own
# dependency. The module names a build gives such a graph come from the
# manifests on disk, and a bundle does not carry those — so the bundled build
# of a program the other two ran correctly reported a module nobody wrote,
# and no gate here could see it.
check_chain() {
    local root="$WORK/chain"
    rm -rf "$root"
    mkdir -p "$root/c/src" "$root/b/src" "$root/a/src"
    cat >"$root/c/grasm.toml" <<'TOML'
[package]
name = "c"
version = "0.1.0"
capabilities = []
TOML
    cat >"$root/c/src/main.gr" <<'SRC'
pub fn triple(n: i64) -> i64 {
    n * 3
}
SRC
    cat >"$root/b/grasm.toml" <<'TOML'
[package]
name = "b"
version = "0.1.0"
capabilities = []

[dependencies.c]
path = "../c"
TOML
    cat >"$root/b/src/main.gr" <<'SRC'
import c
import helper

pub fn six(n: i64) -> i64 {
    helper.twice(c.triple(n))
}
SRC
    cat >"$root/b/src/helper.gr" <<'SRC'
pub fn twice(n: i64) -> i64 {
    n * 2
}
SRC
    cat >"$root/a/grasm.toml" <<'TOML'
[package]
name = "a"
version = "0.1.0"
capabilities = ["io"]

[dependencies.b]
path = "../b"
TOML
    cat >"$root/a/src/main.gr" <<'SRC'
import b

fn main() {
    print("{b.six(2)}")
}
SRC
    local f="$root/a/src/main.gr"
    local i_out n_out b_out
    if ! "$GRASM" build "$f" --native -o "$root/native" >/dev/null 2>&1         || ! "$GRASM" build "$f" -o "$root/bundle" >/dev/null 2>&1; then
        echo "  BUILD FAILED: package_chain"
        fail=$((fail + 1))
        FAILED+=("package_chain: build")
        return
    fi
    i_out=$("$GRASM" run "$f" 2>&1)
    n_out=$("$root/native" 2>&1)
    b_out=$("$root/bundle" 2>&1)
    if [ "$i_out" == "$n_out" ] && [ "$i_out" == "$b_out" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("package_chain")
        echo "  DISAGREE: package_chain"
        echo "    interp: $(tr '\n' '|' <<<"$i_out")"
        echo "    native: $(tr '\n' '|' <<<"$n_out")"
        echo "    bundle: $(tr '\n' '|' <<<"$b_out")"
    fi
}

echo "comparing interpreter / native / bundled ..."

check_chain

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

# Recursion that closes its cycle through a function value, not a name. The
# call graph cannot see it, so the compiled backend left the stack guard off
# and the program took SIGSEGV — exit 139, empty stderr.
check panic_indirect_overflow 'type Box = { f: fn(i64) -> i64 }

fn down(n: i64) -> i64 {
    let b = Box{ f: down }
    b.f(n + 1)
}

fn main() {
    print("{down(0)}")
}'

# Every other panic the library can raise, one program each.
#
# ProjectGold F31: `difftest.sh` compares stdout and the exit code, and a panic
# goes to stderr, so *this* is the only gate that checks a compiled build says
# the same words as the interpreter. It was doing that for six panic shapes and
# the language has fourteen. `remove` is here because F31 found its message
# unverified in exactly this way — its bounds check is redundant with
# `grasm_list_get`'s, so getting it wrong changes only what the panic says.

check panic_index_boundary 'fn main() {
    let xs = [1, 2]
    print("{xs[xs.len()]}")
}'

check panic_code_at 'fn main() {
    let s = "abc"
    print("{s.code_at(s.len())}")
}'

check panic_bytes_at 'fn main() {
    let b = bytes_of([1, 2, 3])
    print("{b.at(b.len())}")
}'

check panic_bytes_slice 'fn main() {
    let b = bytes_of([1, 2, 3])
    print("{b.slice(1, 9)}")
}'

check panic_remove 'fn main() {
    var xs = [1, 2]
    let gone = xs.remove(xs.len())
    print("{gone}")
}'

check panic_to_char 'fn main() {
    print("{1114112.to_char()}")
}'

check panic_to_base 'fn main() {
    print("{255.to_base(1)}")
}'

check panic_parse_base 'fn main() {
    print("{"ff".parse_int_base(99) ?? 0}")
}'

check panic_pow_negative 'fn main() {
    var e = 0 - 1
    print("{2 ^ e}")
}'

check panic_assert 'fn main() {
    assert_eq(2 + 2, 5)
}'

check panic_stale_ref 'type Cell = { n: i64 }

fn main() {
    let r = ref(Cell{ n: 1 })
    release(r)
    print("{r.n}")
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
# A **diamond**: two packages that both depend on a third.
#
# `check_chain` covers a chain, where every module is reached one way. A
# diamond is reached two ways, and the source bundle names each module by the
# path the loader happened to take first — so the package reached through `b`
# is written `b.d`, and `c`'s own plain `import d` matched nothing. The build
# reported success and wrote a bundle that could not be loaded.
#
# This is F81's failure in the case F81's fix did not reach. Gated as a shape
# rather than as the program that found it: any diamond does it.
check_diamond() {
    local root="$WORK/diamond"
    rm -rf "$root"
    mkdir -p "$root/d/src" "$root/b/src" "$root/c/src" "$root/a/src"
    cat >"$root/d/grasm.toml" <<'TOML'
[package]
name = "d"
version = "0.1.0"
capabilities = []
TOML
    cat >"$root/d/src/main.gr" <<'SRC'
pub fn shared(n: i64) -> i64 {
    n + 100
}
SRC
    for pkg in b c; do
        cat >"$root/$pkg/grasm.toml" <<TOML
[package]
name = "$pkg"
version = "0.1.0"
capabilities = []

[dependencies.d]
path = "../d"
TOML
        cat >"$root/$pkg/src/main.gr" <<SRC
import d

pub fn go(n: i64) -> i64 {
    d.shared(n)
}
SRC
    done
    cat >"$root/a/grasm.toml" <<'TOML'
[package]
name = "a"
version = "0.1.0"
capabilities = ["io"]

[dependencies.b]
path = "../b"

[dependencies.c]
path = "../c"
TOML
    cat >"$root/a/src/main.gr" <<'SRC'
import b
import c

fn main() / {io} {
    print("{b.go(1)} {c.go(2)}")
}
SRC
    local src="$root/a/src/main.gr"
    local i n bu bundle_out
    i=$("$GRASM" run "$src" 2>&1)
    "$GRASM" build "$src" --native -o "$WORK/diamond_native" >/dev/null 2>&1
    n=$("$WORK/diamond_native" 2>&1)
    "$GRASM" build "$src" -o "$WORK/diamond_bundle" >/dev/null 2>&1
    bu=$("$WORK/diamond_bundle" 2>&1)
    # And the source bundle, which is the one that was broken: a build that
    # says it wrote an artifact has to have written a loadable one.
    if "$GRASM" build "$src" --source -o "$WORK/diamond.bundle.gr" >/dev/null 2>&1; then
        bundle_out=$("$GRASM" run "$WORK/diamond.bundle.gr" 2>&1)
    else
        bundle_out="the source bundle would not build"
    fi
    if [ "$i" == "101 102" ] && [ "$n" == "101 102" ] && [ "$bu" == "101 102" ] \
        && [ "$bundle_out" == "101 102" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        FAILED+=("diamond_dependency")
        echo "  DISAGREE: diamond_dependency (want '101 102')"
        echo "    interp: $i"
        echo "    native: $n"
        echo "    bundle: $bu"
        echo "    source bundle: $bundle_out"
    fi
}

check_diamond

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

# A datagram carries its sender's address, and all three modes must agree on
# what that address is. Two sockets in one program, so nothing outside it has
# to cooperate and the answer is fixed: the loopback address, and the port the
# system chose for the sender, which the program reads back with `net_port`
# and compares rather than printing — a chosen port is different every run.
check udp_sender 'fn main() / {io, net} {
    let a = match udp_bind(0) {
        Ok(h) -> h
        Err(_e) -> return
    }
    let b = match udp_bind(0) {
        Ok(h) -> h
        Err(_e) -> return
    }
    let port_a = match net_port(a) {
        Ok(p) -> p
        Err(_e) -> return
    }
    let port_b = match net_port(b) {
        Ok(p) -> p
        Err(_e) -> return
    }
    match udp_send(a, "127.0.0.1", port_b, bytes_of([7, 8, 9])) {
        Ok(_n) -> {}
        Err(_e) -> return
    }
    match udp_recv_from(b) {
        Ok(got) -> {
            print("bytes {got.0.len()}")
            print("first {got.0.at(0)}")
            print("host {got.1}")
            print("port matches sender {got.2 == port_a}")
            print("ports differ {port_a != port_b}")
        }
        Err(_e) -> print("receive failed")
    }
    net_close(a)
    net_close(b)
}'

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
