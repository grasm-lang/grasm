#!/usr/bin/env bash
# The same program compiles to the same bytes.
#
# Every fourth corpus program with a `main` is built natively twice, into two
# empty directories under the same name (the linker's ad-hoc signature takes
# the file name as its identifier), and the two executables must be
# identical. They were not: the loader named a package reached two ways by
# whichever path a `HashMap` happened to walk first, and the native backend
# released a scope's variables in the scope map's hash order -- so the same
# program could link to a different binary on every build. MASTERPLAN 14.4
# asks for reproducible builds, and the self-hosting plan's fixed point
# (stage 2 = stage 3) is impossible without them.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "$GRASM"
. "$(dirname "$0")/lib/corpus.sh"
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

built=0
differ=0
skipped=0
i=0
for src in $(corpus_programs); do
    grep -q '^fn main' "$src" || continue
    i=$((i + 1))
    [ $((i % 4)) = 0 ] || continue
    mkdir -p "$WORK/a" "$WORK/b"
    if ! "$GRASM" build "$src" --native --no-incremental -o "$WORK/a/prog" > /dev/null 2>&1; then
        skipped=$((skipped + 1))
        rm -rf "$WORK/a" "$WORK/b"
        continue
    fi
    "$GRASM" build "$src" --native --no-incremental -o "$WORK/b/prog" > /dev/null 2>&1
    built=$((built + 1))
    if ! cmp -s "$WORK/a/prog" "$WORK/b/prog"; then
        differ=$((differ + 1))
        echo "NOT REPRODUCIBLE: $src"
    fi
    rm -rf "$WORK/a" "$WORK/b"
done

echo "built twice: $built   differ: $differ   not buildable natively: $skipped"
[ "$differ" -eq 0 ] || exit 1
# A gate that built nothing has compared nothing.
if [ "$built" -lt 100 ]; then
    echo "only $built programs were built; expected well over 100"
    exit 1
fi
exit 0
