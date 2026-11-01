#!/usr/bin/env bash
# The S2 part three gates must fail on a planted fault (docs/self-hosting.md,
# S2 part three).
#
# Each control copies `compiler/front`, changes one thing in the copy, builds
# it, and runs the gate that should catch it against it (`FRONT=`), on files
# that reach the fault. The gate must fail, and for the reason the control
# names -- a gate failing for some other reason has not shown it sees this.
#
#   reworded        "unknown name" said as "unknown identifier"            diagdiff
#   span-moved      an unknown name's diagnostic one byte late             diagdiff
#   help-dropped    an unknown name's help line left out                   diagdiff
#   unordered       the checker's and the effect pass's diagnostics left
#                   unsorted, so an effect error earlier in the file
#                   comes after a type error later in it                   diagdiff
#   added           a second diagnostic made beside an unknown name's      diagdiff
#   sites-dropped   a ceiling breach's "performed: ... at ..." left out    diagdiff
#   manifest-words  the manifest reader's "expected `key = value`" reworded diagdiff
#   fuzz-nocompare  the fuzzer takes every program as agreeing without
#                   running the Grasm side                                 fuzz_diag
#   fuzz-skips      the fuzzer skips one of its engines                    fuzz_diag
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
export GRASM
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

caught=0
missed=0
ran=0
declare -a MISSED=()

# Files that reach the faults: the conformance negatives, the package cases
# diagdiff writes, and three programs written here.
git grep -l '^//! expect: error$' -- 'spec/conformance/*.gr' >"$WORK/files"
python3 scripts/lib/diagcases.py "$WORK/cases" >>"$WORK/files"
mkdir -p "$WORK/ctl"
printf 'fn main() {\n    print(nope)\n}\n' >"$WORK/ctl/unknown.gr"
printf 'fn f() / {} {\n    print("x")\n}\n\nfn main() {\n    let x: i64 = "a"\n}\n' >"$WORK/ctl/order.gr"
printf 'fn main() {\n    let a = 1\n    let b = also_nope + a\n}\n' >"$WORK/ctl/unknown2.gr"
ls "$WORK"/ctl/*.gr >>"$WORK/files"

# plant NAME FILE OLD NEW [OLD NEW...]: a copy of compiler/front (beside the
# packages/toml it depends on) with each OLD replaced by NEW (each exactly
# once), built to $WORK/NAME. FILE is in compiler/front/src.
plant() {
    local name=$1 file=$2
    shift 2
    rm -rf "${WORK:?}/$name.src"
    mkdir -p "$WORK/$name.src/compiler" "$WORK/$name.src/packages"
    cp -R compiler/front "$WORK/$name.src/compiler/front"
    cp -R packages/toml "$WORK/$name.src/packages/toml"
    file="$WORK/$name.src/compiler/front/src/$file"
    if ! python3 - "$file" "$@" <<'EOF'; then
import sys
path = sys.argv[1]
pairs = sys.argv[2:]
text = open(path).read()
for old, new in zip(pairs[::2], pairs[1::2]):
    if text.count(old) != 1:
        print(f"the control's text occurs {text.count(old)} times in {path}: {old!r}")
        sys.exit(1)
    text = text.replace(old, new)
open(path, "w").write(text)
EOF
        return 1
    fi
    "$GRASM" build "$WORK/$name.src/compiler/front/src/main.gr" --native -o "$WORK/$name" >"$WORK/$name.log" 2>&1
}

# control NAME WHY FILE OLD NEW...: plant, then diagdiff against the planted
# build must fail with output matching WHY.
control() {
    local name=$1 why=$2 file=$3
    shift 3
    ran=$((ran + 1))
    if ! plant "$name" "$file" "$@"; then
        missed=$((missed + 1))
        MISSED+=("$name: could not be planted: $(tail -3 "$WORK/$name.log" 2>/dev/null | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return
    fi
    if FRONT="$WORK/$name" FILES="$WORK/files" GRASM_NO_BUILD=1 scripts/diagdiff.sh >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: diagdiff passed")
        echo "MISSED: $name (diagdiff passed)"
    elif grep -E "$why" "$WORK/$name.out" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | sed 's/^ *//' | cut -c1-160)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: diagdiff failed, but not for its reason: $(grep -m2 -E 'FAILED|differs|rust:|front:' "$WORK/$name.out" | tr '\n' ' ' | cut -c1-240)")
        echo "MISSED: $name failed for another reason"
    fi
}

control reworded 'front: .*error: unknown identifier `nope`' sema.gr \
    '"unknown name `{name}`", "check the spelling, or declare it with `let` first")' \
    '"unknown identifier `{name}`", "check the spelling, or declare it with `let` first")'
control span-moved 'front: .*unknown\.gr:2:12: error: unknown name `nope`' sema.gr \
    'err_help(c, prog.s(p, id), prog.e(p, id), "unknown name `{name}`", "check the spelling, or declare it with `let` first")' \
    'err_help(c, prog.s(p, id) + 1, prog.e(p, id), "unknown name `{name}`", "check the spelling, or declare it with `let` first")'
control help-dropped 'rust: +help: check the spelling, or declare it with `let` first' sema.gr \
    'err_help(c, prog.s(p, id), prog.e(p, id), "unknown name `{name}`", "check the spelling, or declare it with `let` first")' \
    'err(c, prog.s(p, id), prog.e(p, id), "unknown name `{name}`")'
control unordered 'rust: .*order\.gr:1:4: error: function `f` performs effect\(s\) `io` not in its declared row' sema.gr \
    '    sort_diags(c)
    (c, t)' \
    '    (c, t)'
control added 'front: .*error: `(nope|also_nope)` is not declared' sema.gr \
    'err_help(c, prog.s(p, id), prog.e(p, id), "unknown name `{name}`", "check the spelling, or declare it with `let` first")' \
    'push_diag(c, prog.s(p, id), prog.e(p, id), "`{name}` is not declared", "")
    err_help(c, prog.s(p, id), prog.e(p, id), "unknown name `{name}`", "check the spelling, or declare it with `let` first")'
control sites-dropped 'rust: +performed: `' ceiling.gr \
    '                help = help + "\n  performed: " + where_at.join(", ")' \
    '                help = help'
control manifest-words 'front: .*grasm\.toml: line 2: expected a key' rtoml.gr \
    'return refused("{at}: expected `key = value`")' \
    'return refused("{at}: expected a key")'

# The fuzzer, told to take every program as agreeing without running the
# Grasm side, and told to skip an engine: its own counts must give it away.
fuzz_control() {
    local name=$1 why=$2
    shift 2
    ran=$((ran + 1))
    if env "$@" FRONT="${FRONT_OK:-}" GRASM_NO_BUILD=1 python3 scripts/fuzz_diag.py --programs 220 >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: scripts/fuzz_diag.py passed")
        echo "MISSED: $name"
    elif grep -E "$why" "$WORK/$name.out" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-160)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: failed, but not for its reason: $(tail -2 "$WORK/$name.out" | tr '\n' ' ')")
        echo "MISSED: $name failed for another reason"
    fi
}
fuzz_control fuzz-nocompare 'output was not read for every program counted as agreeing' FUZZ_DIAG_FAULT=nocompare
fuzz_control fuzz-skips 'engine `parser` made 0 programs.*an engine that skips is not a pass' FUZZ_DIAG_SKIP=parser

echo
echo "controls: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
fi
[ "$missed" -eq 0 ] && [ "$caught" -eq "$ran" ] && [ "$ran" -ge 9 ]
