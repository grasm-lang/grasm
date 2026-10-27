#!/usr/bin/env bash
# Programs through Grasm alone, front to binary (docs/self-hosting.md, S2
# part two): a smoke test, counted.
#
# Each program's TIR is made by `compiler/front` -- the frontend in Grasm,
# not `grasm tir` -- and then
#
#   * run by `tools/tirinterp`, the TIR interpreter in Grasm, and
#   * compiled by `compiler/s3`, the backend in Grasm, and linked by
#     `compiler/link` (`grasm-link`), the linker in Grasm, against the
#     runtime, and run;
#
# and both must print and exit exactly as `grasm run` does. The programs
# are every fourth conformance program that says what it prints (the
# examples include programs that run for minutes under an interpreter, and
# tirinterp is one); `tirdiff` already holds the TIR to the Rust
# frontend's byte for byte, so this shows the TIR is the program's whole
# meaning when the Grasm frontend makes it, not only when Rust does.
#
# `FRONT=`, `TIRINTERP=`, `S3=` name prebuilt binaries; `S3_RT=` the runtime,
# `LINKER=` the linker.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

build() {
    local var=$1 src=$2
    if [ -n "${!var:-}" ]; then
        return 0
    fi
    printf -v "$var" '%s' "$WORK/$var.bin"
    if ! "$GRASM" build "$src" --native -o "${!var}" >"$WORK/build.log" 2>&1; then
        echo "error: $src does not build:"
        sed 's/^/  /' "$WORK/build.log"
        exit 1
    fi
}
build FRONT compiler/front/src/main.gr
build TIRINTERP tools/tirinterp/src/main.gr
build S3 compiler/s3/src/main.gr
S3_RT=${S3_RT:-./target/release/libgrasm_rt_abi.a}
LINKER=${LINKER:-./target/release/grasm-link}
for f in "$S3_RT" "$LINKER"; do
    if [ ! -e "$f" ]; then
        echo "error: $f is missing"
        exit 1
    fi
done

: >"$WORK/programs"
grep -l '^//! expect: output$' spec/conformance/*.gr | awk 'NR % 4 == 0' >>"$WORK/programs"
listed=$(wc -l <"$WORK/programs" | tr -d ' ')

interp_same=0
native_same=0
outside=0
declare -a FAILED=()
while IFS= read -r f; do
    $RUN_LIMIT "$GRASM" run "$f" >"$WORK/ref.out" 2>"$WORK/ref.err" </dev/null
    rc=$?
    if ! "$FRONT" tir "$f" >"$WORK/p.tir" 2>"$WORK/front.err"; then
        FAILED+=("$f — front tir refuses it: $(head -1 "$WORK/front.err")")
        continue
    fi
    $RUN_LIMIT "$TIRINTERP" "$WORK/p.tir" >"$WORK/i.out" 2>"$WORK/i.err" </dev/null
    ic=$?
    if [ "$ic" = "$rc" ] && cmp -s "$WORK/ref.out" "$WORK/i.out"; then
        interp_same=$((interp_same + 1))
    else
        FAILED+=("$f — tirinterp: exit $ic, grasm run: exit $rc$(cmp -s "$WORK/ref.out" "$WORK/i.out" || echo ', output differs')")
    fi
    "$S3" "$WORK/p.tir" -o "$WORK/p.o" >"$WORK/s3.out" 2>"$WORK/s3.err"
    sc=$?
    if [ "$sc" = 3 ]; then
        outside=$((outside + 1))
        continue
    fi
    if [ "$sc" != 0 ]; then
        FAILED+=("$f — s3 exits $sc: $(head -1 "$WORK/s3.err")")
        continue
    fi
    if ! "$LINKER" "$WORK/p.o" "$S3_RT" -dead_strip -o "$WORK/p" >"$WORK/link.log" 2>&1; then
        FAILED+=("$f — the link fails: $(head -1 "$WORK/link.log")")
        continue
    fi
    $RUN_LIMIT "$WORK/p" >"$WORK/n.out" 2>"$WORK/n.err" </dev/null
    nc=$?
    if [ "$nc" = "$rc" ] && cmp -s "$WORK/ref.out" "$WORK/n.out"; then
        native_same=$((native_same + 1))
    else
        FAILED+=("$f — native: exit $nc, grasm run: exit $rc$(cmp -s "$WORK/ref.out" "$WORK/n.out" || echo ', output differs')")
    fi
    rm -f "$WORK/p" "$WORK/p.o"
done <"$WORK/programs"

echo "programs: $listed   through tirinterp as grasm run: $interp_same   compiled by s3, linked by grasm-link, as grasm run: $native_same   outside s3's subset: $outside"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$interp_same" -ne "$listed" ] || [ $((native_same + outside)) -ne "$listed" ]; then
    echo "the counts do not add up to the $listed programs listed"
    exit 1
fi
if [ "$native_same" -lt 40 ]; then
    echo "only $native_same programs went front to binary; the smoke test wants a few dozen"
    exit 1
fi
exit 0
