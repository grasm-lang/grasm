#!/usr/bin/env bash
# The memory layouts the Grasm runtime shares with the Rust one must be the
# Rust runtime's, to the byte (docs/self-hosting.md §6.7, "Layouts").
#
# `compiler/rt/src/layout.gr` writes each one down as a `pub let`. Two
# witnesses are held against it, by name:
#
#   * `compiler/rt/probe/layout.gr`, a program linked with the Rust runtime
#     that makes values through its C entry points and reads their bytes --
#     the measurement, and the only witness for what Rust laid out itself
#     (a `Vec`'s fields, the enum inside a string);
#   * the Rust source, for what it states: the `layout` module's offsets and
#     tags, the count sentinels, the inline string capacity, the box size.
#
# Every constant must be measured, every measurement must be a constant, and
# a constant the source also states must agree with it -- with one
# exception, named in SOURCE_ONLY below: the state a `OnceLock` is in while
# its value is being made, which is never there to be seen from outside and
# is held to `grasm_sys`'s source alone. A range, which no program can make
# through an entry point it may declare (`grasm_range_new` takes an `i8` that
# a program's `extern fn` cannot declare as the backend does), is measured
# by handing the Rust runtime one made by hand and asking it for the fields;
# its offsets are also derived from `RtPayload`'s declaration (`repr(u8)`,
# so each variant's fields follow its tag as a `repr(C)` struct's). Every
# payload tag is held to that declaration's order as well as to the probe. The probe is also
# linked with the poisoned runtime, to measure its freed-box count: built as
# `scripts/s3rc.sh` builds it, or named by `RT_POISON=<archive built with
# grasm_rt/poison>`; with `RT_POISON=` empty that one is held to the source
# alone. `RT_LAYOUT=<file>` checks another copy of layout.gr (the controls
# plant a fault in one).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if ! "$GRASM" build compiler/rt/probe/layout.gr --native -o "$WORK/probe" >"$WORK/build.log" 2>&1; then
    echo "error: the layout probe does not build:"
    sed 's/^/  /' "$WORK/build.log"
    exit 1
fi
if ! "$WORK/probe" >"$WORK/measured.txt" 2>"$WORK/probe.err" </dev/null; then
    echo "error: the layout probe failed: $(head -2 "$WORK/probe.err")"
    exit 1
fi
: >"$WORK/poisoned.txt"
# Unset, the poisoned runtime is built as `scripts/s3rc.sh` builds it, into
# the same directory, so the suite builds it once; set and empty, it is not
# used.
if [ -z "${RT_POISON+x}" ]; then
    if ! cargo build --release -q -p grasm_rt_abi --features grasm_rt/poison --target-dir target/s3rc-poison; then
        echo "error: the poisoned runtime does not build"
        exit 1
    fi
    RT_POISON=target/s3rc-poison/release/libgrasm_rt_abi.a
fi
if [ -n "${RT_POISON:-}" ]; then
    if ! GRASM_RUNTIME="$RT_POISON" "$GRASM" build compiler/rt/probe/layout.gr --native -o "$WORK/probe_p" >"$WORK/build_p.log" 2>&1; then
        echo "error: the layout probe does not build against $RT_POISON"
        exit 1
    fi
    "$WORK/probe_p" >"$WORK/poisoned.txt" 2>/dev/null </dev/null
fi

python3 - "$WORK/measured.txt" "$WORK/poisoned.txt" "${RT_LAYOUT:-compiler/rt/src/layout.gr}" crates/grasm_rt/src/lib.rs <<'EOF'
import re
import sys

measured_path, poisoned_path, grasm_path, rust_path = sys.argv[1:5]


def pairs(path):
    out = {}
    for line in open(path):
        parts = line.split()
        if len(parts) == 2 and re.fullmatch(r"-?\d+", parts[1]):
            out[parts[0]] = int(parts[1])
    return out


measured = pairs(measured_path)
poisoned = pairs(poisoned_path)
grasm = {}
for line in open(grasm_path):
    m = re.match(r"pub let (\w+) = (-?\d+)\s*$", line)
    if m:
        grasm[m.group(1)] = int(m.group(2))

# What the Rust source states outright.
rust_text = open(rust_path).read()
I64_MAX, I64_MIN = (1 << 63) - 1, -(1 << 63)
source = {}


def stated(name, pattern, convert=int):
    m = re.search(pattern, rust_text)
    if not m:
        print(f"FAIL: the Rust source no longer states {name} ({pattern})")
        sys.exit(1)
    source[name] = convert(m)


stated("TAG", r"pub const PAYLOAD_TAG: usize = (\d+);", lambda m: int(m.group(1)))
stated("TAG_LIST", r"pub const TAG_LIST: u8 = (\d+);", lambda m: int(m.group(1)))
stated("TAG_BYTES", r"pub const TAG_BYTES: u8 = (\d+);", lambda m: int(m.group(1)))
stated("TAG_LIST_SCALAR", r"pub const TAG_LIST_SCALAR: u8 = (\d+);", lambda m: int(m.group(1)))
stated("STR_CAP", r"pub const INLINE_STR_CAP: usize = (\d+);", lambda m: int(m.group(1)))
stated("BOX_SIZE", r"size_of::<RtVal>\(\) == (\d+)\)", lambda m: int(m.group(1)))
stated("IMMORTAL", r"const IMMORTAL: i64 = i64::MAX;", lambda m: I64_MAX)
stated("ARENA", r"const ARENA: i64 = i64::MAX - 1;", lambda m: I64_MAX - 1)
stated("POISON", r"const POISON: i64 = i64::MIN \+ 0x([0-9A-Fa-f]+);", lambda m: I64_MIN + int(m.group(1), 16))
stated("LIST_DISC", r"pub const LIST_DATA_TAG: usize = (\d+);", lambda m: int(m.group(1)))
stated("LIST_LEN", r"pub const LIST_INLINE_LEN: usize = (\d+);", lambda m: int(m.group(1)))
stated("LIST_VALS", r"pub const LIST_INLINE_VALS: usize = (\d+);", lambda m: int(m.group(1)))
stated("LIST_INLINE", r"pub const INLINE: u8 = (\d+);", lambda m: int(m.group(1)))
stated("SCALAR_VEC", r"pub const SCALAR_VEC: usize = (\d+);", lambda m: int(m.group(1)))
stated("TAG_STRUCT", r"pub const TAG_STRUCT: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_F64", r"pub const FK_F64: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_BOOL", r"pub const FK_BOOL: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_REF", r"pub const FK_REF: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_UNIT", r"pub const FK_UNIT: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_U64", r"pub const FK_U64: u8 = (\d+);", lambda m: int(m.group(1)))
stated("SCALAR_DISC", r"pub const SCALAR_DATA_TAG: usize = (\d+);", lambda m: int(m.group(1)))
stated("BYTES_PTR", r"pub const BYTES_PTR: usize = (\d+);", lambda m: int(m.group(1)))
stated("BYTES_LEN", r"pub const BYTES_LEN: usize = (\d+);", lambda m: int(m.group(1)))
stated("TAG_LIST_VALS", r"pub const TAG_LIST_VALS: u8 = (\d+);", lambda m: int(m.group(1)))
stated("STRUCT_DISC", r"pub const STRUCT_FIELDS_TAG: usize = (\d+);", lambda m: int(m.group(1)))
stated("STRUCT_LEN", r"pub const STRUCT_INLINE_LEN: usize = (\d+);", lambda m: int(m.group(1)))
stated("STRUCT_VALS", r"pub const STRUCT_INLINE_VALS: usize = (\d+);", lambda m: int(m.group(1)))
stated("STRUCT_INLINE", r"pub const INLINE: u8 = (\d+);", lambda m: int(m.group(1)))
stated("VALS_VEC", r"pub const VALS_VEC: usize = (\d+);", lambda m: int(m.group(1)))
stated("FK_BOXED", r"pub const FK_BOXED: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_INLIST", r"pub const FK_INLIST: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_INVALS", r"pub const FK_INVALS: u8 = (\d+);", lambda m: int(m.group(1)))
stated("FK_INSTR", r"pub const FK_INSTR: u8 = (\d+);", lambda m: int(m.group(1)))

# `RtPayload`'s variants in declaration order: each one's tag. And the fields
# of the ones whose layout is held to the source alone.
body = rust_text.split("pub enum RtPayload {", 1)[1].split("\n}\n", 1)[0]
variants = re.findall(r"^    ([A-Z]\w*)\s*(?:\(|\{|,)", body, re.M)
camel = {"ListScalar": "LIST_SCALAR", "ListVals": "LIST_VALS", "StrBuf": "STRBUF", "ByteBuf": "BYTEBUF"}
for i, v in enumerate(variants):
    source["TAG_" + camel.get(v, v.upper())] = i
m = re.search(r"^    Range \{\s*start: i64,\s*end: i64,\s*inclusive: bool,\s*\},", body, re.M)
if not m:
    print("FAIL: the Rust source's `Range` is no longer `{ start: i64, end: i64, inclusive: bool }`")
    sys.exit(1)
# After the tag byte at 8, each field at its own alignment.
source["RANGE_START"] = 16
source["RANGE_END"] = 24
source["RANGE_INCL"] = 32
# The runtime's `OnceLock` is `grasm_sys`'s: its state while a value is being
# made is never there to be seen from outside.
sys_text = open(rust_path.replace("grasm_rt/src/lib.rs", "grasm_sys/src/lib.rs")).read()
m = re.search(r"const SETTING: u8 = (\d+);", sys_text)
if not m:
    print("FAIL: grasm_sys no longer states the OnceLock's SETTING")
    sys.exit(1)
source["ONCE_SETTING"] = int(m.group(1))
SOURCE_ONLY = {"ONCE_SETTING"}

bad = 0
compared = 0
source_only = 0
for name, value in sorted(grasm.items()):
    if name in SOURCE_ONLY:
        if name in measured:
            print(f"FAIL: {name} is measured, so it is not one the source alone holds")
            bad = 1
        elif source.get(name) != value:
            print(f"FAIL: {name} is {value} in layout.gr, {source.get(name)} in the Rust source")
            bad = 1
        else:
            source_only += 1
        continue
    if name == "POISON":
        # Only a probe linked with the poisoned runtime sees it.
        if "POISON" in poisoned:
            got = poisoned["POISON"]
            if got != value:
                print(f"FAIL: POISON is {value} in layout.gr, {got} in the poisoned runtime")
                bad = 1
            compared += 1
    elif name not in measured:
        print(f"FAIL: layout.gr's {name} is not measured by the probe")
        bad = 1
        continue
    elif measured[name] != value:
        print(f"FAIL: {name} is {value} in layout.gr, {measured[name]} in the Rust runtime")
        bad = 1
    else:
        compared += 1
    if name in source and source[name] != value:
        print(f"FAIL: {name} is {value} in layout.gr, {source[name]} in the Rust source")
        bad = 1
for name in sorted(measured):
    if name not in grasm:
        print(f"FAIL: the probe measures {name}, which layout.gr does not write down")
        bad = 1
witnessed = sum(1 for n in grasm if n in source and n not in SOURCE_ONLY)
poison_note = "measured in the poisoned runtime" if "POISON" in poisoned else "held to the source only"
print(f"layout constants: {len(grasm)}   measured and equal: {compared}   also stated by the Rust source: {witnessed}   held to the source alone: {source_only}   POISON {poison_note}")
expected = len(grasm) - len(SOURCE_ONLY) if "POISON" in poisoned else len(grasm) - len(SOURCE_ONLY) - 1
if bad or compared != expected or source_only != len(SOURCE_ONLY) or len(grasm) < 30:
    sys.exit(1)
EOF
