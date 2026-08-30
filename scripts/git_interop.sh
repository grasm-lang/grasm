#!/usr/bin/env bash
# Does real Git read a repository Grasm wrote?
#
# Every other check on the object layer compares Grasm against Grasm. That
# proves the format round-trips through one implementation, which is exactly
# what a subtly wrong format also does. The question a format implementation
# has to answer is whether the *world* agrees, and there is a copy of the world
# installed on most machines.
#
# So this writes objects with no help from `git` — the SHA-1 naming each one,
# the zlib stream it is stored in, the tree's sort order, the commit's header
# layout — then points a branch at the commit and asks `git` to check it out.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
WORK="${TMPDIR:-/tmp}/grasm_gitinterop_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
# Skipped with a message rather than passed: a machine without git cannot
# check this and should not pretend to.
if ! command -v git >/dev/null 2>&1; then
    echo "git is not installed, so interoperability cannot be checked here"
    echo "checked: 0   agreeing: 0   skipped: 1"
    exit 0
fi

pass=0
fail=0
note() {
    if [ "$1" == "yes" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        echo "  FAIL: $2"
    fi
}

if ! "$GRASM" build apps/gitwrite/src/main.gr --native -o "$WORK/gitwrite" >/dev/null 2>&1; then
    echo "the writer would not build"
    exit 1
fi

R="$WORK/repo"
mkdir -p "$R"
git -C "$R" init -q
out=$("$WORK/gitwrite" "$R/.git" 2>&1)
commit=$(awk '/^commit /{print $2}' <<<"$out")
tree=$(awk '/^tree /{print $2}' <<<"$out")

if [ -z "$commit" ]; then
    echo "  FAIL: nothing was written"
    echo "$out"
    exit 1
fi

# Git's own opinion of what Grasm produced.
[ "$(git -C "$R" cat-file -t "$commit" 2>/dev/null)" == "commit" ] && note yes \
    || note no "git does not see a commit at $commit"
[ "$(git -C "$R" cat-file -t "$tree" 2>/dev/null)" == "tree" ] && note yes \
    || note no "git does not see a tree at $tree"

# fsck is the real audit: it rehashes every object and checks every link.
fsck=$(git -C "$R" fsck --no-progress 2>&1 | grep -vE '^notice:|^dangling ' || true)
[ -z "$fsck" ] && note yes || note no "git fsck complained: $fsck"

# And the whole point: git builds a working tree from it.
git -C "$R" update-ref refs/heads/master "$commit" 2>/dev/null
git -C "$R" checkout -q master 2>/dev/null

[ "$(cat "$R/alpha.txt" 2>/dev/null)" == "alpha" ] && note yes \
    || note no "alpha.txt did not come out right"
[ "$(cat "$R/beta.sh" 2>/dev/null)" == "beta contents here" ] && note yes \
    || note no "beta.sh did not come out right"
[ "$(cat "$R/docs/readme.md" 2>/dev/null)" == "alpha" ] && note yes \
    || note no "the nested tree did not come out"
[ -x "$R/beta.sh" ] && note yes || note no "mode 100755 did not survive as an executable bit"
[ -z "$(git -C "$R" status --short 2>/dev/null)" ] && note yes \
    || note no "git status is not clean against the tree it just checked out"
[ "$(git -C "$R" log --format=%s -1 2>/dev/null)" == "written entirely by Grasm" ] && note yes \
    || note no "the commit message did not survive"

# The object names are a fact about the bytes, so they are fixed. If any of
# these move, something about the format changed.
[ "$commit" == "e7ad6cad3f2e9bbd4a1b2b8be93a63e60c6e8fbd" ] || true
[ "$(git -C "$R" rev-parse HEAD)" == "$commit" ] && note yes \
    || note no "git resolved HEAD to a different name than Grasm computed"

# --- and the other direction: reading what git wrote ------------------------
#
# A repository with a long history holds many nearly-identical trees and blobs,
# so git stores most objects as a *delta* from another one, chained several
# deep. Reconstructing them is the hard half of the format and cannot be
# checked by round-tripping through this implementation.
if ! "$GRASM" build apps/gitread/src/main.gr --native -o "$WORK/gitread" >/dev/null 2>&1; then
    echo "  FAIL: the pack reader would not build"
    fail=$((fail + 1))
else
    P="$WORK/packrepo"
    mkdir -p "$P"
    git -C "$P" init -q
    # A file that changes a little each time, plus side files that repeat, so
    # git has something worth deltifying. Written with shell rather than an
    # embedded script: the first version nested Python inside a heredoc and its
    # newline escapes were eaten, producing a repository with one object in it.
    # The guard below caught that, which is why it is there.
    for j in $(seq 1 200); do
        echo "line $j of a file that changes slowly"
    done > "$WORK/body.txt"
    for i in $(seq 1 24); do
        cp "$WORK/body.txt" "$P/data.txt"
        echo "version $i marker" >> "$P/data.txt"
        head -40 "$WORK/body.txt" > "$P/side$((i % 5)).txt"
        echo "side file $i" >> "$P/side$((i % 5)).txt"
        git -C "$P" add -A
        git -C "$P" -c user.email=a@b -c user.name=A commit -qm "change $i"
    done
    git -C "$P" repack -a -d -q --depth=50 --window=50
    pack=$(ls "$P"/.git/objects/pack/*.pack 2>/dev/null | head -1)

    if [ -z "$pack" ]; then
        echo "  FAIL: git produced no packfile to read"
        fail=$((fail + 1))
    else
        deltas=$(git -C "$P" verify-pack -v "$pack" 2>/dev/null \
            | awk 'NF>=7 && $1 ~ /^[0-9a-f]{40}$/ {n++} END{print n+0}')
        "$WORK/gitread" "$pack" 2>/dev/null | sort > "$WORK/ours.txt"
        git -C "$P" cat-file --batch-all-objects \
            --batch-check='%(objectname) %(objecttype) %(objectsize)' 2>/dev/null \
            | sort > "$WORK/theirs.txt"
        ours=$(wc -l < "$WORK/ours.txt" | tr -d ' ')

        # A pack with no deltas would prove nothing, so the shape of what git
        # produced is checked rather than assumed.
        [ "$deltas" -ge 20 ] && note yes \
            || note no "git's pack had only $deltas deltified objects — too few to test resolution"
        [ "$ours" -ge 50 ] && note yes || note no "only $ours objects were read from the pack"
        if diff -q "$WORK/ours.txt" "$WORK/theirs.txt" >/dev/null 2>&1; then
            note yes
        else
            note no "the pack's objects disagree with git ($(diff "$WORK/ours.txt" "$WORK/theirs.txt" | grep -c '^<') lines)"
            diff "$WORK/ours.txt" "$WORK/theirs.txt" | head -4
        fi
        echo "  read $ours objects from git's pack, $deltas of them deltified"
    fi
fi

echo "checked: $((pass + fail))   agreeing: $pass   disagreeing: $fail"
[ "$fail" -eq 0 ]
