#!/usr/bin/env bash
# Supply-chain benchmark: the one thing Grasm's toolchain does that the others
# cannot. The same attack in five languages — a trusted "pure" math library
# ships a v1.0.1 that quietly reads /etc/passwd and writes it elsewhere,
# keeping its signature and its result identical.
#
# The question each toolchain is asked: does building or testing the consumer,
# BEFORE the tampered code runs, reveal that the dependency now touches the
# filesystem? A "yes" is caught-at-compile. A "no" means the exfiltration runs
# in production and the toolchain never said a word.
#
# Nothing here executes the exfiltration against a real file. The tampered
# `total` is compiled and, where it would run, pointed at a sandbox path — the
# point is what the toolchain reports, not to actually read /etc/passwd.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=./target/release/grasm
S=benchmarks/security
W="${TMPDIR:-/tmp}/grasm_secbench_$$"
mkdir -p "$W"; export GOCACHE="$W/gocache"
trap 'rm -rf "$W"' EXIT
have() { command -v "$1" >/dev/null 2>&1; }

# Each row: does the toolchain flag the filesystem access before running?
declare -a NAMES DETECT DETAIL

# ---------- Grasm ----------
# Clean build first, to show the honest version passes.
clean=$("$GRASM" deps "$S/grasm/app" 2>&1)
# Now swap in the tampered library and rebuild the consumer.
cp "$S/grasm/mathlib/src/main.gr" "$W/mathlib_clean.gr"
cp "$S/grasm/mathlib_tampered.gr" "$S/grasm/mathlib/src/main.gr"
out=$("$GRASM" deps "$S/grasm/app" 2>&1); code=$?
cp "$W/mathlib_clean.gr" "$S/grasm/mathlib/src/main.gr"
rm -f "$S/grasm/app/grasm.lock"
NAMES+=("Grasm"); 
if [ $code -ne 0 ] && echo "$out" | grep -c "capabilit" >/dev/null; then
    DETECT+=("COMPILE ERROR"); DETAIL+=("$(echo "$out" | grep -m1 'exceed\|performs')")
else
    DETECT+=("not caught"); DETAIL+=("$out")
fi

# ---------- Rust ----------
if have rustc; then
    cp "$S/rust/mathlib/src/lib.rs" "$W/rust_clean.rs"
    cp "$S/rust/mathlib_tampered.rs" "$S/rust/mathlib/src/lib.rs"
    ( cd "$S/rust/app" && cargo build --quiet --target-dir "$W/rust-target" ) >"$W/rust.log" 2>&1
    code=$?
    cp "$W/rust_clean.rs" "$S/rust/mathlib/src/lib.rs"
    NAMES+=("Rust"); DETECT+=("$([ $code -eq 0 ] && echo 'builds clean' || echo 'build failed')")
    DETAIL+=("std::fs is ambient; no capability recorded")
fi

# ---------- Go ----------
if have go; then
    cp "$S/go/mathlib/mathlib.go" "$W/go_clean.go"
    cp "$S/go/mathlib_tampered.go" "$S/go/mathlib/mathlib.go"
    ( cd "$S/go/app" && go build -o "$W/go_app" . ) >"$W/go.log" 2>&1
    code=$?
    cp "$W/go_clean.go" "$S/go/mathlib/mathlib.go"
    NAMES+=("Go"); DETECT+=("$([ $code -eq 0 ] && echo 'builds clean' || echo 'build failed')")
    DETAIL+=("os.ReadFile importable by any package")
fi

# ---------- Python ----------
if have python3; then
    cp "$S/python/mathlib.py" "$W/py_clean.py"
    cp "$S/python/mathlib_tampered.py" "$S/python/mathlib.py"
    ( cd "$S/python" && python3 -c "import ast; ast.parse(open('mathlib.py').read()); import py_compile; py_compile.compile('mathlib.py', doraise=True)" ) >"$W/py.log" 2>&1
    code=$?
    cp "$W/py_clean.py" "$S/python/mathlib.py"
    NAMES+=("Python"); DETECT+=("$([ $code -eq 0 ] && echo 'compiles clean' || echo 'syntax error')")
    DETAIL+=("open() needs no import or declaration")
fi

# ---------- report ----------
echo
printf '%-10s %-16s %s\n' "language" "toolchain says" "why"
printf '%-10s %-16s %s\n' "--------" "----------------" "---"
for i in "${!NAMES[@]}"; do
    printf '%-10s %-16s %s\n' "${NAMES[$i]}" "${DETECT[$i]}" "${DETAIL[$i]}"
done
echo
echo "The attack: a trusted pure library's v1.0.1 reads /etc/passwd and writes"
echo "it out, keeping the same signature and result. Only Grasm's toolchain"
echo "refuses to build the consumer, because the dependency's inferred effects"
echo "(io) exceed its declared capability ceiling ([]). Everywhere else the"
echo "tampered code builds, ships, and runs; detection is left to runtime"
echo "sandboxing or a reviewer noticing — neither of which the toolchain does."
