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
# Timed inside one process. Shelling out for each timestamp put the cost of
# starting that helper *inside* the interval being measured — about 25 ms, or a
# sixth of the figure, which is how a measurement quietly becomes a measurement
# of itself. Best of five, because the number is meant to describe the compiler
# rather than whatever else the machine is doing.
read -r work_ms rc <<EOF
$(python3 - "$GRASM" "$WORK/big.gr" <<'PYEOF'
import subprocess, sys, time
grasm, path = sys.argv[1], sys.argv[2]
def once(args):
    t = time.perf_counter()
    rc = subprocess.run([grasm, *args], stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL).returncode
    return (time.perf_counter() - t) * 1000, rc
floor = min(once(["version"])[0] for _ in range(3))
runs = [once(["check", path]) for _ in range(5)]
best = min(t for t, _ in runs)
rc = max(r for _, r in runs)
print(int(max(best - floor, 1)), rc)
PYEOF
)
EOF
if [ "${rc:-1}" -ne 0 ]; then
    echo "error: the generated program does not check — the measurement is meaningless"
    exit 1
fi
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
# Each language counted by its *own* lexer. A regular expression cannot do this
# fairly: Python's grammar has INDENT, DEDENT and NEWLINE tokens and Go's
# inserts semicolons, so a regex over punctuation gives both of those away free
# and charges Grasm for every brace. Measured that way Grasm read 1.22x of
# Python; measured properly it reads 1.04x. The difference was the ruler.
GOTOK=""
if command -v go >/dev/null 2>&1; then
    # Built in a scratch module of its own: this repository is not a Go module,
    # and `go build` refuses a bare file inside one that is not.
    mkdir -p "$WORK/gotok"
    cp scripts/gotok/main.go "$WORK/gotok/main.go"
    if (cd "$WORK/gotok" && go mod init gotok >/dev/null 2>&1 \
        && go build -o gotok main.go >/dev/null 2>&1); then
        GOTOK="$WORK/gotok/gotok"
    fi
fi
GRASM="$GRASM" GOTOK="$GOTOK" python3 - <<'G11PY'
import io, os, subprocess, tokenize

grasm = os.environ["GRASM"]
gotok = os.environ.get("GOTOK") or None
progs = ["fib", "sieve", "matmul", "wordfreq", "sort", "parmap"]

def grasm_tokens(path):
    out = subprocess.run([grasm, "tokens", path], capture_output=True, text=True).stdout
    return sum(1 for l in out.splitlines() if l.strip() and not l.endswith("Eof"))

def python_tokens(path):
    skip = {tokenize.ENCODING, tokenize.ENDMARKER, tokenize.NL, tokenize.COMMENT}
    with open(path, "rb") as f:
        return sum(1 for t in tokenize.tokenize(f.readline) if t.type not in skip)

def go_tokens(path):
    if not gotok:
        return None
    out = subprocess.run([gotok, path], capture_output=True, text=True).stdout.strip()
    return int(out) if out.isdigit() else None

g = p = o = 0
for name in progs:
    a = grasm_tokens(f"benchmarks/grasm/{name}.gr")
    b = python_tokens(f"benchmarks/python/{name}.py")
    c = go_tokens(f"benchmarks/go/{name}.go")
    if not (a and b):
        raise SystemExit("error: the benchmark programs needed for G11 are missing")
    g += a
    p += b
    if c:
        o += c
print(f"{'G11':<4} {'tokens vs Python':<34} {'<= 1.00x':<22} {g/p:.2f}x")
if o:
    print(f"{'':<4} {'tokens vs Go':<34} {'<= 0.60x':<22} {g/o:.2f}x")
else:
    print(f"{'':<4} {'tokens vs Go':<34} {'<= 0.60x':<22} skipped (no Go toolchain)")
G11PY

# ---------------------------------------------------------------- G1
printf '%-4s %-34s %-22s %s\n' "G1" "time for a Go/TS dev to ship" "<= 1 week" "unmeasurable without users"

echo
echo "G2 (performance vs Rust) is measured by scripts/bench.sh, which refuses to"
echo "run on a loaded machine; G4 (safety) by difftest, leakcheck and the row"
echo "and extern fuzzers; G7 (capability sandboxing) by security_bench.sh and"
echo "the package tests; G6 (parallelism) partly, by the parmap and parwords"
echo "rows of scripts/bench.sh — parwords is the one that shares values rather"
echo "than words. G8, G9 and G10 have no measurement here because the features"
echo "they measure are not built."
