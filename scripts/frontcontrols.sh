#!/usr/bin/env bash
# The S2 gates must fail on a planted fault (docs/self-hosting.md, S2).
#
# Each control copies `compiler/front`, changes one thing in the copy, builds
# it, and runs the gate that should catch it against it (`FRONT=`), on files
# that reach the fault. The gate must fail, and for the reason the control
# names -- a gate failing for some other reason has not shown it sees this.
#
#   span-start      an identifier token starts one byte late     tokentest
#   span-end        an identifier token ends one byte late       asttest
#   keyword-ident   `while` lexed as an identifier               tokentest
#   precedence      `+` binds tighter than `*`                   asttest
#   error-dropped   an error node left out of the CST            asttest
#   diag-span       "expected X but found Y" one byte off        parsediag
#   fuzz-skips      the fuzzer skips one program in ten          fuzz_front
#
# `GRASM=` must have `grasm ast`.
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

# Files that reach the faults: the conformance programs, the frontend's
# own source, and 150 fuzzer programs, most of which the parser rejects.
git ls-files 'spec/conformance/*.gr' >"$WORK/files"
git ls-files 'compiler/front/src/*.gr' >>"$WORK/files"
python3 scripts/fuzz_front.py --emit "$WORK/fuzz" --programs 150 --seed 11 >/dev/null
find "$WORK/fuzz" -name '*.gr' | sort >>"$WORK/files"

# plant NAME FILE OLD NEW [OLD NEW...]: a copy of compiler/front with each
# OLD replaced by NEW (each exactly once), built to $WORK/NAME.
plant() {
    local name=$1 file=$2
    shift 2
    rm -rf "${WORK:?}/$name.src"
    # The frontend reads manifests with packages/toml, a dependency named by
    # a path relative to it, so the copy keeps the two where they are.
    mkdir -p "$WORK/$name.src/compiler" "$WORK/$name.src/packages"
    cp -R compiler/front "$WORK/$name.src/compiler/front"
    cp -R packages/toml "$WORK/$name.src/packages/toml"
    if ! python3 - "$WORK/$name.src/compiler/front/src/$file" "$@" <<'EOF'; then
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

# expect NAME WHY GATE...: the gate, run against the planted build, fails
# with output matching WHY.
expect() {
    local name=$1 why=$2
    shift 2
    ran=$((ran + 1))
    if FRONT="$WORK/$name" FILES="$WORK/files" GRASM_NO_BUILD=1 "$@" >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: $* passed")
        echo "MISSED: $name ($*)"
    elif grep -qE "$why" "$WORK/$name.out"; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-150)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: $* failed, but not for its reason: $(tail -2 "$WORK/$name.out" | tr '\n' ' ' | cut -c1-200)")
        echo "MISSED: $name failed for another reason"
    fi
}

control() {
    local name=$1 why=$2 gate=$3 file=$4
    shift 4
    if ! plant "$name" "$file" "$@"; then
        ran=$((ran + 1))
        missed=$((missed + 1))
        MISSED+=("$name: could not be planted: $(tail -3 "$WORK/$name.log" 2>/dev/null | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return
    fi
    expect "$name" "$why" "$gate"
}

control span-start 'MISMATCH' scripts/tokentest.sh lex.gr \
    'push_tok(tb, T_IDENT, start, pos, out.text.len() - 1, word)' \
    'push_tok(tb, T_IDENT, start + 1, pos, out.text.len() - 1, word)'
control span-end 'MISMATCH' scripts/asttest.sh lex.gr \
    'push_tok(tb, T_IDENT, start, pos, out.text.len() - 1, word)' \
    'push_tok(tb, T_IDENT, start, pos + 1, out.text.len() - 1, word)'
control keyword-ident 'MISMATCH' scripts/tokentest.sh lex.gr \
    '        m.set(kind_text(k), k)' \
    '        if k != KW_WHILE {
            m.set(kind_text(k), k)
        }'
control precedence 'MISMATCH' scripts/asttest.sh parse.gr \
    '    } else if k == lex.T_PLUS or k == lex.T_MINUS {
        6
    } else if k == lex.T_STAR or k == lex.T_SLASH or k == lex.T_PERCENT {
        7' \
    '    } else if k == lex.T_PLUS or k == lex.T_MINUS {
        7
    } else if k == lex.T_STAR or k == lex.T_SLASH or k == lex.T_PERCENT {
        6'
control error-dropped 'NOT LOSSLESS' scripts/asttest.sh cst.gr \
    '                pending.push(id)
                open.push(id)' \
    '                if ev[(evi) * 9 + 1] != syn.N_ERROR {
                    pending.push(id)
                }
                open.push(id)'
# The runtime dialect's declaration ([T-RUNTIME]) read as a thread-local
# cell: spec/conformance/T-RUNTIME.gr declares one, so the AST differs.
control process-as-tls 'MISMATCH' scripts/asttest.sh ast.gr \
    '        let d = if k == syn.N_PROCESS_LOCAL { D_PLS } else { D_TLS }' \
    '        let d = D_TLS'
control diag-span 'differ: [1-9]' scripts/parsediag.sh parse.gr \
    '    let s = cur_s(p, ev)
    let e = p.lx.end[p.i]
    let found = describe(p, p.i)
    let msg = "expected {kind_describe(k)}' \
    '    let s = cur_s(p, ev) + 1
    let e = p.lx.end[p.i]
    let found = describe(p, p.i)
    let msg = "expected {kind_describe(k)}'

# The fuzzer, told to skip: its own counts must give it away.
ran=$((ran + 1))
if FUZZ_FRONT_FAULT=skip GRASM_NO_BUILD=1 python3 scripts/fuzz_front.py --programs 60 >"$WORK/fuzz.out" 2>&1; then
    missed=$((missed + 1))
    MISSED+=("fuzz-skips: scripts/fuzz_front.py passed while skipping")
    echo "MISSED: fuzz-skips"
elif grep -q 'the counts do not add up' "$WORK/fuzz.out"; then
    caught=$((caught + 1))
    echo "caught: fuzz-skips -- $(grep -h 'the counts do not add up' "$WORK/fuzz.out" | head -1)"
else
    missed=$((missed + 1))
    MISSED+=("fuzz-skips: failed, but not for its reason: $(tail -2 "$WORK/fuzz.out" | tr '\n' ' ')")
    echo "MISSED: fuzz-skips failed for another reason"
fi

echo
echo "controls: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
fi
[ "$missed" -eq 0 ] && [ "$caught" -eq "$ran" ] && [ "$ran" -ge 8 ]
