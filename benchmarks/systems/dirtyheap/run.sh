#!/usr/bin/env bash
# Builds all variants and runs each once at full size (N=5,000,000 unless N is
# set). Prints wall ms, the drop/pause ms the program measured, peak RSS and
# the checksum, and fails if any checksum differs.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/build.sh"
N="${N:-5000000}"
printf '%-13s %10s %10s %10s %9s  %s\n' variant wall_ms drop_ms prog_ms rss_mb checksum
first=""
status=0
for v in "${VARIANTS[@]}"; do
    # first exec of a fresh binary pays a one-off macOS check; pay it on N=1
    "$OUT/$v" 1 >/dev/null
    /usr/bin/time -l "$OUT/$v" "$N" >"$OUT/$v.out" 2>"$OUT/$v.time"
    sum=$(grep '^checksum' "$OUT/$v.out")
    drop=$(awk '/^drop_ms/{print $2}' "$OUT/$v.out")
    prog=$(awk '/^total_ms/{print $2}' "$OUT/$v.out")
    wall=$(awk '/ real /{printf "%d", $1*1000}' "$OUT/$v.time")
    rss=$(awk '/maximum resident set size/{printf "%.1f", $1/1048576}' "$OUT/$v.time")
    printf '%-13s %10s %10s %10s %9s  %s\n' "$v" "$wall" "$drop" "$prog" "$rss" "$sum"
    grep '^gc_' "$OUT/$v.out" | sed 's/^/              /' || true
    if [ -z "$first" ]; then first="$sum"; elif [ "$sum" != "$first" ]; then
        echo "CHECKSUM MISMATCH: $v" >&2; status=1
    fi
done
exit $status
