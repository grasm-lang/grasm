#!/usr/bin/env bash
# Builds everything, generates the input if absent, runs each implementation
# once at full size, and prints name, wall ms, peak RSS (MB) and the output
# line. Fails if any output differs.
#
# Usage: ./run.sh [SIZE_BYTES]   (default 1073741824 = 1 GiB; the original
#                                 spec's 10 GB is 10000000000)
# Env:   ETL_DATA (input dir), OUT (binaries), SEED (default 42)
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build.sh"

SIZE="${1:-1073741824}"
input="$DATA/etl-full-$SIZE.jsonl"
# Keep at most one large input on disk.
for old in "$DATA"/etl-full-*.jsonl; do
    [ -e "$old" ] && [ "$old" != "$input" ] && rm -f "$old"
done
ensure_input "$SIZE" "$input"
warm_up

# Wall clock of one command, in ms, measured by a single perl process so its
# own startup is outside the interval. stdout/stderr go to $1/$2.
timed_ms() {
    perl -MTime::HiRes=time -e '
        my ($out, $err, @cmd) = @ARGV;
        open(STDOUT, ">", $out) or die; open(STDERR, ">", $err) or die;
        my $t = time; system(@cmd); my $rc = $?;
        printf STDERR "\nwall_ms %d\n", (time - $t) * 1000; exit($rc >> 8);
    ' "$@"
}

echo "etl: $input ($(wc -c <"$input" | tr -d ' ') bytes)"
printf "%-6s %10s %10s  %s\n" name wall_ms rss_mb output
first=""
status=0
for v in "${VARIANTS[@]}"; do
    out="$OUT/$v.out"; err="$OUT/$v.time"
    timed_ms "$out" "$err" /usr/bin/time -l "$OUT/$v" "$input" \
        || { echo "FAIL: $v exited non-zero" >&2; cat "$err" >&2; exit 1; }
    ms=$(awk '/^wall_ms/ { print $2 }' "$err")
    rss=$(awk '/maximum resident set size/ { printf "%.1f", $1 / 1048576 }' "$err")
    got=$(cat "$out")
    printf "%-6s %10d %10s  %s\n" "$v" "$ms" "$rss" "$got"
    if [ -z "$first" ]; then
        first="$got"
    elif [ "$got" != "$first" ]; then
        echo "OUTPUT MISMATCH: $v printed '$got', expected '$first'" >&2
        status=1
    fi
done
[ "$status" -eq 0 ] || { echo "FAIL: outputs differ" >&2; exit 1; }
echo "ok: all outputs match"
