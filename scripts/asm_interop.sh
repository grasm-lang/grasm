#!/usr/bin/env bash
# Does the x86-64 assembler emit the bytes a real assembler emits?
#
# An instruction encoding cannot be checked against itself. x86-64 is a
# variable-length format where the same mnemonic becomes different bytes
# depending on which registers it names and how large the immediate is, and the
# only way to know an encoder is right is to put the same instructions through
# something that already is.
#
# clang assembles the reference. Where the two legitimately differ — x86-64 has
# several legal encodings for one effect — the pair is listed by name below, so
# a *new* difference fails even though the known ones do not.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
WORK="${TMPDIR:-/tmp}/grasm_asm_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

# Named by absolute path rather than put on PATH. Prepending the command-line
# tools shadows the `cc` the Grasm build uses to link, and the encoder then
# fails to build for a reason that has nothing to do with encoding — which is
# how the first version of this gate reported "the encoder would not build".
CLANG=${CLANG:-/usr/bin/clang}
OBJDUMP=${OBJDUMP:-/Library/Developer/CommandLineTools/usr/bin/objdump}

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if [ ! -x "$CLANG" ] || [ ! -x "$OBJDUMP" ]; then
    echo "no assembler or disassembler here, so encodings cannot be checked"
    echo "checked: 0   agreeing: 0   skipped: 1"
    exit 0
fi
if ! "$CLANG" -arch x86_64 -c -x assembler -o "$WORK/probe.o" /dev/null 2>/dev/null; then
    echo "this clang cannot target x86_64, so encodings cannot be checked here"
    echo "checked: 0   agreeing: 0   skipped: 1"
    exit 0
fi

if ! "$GRASM" build apps/asmcheck/src/main.gr --native -o "$WORK/asmcheck" >/dev/null 2>&1; then
    echo "the encoder would not build"
    exit 1
fi
"$WORK/asmcheck" > "$WORK/ours.txt" 2>&1

# The same instructions in assembly, in the same order.
cat > "$WORK/ref.s" <<'ASM'
.text
.globl _f
_f:
    movq %rdi, %rax
    movq %r15, %r8
    movq %rbp, %rsp
    addq %rsi, %rax
    subq %rcx, %rbx
    xorq %rax, %rax
    andq %r11, %r10
    orq %r9, %rdx
    cmpq %rsi, %rdi
    imulq %rbx, %rax
    movq $42, %rax
    movq $7, %r13
    movabsq $1099511627776, %rax
    movq $-5, %rcx
    addq $8, %rax
    addq $100000, %rax
    subq $32, %rsp
    cmpq $0, %rdi
    pushq %rbp
    popq %r12
    retq
    syscall
    movq (%rdi), %rax
    movq 8(%rdi), %rax
    movq 16(%rsp), %rax
    movq (%rbp), %rax
    movq 4(%r12), %r12
    movq (%r13), %rax
    movq 5000(%rdi), %rax
    movq %rdx, 24(%rsi)
ASM
"$CLANG" -arch x86_64 -c -x assembler -o "$WORK/ref.o" "$WORK/ref.s" 2>/dev/null
"$OBJDUMP" -d "$WORK/ref.o" 2>/dev/null \
    | grep -E "^ *[0-9a-f]+:" \
    | sed -E 's/^ *[0-9a-f]+:[ \t]*//' \
    | awk -F'\t' '{ gsub(/ /,"",$1); print $1 }' > "$WORK/theirs.txt"

# Known-different pairs: a shorter encoding with the same effect. Anything else
# differing is a failure.
known_ok() {
    case "$1" in
        mov_ri_small|mov_ri_r13) return 0 ;;
        *) return 1 ;;
    esac
}

pass=0; fail=0; allowed=0
n=$(wc -l < "$WORK/theirs.txt" | tr -d ' ')
if [ "$n" -lt 25 ]; then
    echo "the reference assembled only $n instructions — the comparison would prove little"
    exit 1
fi
i=1
while read -r name ours; do
    theirs=$(sed -n "${i}p" "$WORK/theirs.txt")
    if [ "$ours" == "$theirs" ]; then
        pass=$((pass + 1))
    elif known_ok "$name"; then
        allowed=$((allowed + 1))
    else
        fail=$((fail + 1))
        printf '  DIFF %-16s ours %-24s clang %s\n' "$name" "$ours" "$theirs"
    fi
    i=$((i + 1))
done < "$WORK/ours.txt"

echo "instructions: $((pass + allowed + fail))   byte-identical: $pass   equivalent: $allowed   wrong: $fail"
[ "$fail" -eq 0 ]
