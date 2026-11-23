# What a wasm module cannot run, and why: the rules `scripts/wasmtest.sh` set
# aside programs by, in one place, so that the gates holding the two wasm
# engines (`wasmtest`, the Rust interpreter built for wasm; `wasmsubset`, the
# Grasm backend's modules, docs/self-hosting.md §6.24) set aside the same
# programs for the same reasons and cannot drift apart.
#
# `wasm_skip_reason <src> <subject>` prints the reason `<src>` is set aside --
# the words `wasmtest` prints after the program's name -- or nothing when it
# runs. `<subject>` is what the shape checks read: the program's source
# bundle when it imports modules, so a library that opens a socket counts.
#
# The comments on each rule are `wasmtest`'s, which found each one the hard
# way.

wasm_skip_reason() {
    local src=$1 subject=$2 need
    # This list is the harness's model of what the target cannot do, and it
    # has to grow when the library does. `write_bytes` and `proc_pipeline`
    # arrived after it was written, so two conformance programs were reported
    # as *mismatches* rather than skips. It happened again with the five file
    # calls a store needed. Anything added to `[L-LIB-BYTES]`'s file table
    # belongs here on the same day.
    #
    # A `concurrent` block's tasks are fibers, and wasm32 has no way to switch
    # stacks. The pattern is the block, not the word, so T-EFFECT-BUILTIN's
    # `fn concurrent()` still runs; it holds no colon, because the reason
    # starts at the first one.
    local threads="par_map:threads par_filter:threads spawn:threads"
    # `WASM_SKIP_NO_THREADS=1` leaves the three thread rules out, for a gate
    # that holds those programs (the Rust module runs them, on one core).
    [ "${WASM_SKIP_NO_THREADS:-}" = 1 ] && threads=""
    for need in $threads \
                tcp_listen:sockets tcp_accept:sockets tcp_connect:sockets \
                udp_bind:sockets net_wait:sockets \
                read_file:filesystem write_file:filesystem \
                read_bytes:filesystem write_bytes:filesystem \
                read_bytes_at:filesystem append_bytes:filesystem \
                file_size:filesystem sync_file:filesystem \
                rename_file:filesystem remove_file:filesystem \
                file_exists:filesystem \
                random_bytes:filesystem \
                list_dir:filesystem make_dir:filesystem \
                proc_run:processes proc_pipeline:processes \
                'concurrent[^a-z_(]*{:stack-switching'; do
        # Code, not prose: a name in a comment is not a need. `grep -c` rather
        # than `grep -q`, which exits at the first match and lets `pipefail`
        # take the killed upstream's status; `[[:space:]]` rather than `\s`,
        # which not every grep reads as whitespace.
        if grep -vE '^[[:space:]]*(//|#)' "$subject" | grep -c "${need%%:*}" >/dev/null; then
            echo "needs ${need##*:}, which wasm32-unknown-unknown has none of"
            return 0
        fi
    done
    # `extern fn` reaches a C symbol, which needs a linker and a process that
    # has one loaded. A wasm module has neither, and says so by name.
    if grep -qE '^\s*(pub )?extern fn ' "$subject"; then
        echo "calls a C function, and a wasm module has no linker"
        return 0
    fi
    # A wasm module's stack is fixed, and cannot be enlarged from inside, so
    # the depth [E-RECURSION-DEPTH] promises is not reachable here. These
    # programs exist to recurse until something gives (and `scheme-demo` is
    # an interpreter run by an interpreter, every Scheme call two stacks).
    case "$src" in
        */E-RECURSION-DEPTH.gr|*/E-PANIC-TRACE.gr|*/scheme-demo/src/main.gr)
            echo "recurses past the stack a wasm module is given"
            return 0
            ;;
    esac
    return 0
}
