#!/usr/bin/env bash
# Builds all variants, runs each once at full size, and prints
# name, wall-clock ms, peak RSS (MB) and checksum. Fails if checksums differ.
#
# Usage: ./run.sh [N] [S]   (defaults N=1000000, S=100)
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build.sh"

N="${1:-1000000}"
S="${2:-100}"
# Wall clock of one command, in ms, measured by a single perl process so its
# own startup is outside the interval. stdout/stderr go to $2/$3.
timed_ms() {
    perl -MTime::HiRes=time -e '
        my ($out, $err, @cmd) = @ARGV;
        open(STDOUT, ">", $out) or die; open(STDERR, ">", $err) or die;
        my $t = time; system(@cmd); my $rc = $?;
        printf STDERR "\nwall_ms %d\n", (time - $t) * 1000; exit($rc >> 8);
    ' "$@"
}

# A freshly written executable pays a one-off macOS launch cost (~0.5 s) the
# first time it runs; run each binary once at a trivial size so that cost is
# not counted against whichever language happens to be measured.
for v in "${VARIANTS[@]}"; do "$BUILD_DIR/$v" 1 1 >/dev/null; done

echo "ecs: N=$N S=$S"
printf "%-10s %10s %10s  %s\n" name wall_ms rss_mb checksum
first=""
status=0
for v in "${VARIANTS[@]}"; do
    out="$BUILD_DIR/$v.out"; err="$BUILD_DIR/$v.time"
    timed_ms "$out" "$err" /usr/bin/time -l "$BUILD_DIR/$v" "$N" "$S" \
        || { echo "FAIL: $v exited non-zero" >&2; cat "$err" >&2; exit 1; }
    ms=$(awk '/^wall_ms/ { print $2 }' "$err")
    rss=$(awk '/maximum resident set size/ { printf "%.1f", $1 / 1048576 }' "$err")
    sum=$(cat "$out")
    printf "%-10s %10d %10s  %s\n" "$v" "$ms" "$rss" "$sum"
    if [ -z "$first" ]; then
        first="$sum"
    elif [ "$sum" != "$first" ]; then
        echo "CHECKSUM MISMATCH: $v printed '$sum', expected '$first'" >&2
        status=1
    fi
done
[ "$status" -eq 0 ] || { echo "FAIL: checksums differ" >&2; exit 1; }
echo "ok: all checksums match"
