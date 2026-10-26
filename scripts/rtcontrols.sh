#!/usr/bin/env bash
# The S5 gates must fail on a planted fault (docs/self-hosting.md §6.7).
#
# Each control copies `compiler/rt`, changes one thing in the copy, builds it
# with the Grasm backend as the gates build the real one, and runs the gate
# that should catch it against it (`RT_OBJ=`, `RT_LAYOUT=`), on the programs
# that reach the fault. The gate must fail, and for the reason the control
# names -- a gate that fails for some other reason has not shown it can see
# this one.
#
#   rc-twice        `dec` takes two from a count instead of one
#   float-short     a double is written one digit short
#   nan-min         `min` answering NaN for a signalling NaN, where Rust's
#                   answers the other operand (the fault `fuzz_rtmath` found)
#   strlen-off      a string value made one byte longer than its text
#   strlen-count    `len` one more than the characters of short ASCII text
#   panic-exit-1    a panic exits 1 instead of 101
#   layout-moved    a layout field moved: the inline length one byte on
#   fallback-gone   a moved function no longer exported, so programs take
#                   Rust's again
#   fallback-call   a moved function calling Rust's own instead of working
#
# `RT_CONTROLS=<regex>` runs only the controls it matches. `RT_POISON=` as
# for `scripts/rtswap.sh`.
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
s3_driver
link_driver
export S3 LINK GRASM

caught=0
missed=0
ran=0
declare -a MISSED=()

# Copies compiler/rt to $WORK/$1 and replaces the text $3 with $4 in its
# file $2 (exactly once), then builds the copy's object, $WORK/$1.o.
plant() {
    local name=$1 file=$2
    rm -rf "${WORK:?}/$name"
    cp -R compiler/rt "$WORK/$name"
    if ! python3 - "$WORK/$name/$file" "$3" "$4" <<'EOF'; then
import sys
path, old, new = sys.argv[1:4]
text = open(path).read()
if text.count(old) != 1:
    print(f"the control's text occurs {text.count(old)} times in {path}")
    sys.exit(1)
open(path, "w").write(text.replace(old, new))
EOF
        return 1
    fi
    "$GRASM" tir "$WORK/$name/src/main.gr" >"$WORK/$name.tir" 2>"$WORK/$name.log" &&
        "$S3" "$WORK/$name.tir" -o "$WORK/$name.o" --runtime >>"$WORK/$name.log" 2>&1
}

# Runs `$3...` (a gate, with the planted object in RT_OBJ) and requires it to
# fail with output matching `$2`.
expect() {
    local name=$1 why=$2
    shift 2
    ran=$((ran + 1))
    if RT_OBJ="$WORK/$name.o" "$@" >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: $* passed")
        echo "MISSED: $name ($*)"
    elif grep -qE "$why" "$WORK/$name.out"; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-150)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: $* failed, but not for its reason: $(tail -2 "$WORK/$name.out" | tr '\n' ' ' | cut -c1-200)")
        echo "MISSED: $name failed for another reason"
    fi
}

control() {
    [ -z "${RT_CONTROLS:-}" ] || [[ "$1" =~ $RT_CONTROLS ]]
}

if control rc-twice; then
    plant rc-twice src/val.gr 'v.atomic_add_i64(-1, Release) - 1 > 0' 'v.atomic_add_i64(-2, Release) - 2 > 0' ||
        { echo "error: rc-twice does not build"; exit 1; }
    expect rc-twice 'FAIL: ' env RT_S3=0 RT_PROGRAMS='^examples/' bash scripts/rtswap.sh
fi

if control float-short; then
    plant float-short src/fmt.gr '    let count = shortest(mant, minus, plus, exp, inclusive, digits, k)' \
        '    let full = shortest(mant, minus, plus, exp, inclusive, digits, k)
    let count = if full > 1 { full - 1 } else { full }' ||
        { echo "error: float-short does not build"; exit 1; }
    expect float-short 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control nan-min; then
    plant nan-min src/math.gr 'unsafe { fmin(quiet(a), quiet(b)) }' 'unsafe { fmin(a, b) }' ||
        { echo "error: nan-min does not build"; exit 1; }
    expect nan-min 'the two runtimes differ' env RTMATH_CHUNKS=1 RTMATH_COUNT=20000 bash scripts/fuzz_rtmath.sh
fi

if control strlen-off; then
    plant strlen-off src/numstr.gr 'v.offset(layout.STR_LEN).store_u8(n.as_u8())' 'v.offset(layout.STR_LEN).store_u8(n.as_u8() + 1)' ||
        { echo "error: strlen-off does not build"; exit 1; }
    expect strlen-off 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control strlen-count; then
    plant strlen-count src/strread.gr 'return if ascii(p, n) { n } else { count_chars(p, n) }' 'return if ascii(p, n) { n + 1 } else { count_chars(p, n) }' ||
        { echo "error: strlen-count does not build"; exit 1; }
    expect strlen-count 'FAIL: spec/conformance/(E-STR|L-LIB-STR)' env RT_S3=0 RT_POISON= RT_PROGRAMS='spec/conformance/(E-STR-CHARS|L-LIB-STR)\.gr' bash scripts/rtswap.sh
fi

if control panic-exit-1; then
    plant panic-exit-1 src/panic.gr 'pub fn div_zero() {
    let b = buf()
    let n = text.div_zero(b)
    unsafe { grasm_panic_msg(b, n) }
}' 'extern fn _exit(code: i32)

pub fn div_zero() {
    unsafe { _exit(1) }
}' || { echo "error: panic-exit-1 does not build"; exit 1; }
    expect panic-exit-1 'exit 101 \(Rust runtime\) vs 1' env RT_S3=0 RT_POISON= RT_PROGRAMS='spec/conformance/E-DIV-ZERO' bash scripts/rtswap.sh
fi

if control layout-moved; then
    plant layout-moved src/layout.gr 'pub let STR_LEN = 17' 'pub let STR_LEN = 18' ||
        { echo "error: layout-moved does not build"; exit 1; }
    expect layout-moved 'STR_LEN is 18 in layout.gr, 17 in the Rust runtime' env RT_POISON= RT_LAYOUT="$WORK/layout-moved/src/layout.gr" bash scripts/rtlayout.sh
    expect layout-moved 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control fallback-gone; then
    plant fallback-gone src/math.gr 'export fn grt_f64_sqrt(' 'fn grt_f64_sqrt(' ||
        { echo "error: fallback-gone does not build"; exit 1; }
    expect fallback-gone 'says f64_sqrt has moved, but the Grasm runtime does not export' env RT_S3=0 RT_POISON= RT_PROGRAMS='^examples/hello' bash scripts/rtswap.sh
fi

if control fallback-call; then
    plant fallback-call src/math.gr 'export fn grt_f64_floor(v: f64) -> f64 {
    unsafe { floor(v) }
}' 'extern fn grasm_f64_floor(x: f64) -> f64

export fn grt_f64_floor(v: f64) -> f64 {
    unsafe { grasm_f64_floor(v) }
}' || { echo "error: fallback-call does not build"; exit 1; }
    expect fallback-call "calls the Rust runtime's _grasm_f64_floor" env RT_S3=0 RT_POISON= RT_PROGRAMS='^examples/hello' bash scripts/rtswap.sh
fi

echo
echo "checks of planted faults: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
[ "$ran" -ge 1 ] && [ "$caught" -eq "$ran" ]
