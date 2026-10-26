# A gate must produce the artefact it tests.
#
# Every script in this directory read `./target/release/grasm` and none of them
# built it. So a gate could pass against a compiler that no longer compiles —
# which is not hypothetical: `scripts/wasmtest.sh` did exactly that for two full
# runs after the wasm target stopped building, and went on reporting 182
# identical against a module from the previous week (F36).
#
# Cargo is a no-op when nothing changed, so the cost of closing the whole class
# is a tenth of a second per gate.
#
# Skipped when the caller named its own binary through `GRASM=`: then the
# artefact under test is theirs, and building ours would test the wrong thing.
# `GRASM_NO_BUILD=1` skips it outright, for a machine with no toolchain.
fresh_or_stop() {
    default=$1
    [ "${GRASM_NO_BUILD:-}" = 1 ] && return 0
    [ "$GRASM" = "$default" ] || return 0
    if ! cargo build --release --workspace -q; then
        echo "error: the workspace does not build — every result below would be"
        echo "       measured against a stale binary, so there are none."
        exit 1
    fi
    if ! linker_fresh; then
        echo "error: compiler/link does not build — grasm build --native links with it"
        exit 1
    fi
}

# `target/release/grasm-link`, the linker `grasm build --native` links with on
# arm64 macOS (compiler/link, docs/self-hosting.md stage S4), is a Grasm
# program, so cargo does not build it: the compiler just built does, when the
# linker's source, a package it uses or the compiler is newer than it. Linked
# first by `cc` and then by itself, so the one the gates use was linked the
# way every other native build is.
linker_fresh() {
    [ "$(uname -s)-$(uname -m)" = Darwin-arm64 ] || return 0
    # From the checkout's root, wherever the gate runs from (ProjectGold/run.sh
    # runs in ProjectGold/).
    local root
    root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd) || return 1
    local out="$root/target/release/grasm-link" newer
    if [ -x "$out" ]; then
        newer=$(cd "$root" && find compiler/link packages/codesig packages/exporttrie packages/crypto_hash \
            packages/machoread packages/sha1 packages/sha512 target/release/grasm -newer "$out" 2>/dev/null | head -1)
        [ -z "$newer" ] && return 0
    fi
    GRASM_LINKER=cc "$root/target/release/grasm" build "$root/compiler/link/src/main.gr" --native -o "$out.stage" >/dev/null &&
        GRASM_LINKER="$out.stage" "$root/target/release/grasm" build "$root/compiler/link/src/main.gr" --native -o "$out" >/dev/null &&
        rm -f "$out.stage"
}
