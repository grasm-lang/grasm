#!/usr/bin/env bash
# Builds all variants and checks that they print identical output at small
# sizes (and that the Grasm interpreter agrees at the smallest one).
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build.sh"

fail=0
check() {
    local label="$1" want="$2" got="$3"
    if [ "$got" == "$want" ]; then
        printf "  %-22s %s\n" "$label" "$got"
    else
        printf "  %-22s %s   MISMATCH (want %s)\n" "$label" "$got" "$want"
        fail=1
    fi
}

for size in "1000 5" "50000 10" "3 1000"; do
    set -- $size
    echo "N=$1 S=$2"
    want="$("$BUILD_DIR/rust" "$1" "$2")"
    for v in "${VARIANTS[@]}"; do
        check "$v" "$want" "$("$BUILD_DIR/$v" "$1" "$2")"
    done
    if [ "$1" -le 1000 ]; then
        check "grasm run ecs.gr" "$want" "$("$GRASM" run "$HERE/grasm/ecs.gr" -- "$1" "$2")"
        check "grasm run ecs_soa.gr" "$want" "$("$GRASM" run "$HERE/grasm/ecs_soa.gr" -- "$1" "$2")"
    fi
done
[ "$fail" -eq 0 ] || { echo "FAIL: outputs differ" >&2; exit 1; }
echo "ok: all variants agree"
