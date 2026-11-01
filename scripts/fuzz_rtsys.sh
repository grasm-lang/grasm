#!/usr/bin/env bash
# The system families on the Grasm runtime must be the Rust runtime's
# (docs/self-hosting.md §6.12): files and directories, arguments, the
# environment, standard input, other programs, the network, and the
# record-and-replay journal they all go through.
#
# Three programs in `compiler/rtsys`, each built twice -- linked as it ships
# and with the Grasm runtime alone (`scripts/lib/rtswap.sh`) -- and the
# swapped builds must take the families' names from the Grasm runtime:
#
#   * `sysfuzz.gr`: random operations on files and directories in an empty
#     scratch directory (the same path for both builds, made afresh for
#     each run), with arguments, environment variables set, empty, unset and
#     not UTF-8, and standard input of lines with a carriage return and bytes
#     that are not UTF-8; and other programs run, found and not, alone and
#     as pipelines fed input. Every seed must print, write to standard error
#     and exit the same on both. Then each seed is recorded on both builds
#     (`GRASM_RECORD`): the two journals must be the same bytes, and each,
#     replayed on the other build (`GRASM_REPLAY`), must print what the
#     recording run printed. And each of the family's panics and exits.
#   * `netfuzz.gr`: TCP and UDP on the loopback address -- connections,
#     text and bytes, waits, closes, handles used after, the wrong kind of
#     socket, ports out of range and unused -- printed only where timing and
#     the ports the system chose cannot show.
#   * `jfuzz.gr` replays journals written by hand (`scripts/rtjournal.py`):
#     values of every shape a journal holds read back, the clocks answering
#     what the journal says, and every way a replay stops -- a journal run
#     out, a call it does not name, a name not UTF-8, a file that is not a
#     journal or ends mid-record or mid-value, bytes after a value, a tag
#     unknown, a C call -- the same on both. Some journals must replay whole
#     and some be refused, or the generator compared one kind only.
#
#   RTSYS_SEEDS (12) seeds of RTSYS_ROUNDS (60) rounds; RTSYS_NET (6) network
#   seeds; RTSYS_JOURNALS (200) hand-written journals. `RT_OBJ=`, `S3=` and
#   `LINK=` as for `scripts/rtswap.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
. "$(dirname "$0")/lib/rtswap.sh"
s3_driver
link_driver
rtswap_runtime
rtswap_families
rtswap_linker

SEEDS=${RTSYS_SEEDS:-12}
ROUNDS=${RTSYS_ROUNDS:-60}
NET=${RTSYS_NET:-6}
JOURNALS=${RTSYS_JOURNALS:-200}
PY=${PYTHON:-python3}

for p in sysfuzz netfuzz jfuzz; do
    mkdir -p "$WORK/$p"
    if ! "$GRASM" build "compiler/rtsys/$p.gr" --native -o "$WORK/$p/f" >"$WORK/build.log" 2>&1 || [ ! -x "$WORK/$p/f.swap" ]; then
        echo "error: compiler/rtsys/$p.gr does not build:"
        sed 's/^/  /' "$WORK/build.log" | head -10
        exit 1
    fi
done
need_names() {
    local log=$1 n
    shift
    for n in "$@"; do
        if ! grep -qx "grasm $n" "$log"; then
            echo "FAIL: $log: the swapped build did not take $n from the Grasm runtime, so it compared nothing"
            exit 1
        fi
    done
}
need_names "$WORK/sysfuzz/f.swaplog" read_file write_file append_bytes read_bytes read_bytes_at file_size file_exists remove_file rename_file make_dir make_dir_new list_dir sync_file write_bytes proc_run proc_pipeline args env input mem_alloc str_c_new
need_names "$WORK/netfuzz/f.swaplog" tcp_listen tcp_connect tcp_accept tcp_read tcp_read_bytes tcp_write tcp_write_bytes tcp_peer net_close net_wait net_port udp_bind udp_send udp_recv udp_recv_from
need_names "$WORK/jfuzz/f.swaplog" now_millis monotonic_millis monotonic_nanos sleep_millis random_bytes foreign_call
taken=$(cat "$WORK"/*/f.swaplog | grep '^grasm ' | LC_ALL=C sort -u | awk '{ print $2 }' | LC_ALL=C comm -12 - <(awk '$1 == "sys" { print $2 }' "$RT_FAMILIES" | LC_ALL=C sort -u) | grep -c .)

DIR="$WORK/scratch"
printf 'line one\r\nsecond \xc3\xa9\n\xff\xfe bad\nlast' >"$WORK/stdin.txt"

# Runs build `$1` of sysfuzz in a fresh scratch directory, the rest its
# arguments after the directory; output to `$WORK/<$2>.out` and `.err`.
sysrun() {
    local bin=$1 tag=$2
    shift 2
    rm -rf "$DIR"
    mkdir -p "$DIR"
    env SYSFUZZ_SET=value SYSFUZZ_EMPTY= SYSFUZZ_BAD=$'\xff' SYSFUZZ_UTF8='é ü' "$bin" "$DIR" "$@" <"$WORK/stdin.txt" >"$WORK/$tag.out" 2>"$WORK/$tag.err"
}

# Whether runs `a` and `b` printed, wrote and exited alike.
alike() {
    [ "$1" = "$2" ] && cmp -s "$WORK/a.out" "$WORK/b.out" && cmp -s "$WORK/a.err" "$WORK/b.err"
}

differ() {
    echo "FAIL: $1 (exit $2 and $3)"
    diff "$WORK/a.out" "$WORK/b.out" | head -8 | cut -c1-200 | sed 's/^/    /'
    diff "$WORK/a.err" "$WORK/b.err" | head -4 | cut -c1-200 | sed 's/^/    /'
    exit 1
}

same=0
recorded=0
replayed=0
k=1
while [ "$k" -le "$SEEDS" ]; do
    sysrun "$WORK/sysfuzz/f" a "$k" "$ROUNDS"
    ac=$?
    sysrun "$WORK/sysfuzz/f.swap" b "$k" "$ROUNDS"
    bc=$?
    alike "$ac" "$bc" || differ "sysfuzz seed $k: the two runtimes differ" "$ac" "$bc"
    if [ "$ac" != 0 ] || [ "$(grep -c . "$WORK/a.out")" -lt "$ROUNDS" ]; then
        echo "FAIL: sysfuzz seed $k: exit $ac, $(grep -c . "$WORK/a.out") lines"
        exit 1
    fi
    same=$((same + 1))
    # Recorded on both: the same journal; each replayed on the other build.
    GRASM_RECORD="$WORK/a.jrnl" sysrun "$WORK/sysfuzz/f" a "$k" "$ROUNDS"
    ac=$?
    cp "$WORK/a.out" "$WORK/rec.out"
    GRASM_RECORD="$WORK/b.jrnl" sysrun "$WORK/sysfuzz/f.swap" b "$k" "$ROUNDS"
    bc=$?
    alike "$ac" "$bc" || differ "sysfuzz seed $k, recording: the two runtimes differ" "$ac" "$bc"
    if ! cmp -s "$WORK/a.jrnl" "$WORK/b.jrnl"; then
        echo "FAIL: sysfuzz seed $k: the two runtimes wrote different journals ($(wc -c <"$WORK/a.jrnl" | tr -d ' ') and $(wc -c <"$WORK/b.jrnl" | tr -d ' ') bytes; first difference: $(cmp "$WORK/a.jrnl" "$WORK/b.jrnl" | head -1))"
        exit 1
    fi
    if [ "$(wc -c <"$WORK/a.jrnl")" -le 10 ]; then
        echo "FAIL: sysfuzz seed $k: the journal holds no record"
        exit 1
    fi
    recorded=$((recorded + 1))
    GRASM_REPLAY="$WORK/a.jrnl" "$WORK/sysfuzz/f.swap" /nowhere 0 0 >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_REPLAY="$WORK/b.jrnl" "$WORK/sysfuzz/f" /nowhere 0 0 >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    alike "$ac" "$bc" || differ "sysfuzz seed $k: the replays differ" "$ac" "$bc"
    if [ "$ac" != 0 ] || ! cmp -s "$WORK/a.out" "$WORK/rec.out"; then
        echo "FAIL: sysfuzz seed $k: a replay did not print what the recording run printed (exit $ac)"
        diff "$WORK/rec.out" "$WORK/a.out" | head -6 | sed 's/^/    /'
        exit 1
    fi
    replayed=$((replayed + 2))
    k=$((k + 1))
done

panics=0
p=0
while [ "$p" -le 5 ]; do
    sysrun "$WORK/sysfuzz/f" a 1 3 "$p"
    ac=$?
    sysrun "$WORK/sysfuzz/f.swap" b 1 3 "$p"
    bc=$?
    alike "$ac" "$bc" || differ "sysfuzz panic case $p: the two runtimes differ" "$ac" "$bc"
    if [ "$ac" = 0 ]; then
        echo "FAIL: sysfuzz panic case $p ended with exit 0"
        exit 1
    fi
    panics=$((panics + 1))
    p=$((p + 1))
done

net=0
k=1
while [ "$k" -le "$NET" ]; do
    "$WORK/netfuzz/f" "$k" "$ROUNDS" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    "$WORK/netfuzz/f.swap" "$k" "$ROUNDS" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    alike "$ac" "$bc" || differ "netfuzz seed $k: the two runtimes differ" "$ac" "$bc"
    if [ "$ac" != 0 ] || [ "$(grep -c . "$WORK/a.out")" -lt "$ROUNDS" ]; then
        echo "FAIL: netfuzz seed $k: exit $ac, $(grep -c . "$WORK/a.out") lines"
        exit 1
    fi
    net=$((net + 1))
    k=$((k + 1))
done

js=0
whole=0
refused=0
k=1
while [ "$k" -le "$JOURNALS" ]; do
    kind=$("$PY" scripts/rtjournal.py "$k" "$WORK/j.jrnl") || { echo "error: scripts/rtjournal.py failed"; exit 1; }
    GRASM_REPLAY="$WORK/j.jrnl" "$WORK/jfuzz/f" >"$WORK/a.out" 2>"$WORK/a.err" </dev/null
    ac=$?
    GRASM_REPLAY="$WORK/j.jrnl" "$WORK/jfuzz/f.swap" >"$WORK/b.out" 2>"$WORK/b.err" </dev/null
    bc=$?
    alike "$ac" "$bc" || differ "journal $k ($kind): the two runtimes differ" "$ac" "$bc"
    if [ "$ac" = 0 ]; then
        whole=$((whole + 1))
    elif [ "$ac" = 7 ] && grep -q '^grasm: ' "$WORK/a.err"; then
        refused=$((refused + 1))
    else
        echo "FAIL: journal $k ($kind): exit $ac, neither replayed whole nor refused"
        head -3 "$WORK/a.err" | sed 's/^/    /'
        exit 1
    fi
    js=$((js + 1))
    k=$((k + 1))
done
if [ "$JOURNALS" -gt 20 ] && { [ "$whole" -lt 5 ] || [ "$refused" -lt 5 ]; }; then
    echo "FAIL: the hand-written journals were $whole replayed whole and $refused refused: the generator compared one kind only"
    exit 1
fi

echo "files, input, environment, programs: seeds identical: $same of $SEEDS ($((SEEDS * ROUNDS)) rounds)   journals recorded byte-identical: $recorded   replayed on the other runtime as recorded: $replayed   panics and exits identical: $panics of 6"
echo "network: seeds identical: $net of $NET ($((NET * ROUNDS)) rounds)"
echo "hand-written journals identical: $js of $JOURNALS ($whole replayed whole, $refused refused)   names of the family taken from Grasm: $taken"
[ "$same" -eq "$SEEDS" ] && [ "$net" -eq "$NET" ] && [ "$js" -eq "$JOURNALS" ] && [ "$SEEDS" -ge 1 ]
