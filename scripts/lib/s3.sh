# What the S3 gates share: the backend's driver, and the corpus programs in
# the subset it compiles (docs/self-hosting.md, stage S3).
#
# Sourced after `corpus.sh` and `fresh.sh`, with `GRASM` and `WORK` set.

# The driver under test, in `$S3`. Built here from `compiler/s3`, natively
# and once per gate, so no gate passes against a backend older than its
# source. A caller that names its own binary through `S3=` -- the mutation
# controls do, with a deliberately broken one -- gets that binary instead.
s3_driver() {
    if [ -n "${S3:-}" ]; then
        if [ ! -x "$S3" ]; then
            echo "error: S3=$S3 is not an executable"
            exit 1
        fi
        return 0
    fi
    S3="$WORK/s3"
    if ! "$GRASM" build compiler/s3/src/main.gr --native -o "$S3" >"$WORK/s3-build.log" 2>&1; then
        echo "error: compiler/s3 does not build:"
        sed 's/^/  /' "$WORK/s3-build.log"
        exit 1
    fi
}

# Links object `$1` into executable `$2` with `cc` and the runtime, as `grasm
# build --native` links a Cranelift object, writing through `$2` if it exists
# so the file keeps its inode and macOS its verdict on it (`lib/native.sh`
# explains the ~450ms that saves per program). The linker's own output goes
# to `$3`. Fails, leaving no executable behind, when the link does.
s3_link() {
    local obj=$1 out=$2 log=$3 stage="$2.stage"
    if ! cc "$obj" "$S3_RT" -o "$stage" -Wl,-dead_strip >"$log" 2>&1; then
        rm -f "$out" "$stage"
        return 1
    fi
    if [ -e "$out" ]; then
        cat "$stage" >"$out"
        rm -f "$stage"
    else
        mv "$stage" "$out"
    fi
}

# The runtime every native build links against.
s3_runtime() {
    S3_RT=./target/release/libgrasm_rt_abi.a
    if [ ! -f "$S3_RT" ]; then
        echo "error: $S3_RT is missing"
        exit 1
    fi
}

# Every corpus program with a `main`, its TIR, and whether the subset rule
# (`scripts/s3_subset.py`) has it in: `S3_SRC[k]`, `S3_TIR[k]` and
# `S3_RULE[k]` ("in" or "out: <reasons>"). A program that emits no TIR is
# recorded in `S3_NOTIR` and fails the gate: `tirtest` requires the whole
# corpus to emit. The work is `scripts/s3_collect.py`'s, which emits the TIR
# eight programs at a time and asks the rule of each, in the corpus's order.
s3_collect() {
    S3_SRC=()
    S3_TIR=()
    S3_RULE=()
    S3_NOTIR=()
    corpus_programs >"$WORK/programs.txt" || exit 1
    if ! python3 scripts/s3_collect.py "$GRASM" "$WORK/tir" <"$WORK/programs.txt" >"$WORK/collected.tsv"; then
        echo "error: scripts/s3_collect.py failed"
        exit 1
    fi
    local a b c
    while IFS=$'\t' read -r a b c; do
        if [ "$a" = "NOTIR" ]; then
            S3_NOTIR+=("$b — emits no TIR: $c")
        else
            S3_SRC+=("$a")
            S3_TIR+=("$b")
            S3_RULE+=("$c")
        fi
    done <"$WORK/collected.tsv"
}

# How many programs the rule has in; a floor below which the subset is too
# small for agreement on it to mean much. 37 programs were in when the gates
# were written (2026-10-21); a shrinking subset should be noticed.
S3_SUBSET_FLOOR=30
