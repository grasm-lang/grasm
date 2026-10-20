#!/usr/bin/env bash
# Checks release archives the way someone who downloads one meets them.
#
#   scripts/verify_release.sh ARCHIVE.tar.gz...    archives on disk
#   scripts/verify_release.sh --tag vX.Y.Z [-R owner/repo]
#                                                  the published release
#
# Each archive is unpacked into an empty directory outside the source tree --
# so nothing from a checkout or `target/` can stand in for a missing file --
# and must: run a program with `grasm run`, build it with `--native`, and run
# the binary that built. With --tag, the assets are downloaded with `gh`
# first and every archive is checked against the release's SHA256SUMS, which
# must list it. This is the check v1.0.0 never had.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

archives=()
if [ "${1:-}" = --tag ]; then
    TAG=$2; shift 2
    REPO_ARGS=()
    if [ "${1:-}" = -R ]; then REPO_ARGS=(-R "$2"); fi
    mkdir -p "$WORK/download"
    gh release download "$TAG" "${REPO_ARGS[@]}" -D "$WORK/download" -p '*.tar.gz' -p SHA256SUMS
    [ -f "$WORK/download/SHA256SUMS" ] || { echo "$TAG has no SHA256SUMS"; exit 1; }
    for a in "$WORK/download"/*.tar.gz; do
        [ -e "$a" ] || { echo "$TAG has no .tar.gz archives"; exit 1; }
        n=$(basename "$a")
        grep -q " $n\$" "$WORK/download/SHA256SUMS" || { echo "SHA256SUMS does not list $n"; exit 1; }
        archives+=("$a")
    done
    (cd "$WORK/download" && shasum -a 256 -c SHA256SUMS)
else
    for a in "$@"; do archives+=("$(cd "$(dirname "$a")" && pwd)/$(basename "$a")"); done
fi
[ ${#archives[@]} -gt 0 ] || { echo "usage: $0 ARCHIVE.tar.gz... | --tag vX.Y.Z [-R owner/repo]"; exit 1; }

fail=0
for a in "${archives[@]}"; do
    n=$(basename "$a" .tar.gz)
    case "$n" in
        *macos-arm64) [ "$(uname -s)-$(uname -m)" = Darwin-arm64 ] || { echo "skip $n: not this machine"; continue; } ;;
        # An arm64 Mac runs it under Rosetta, when Rosetta is installed.
        *macos-x86_64) [ "$(uname -s)" = Darwin ] && arch -x86_64 /usr/bin/true 2>/dev/null \
                           || { echo "skip $n: not this machine"; continue; } ;;
        *linux-x86_64) [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || { echo "skip $n: not this machine"; continue; } ;;
        *linux-arm64) [ "$(uname -s)-$(uname -m)" = Linux-aarch64 ] || { echo "skip $n: not this machine"; continue; } ;;
    esac
    d="$WORK/$n"
    mkdir -p "$d/unpacked" "$d/project"
    tar -xzf "$a" -C "$d/unpacked"
    g=$(ls -d "$d/unpacked"/*/grasm)
    for f in grasm libgrasm_rt_abi.a BUILD-INFO; do
        [ -f "$(dirname "$g")/$f" ] || { echo "FAIL $n: the archive has no $f"; fail=1; continue 2; }
    done
    cd "$d/project"
    printf 'fn main() {\n    let xs = [3, 1, 2]\n    print("release ok {xs.len()}")\n}\n' > check.gr
    # Nothing from the environment may point back at a build tree.
    if ! out=$(env -u GRASM_RUNTIME "$g" run check.gr 2>&1) || [ "$out" != "release ok 3" ]; then
        echo "FAIL $n: grasm run: $out"; fail=1; cd "$ROOT"; continue
    fi
    if ! out=$(env -u GRASM_RUNTIME "$g" build check.gr --native -o check 2>&1); then
        echo "FAIL $n: grasm build --native: $out"; fail=1; cd "$ROOT"; continue
    fi
    if ! out=$(./check 2>&1) || [ "$out" != "release ok 3" ]; then
        echo "FAIL $n: the native binary: $out"; fail=1; cd "$ROOT"; continue
    fi
    echo "ok $n: run, build --native and the native binary all work from the archive alone ($(grep '^commit' "$(dirname "$g")/BUILD-INFO"))"
    cd "$ROOT"
done
exit $fail
