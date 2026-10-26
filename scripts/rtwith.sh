#!/usr/bin/env bash
# A gate or a generator, with every native program it builds linked with the
# Grasm runtime in front of the Rust one (docs/self-hosting.md §6.7).
#
#     scripts/rtwith.sh python3 scripts/fuzz_operators.py [its arguments]
#     scripts/rtwith.sh bash scripts/leakcheck.sh
#
# Builds the Grasm runtime as `scripts/rtswap.sh` does and holds it to
# `compiler/rt/families.txt`, then runs the command with
# `scripts/lib/rtswap_link` (`RTSWAP_ONLY=1`) as the linker of both native
# engines: `grasm build --native` (`GRASM_LINKER`) and the Grasm backend's
# (`S3_LINKER`, `scripts/lib/s3engine.py`, `scripts/lib/s3.sh`). Everything
# else the command compares against -- the interpreter, the other engine,
# its leak check -- is unchanged, so what it reports is about the Grasm
# runtime. `GRASM=` must name the compiler: the command's own freshness check
# would otherwise rebuild it without the switch.
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
export RTSWAP_LINK="$LINK" RTSWAP_RT="$RT_OBJ" RTSWAP_ONLY=1
export GRASM_LINKER="$PWD/scripts/lib/rtswap_link" S3_LINKER="$PWD/scripts/lib/rtswap_link"
# That the switch is in effect: a program built under it took runtime names
# from Grasm.
if ! RTSWAP_LOG="$WORK/canary.log" "$GRASM" build examples/hello.gr --native -o "$WORK/canary" >"$WORK/canary.out" 2>&1 ||
    ! grep -q '^grasm ' "$WORK/canary.log"; then
    echo "error: a program built with the switch took nothing from the Grasm runtime: $(head -2 "$WORK/canary.out")"
    exit 1
fi
echo "the switch is in effect: examples/hello.gr took $(grep -c '^grasm ' "$WORK/canary.log") runtime names from Grasm"
"$@"
