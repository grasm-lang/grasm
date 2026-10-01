#!/usr/bin/env bash
# Correctness only: every variant must print the same checksum at small sizes.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/build.sh"
status=0
for n in ${SIZES:-1 1000 100000}; do
    first=""
    for v in "${VARIANTS[@]}"; do
        sum=$("$OUT/$v" "$n" | grep '^checksum')
        echo "$v: $sum"
        if [ -z "$first" ]; then first="$sum"; elif [ "$sum" != "$first" ]; then
            echo "MISMATCH at N=$n: $v" >&2; status=1
        fi
    done
done
[ $status -eq 0 ] && echo "all variants agree"
exit $status
