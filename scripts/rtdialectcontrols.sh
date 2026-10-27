#!/usr/bin/env bash
# Planted faults in the Grasm backend's lowering of the runtime dialect
# ([T-RUNTIME]), each of which scripts/rtdialect.sh must catch for its
# reason. Each control copies compiler/, changes one thing, builds `s3` from
# the copy and runs the gate with it:
#
#   switch-forgets ctx_switch does not save x19 and x20: what a caller keeps
#                  there is lost across a switch
#   switch-signed  ctx_switch signs its return address like a function
#   cells-one      every process cell given the first cell's symbol
#   static-len     `mem_static_len` one byte long
#   tlv-shared     the thread-local getter answers the descriptor, which every
#                  thread shares, instead of calling its thunk
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
export GRASM
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/link.sh"
link_driver
export LINK

caught=0
ran=0
declare -a MISSED=()

plant() { # name file old new
    local name=$1 file=$2
    rm -rf "${WORK:?}/$name"
    mkdir -p "$WORK/$name"
    cp -R compiler "$WORK/$name/compiler"
    if ! python3 - "$WORK/$name/compiler/$file" "$3" "$4" <<'EOF'; then
import sys
path, old, new = sys.argv[1:4]
text = open(path).read()
if text.count(old) != 1:
    print(f"the control's text occurs {text.count(old)} times in {path}")
    sys.exit(1)
open(path, "w").write(text.replace(old, new))
EOF
        return 1
    fi
    "$GRASM" build "$WORK/$name/compiler/s3/src/main.gr" --native -o "$WORK/$name/s3" >"$WORK/$name/build.log" 2>&1
}

control() { # name why file old new
    local name=$1 why=$2
    ran=$((ran + 1))
    if ! plant "$name" "$3" "$4" "$5"; then
        MISSED+=("$name: could not be planted: $(tail -2 "$WORK/$name/build.log" 2>/dev/null | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return
    fi
    if S3="$WORK/$name/s3" GRASM_NO_BUILD=1 bash scripts/rtdialect.sh >"$WORK/$name/out" 2>&1; then
        MISSED+=("$name: scripts/rtdialect.sh passed")
        echo "MISSED: $name"
    elif grep -qE "$why" "$WORK/$name/out"; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name/out" | head -1 | cut -c1-160)"
    else
        MISSED+=("$name: failed, but not for its reason: $(grep -h 'FAIL' "$WORK/$name/out" | head -2 | tr '\n' ' ' | cut -c1-200)")
        echo "MISSED: $name failed for another reason"
    fi
}

control switch-forgets 'FAIL: linked by (link|cc)' aarch64/src/leaves.gr \
    '    var k = 0
    while k < 6 {
        try put(l, mach.A_STP_OFF, 19 + 2 * k, 0, 20 + 2 * k, 16 * k)' \
    '    var k = 1
    while k < 6 {
        try put(l, mach.A_STP_OFF, 19 + 2 * k, 0, 20 + 2 * k, 16 * k)'
control switch-signed 'FAIL: _grt.ctx_switch signs or authenticates' aarch64/src/leaves.gr \
    '    var l = new_leaf("_grt.ctx_switch")' \
    '    var l = new_leaf("_grt.ctx_switch")
    try put(l, mach.A_PACIBSP, -1, -1, -1, 0)'
control cells-one 'cells: [0-5] of 6 cells' s3/src/main.gr \
    '        ir.cell_sym(p.pcells[id])' \
    '        ir.cell_sym(p.pcells[0])'
control static-len 'static: literal 0 is 14 bytes' mir/src/lower.gr \
    '            return konst(v, text.to_bytes().len())' \
    '            return konst(v, text.to_bytes().len() + 1)'
control tlv-shared 'FAIL: linked by (link|cc).*tls' aarch64/src/leaves.gr \
    '    try put(l, mach.A_LDR, 16, 0, -1, 0)
    try put(l, mach.A_BLR, -1, 16, -1, 0)' \
    '    try put(l, mach.A_ADDI, 16, 0, -1, 0)'

echo
echo "controls: $ran   caught: $caught   missed: $((ran - caught))"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
[ "$ran" -ge 5 ] || exit 1
exit 0
