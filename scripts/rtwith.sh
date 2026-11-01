#!/usr/bin/env bash
# A gate or a generator, with every native program it builds linked with the
# Grasm runtime alone in place of the Rust one (docs/self-hosting.md §6.7,
# §6.12).
#
#     scripts/rtwith.sh python3 scripts/fuzz_operators.py [its arguments]
#     scripts/rtwith.sh bash scripts/leakcheck.sh
#     RTWITH_POISON=1 scripts/rtwith.sh bash scripts/usecheck.sh
#
# Builds the Grasm runtime as `scripts/rtswap.sh` does and holds it to
# `compiler/rt/families.txt`, then runs the command with
# `scripts/lib/rtswap_link` (`RTSWAP_ONLY=1`) as the linker of both native
# engines: `grasm build --native` (`GRASM_LINKER`) and the Grasm backend's
# (`S3_LINKER`, `scripts/lib/s3engine.py`, `scripts/lib/s3.sh`), each link
# without the Rust runtime archive on its line. Everything else the command
# compares against -- the interpreter, the other engine -- is unchanged, so
# what it reports is about the Grasm runtime. `RTWITH_POISON=1` links the
# Grasm runtime's poisoned build instead, for `usecheck` (which then need
# not build the Rust runtime's: `RT_GRASM_ALONE`). `GRASM=` must name the
# compiler: the command's own freshness check would otherwise rebuild it
# without the switch.
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
if [ $# -lt 1 ]; then
    echo "usage: scripts/rtwith.sh <command> [arguments]"
    exit 2
fi
s3_driver
link_driver
rtswap_runtime
rtswap_families
export S3 GRASM
export RTSWAP_LINK="$LINK" RTSWAP_RT="$RT_OBJ" RTSWAP_ONLY=1 RT_GRASM_ALONE=1
if [ "${RTWITH_POISON:-}" = 1 ]; then
    export RTSWAP_RT="$RT_OBJ_POISON"
fi
export GRASM_LINKER="$PWD/scripts/lib/rtswap_link" S3_LINKER="$PWD/scripts/lib/rtswap_link"
# That the switch is in effect: a program built under it took runtime names
# from Grasm.
if ! RTSWAP_LOG="$WORK/canary.log" "$GRASM" build examples/hello.gr --native -o "$WORK/canary" >"$WORK/canary.out" 2>&1 ||
    ! grep -q '^grasm ' "$WORK/canary.log"; then
    echo "error: a program built with the switch took nothing from the Grasm runtime: $(head -2 "$WORK/canary.out")"
    exit 1
fi
if grep -q '^rust ' "$WORK/canary.log"; then
    echo "error: a program built with the switch took names from the Rust runtime: $(grep '^rust ' "$WORK/canary.log" | head -3 | tr '\n' ' ')"
    exit 1
fi
echo "the switch is in effect: examples/hello.gr took $(grep -c '^grasm ' "$WORK/canary.log") runtime names from Grasm and none from Rust, linked with no Rust runtime archive"
"$@"
