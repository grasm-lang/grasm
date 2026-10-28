#!/usr/bin/env bash
# Upper and lower case on the Grasm runtime must be the Rust runtime's, for
# every code point (docs/self-hosting.md §6.11).
#
# `compiler/rt/probe/casedump.gr` writes, for every scalar value, its
# lowercase and uppercase where they differ and what the final-sigma rule
# makes of it before a capital sigma and between one and a cased letter. It
# is built twice, linked as it ships and with the Grasm runtime interposed
# (`scripts/lib/rtswap.sh`); the swapped build must take `str_upper` and
# `str_lower` from the Grasm runtime, and the two must write the same bytes.
# When they differ because the Rust toolchain's Unicode moved, the Grasm
# runtime's tables are made again from the Rust runtime's answers:
# `python3 scripts/rtcase.py <the dump on the Rust runtime>`. The sigma rule
# over longer company is `scripts/fuzz_rtstrings.sh`'s.
#
#   `RT_OBJ=`, `S3=` and `LINK=` as for `scripts/rtswap.sh`.
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

if ! "$GRASM" build compiler/rt/probe/casedump.gr --native -o "$WORK/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/f.swap" ]; then
    echo "error: compiler/rt/probe/casedump.gr does not build:"
    sed 's/^/  /' "$WORK/build.log" | head -10
    exit 1
fi
for need in str_upper str_lower; do
    if ! grep -qx "grasm $need" "$WORK/f.swaplog"; then
        echo "FAIL: the swapped build did not take $need from the Grasm runtime, so it compared nothing"
        exit 1
    fi
done
"$WORK/f" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
ac=$?
"$WORK/f.swap" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
bc=$?
if [ "$ac" != 0 ] || [ "$bc" != 0 ] || ! cmp -s "$WORK/a.out" "$WORK/b.out"; then
    echo "FAIL: the two runtimes differ on case (exit $ac and $bc)"
    diff "$WORK/a.out" "$WORK/b.out" | head -6 | sed 's/^/    /'
    head -3 "$WORK/b.err" | sed 's/^/    /'
    exit 1
fi
count() { grep -c "^$1 " "$WORK/a.out"; }
lower=$(count L)
upper=$(count U)
ign=$(count I)
cased=$(count C)
if [ "$lower" -lt 1000 ] || [ "$upper" -lt 1000 ] || [ "$ign" -lt 1000 ] || [ "$cased" -lt 1000 ]; then
    echo "FAIL: the dump is too short to be every code point's ($lower $upper $ign $cased)"
    exit 1
fi
echo "code points 0 to 10ffff identical: lowercase changes $lower, uppercase $upper, case-ignorable $ign, cased $cased"
