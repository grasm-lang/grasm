#!/usr/bin/env bash
# A panic leaves with 101, says `panic: ` on stderr, and writes nothing to
# stdout -- on every engine.
#
# Nothing checked this on a compiled program. difftest skips the conformance
# programs tagged `expect: error`, the one tagged E-PANIC-EXIT *catches* its
# panic and exits 0, and no ProjectGold program panics. The only cover was
# `cargo test`, and when the path broke that suite did not go red: it **hung**,
# for an hour and a half, which is not a result anybody reads.
#
# Three runtime paths print and then exit while holding that lock: a panic, an
# unhandled error from main, and the leak report. The first two are exercised
# below. The third is not, and cannot be from here: it needs a program that
# actually leaks under GRASM_LEAK_CHECK=1, and a leak that exists on purpose is
# a defect, not a fixture. Its guard is scoped and `flush_all` skips a held
# stream, which is the cover it has.
#
# So every run here has a timeout, and a timeout is a failure. That is the
# point of the gate as much as the exit code is: `rt_panic` held the stderr
# lock across `crate::sys::exit`, which flushes stderr again, so every panic in
# every natively compiled program printed its message and then deadlocked in
# `pthread_mutex_lock` (F101). `std`'s stderr lock is reentrant and hid it; the
# `no_std` runtime's plain pthread mutex is not.
set -u
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Long enough that a slow machine is not a failure, short enough that a
# deadlock is reported rather than waited out.
LIMIT=20

prog() {
  case "$1" in
    message)  printf 'fn boom() -> i64 {\n    panic("mid")\n}\n\nfn main() {\n    print("{boom()}")\n}\n' ;;
    divzero)  printf 'fn main() {\n    let n = input().len()\n    print("{10 / n}")\n}\n' ;;
    assert)   printf 'fn main() {\n    assert(input().len() > 100)\n    print("unreached")\n}\n' ;;
    after)    printf 'fn main() {\n    print("before")\n    assert(input().len() > 100)\n}\n' ;;
    mainerr)  printf 'fn main() {\n    print("before")\n    write_file("/no/such/dir/x.txt", "data")\n}\n' ;;
  esac
}

# Reading stdin keeps the panic out of the constant folder: a literal zero
# divisor is refused at build time, which is right, and is not what this gate
# is for. Every program here is fed nothing, so `input()` gives "" and its
# length is the zero the divisor needs.

# Run $2 under a time limit. Prints "exit:<code>" or "timeout".
limited() {
  local out_f="$1"; shift
  local err_f="$1"; shift
  local in_f="$1"; shift
  "$@" < "$in_f" > "$out_f" 2> "$err_f" &
  local pid=$!
  local waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge "$LIMIT" ]; then
      kill -9 "$pid" 2>/dev/null
      wait "$pid" 2>/dev/null
      echo "timeout"
      return
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$pid"; echo "exit:$?"
}

# What each program must leave behind. The three runtime paths that print and
# then exit do not share an exit code -- a panic is 101, an unhandled error
# from main is 1 -- and the gate is about the exit happening at all.
expected() {
  case "$1" in
    mainerr) echo "1|error: unhandled error from main" ;;
    *)       echo "101|panic: " ;;
  esac
}

fail=0
checked=0
for name in message divzero assert after mainerr; do
    src="$WORK/$name.gr"
    prog "$name" > "$src"
    : > "$WORK/in.txt"
    # What stdout is allowed to hold: a panic must not corrupt the program's
    # output, and `after` is here because something was printed before it.
    case "$name" in after|mainerr) want_out="before" ;; *) want_out="" ;; esac
    want=$(expected "$name")
    want_code=${want%%|*}
    want_err=${want#*|}

    if ! "$GRASM" build "$src" -o "$WORK/$name.bundle" >/dev/null 2>&1; then
        echo "$name: the bundle did not build"; fail=1; continue
    fi
    if ! "$GRASM" build "$src" --native -o "$WORK/$name.native" >/dev/null 2>&1; then
        echo "$name: the native build did not build"; fail=1; continue
    fi

    for engine in interpreter bundle native; do
        case "$engine" in
            interpreter) set -- "$GRASM" run "$src" ;;
            bundle)      set -- "$WORK/$name.bundle" ;;
            native)      set -- "$WORK/$name.native" ;;
        esac
        verdict=$(limited "$WORK/o" "$WORK/e" "$WORK/in.txt" "$@")
        checked=$((checked + 1))
        got_out=$(cat "$WORK/o")
        got_err=$(cat "$WORK/e")
        if [ "$verdict" = "timeout" ]; then
            echo "$name on the $engine: did not finish in ${LIMIT}s -- a panic deadlocked"
            echo "    stderr so far: $(printf '%s' "$got_err" | head -1)"
            fail=1
            continue
        fi
        code=${verdict#exit:}
        problems=""
        [ "$code" = "$want_code" ] || problems="$problems exit=$code(want $want_code)"
        case "$got_err" in *"$want_err"*) ;; *) problems="$problems stderr-missing-[$want_err]" ;; esac
        [ "$got_out" = "$want_out" ] || problems="$problems stdout=[$got_out](want [$want_out])"
        if [ -n "$problems" ]; then
            echo "$name on the $engine:$problems"; fail=1
        fi
    done
done

if [ "$fail" -eq 0 ]; then
    echo "panics: $checked runs, every one left with its own code, its line and clean stdout"
else
    echo "panics: FAILED"
fi
exit "$fail"
