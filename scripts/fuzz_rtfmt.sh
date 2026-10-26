#!/usr/bin/env bash
# Numbers formatted by the Grasm runtime must be the Rust runtime's, byte for
# byte (docs/self-hosting.md §6.7): integers, booleans, and above all
# doubles, which Rust writes as the shortest digits that read back as the
# same double.
#
# `compiler/rt/probe/fmtfuzz.gr` draws doubles where printing them goes
# wrong (any 64 bits, powers of two and their neighbours, subnormals,
# integers near 2^53, short decimals, values next to powers of ten) and
# integers likewise, and writes each through `print`, interpolation and
# `to_str`. It is built twice -- linked as it ships, and with the Grasm
# runtime interposed (`scripts/lib/rtswap.sh`) -- and each chunk of draws
# must give the same bytes and exit status from both. The swapped build's
# doubles are also held to Python's `repr` digits, laid out as Rust lays
# them out: a third witness, so that a difference the two runtimes share is
# seen too, and named rather than copied.
#
#   RTFMT_CHUNKS (20) x RTFMT_COUNT (50000) draws, from RTFMT_SEED (1) up;
#   RTFMT_PY (20000) of them against Python.
# `RT_OBJ=`, `S3=` and `LINK=` as for `scripts/rtswap.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver
rtswap_runtime
rtswap_families
rtswap_linker

CHUNKS=${RTFMT_CHUNKS:-20}
COUNT=${RTFMT_COUNT:-50000}
SEED=${RTFMT_SEED:-1}
PYN=${RTFMT_PY:-20000}

if ! "$GRASM" build compiler/rt/probe/fmtfuzz.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/fmtfuzz.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
taken=$(grep -c '^grasm ' "$WORK/f.swaplog")
fell=$(awk '$1 == "rust" { print $2 }' "$WORK/f.swaplog" | LC_ALL=C sort | LC_ALL=C comm -12 - "$WORK/moved.txt" | tr '\n' ' ')
if [ -n "$fell" ]; then
    echo "FAIL: the swapped build took moved names from the Rust runtime: $fell"
    exit 1
fi
for need in out_f64 str_build_f64 str_from_f64 out_i64 out_u64 str_build_i64 str_from_i64 str_from_u64 out_bool str_build_bool; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing there"
        exit 1
    fi
done

same=0
k=0
while [ "$k" -lt "$CHUNKS" ]; do
    seed=$((SEED + k))
    a=$("$WORK/f" "$seed" "$COUNT" </dev/null 2>"$WORK/a.err" | shasum -a 256)
    ac=${PIPESTATUS[0]}
    b=$("$WORK/f.swap" "$seed" "$COUNT" </dev/null 2>"$WORK/b.err" | shasum -a 256)
    bc=${PIPESTATUS[0]}
    if [ "$a" != "$b" ] || [ "$ac" != "$bc" ] || ! cmp -s "$WORK/a.err" "$WORK/b.err"; then
        echo "FAIL: seed $seed: the two runtimes differ (exit $ac and $bc)"
        "$WORK/f" "$seed" "$COUNT" >"$WORK/a.out" 2>/dev/null </dev/null
        "$WORK/f.swap" "$seed" "$COUNT" >"$WORK/b.out" 2>/dev/null </dev/null
        diff "$WORK/a.out" "$WORK/b.out" | head -8 | cut -c1-200 | sed 's/^/    /'
        exit 1
    fi
    same=$((same + 1))
    k=$((k + 1))
done

# Python's shortest digits, laid out as Rust's `{}` lays out shortest
# digits, against the swapped build.
"$WORK/f.swap" "$SEED" "$PYN" bits >"$WORK/bits.txt" </dev/null
python3 - "$WORK/bits.txt" <<'EOF'
import decimal
import fractions
import struct
import sys

def rust_display(x):
    if x != x:
        return "NaN"
    if x in (float("inf"), float("-inf")):
        return "inf" if x > 0 else "-inf"
    sign = "-" if struct.pack(">d", x)[0] & 0x80 else ""
    if x == 0:
        return sign + "0.0"
    d = decimal.Decimal(repr(abs(x))).normalize()
    _, digits, exp = d.as_tuple()
    ds = "".join(map(str, digits))
    k = len(ds) + exp
    if k <= 0:
        return sign + "0." + "0" * (-k) + ds
    if k < len(ds):
        return sign + ds[:k] + "." + ds[k:]
    return sign + ds + "0" * (k - len(ds)) + ".0"

def digits_of(text):
    """The decimal digits of a positional rendering, and its value."""
    t = text.lstrip("-")
    return t.replace(".", "").lstrip("0").rstrip("0"), fractions.Fraction(decimal.Decimal(text))

# One difference is Rust's and is expected: when the double's exact value
# lies exactly halfway between two candidates of the fewest digits, Rust's
# `flt2dec` takes the one farther from zero, and Python the even one.
# [L-DISPLAY] asks for the fewest digits and does not say which; each such
# case is checked to be exactly that, and anything else fails.
n = 0
ties = 0
bad = []
for line in open(sys.argv[1]):
    b, text = line.split()
    x = struct.unpack("<d", struct.pack("<q", int(b)))[0]
    want = rust_display(x)
    n += 1
    if text == want:
        continue
    if x == x and abs(x) != float("inf") and float(text) == x:
        ours, ov = digits_of(text)
        theirs, tv = digits_of(want)
        exact = fractions.Fraction(x)
        if len(ours) == len(theirs) and abs(ov) > abs(tv) and ov - exact == exact - tv:
            ties += 1
            continue
    bad.append((b, text, want))
for b, text, want in bad[:10]:
    print(f"  bits {int(b) & ((1 << 64) - 1):016x}: runtimes {text[:60]}, Python {want[:60]}")
print(f"against Python: {n} doubles, {n - ties - len(bad)} the same, {ties} exact ties Rust breaks away from zero, {len(bad)} differing otherwise")
sys.exit(1 if bad or n == 0 else 0)
EOF
py=$?
draws=$((CHUNKS * COUNT))
echo "chunks identical: $same of $CHUNKS ($draws draws: $((draws * 4)) doubles and $((draws * 6)) integers formatted, each on both runtimes); names taken from Grasm: $taken"
[ "$py" = 0 ] && [ "$same" -eq "$CHUNKS" ] && [ "$CHUNKS" -ge 1 ]
