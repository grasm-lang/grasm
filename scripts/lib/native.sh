# One rule for building a program the gates are about to run once.

# Most of what a gate spends on a native program is neither compiling it nor
# running it. On macOS a freshly created executable costs ~450ms the first time
# it runs and ~3ms every time after, and a byte-identical copy at a new path
# pays the ~450ms again, so the cost belongs to the file rather than to the
# program. It does not grow with size -- a 12MB binary is admitted no slower
# than a 0.5MB one -- so the way out is fewer files, not smaller ones.
#
# Writing each new binary through one existing file, without unlinking it,
# keeps the inode and the decision already made about it. What executes is the
# new program; what is reused is the verdict, and the verdict is the part worth
# nothing here, on a binary this machine compiled seconds ago from source in
# this repository. This belongs to the gates and to nothing `grasm` emits for
# anyone else. Should a future macOS decide again on every write, the gates get
# slower and stay correct.
#
# Writes the build's own output on stdout, as running the compiler directly
# did, and leaves no binary behind when the build failed -- so the `[ -x ]`
# test every caller already makes cannot read the previous program's binary as
# this one's.
grasm_build_native() { # SRC OUT [extra grasm build args...]
    local src=$1 out=$2
    shift 2
    local stage="$out.stage" log rc
    log=$("$GRASM" build "$src" --native "$@" -o "$stage" 2>&1)
    rc=$?
    if [ ! -x "$stage" ]; then
        rm -f "$out" "$stage"
        printf '%s' "$log"
        return "$rc"
    fi
    if [ -e "$out" ]; then
        cat "$stage" >"$out"
        rm -f "$stage"
    else
        mv "$stage" "$out"
    fi
    printf '%s' "$log"
    return "$rc"
}
