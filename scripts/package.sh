#!/usr/bin/env bash
# Builds one release archive and proves it works from nothing but itself.
#
#   scripts/package.sh [TARGET-TRIPLE]      default: this machine's
#
# Leaves dist/grasm-<platform>.tar.gz and a line for it in dist/SHA256SUMS.
# The archive holds `grasm`, `libgrasm_rt_abi.a` (what `--native` links),
# on arm64 macOS `grasm-link` (what links it, docs/self-hosting.md S4), the
# README, licence and language pack, and BUILD-INFO naming the commit it
# was built from. v1.0.0 shipped two bare binaries: `grasm run` worked from
# the download and `grasm build --native` failed with "cannot find the Grasm
# runtime", because the runtime was not in it. Nothing had tried.
#
# So the last step is scripts/verify_release.sh on the archive just made: it
# unpacks into an empty directory and runs a program, builds it natively, and
# runs the result. A release that fails that is not written to SHA256SUMS.
#
# A dirty tree is refused, because BUILD-INFO would name a commit the archive
# was not built from; GRASM_PACKAGE_DIRTY=1 overrides for a trial run, and
# BUILD-INFO then says so.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET=${1:-$(rustc -vV | sed -n 's/^host: //p')}
case "$TARGET" in
    aarch64-apple-darwin) NAME=macos-arm64 ;;
    x86_64-apple-darwin) NAME=macos-x86_64 ;;
    x86_64-unknown-linux-gnu) NAME=linux-x86_64 ;;
    aarch64-unknown-linux-gnu) NAME=linux-arm64 ;;
    *) echo "no archive name for $TARGET"; exit 1 ;;
esac
COMMIT=$(git rev-parse HEAD)
DIRTY=no
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    if [ "${GRASM_PACKAGE_DIRTY:-}" != 1 ]; then
        echo "the working tree has uncommitted changes; an archive built now would"
        echo "not be the commit BUILD-INFO names. Commit first, or set"
        echo "GRASM_PACKAGE_DIRTY=1 for a trial archive that says it is one."
        exit 1
    fi
    DIRTY=yes
fi

HOST=$(rustc -vV | sed -n 's/^host: //p')
if [ "$TARGET" = "$HOST" ]; then
    cargo build --release -p grasm -p grasm_rt_abi
    OUT=target/release
else
    cargo build --release -p grasm -p grasm_rt_abi --target "$TARGET"
    OUT=target/$TARGET/release
fi

# The linker `--native` links with on arm64 macOS (compiler/link), built by
# the compiler just built, linked by cc and then by itself; elsewhere `cc`
# links, and the archive has no linker of its own.
LINKER=()
if [ "$TARGET" = aarch64-apple-darwin ] && [ "$TARGET" = "$HOST" ]; then
    GRASM_LINKER=cc "$OUT/grasm" build compiler/link/src/main.gr --native -o "$OUT/grasm-link.stage"
    GRASM_LINKER="$PWD/$OUT/grasm-link.stage" "$OUT/grasm" build compiler/link/src/main.gr --native -o "$OUT/grasm-link"
    rm -f "$OUT/grasm-link.stage"
    LINKER=("$OUT/grasm-link")
fi

DIR=dist/grasm-$NAME
rm -rf "$DIR" "dist/grasm-$NAME.tar.gz"
mkdir -p "$DIR"
cp "$OUT/grasm" "$OUT/libgrasm_rt_abi.a" ${LINKER[@]+"${LINKER[@]}"} README.md LICENSE NOTICE "$DIR/"
[ -f dist/grasm-language-pack.md ] && cp dist/grasm-language-pack.md "$DIR/"
{
    echo "commit: $COMMIT"
    echo "uncommitted changes: $DIRTY"
    echo "target: $TARGET"
    echo "rustc: $(rustc -V)"
    echo "version: $("$OUT/grasm" --version 2>/dev/null || echo unknown)"
} > "$DIR/BUILD-INFO"
tar -czf "dist/grasm-$NAME.tar.gz" -C dist "grasm-$NAME"

bash scripts/verify_release.sh "dist/grasm-$NAME.tar.gz"

touch dist/SHA256SUMS
grep -v " grasm-$NAME.tar.gz\$" dist/SHA256SUMS > dist/SHA256SUMS.new || true
(cd dist && shasum -a 256 "grasm-$NAME.tar.gz") >> dist/SHA256SUMS.new
sort -k2 dist/SHA256SUMS.new > dist/SHA256SUMS
rm dist/SHA256SUMS.new
echo "packaged dist/grasm-$NAME.tar.gz from $COMMIT (uncommitted changes: $DIRTY)"
