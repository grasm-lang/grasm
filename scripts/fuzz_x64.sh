#!/usr/bin/env bash
# The fuzzers pointed at the x86-64 backend (docs/self-hosting.md §6.16,
# stage S7): `scripts/fuzz_s3.py` and every generator that holds the Grasm
# backend to the Cranelift build through `scripts/lib/s3engine.py`, each run
# with `S3_TARGET=x86_64` -- compiled with `s3 --target x86_64`, linked for
# x86-64, and held to the Cranelift build for x86_64-apple-darwin, both run
# under Rosetta.
#
# Each must pass, and must say it checked x86-64: fuzz_s3 its "target: x86_64"
# line, a generator its engine's "s3 engine (x86_64, under Rosetta):" line
# with something agreed and nothing failed. The generators are the
# `scripts/fuzz_*.py` that load the engine, counted here and all run.
#
# `S3=`, `S3_EXTRA`, `S3_LINKER` and `S3_RT_X64=` pass through to each, as the
# engine takes them; `FUZZ_X64_ONLY=<regex>` runs the generators whose name
# matches (and fuzz_s3 only if it does); `S3_STOP_AT_FIRST=1` stops at the
# first that fails (the controls).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
export GRASM
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
TARGET=x86_64-apple-darwin
if [ -z "${S3_RT_X64:-}" ]; then
    if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build"
        exit 1
    fi
    S3_RT_X64=./target/$TARGET/release/libgrasm_rt_abi.a
fi
[ -f "$S3_RT_X64" ] || { echo "error: $S3_RT_X64 is missing"; exit 1; }
export S3_RT_X64
export S3_TARGET=x86_64

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

gens=()
for f in scripts/fuzz_*.py; do
    [ "$f" = scripts/fuzz_s3.py ] && continue
    grep -c 's3engine' "$f" >/dev/null || continue
    gens+=("$f")
done
if [ ${#gens[@]} -lt 1 ]; then
    echo "no generator loads scripts/lib/s3engine.py: nothing would be checked"
    exit 1
fi

ran=0
passed=0
agreed=0
declare -a FAILED=()
one() { # SCRIPT
    local f=$1 name log
    name=$(basename "$f" .py)
    if [ -n "${FUZZ_X64_ONLY:-}" ] && ! [[ "$name" =~ $FUZZ_X64_ONLY ]]; then
        return
    fi
    if [ -n "${S3_STOP_AT_FIRST:-}" ] && [ ${#FAILED[@]} -gt 0 ]; then
        return
    fi
    log="$WORK/$name.log"
    ran=$((ran + 1))
    python3 "$f" >"$log" 2>&1
    local code=$?
    if [ "$code" != 0 ]; then
        FAILED+=("$name: exit $code")
        echo "FAILED: $name (exit $code)"
        grep -E 'DISAGREES|FAILED|failed:|Error' "$log" | head -6 | sed 's/^/    /'
        tail -3 "$log" | sed 's/^/    /'
        return
    fi
    if [ "$name" = fuzz_s3 ]; then
        local line
        line=$(grep -E '^target: x86_64' "$log" | tail -1)
        if [ -z "$line" ]; then
            FAILED+=("$name: passed without saying it checked x86-64")
            return
        fi
        local n
        n=$(echo "$line" | sed -E 's/.*identical: ([0-9]+).*/\1/')
        agreed=$((agreed + n))
        echo "$name: $line"
    else
        local line
        line=$(grep -E '^s3 engine \(x86_64, under Rosetta\):' "$log" | tail -1)
        if [ -z "$line" ]; then
            FAILED+=("$name: passed without the x86-64 engine's line")
            return
        fi
        local n f0
        n=$(echo "$line" | sed -E 's/.*agreed: ([0-9]+).*/\1/')
        f0=$(echo "$line" | sed -E 's/.*failed: ([0-9]+).*/\1/')
        if [ "$n" -lt 1 ] || [ "$f0" != 0 ]; then
            FAILED+=("$name: $line")
            return
        fi
        agreed=$((agreed + n))
        echo "$name: $line"
    fi
    passed=$((passed + 1))
}

one scripts/fuzz_s3.py
for f in "${gens[@]}"; do
    one "$f"
done

echo
echo "target: x86_64 under Rosetta   fuzzers: $ran of $((${#gens[@]} + 1)) (fuzz_s3 and ${#gens[@]} generators on the s3 engine)   passed: $passed   programs agreed: $agreed"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$passed" -ne "$ran" ] || [ "$ran" -lt 1 ]; then
    echo "not every fuzzer ran and passed"
    exit 1
fi
if [ -z "${FUZZ_X64_ONLY:-}" ] && [ "$ran" -ne $((${#gens[@]} + 1)) ]; then
    echo "ran $ran fuzzers of $((${#gens[@]} + 1))"
    exit 1
fi
exit 0
