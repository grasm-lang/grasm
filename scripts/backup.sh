#!/usr/bin/env bash
# A complete, verified copy of this repository's history in one file.
#
# There is no remote, so `git push` is not the backup — this is. A bundle
# holds every commit, branch and tag; `git clone <file>` restores the lot.
# It is about eleven megabytes because `.git` is small: the two gigabytes in
# this checkout are `target/`, which is build output and is not worth keeping.
#
# The verify step is the point. A backup that has never been restored is a
# hope, so this one is cloned into a temporary directory and its head commit
# compared against the one it was made from, every time.
set -uo pipefail
cd "$(dirname "$0")/.."

WHERE=${1:-$HOME/backups/grasm}
mkdir -p "$WHERE"
stamp=$(date +%Y%m%d-%H%M)
bundle="$WHERE/grasm-$stamp.bundle"

git gc --quiet 2>/dev/null
if ! git bundle create "$bundle" --all >/dev/null 2>&1; then
    echo "backup: could not write $bundle"
    exit 1
fi

if ! git bundle verify "$bundle" >/dev/null 2>&1; then
    echo "backup: $bundle does not verify"
    exit 1
fi

check=$(mktemp -d)
trap 'rm -rf "$check"' EXIT
if ! git clone -q "$bundle" "$check/repo" >/dev/null 2>&1; then
    echo "backup: $bundle does not restore"
    exit 1
fi
here=$(git rev-parse HEAD)
there=$(cd "$check/repo" && git rev-parse HEAD)
if [ "$here" != "$there" ]; then
    echo "backup: restored head $there is not $here"
    exit 1
fi

count=$(cd "$check/repo" && git log --oneline | wc -l | tr -d ' ')
size=$(du -h "$bundle" | cut -f1)
echo "backup: $bundle — $size, $count commits, restores to $here"

# Keep the last ten, so a bad state cannot quietly overwrite every good one.
ls -t "$WHERE"/grasm-*.bundle 2>/dev/null | tail -n +11 | while read -r old; do
    rm -f "$old"
    echo "backup: removed $old"
done
