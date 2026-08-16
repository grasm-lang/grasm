#!/usr/bin/env bash
# The MASTERPLAN §2.3 north-star goals, measured.
#
# Four of them had never been measured at all — not missed, *unmeasured*, which
# is worse, because a number nobody takes cannot be argued with or improved. G3
# in particular was found to be violated only by accident, when an evaluator
# happened to time a chain of field accesses and found the checker taking
# exponential time.
#
# This prints the scorecard. It does not fail on a miss: three of these are
# missed today and a gate that is red on purpose is a gate people learn to
# ignore. It fails when a measurement cannot be taken at all, because that is
# how a metric quietly stops being collected.
set -uo pipefail
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

WORK="${TMPDIR:-/tmp}/grasm_northstar_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

ms_now() { python3 -c 'import time;print(int(time.time()*1000))'; }

printf '%-4s %-34s %-22s %s\n' "goal" "what" "target" "measured"
printf '%-4s %-34s %-22s %s\n' "----" "----------------------------------" "----------------------" "--------"

# ---------------------------------------------------------------- G3: speed
# A large, ordinary program: many small functions, structs and matches, which
# is the shape real code has. A pathological one would measure the wrong thing.
python3 - "$WORK" <<'PY'
import sys
work = sys.argv[1]
parts = ["type Shape = | Circle(r: f64) | Rect(w: f64, h: f64)\n",
         "type P = { x: i64, y: i64, tag: str }\n"]
for i in range(1, 4001):
    parts.append(f"""
fn f{i}(a: i64, b: i64) -> i64 {{
    let c = a + b * {i % 7 + 1}
    if c > 100 {{
        return c - 100
    }}
    c
}}

fn g{i}(s: Shape) -> f64 {{
    match s {{
        Circle(r) -> r * r * 3.14
        Rect(w, h) -> w * h
    }}
}}

fn h{i}(p: P) -> str {{
    "{{p.tag}}:{{p.x + p.y}}"
}}
""")
parts.append('\nfn main() {\n    print("{f1(1, 2)}")\n}\n')
open(f"{work}/big.gr", "w").write("".join(parts))
PY

lines=$(wc -l < "$WORK/big.gr")
# The process floor, so throughput is the frontend's and not the loader's.
t0=$(ms_now); "$GRASM" version >/dev/null 2>&1; floor=$(( $(ms_now) - t0 ))
t0=$(ms_now); "$GRASM" check "$WORK/big.gr" >/dev/null 2>&1; rc=$?; total=$(( $(ms_now) - t0 ))
if [ $rc -ne 0 ]; then
    echo "error: the generated program does not check — the measurement is meaningless"
    exit 1
fi
work_ms=$(( total - floor )); [ $work_ms -lt 1 ] && work_ms=1
locs=$(( lines * 1000 / work_ms ))
printf '%-4s %-34s %-22s %s\n' "G3" "frontend throughput" ">= 500,000 LOC/s" "$(printf "%'d" $locs) LOC/s   ($lines lines in ${work_ms}ms)"
printf '%-4s %-34s %-22s %s\n' "" "incremental edit-to-diagnostic" "<= 50 ms" "not built (no query engine)"

# ---------------------------------------------------------------- G5: size
printf 'fn main() {\n    print("hello")\n}\n' > "$WORK/hello.gr"
if "$GRASM" build "$WORK/hello.gr" --native -o "$WORK/hello" >/dev/null 2>&1; then
    kb=$(( $(wc -c < "$WORK/hello") / 1024 ))
    printf '%-4s %-34s %-22s %s\n' "G5" "hello-world static binary" "<= 300 KB" "${kb} KB"
else
    echo "error: could not build a hello-world binary"
    exit 1
fi

# ---------------------------------------------------------------- G11: concision
python3 - <<'PY'
import os, re
progs = ["fib", "sieve", "matmul", "wordfreq", "sort", "parmap"]

def toks(path):
    if not os.path.exists(path):
        return None
    s = open(path).read()
    s = re.sub(r"//.*|#.*", "", s)
    return len(re.findall(r"[A-Za-z_]\w*|\d+\.?\d*|[^\sA-Za-z_0-9]", s))

g = p = o = 0
for name in progs:
    a, b, c = (toks(f"benchmarks/grasm/{name}.gr"),
               toks(f"benchmarks/python/{name}.py"),
               toks(f"benchmarks/go/{name}.go"))
    if a and b and c:
        g += a; p += b; o += c
if not (g and p and o):
    raise SystemExit("error: the benchmark programs needed for G11 are missing")
print(f"{'G11':<4} {'tokens vs Python':<34} {'<= 1.00x':<22} {g/p:.2f}x")
print(f"{'':<4} {'tokens vs Go':<34} {'<= 0.60x':<22} {g/o:.2f}x")
PY

# ---------------------------------------------------------------- G1
printf '%-4s %-34s %-22s %s\n' "G1" "time for a Go/TS dev to ship" "<= 1 week" "unmeasurable without users"

echo
echo "G2 (performance vs Rust) is measured by scripts/bench.sh, which refuses to"
echo "run on a loaded machine; G4 (safety) by difftest, leakcheck and the row"
echo "and extern fuzzers; G7 (capability sandboxing) by security_bench.sh and"
echo "the package tests. G6, G8, G9 and G10 have no measurement here because"
echo "the features they measure are not built."
