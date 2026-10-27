#!/usr/bin/env bash
# The runtime dialect, lowered by the Grasm backend ([T-RUNTIME],
# docs/self-hosting.md §5.6 and §6.9): `scripts/fixtures/rtdialect`, a
# runtime package using every part of the dialect, compiled with
# `s3 --runtime` and driven from C.
#
# The object, compiled twice, must be the same bytes, carry the forms the
# dialect lowers to, and pass `dwarfdump --verify`:
#
#   * each process cell a local symbol in `__DATA,__bss` (S_ZEROFILL), taking
#     no room in the file, addressed by `adrp`/`add`;
#   * each `mem_static` literal in `__TEXT,__cstring`, the one holding a NUL
#     in `__TEXT,__const` with a NUL of its own;
#   * each of the runtime's own `thread_local` cells a Mach-O thread-local
#     variable -- a descriptor in `__thread_vars` bound to `__tlv_bootstrap`,
#     a template in `__thread_bss` -- reached through TLVP relocations by a
#     getter that signs its return address like any compiled function;
#   * `ctx_switch` and `ctx_init` as leaves that save and load x19..x30, sp
#     and d8..d15 and sign nothing (no `pacibsp`, no `retab`), while every
#     compiled function signs its return address (`pacibsp` ... `retab`).
#
# Then `driver.c` is linked with it and the Rust runtime twice -- by
# `compiler/link` and by `cc` -- and both must print the same checks, every
# one `ok` with the count it should have: the cells zero, distinct and
# 8-aligned; six threads adding to one cell to the exact total; the
# literals' bytes, lengths and NULs, and a write to one faulting in a child;
# the thread-local cells one per thread, zero on each; three fibers on
# mapped stacks switched round-robin for 200 rounds, each yielding from two
# calls deep and keeping its values across every switch, against a model of
# which fiber runs and what each computes. Whether the return-address keys
# are on in the driver's process is reported, not judged: in an arm64 (not
# arm64e) process on macOS `pacibsp` may leave the address as it was, and
# then the signing on either side of a switch is present but not enforced.
#
# `S3=` and `LINK=` name prebuilt tools (`scripts/rtdialectcontrols.sh`
# plants faults in them).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
s3_driver
link_driver
s3_runtime

FX=scripts/fixtures/rtdialect
failed=0
declare -a FAILED=()
fail() {
    failed=$((failed + 1))
    FAILED+=("$1")
    echo "FAIL: $1"
}

if ! "$GRASM" tir "$FX/src/main.gr" >"$WORK/fx.tir" 2>"$WORK/tir.log"; then
    echo "error: the fixture does not emit TIR:"
    sed 's/^/  /' "$WORK/tir.log" | head -20
    exit 1
fi
mkdir -p "$WORK/b"
if ! "$S3" "$WORK/fx.tir" -o "$WORK/fx.o" --runtime >"$WORK/s3.log" 2>&1 ||
    ! "$S3" "$WORK/fx.tir" -o "$WORK/b/fx.o" --runtime >>"$WORK/s3.log" 2>&1; then
    echo "error: the Grasm backend does not compile the fixture:"
    sed 's/^/  /' "$WORK/s3.log" | head -20
    exit 1
fi
cmp -s "$WORK/fx.o" "$WORK/b/fx.o" || fail "two compilations of the fixture differ"

# Without `--runtime` the dialect is refused, by name.
if "$S3" "$WORK/fx.tir" -o "$WORK/plain.o" >"$WORK/plain.log" 2>&1; then
    fail "the backend compiled the dialect without --runtime"
elif ! grep -q 'outside the runtime (`--runtime`)' "$WORK/plain.log"; then
    fail "the backend refused the dialect without --runtime, but not by name: $(head -1 "$WORK/plain.log")"
fi

# The object's forms.
otool -lv "$WORK/fx.o" >"$WORK/load.txt"
sect() { # the section header block of section $1
    awk -v s="$1" '$1 == "sectname" { on = ($2 == s) } on' "$WORK/load.txt"
}
sect __bss | grep -c 'type S_ZEROFILL' >/dev/null || fail "no __bss section of type S_ZEROFILL"
sect __bss | grep -c 'offset 0$' >/dev/null || fail "__bss takes room in the file"
sect __thread_vars | grep -c 'type S_THREAD_LOCAL_VARIABLES' >/dev/null || fail "no __thread_vars section"
sect __thread_bss | grep -c 'type S_THREAD_LOCAL_ZEROFILL' >/dev/null || fail "no __thread_bss section"
nm -m "$WORK/fx.o" >"$WORK/nm.txt"
cells=$(grep -c '(__DATA,__bss) non-external _grt\.cell\.main\.' "$WORK/nm.txt")
[ "$cells" = 6 ] || fail "$cells process cells in __bss, not 6"
tlvs=$(grep -c '(__DATA,__thread_vars) non-external _grt\.tls\.main\.' "$WORK/nm.txt")
inits=$(grep -c '(__DATA,__thread_bss) non-external _grt\.tls\.main\..*\$tlv\$init' "$WORK/nm.txt")
[ "$tlvs" = 2 ] && [ "$inits" = 2 ] || fail "$tlvs thread-local descriptors and $inits templates, not 2 and 2"
grep -q '(undefined) external __tlv_bootstrap' "$WORK/nm.txt" || fail "nothing binds the descriptors to __tlv_bootstrap"
otool -rv "$WORK/fx.o" >"$WORK/rel.txt"
tlvp=$(grep -c 'TLVLDP' "$WORK/rel.txt")
[ "$tlvp" = 4 ] || fail "$tlvp TLVP relocations, not 4 (two per getter)"
otool -s __TEXT __cstring -v "$WORK/fx.o" >"$WORK/cstr.txt"
for want in 'out of memory' 'tab\there "quoted" back\\slash\n'; do
    grep -qF "$want" "$WORK/cstr.txt" || fail "__cstring does not hold \"$want\""
done
otool -tv "$WORK/fx.o" >"$WORK/text.txt"
body() { # the disassembly of function $1
    awk -v f="$1:" '/^[^ \t].*:$/ { on = ($1 == f) ; next } on' "$WORK/text.txt"
}
for leaf in _grt.ctx_switch _grt.ctx_init _grt.ctx_start; do
    n=$(body "$leaf" | grep -c .)
    [ "$n" -gt 0 ] || fail "no leaf $leaf"
    body "$leaf" | grep -cE 'pacibsp|retab|autib' >/dev/null && fail "$leaf signs or authenticates"
done
[ "$(body _grt.ctx_switch | grep -cE '\bstp\b')" = 10 ] || fail "ctx_switch does not save x19..x30 and d8..d15 in ten pairs"
[ "$(body _grt.ctx_switch | grep -cE '\bldp\b')" = 10 ] || fail "ctx_switch does not load x19..x30 and d8..d15 in ten pairs"
body _grt.ctx_switch | tail -1 | grep -cE '\bret$' >/dev/null || fail "ctx_switch does not end in a plain ret"
signed=0
for fn in $(awk '/^[^ \t].*:$/ && $1 != "_grt.ctx_switch:" && $1 != "_grt.ctx_init:" && $1 != "_grt.ctx_start:" && $1 !~ /^[(\/]/ { sub(/:$/, "", $1); print $1 }' "$WORK/text.txt"); do
    if body "$fn" | head -1 | grep -c pacibsp >/dev/null && body "$fn" | grep -c retab >/dev/null; then
        signed=$((signed + 1))
    else
        fail "$fn does not sign its return address"
    fi
done
[ "$signed" -ge 30 ] || fail "only $signed compiled functions were found signing their return address"
if ! dwarfdump --verify "$WORK/fx.o" >"$WORK/verify.txt" 2>&1 || ! grep -cx "No errors." "$WORK/verify.txt" >/dev/null; then
    fail "dwarfdump --verify: $(grep -iE 'error|warning' "$WORK/verify.txt" | head -2 | tr '\n' ' ')"
fi

# Run it, linked both ways.
if ! cc -O2 -c "$FX/driver.c" -o "$WORK/driver.o" 2>"$WORK/cc.log"; then
    echo "error: driver.c does not compile:"
    sed 's/^/  /' "$WORK/cc.log" | head -10
    exit 1
fi
expected="ok cells 6
ok shared 600000
ok static 5
ok tls 4
ok switch 603
checks failed: 0"
ran=0
for how in link cc; do
    if [ "$how" = link ]; then
        "$LINK" "$WORK/driver.o" "$WORK/fx.o" "$S3_RT" -dead_strip -o "$WORK/fx-$how" >"$WORK/$how.log" 2>&1
    else
        cc "$WORK/driver.o" "$WORK/fx.o" "$S3_RT" -o "$WORK/fx-$how" -Wl,-dead_strip >"$WORK/$how.log" 2>&1
    fi
    if [ $? -ne 0 ]; then
        fail "linked by $how: $(head -2 "$WORK/$how.log" | tr '\n' ' ')"
        continue
    fi
    $S3_RUN "$WORK/fx-$how" >"$WORK/$how.out" 2>"$WORK/$how.err" </dev/null
    code=$?
    ran=$((ran + 1))
    if [ "$code" != 0 ] || [ "$(cat "$WORK/$how.out")" != "$expected" ]; then
        fail "linked by $how, the checks say (exit $code): $(grep -v '^ok ' "$WORK/$how.out" | head -3 | tr '\n' ' ') $(head -2 "$WORK/$how.err" | tr '\n' ' ')"
    fi
done
oks=$(grep -c '^ok ' "$WORK/link.out" 2>/dev/null)
grep -h '^pointer authentication' "$WORK/link.err" 2>/dev/null | head -1
echo "object: process cells $cells, thread-local variables $tlvs, TLVP relocations $tlvp, leaves unsigned 3, functions signed $signed"
echo "runs: $ran of 2   checks passed in the compiler/link build: ${oks:-0} of 5"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
[ "$ran" = 2 ] && [ "${oks:-0}" = 5 ] || exit 1
exit 0
