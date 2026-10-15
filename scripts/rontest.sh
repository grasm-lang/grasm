#!/usr/bin/env bash
# Does `packages/ron` agree with the Rust `ron` crate?
#
# Two layers. `scripts/fixtures/ron/cases.txt` holds one value a line beside
# the JSON the reference produces, recorded rather than invented -- so the
# file is a record of the reference and not a snapshot of ourselves. The two
# documents beside it are the shapes a line cannot hold: comments, nested
# block comments, raw strings with hashes, and a realistic config.
#
# The JSON rendering is the comparison surface, and matching it is deliberate
# work rather than a convenience: `ron::Value` through serde turns a tuple
# into an array, drops a struct's name, sorts its fields, and collapses
# `Some(x)` to `x` and `None` to null. `to_json` mirrors those so the bytes
# can be compared at all.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
FIX=scripts/fixtures/ron
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/ron-demo/src/main.gr --native -o "$work/ron" \
    >"$work/build.log" 2>&1; then
    echo "    the parser did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
while IFS=$'\t' read -r src want; do
    case "$src" in ''|\#*) continue ;; esac
    printf '%s' "$src" > "$work/case.ron"
    got=$("$work/ron" "$work/case.ron" 2>&1 | head -1)
    if [ "$got" != "$want" ]; then
        echo "    $src"
        echo "      got  $got"
        echo "      want $want"
        exit 1
    fi
    checked=$((checked + 1))
done < "$FIX/cases.txt"

# The documents, whose expected JSON is recorded beside them.
for doc in "$FIX"/*.ron; do
    name=$(basename "$doc" .ron)
    ref="$FIX/$name.json"
    [ -f "$ref" ] || { echo "    no recorded JSON for $name"; exit 1; }
    got=$("$work/ron" "$doc" 2>&1 | head -1)
    want=$(cat "$ref")
    if [ "$got" != "$want" ]; then
        echo "    $name.ron does not match the reference:"
        echo "      got  ${got:0:120}"
        echo "      want ${want:0:120}"
        exit 1
    fi
    checked=$((checked + 1))
done

echo "    values: $checked   all match the reference's JSON"
# A case file that shrank is a gate that stopped checking (F76).
[ "$checked" -ge 39 ]
