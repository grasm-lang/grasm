#!/usr/bin/env bash
# Does `packages/walkdir` find what `find` finds?
#
# `find` is the oracle and it is a good one: it is on every machine this
# runs on, it shares no code with us, and it has had forty years to get
# directory traversal right.
#
# Three things are checked, and the third is the one the package exists to
# promise:
#
#   * the file set matches `find -type f` exactly, at several depths;
#   * the directory set matches `find -mindepth 1 -type d`;
#   * a **symlink loop terminates**. The runtime offers no way to ask
#     whether a path is a link -- no `lstat`, no `read_link` -- so a link to
#     its own parent descends forever, and each step has a different path,
#     which means remembering visited paths does not stop it. The package's
#     answer is a required depth limit that reports when it was reached. A
#     walk that hangs here, or that returns quietly without saying it was
#     cut short, is the defect.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/walkdir-demo/src/main.gr --native -o "$work/wd" \
    >"$work/build.log" 2>&1; then
    echo "    the walker did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

tree="$work/tree"
mkdir -p "$tree/a/b/c" "$tree/a/d" "$tree/e" "$tree/empty"
for f in top.txt a/one.txt a/b/two.txt a/b/c/three.txt a/d/four.txt e/five.txt; do
    echo x >"$tree/$f"
done
# A name that sorts after its siblings and a name with a space, because
# sorting and joining are where a walker breaks quietly.
echo x >"$tree/zz last.txt"
mkdir -p "$tree/a/b/c/d/e/f"
echo x >"$tree/a/b/c/d/e/f/deep.txt"

checked=0
for depth in 16 8 3; do
    "$work/wd" "$tree" "$depth" >"$work/out.$depth" 2>&1 || {
        echo "    the walk failed at depth $depth:"
        sed 's/^/      /' "$work/out.$depth"
        exit 1
    }
    # At a depth that covers the tree, `find` is the reference. Below that,
    # `find -maxdepth` is -- the walker reports entries *at* its limit and
    # descends no further, which is what -maxdepth means.
    sed -n 's/^F //p' "$work/out.$depth" | sort >"$work/files.$depth"
    find "$tree" -maxdepth "$depth" -type f | sort >"$work/wantf.$depth"
    if ! diff -q "$work/files.$depth" "$work/wantf.$depth" >/dev/null; then
        echo "    files differ from find at depth $depth:"
        diff "$work/files.$depth" "$work/wantf.$depth" | head -8 | sed 's/^/      /'
        exit 1
    fi
    sed -n 's/^D //p' "$work/out.$depth" | sort >"$work/dirs.$depth"
    find "$tree" -mindepth 1 -maxdepth "$depth" -type d | sort >"$work/wantd.$depth"
    if ! diff -q "$work/dirs.$depth" "$work/wantd.$depth" >/dev/null; then
        echo "    directories differ from find at depth $depth:"
        diff "$work/dirs.$depth" "$work/wantd.$depth" | head -8 | sed 's/^/      /'
        exit 1
    fi
    checked=$((checked + 1))
done

# Deterministic: the same tree walked twice is byte-identical.
"$work/wd" "$tree" 16 >"$work/again" 2>&1
cmp -s "$work/out.16" "$work/again" || {
    echo "    two walks of an unchanged tree disagreed"
    exit 1
}

# The limit must be reported when it bites, and not when it does not.
grep -q "^hit_limit false$" "$work/out.16" || {
    echo "    a walk deep enough for the tree still reported hit_limit"
    exit 1
}
grep -q "^hit_limit true$" "$work/out.3" || {
    echo "    a walk cut short at depth 3 did not report hit_limit"
    exit 1
}

# And the loop. Without the limit this does not return.
ln -s "$tree" "$tree/a/loop"
if ! timeout 60 "$work/wd" "$tree" 4 >"$work/loop.out" 2>&1; then
    echo "    a symlink loop was not bounded -- the walk did not finish"
    exit 1
fi
grep -q "^hit_limit true$" "$work/loop.out" || {
    echo "    a symlink loop finished without reporting that it was cut short"
    exit 1
}
rm -f "$tree/a/loop"

# A limit of zero is refused rather than meaning "unlimited".
if "$work/wd" "$tree" 0 >/dev/null 2>&1; then
    echo "    a depth limit of 0 was accepted; unlimited is what this prevents"
    exit 1
fi

echo "    depths: $checked against find, deterministic, and a symlink loop bounded"
[ "$checked" -ge 3 ]
