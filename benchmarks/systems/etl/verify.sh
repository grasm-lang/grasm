#!/usr/bin/env bash
# Correctness only: builds everything, then runs each implementation on a small
# (2 MB) and a medium (50 MB) input and fails unless all outputs are identical.
#
# Usage: ./verify.sh [SIZE...]   (default sizes: 2000000 50000000 bytes)
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build.sh"

sizes=("$@")
[ "${#sizes[@]}" -gt 0 ] || sizes=(2000000 50000000)
status=0
for size in "${sizes[@]}"; do
    input="$DATA/etl-verify-$size.jsonl"
    ensure_input "$size" "$input" 2>/dev/null
    echo "size $size bytes ($(wc -l <"$input" | tr -d ' ') lines)"
    first=""
    for v in "${VARIANTS[@]}"; do
        got=$("$OUT/$v" "$input") || { echo "FAIL: $v exited non-zero" >&2; exit 1; }
        printf '  %-6s %s\n' "$v" "$got"
        if [ -z "$first" ]; then
            first="$got"
        elif [ "$got" != "$first" ]; then
            echo "MISMATCH at size $size: $v" >&2
            status=1
        fi
    done
done
[ "$status" -eq 0 ] || { echo "FAIL: outputs differ" >&2; exit 1; }
echo "ok: all outputs match"
