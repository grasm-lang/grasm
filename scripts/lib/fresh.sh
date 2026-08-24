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
}
