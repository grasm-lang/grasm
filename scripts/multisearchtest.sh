#!/usr/bin/env bash
# Does `packages/multisearch` find what a quadratic search finds?
#
# Aho-Corasick replaces one pass per pattern with one pass total, and the
# thing that makes that work -- a failure link on every node pointing at the
# longest proper suffix that is also a prefix -- is also the thing that is
# easy to get wrong. A wrong link does not crash; it silently misses matches
# that span a failure, and only overlapping patterns reveal it.
#
# So the oracle is brute force: for every end offset, for every pattern,
# does the text end with it there. It shares no structure with the
# automaton, which is the point.
#
# Twelve named cases cover the shapes that break it -- patterns that are
# suffixes of each other, prefixes of each other, a pattern inside another
# match, overlapping runs, a duplicate pattern, bytes above 127 -- and 200
# random cases over a three-letter alphabet cover the rest. Three letters
# rather than 256 on purpose: over the full byte range a random pattern set
# almost never overlaps, so the fuzzer would exercise the trie and never
# touch the failure links.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for prog in multisearch-demo multisearch-fuzz; do
    if ! "$GRASM" build "packages/$prog/src/main.gr" --native -o "$work/$prog" \
        >"$work/$prog.log" 2>&1; then
        echo "    $prog did not build:"
        sed 's/^/      /' "$work/$prog.log"
        exit 1
    fi
done

"$work/multisearch-demo" >"$work/all.txt" 2>&1 || {
    echo "    the demo did not run:"; sed 's/^/      /' "$work/all.txt"; exit 1
}
"$work/multisearch-fuzz" >>"$work/all.txt" 2>&1 || {
    echo "    the fuzzer did not run:"; tail -3 "$work/all.txt" | sed 's/^/      /'
    exit 1
}

python3 scripts/lib/multisearchcheck.py <"$work/all.txt" || exit 1

# The automaton must be smaller than the patterns laid end to end, or the
# trie is not sharing prefixes and this is a list search wearing a hat.
nodes=$(sed -n 's/^nodes //p' "$work/all.txt")
[ -n "$nodes" ] || { echo "    the demo did not report a node count"; exit 1; }
# he/she/his/hers is 13 bytes of pattern; a trie sharing `he` needs 10 nodes.
[ "$nodes" -le 11 ] || {
    echo "    the classic set built $nodes nodes; a prefix-sharing trie needs 10"
    exit 1
}
echo "    automaton: $nodes nodes for 13 bytes of pattern, prefixes shared"
