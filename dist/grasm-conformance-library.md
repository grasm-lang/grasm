# Grasm — conformance programs: library

The standard library. 65 executable program(s), one per normative clause of
`grasm-language-pack.md`. Each header states the tag it demonstrates and the
exact expected behaviour, and every one is run by `cargo test -p grasm_spec`, so
a program here cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

The programs are split by area because together they no longer fit in one
context, and a reader with a rule in dispute needs one area rather than four.
The other parts are `grasm-conformance-grammar`, `grasm-conformance-types`, `grasm-conformance-semantics`, `grasm-conformance-library`.

## `L-BOOL.gr`

```grasm
//! tag: L-BOOL
//! expect: output
//! true false
fn main() {
    print("{true} {false}")
}
```

## `L-BUNDLE-SOURCE.gr`

```grasm
//! tag: L-BUNDLE-SOURCE
//! expect: output
//! a program is its modules
//! and a bundle is a program
// A **source bundle** is a whole program as one file: every module it imports,
// transitively, behind a `//!module <name>` line, entry first.
//
// It exists because an embedder may have no filesystem. The wasm engine takes
// one source string, so before this a program with a single `import` could not
// run in a browser however small it was — and every program worth writing has
// imports, so the fourth engine could run almost nothing.
//
// Nothing in a bundle is renamed or rewritten. The modules it loads are the
// modules the disk would have loaded, which is why this file can be a bundle
// *of itself* and mean the same thing either way.
//
// Written by `grasm build <file> --source -o <out>.gr`, and opened by `run`,
// `check`, `build`, `test` and the language server without any of them being
// told the format exists: the detection is in the loader they all go through.
//
// This program has no import, so it is a bundle of one module — the case that
// has to keep working, because it is what every single-file program becomes.
fn main() {
    print("a program is its modules")
    print("and a bundle is a program")
}
```

## `L-COMMENT-BLOCK.gr`

```grasm
//! tag: L-COMMENT-BLOCK
//! expect: output
//! 1
fn main() {
    /* outer /* nested */ still a comment */
    print("1")
}
```

## `L-COMMENT-LINE.gr`

```grasm
//! tag: L-COMMENT-LINE
//! expect: output
//! 1
fn main() {
    // a whole-line comment
    print("1") // and a trailing one
}
```

## `L-CONTEXTUAL.gr`

```grasm
//! tag: L-CONTEXTUAL
//! expect: output
//! 1
//! 2
//! has 2
//! two
//! 2
// A contextual keyword is an ordinary name everywhere else.
//
// `region` is an *expression* form, and the rule that a following `{` opens a
// body — the one that makes `if User{..}.ok` need brackets — was not applied
// to it. So `for x in region { … }` read the loop's own brace as a region
// body, and a variable called `region` could not be used in a condition, a
// loop header or a `match` scrutinee at all.
fn main() {
    let region = [1, 2]
    for x in region {
        print("{x}")
    }
    if region.len() > 0 {
        print("has {region.len()}")
    }
    match region.len() {
        2 -> print("two")
        _ -> print("other")
    }
    // And the expression form still works where a brace really does open one.
    print("{region { 1 + 1 }}")
}
```

## `L-DISPLAY.gr`

```grasm
//! tag: L-DISPLAY
//! expect: output
//! -7
//! 1.5
//! 1000000.0
//! 1000000000000000.0
//! 1e300 has a point too
//! ()
//! true
//! b c
//! [1, 2, 3]
//! [a, b c]
//! [[1, 2], [3]]
//! (1, one, true)
//! {k: 1, z: 2}
//! P{x: 3, y: hi}
//! A
//! B(7)
//! Some(1)
//! None
//! [(), ()]
//! (1, ())
//! Some(())
//! ref#0.0
//! [ref#0.0]
//! Some(ref#0.0)
//! <fn main::dbl>
//! <closure>
// Every type's rendering, because a program's output is what its correctness
// is judged on. Strings are unquoted even inside a collection, so `[a, b c]`
// is two elements and the second contains a space.
type P = { x: i64, y: str }

type S =
    | A
    | B(n: i64)

type N = { v: i64 }

fn main() {
    print("{-7}")
    print("{1.5}")
    print("{1000000.0}")
    // Past 1e15, where the old rule quietly stopped applying: a float has to
    // stay distinguishable from an integer at every magnitude, or reading a
    // program's output means guessing which type produced it.
    print("{1000000000000000.0}")
    let big = 1.0e300
    let shown = "{big}"
    print("1e300 has a point too")
    assert(shown.contains("."))
    // Unit, which [T-PRIM] lists and the table did not.
    print("{()}")
    print("{true}")
    print("{"b c"}")
    print("{[1, 2, 3]}")
    print("{["a", "b c"]}")
    print("{[[1, 2], [3]]}")
    print("{(1, "one", true)}")
    var m = map()
    m.set("z", 2)
    m.set("k", 1)
    print("{m}")
    print("{P{ x: 3, y: "hi" }}")
    print("{A}")
    print("{B(7)}")
    print("{[1].first()}")
    print("{[1].get(9)}")
    // A value keeps its rendering inside a container. Both of these were
    // boxed as plain integers by the compiled backend, so a list of units
    // printed `[0, 0]` and a list of references printed the packed slot and
    // generation as a number — a wrong answer rather than a refusal, and for
    // the reference also a disclosure of where the slab put it.
    print("{[(), ()]}")
    print("{(1, ())}")
    print("{Some(())}")
    let r = ref(N{ v: 1 })
    print("{r}")
    print("{[r]}")
    print("{Some(r)}")
    release(r)
    // A function value shows the name it was declared with; a closure has
    // none to show. Both render, and [L-DISPLAY] is what output is judged on.
    print("{dbl}")
    let c: fn(i64) -> i64 = |x| x + 1
    print("{c}")
}

fn dbl(n: i64) -> i64 {
    n * 2
}
```

## `L-ENCODING.gr`

```grasm
﻿//! tag: L-ENCODING
//! expect: output
//! héllo ✓ 日本語
//! no bom in sight
fn main() {
    // Source is UTF-8; strings and comments may hold any Unicode: ✓ 日本語
    print("héllo ✓ 日本語")
    // This file begins with a byte order mark. It is ignored — legal UTF-8
    // that editors write unasked, and lexed as a character it made the first
    // declaration "unexpected character" for something that renders as
    // nothing.
    print("no bom in sight")
}
```

## `L-ESCAPE.gr`

```grasm
//! tag: L-ESCAPE
//! expect: output
//! a	b
//! quote" brace{ close}
//! [1]
//! 😀 1 [128512]
//! é
//! [a, b]
//! A3B
//! a}b
fn main() {
    print("a\tb")
    print("quote\" brace\{ close\}")
    // A character with no other spelling. A group separator used to be
    // written `"{1.to_char()}"` — an interpolation whose only purpose was to
    // reach a character the source could not say.
    print("\u{1}".codes())
    let smile = "\u{1F600}"
    print("{smile} {smile.len()} {smile.codes()}")
    print("\u{e9}")
    print("a\u{1}b".split("\u{1}"))
    // An escape covers the byte after it, inside an interpolation too: the
    // `\}` of a nested string is not the brace that closes the
    // interpolation. Counted as one, it ended the interpolation early and the
    // diagnostic that followed carried a raw NUL byte.
    print("A{"x\}y".len()}B")
    print("a\}b")
}
```

## `L-FLOAT.gr`

```grasm
//! tag: L-FLOAT
//! expect: output
//! 3.14 1000000000.0 0.0025
fn main() {
    print("{3.14} {1e9} {2.5e-3}")
}
```

## `L-IDENT.gr`

```grasm
//! tag: L-IDENT
//! expect: output
//! 6
fn main() {
    let snake_case = 1
    let _leading = 2
    let with2digits = 3
    print("{snake_case + _leading + with2digits}")
}
```

## `L-INT.gr`

```grasm
//! tag: L-INT
//! expect: output
//! 255 10 1000000
//! -1 -9223372036854775808 9223372036854775807
//! -7046029254386353131 -1
fn main() {
    print("{0xFF} {0b1010} {1_000_000}")
    // Hex and binary name a bit pattern, so the whole 64-bit range is
    // writable and the top bit is not special. Every hash constant, mask and
    // generator multiplier is published this way, and none of them fits in
    // the non-negative half.
    print("{0xffffffffffffffff} {0x8000000000000000} {0x7fffffffffffffff}")
    print("{0x9e3779b97f4a7c15} {0b1111111111111111111111111111111111111111111111111111111111111111}")
}
```

## `L-INTERP-ONE.gr`

```grasm
//! tag: L-INTERP-ONE
//! expect: error
//! an interpolation holds one expression
// `{...}` holds exactly one expression, and anything left over is an error.
//
// It used to be a silent truncation: `"A{1 2}B"` printed `A1B`. The parser
// read one expression and discarded the rest of the braces' contents without
// a word, so a typo inside `{...}` quietly deleted part of the output. Both
// engines agreed on the wrong answer, which is why no differential test could
// ever have found it.
fn main() {
    print("A{1 2}B")
}
```

## `L-INTERP.gr`

```grasm
//! tag: L-INTERP
//! expect: output
//! n=3 sum=7 nested=YES
fn main() {
    let n = 3
    print("n={n} sum={3 + 4} nested={if n > 1 { "YES" } else { "no" }}")
}
```

## `L-KEYWORD.gr`

```grasm
//! tag: L-KEYWORD
//! expect: error
//! expected an identifier
fn main() {
    // `type` is reserved and cannot name a variable.
    let type = 1
}
```

## `L-LEADING-DOT.gr`

```grasm
//! tag: L-LEADING-DOT
//! expect: output
//! hello; world
//! 5
//! 3 3
//! 5 chained across five lines
//! 10
// A newline before a leading `.` does not end the statement, so a chain reads
// down the left margin instead of along one unreadable line.
//
// Safe for exactly one reason: no statement may begin with a `.`, so a newline
// in front of one was never separating two statements.
fn main() / {io} {
    let cleaned = "  Hello, World  "
        .trim()
        .to_lower()
        .replace(",", ";")
    print(cleaned)

    // Bytes, which is where this was first missed: a packet is built out of
    // one `concat` per field.
    let packet = bytes_of([1, 2])
        .concat(bytes_of([3]))
        .concat(bytes_of([4, 5]))
    print("{packet.len()}")

    // Blank lines and comments between the links are fine — consecutive
    // newlines collapse into one, and one is absorbed.
    let parts = "x,y,z"
        .split(",")

        // still the same statement
        .len()
    print("{parts} {"a b c".words().len()}")

    // A chain that spans more lines than it has calls.
    let long = "  MIXED  "
        .trim()
        .to_lower()
        .repeat(1)
        .len()
    print("{long} chained across five lines")

    // The rule is about the newline, not about chaining: a chain on one line
    // means the same thing.
    print("{"aaaaa".repeat(2).len()}")
}
```

## `L-LIB-AS-I64.gr`

```grasm
//! tag: L-LIB-AS-I64
//! expect: output
//! 3 -3
//! 9223372036854775807 -9223372036854775808 0
// `as_i64` saturates. It is total, so it has an answer for every f64 and never
// panics — but "truncates toward zero" was the whole of what was written down,
// which says nothing about the values that have no i64 at all.
fn main() {
    print("{3.7.as_i64()} {(-3.7).as_i64()}")
    let huge = 1.0e30
    let nan = 0.0 / 0.0
    print("{huge.as_i64()} {(-huge).as_i64()} {nan.as_i64()}")
}
```

## `L-LIB-ATOMIC.gr`

```grasm
//! tag: L-LIB-ATOMIC
//! expect: output
//! 10 15 15
//! 15 99 false 99
//! 255 0 -128 127
//! 18446744073709551615 0 1
//! 7 7 7 7 7
// Each atomic answers what was in memory before it acted: a fetch-add the old
// value, a compare-exchange the value it found, whether or not it stored. Each
// works at its type's width and wraps there, and touches no byte beside it.
fn main() / {io, unsafe, mem} {
    unsafe {
        let a = mem_alloc(16, 8)
        a.atomic_store_i64(10, Release)
        let old = a.atomic_add_i64(5, AcqRel)
        print("{old} {a.atomic_load_i64(Acquire)} {a.load_i64()}")
        // A compare-exchange stores only when it finds what it expects.
        let hit = a.atomic_cas_i64(15, 99, SeqCst)
        let miss = a.atomic_cas_i64(15, 7, SeqCst)
        print("{hit} {a.load_i64()} {miss == 15} {miss}")
        // A u8 wraps at eight bits, and the byte after it does not move.
        a.offset(8).store_u8(255)
        a.offset(9).store_i8(-128)
        let b = a.offset(8).atomic_add_u8(1, Relaxed)
        print("{b} {a.offset(8).atomic_load_u8(Relaxed)} {a.offset(9).atomic_add_i8(-1, SeqCst)} {a.offset(9).load_i8()}")
        let top: u64 = 18446744073709551615
        a.atomic_store_u64(top, SeqCst)
        print("{a.atomic_add_u64(1, Relaxed)} {a.atomic_load_u64(SeqCst)} {a.atomic_add_u64(1, Relaxed) + 1}")
        // Every width and ordering a load may have.
        a.atomic_store_i32(7, Relaxed)
        a.offset(4).atomic_store_u16(7, SeqCst)
        a.offset(6).atomic_store_i16(7, Release)
        print("{a.atomic_load_i32(Relaxed)} {a.atomic_load_u32(SeqCst)} {a.offset(4).atomic_load_u16(Acquire)} {a.offset(6).atomic_load_i16(Relaxed)} {a.offset(4).atomic_cas_u16(7, 7, AcqRel)}")
        mem_free(a)
    }
}
```

## `L-LIB-BASE.gr`

```grasm
//! tag: L-LIB-BASE
//! expect: output
//! ff 11111111 377 73
//! -ff 0 1 -8000000000000000
//! 7fffffffffffffff 1000000000000000000000000000000000000000000000000000000000000000
//! 255 10 1295 -255
//! not a digit in this base: true
//! a prefix is not accepted: true
//! to_base(10) agrees with to_str: true
//! round trip 2..36 over -600..600: true
// Base ten is not the only base a systems program needs. Every protocol, hash,
// colour and permission mask is read and written in hex or binary.
fn main() / {io} {
    print("{255.to_base(16)} {255.to_base(2)} {255.to_base(8)} {255.to_base(36)}")

    // `i64::MIN` goes through its unsigned magnitude, because negating it
    // overflows — the classic place a hand-written conversion is wrong.
    let min = 0 - 9223372036854775807 - 1
    print("{(0 - 255).to_base(16)} {0.to_base(2)} {1.to_base(10)} {min.to_base(16)}")
    print("{9223372036854775807.to_base(16)} {min.to_base(2).substr(1, 64)}")

    print("{"ff".parse_int_base(16) ?? 0 - 1} {"1010".parse_int_base(2) ?? 0 - 1} {"zz".parse_int_base(36) ?? 0 - 1} {"-ff".parse_int_base(16) ?? 0 - 1}")

    // A digit outside the base is not read as something else.
    print("not a digit in this base: {"19".parse_int_base(8) == None}")

    // The base is the argument, not something guessed from the text.
    print("a prefix is not accepted: {"0xff".parse_int_base(16) == None}")

    var same = true
    for n in 0 - 600..600 {
        if n.to_base(10) != n.to_str() {
            same = false
        }
    }
    print("to_base(10) agrees with to_str: {same}")

    var ok = true
    for n in 0 - 600..600 {
        for b in 2..37 {
            if (n.to_base(b).parse_int_base(b) ?? n + 1) != n {
                ok = false
            }
        }
    }
    print("round trip 2..36 over -600..600: {ok}")
}
```

## `L-LIB-BITS-WIDTH.gr`

```grasm
//! tag: L-LIB-BITS-WIDTH
//! expect: output
//! 15 120 4 0 4 240
//! -128 -1 8 0 8
//! 13330 256 2014458966 2014458966
//! 32 32 0 -9223372036854775808 63 64
//! 578437695752307201 1 3752061439553044480 7
// Rotations, counts and byte swaps read the bits at the receiver's own width:
// an i8 has eight of them, a u64 sixty-four.
fn main() {
    let b: u8 = 0xF0
    print("{b.rotate_left(4)} {b.rotate_right(1)} {b.count_ones()} {b.leading_zeros()} {b.trailing_zeros()} {b.swap_bytes()}")
    let one: i8 = 1
    let all: i8 = -1
    print("{one.rotate_right(1)} {all.rotate_left(3)} {all.count_ones()} {all.leading_zeros()} {(0).as_i8().trailing_zeros()}")
    let h: u16 = 0x1234
    let s: i16 = 1
    let w: u32 = 0x12345678
    print("{h.swap_bytes()} {s.swap_bytes()} {w.rotate_right(8)} {w.rotate_left(-8)}")
    let z: i32 = 0
    print("{z.leading_zeros()} {z.trailing_zeros()} {z.count_ones()} {(1).rotate_right(1)} {(1).leading_zeros()} {(0).trailing_zeros()}")
    let q: u64 = 0x0102030405060708
    let u: u64 = 1
    print("{q.swap_bytes()} {u.rotate_left(64)} {(0x1234).swap_bytes()} {(0xFF00).as_u16().count_ones() - 1}")
}
```

## `L-LIB-BITS.gr`

```grasm
//! tag: L-LIB-BITS
//! expect: output
//! and 48 or 252 xor 204 not -241
//! shl 3840 shr 15
//! -4 15
//! 1 -9223372036854775808 0
//! rotr32 of 0x12345678 by 8 = 0x78123456
//! masked to 32 bits: 4294967295
// Bit manipulation on `i64`, as methods ([L-LIB-BITS]).
fn main() {
    let a = 0xF0
    let b = 0x3C
    print("and {a.bit_and(b)} or {a.bit_or(b)} xor {a.bit_xor(b)} not {a.bit_not()}")
    print("shl {a.shl(4)} shr {a.shr(4)}")

    // `shr` keeps the sign, `ushr` fills with zero. That is the whole reason
    // both exist.
    print("{(0 - 16).shr(2)} {(0 - 16).ushr(60)}")

    // The shift count is masked to 0..63, so none of these traps.
    print("{1.shl(64)} {1.shl(63)} {256.shr(0 - 1)}")

    // A 32-bit rotate, which is what SHA-256 is built from. There is no
    // unsigned 32-bit type, so the mask is written out.
    let mask32 = 0xFFFFFFFF
    let v = 0x12345678
    let n = 8
    let rotated = (v.ushr(n)).bit_or(v.shl(32 - n)).bit_and(mask32)
    print("rotr32 of 0x12345678 by 8 = 0x{hex(rotated)}")
    print("masked to 32 bits: {mask32.bit_and(0 - 1)}")
}

fn hex(v: i64) -> str {
    let digits = "0123456789abcdef"
    var out = ""
    var i = 7
    while i >= 0 {
        let nibble = (v.ushr(i * 4)).bit_and(15)
        out = out + digits.substr(nibble, 1)
        i = i - 1
    }
    out
}
```

## `L-LIB-BUILDER.gr`

```grasm
//! tag: L-LIB-BUILDER
//! expect: output
//! héllo, world 12 false
//! héllo 5
//! 0x41ff8000c3a9 6 255 0x41ff8000c3a9
//! 0x41ff00 0x41ff8000c3a9
//! 30000 true
// Builders append at the end in amortized constant time, and stay values: a
// copy is a copy, before and after an append.
type Out = { text: StrBuilder, n: i64 }

fn emit(inout o: Out, s: str) {
    o.text.push_str(s)
    o.n += 1
}

fn main() {
    var s = str_builder()
    s.push_str("héllo")
    let kept = s
    s.push_str(", world")
    print("{s} {s.len()} {s.is_empty()}")
    print("{kept} {kept.len()}")

    var b = byte_builder()
    b.push_byte(0x41)
    b.push_byte(255)
    b.push_bytes(bytes_of([128, 0]))
    let before = b
    b.push_str("é")
    print("{b} {b.len()} {b.at(1)} {b.to_bytes()}")
    var c = before
    c.set(2, 0)
    print("{c.to_bytes().slice(0, 3)} {b}")

    var o = Out{ text: str_builder(), n: 0 }
    var i = 0
    while i < 10000 {
        emit(o, "abc")
        i += 1
    }
    print("{o.text.len()} {o.text.to_str() == "abc".repeat(10000)}")
}
```

## `L-LIB-BYTES-WRITE.gr`

```grasm
//! tag: L-LIB-BYTES-WRITE
//! expect: panic
//! filled: [9, 9, 9, 9]
//! set: [9, 1, 9, 2]
//! fill: [7, 7, 7, 7]
//! copy_from: [0, 0, 3, 4, 5, 0]
//! a write does not reach a value taken before it: [7, 7, 7, 7] [9, 1, 9, 2]
//! nor a window on the same buffer: [1, 1] [9, 1]
//! onto itself: [3, 4, 5]
//! a nested place: [0, 8, 0]
//! length is unchanged by every write: true
//! index 4 out of bounds (len 4)
// Writing into a `bytes` ([L-LIB-BYTES-WRITE]). The length never changes, and
// a write copies the buffer first when anything else can see it ([E-COW]).
type Tile = { px: bytes }

fn main() {
    var b = bytes_filled(4, 9)
    print("filled: {b.to_list()}")

    b.set(1, 1)
    b.set(3, 2)
    print("set: {b.to_list()}")

    // Taken before the `fill`, and therefore unaffected by it.
    let taken = b

    b.fill(7)
    print("fill: {b.to_list()}")

    var dst = bytes_filled(6, 0)
    dst.copy_from(2, bytes_of([3, 4, 5]))
    print("copy_from: {dst.to_list()}")

    print("a write does not reach a value taken before it: {b.to_list()} {taken.to_list()}")

    // A window shares the allocation until one side is written.
    var w = bytes_filled(2, 1)
    let win = w.slice(0, 2)
    w.set(0, 9)
    print("nor a window on the same buffer: {win.to_list()} {w.to_list()}")

    // The source and the destination being one buffer is a program somebody
    // writes, and at offset zero it is already done.
    var same = bytes_of([3, 4, 5])
    same.copy_from(0, same)
    print("onto itself: {same.to_list()}")

    var t = Tile{ px: bytes_filled(3, 0) }
    t.px.set(1, 8)
    print("a nested place: {t.px.to_list()}")

    let before = b.len()
    b.set(0, 1)
    b.fill(2)
    b.copy_from(0, bytes_of([3, 4, 5, 6]))
    print("length is unchanged by every write: {before == b.len()}")

    // Out of range panics, as `at` does.
    b.set(4, 0)
}
```

## `L-LIB-BYTES.gr`

```grasm
//! tag: L-LIB-BYTES
//! expect: panic
//! 0x48c7c001000000
//! 7 bytes, first 72, third 192
//! 0x48c7c0 and 0x48c7ff
//! [72, 199, 192, 1, 0, 0, 0]
//! text is 5 characters and 6 bytes
//! round trip: héllo
//! written and read back identically: true
//! empty is empty, seven bytes are not: true false
//! last byte and last slice: 0 0x00
//! slice means the same on bytes as on a list: true true true true
//! and refuses the end a list would clamp: true
//! index_of: Some(1) Some(3) None None None
//! a temporary's length: 337 0.337
//! scanned: 13 0 3 1
//! index 5 out of bounds (len 5)
// `bytes` is the type `str` deliberately is not ([L-LIB-BYTES]).
//
// The sequence below is x86-64 `mov rax, 1`. Written through a `str` it lands
// on disk as nine bytes, because 0xC7 is re-encoded as UTF-8 — which is the
// reason this type exists.
fn main() / {io} {
    let code = bytes_of([0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00])
    print("{code}")
    print("{code.len()} bytes, first {code.at(0)}, third {code.at(2)}")
    print("{code.slice(0, 3)} and {code.slice(0, 2).concat(bytes_of([0xFF]))}")
    print("{code.to_list()}")

    // A byte is not a character.
    let text = "héllo"
    print("text is {text.len()} characters and {text.to_bytes().len()} bytes")
    print("round trip: {text.to_bytes().to_str()}")

    // Under `target/`, which git ignores: there is no way to remove a
    // directory, so whatever this makes stays, and the gates run every
    // conformance program from the repository root.
    let dir = "target/grasm_bytes_conformance"
    let _mk = make_dir(dir) ?? ()
    let path = dir + "/code.bin"
    match write_bytes(path, code) {
        Err(e) -> print("could not write: {e}")
        Ok(_) -> match read_bytes(path) {
            Err(e) -> print("could not read: {e}")
            Ok(back) -> print("written and read back identically: {back.to_list() == code.to_list()}")
        }
    }
    let _rm = remove_file(path) ?? ()

    // `is_empty` was the one method here the compiled backend could not build:
    // it widened `icmp`'s answer to the width it already had, which Cranelift
    // rejects, so every `b.is_empty()` in the language was a "codegen bug".
    print("empty is empty, seven bytes are not: {bytes_of([]).is_empty()} {code.is_empty()}")

    // `at` and `slice` are specified to panic out of range, and the index
    // worth testing is the first one that is. The last byte and the last
    // one-byte slice must work; one past must not.
    print("last byte and last slice: {code.at(code.len() - 1)} {code.slice(code.len() - 1, code.len())}")

    // The two `slice` methods take the same pair of indices and mean the same
    // thing by them. They did not until F97 — `bytes` took a length where a
    // list takes an end — and the two are identical enough to write side by
    // side that nothing pointed it out.
    print("slice means the same on bytes as on a list: " +
        "{same_slice(code, 0, 3)} {same_slice(code, 2, 5)} " +
        "{same_slice(code, 3, 3)} {same_slice(code, 0, code.len())}")

    // Where they part company, deliberately: a list clamps a too-large end
    // because it is usually being trimmed, and `bytes` refuses one because it
    // is usually being parsed, where a short read is a truncated message.
    print("and refuses the end a list would clamp: " +
        "{code.to_list().slice(0, 99).len() == 7}")

    // A search from `from` on, which is how a line splitter walks a buffer
    // without slicing it. From the very end there is nothing left to find,
    // and a value no byte can hold is simply absent.
    let text = "a,b,c".to_bytes()
    print("index_of: {text.index_of(44, 0)} {text.index_of(44, 2)} " +
        "{text.index_of(44, 4)} {text.index_of(44, text.len())} {text.index_of(300, 0)}")

    // The length of a value released straight after it is read: the read has
    // to happen before the release, wherever the answer is used.
    print("a temporary's length: {made(337).len()} {made(337).len().as_f64() / 1000.0}")

    // A scan over a buffer: the index is read at every step, and the loop's
    // own bound is what keeps it inside. Counted from several starts, and
    // over an empty range, because a compiler may prove the bound once and
    // read without checking -- and then these are the answers it has to keep.
    let text2 = "a,b,c".to_bytes()
    print("scanned: {commas(text2, 0, text2.len())} {commas(text2, 0, 0)} " +
        "{commas(text2, 3, text2.len())} {commas(text2, 0, 3)}")

    // Past the end: whatever the loop was proved to allow, this is still the
    // read that fails, with the message every other out-of-range read gives.
    print("{commas(text2, 0, text2.len() + 2)}")
}

/// Where the commas are, as a number built from their positions: a scan
/// whose bound is the caller's, so it is only inside the buffer when the
/// caller's bound is.
fn commas(b: bytes, from: i64, upto: i64) -> i64 {
    var i = from
    var acc = 0
    while i < upto {
        if b.at(i) == 44 {
            acc = acc * 10 + i
        }
        i = i + 1
    }
    acc
}

fn same_slice(b: bytes, from: i64, to: i64) -> bool {
    return b.slice(from, to).to_list() == b.to_list().slice(from, to)
}

/// A fresh buffer of `n` bytes, so its caller holds the only reference.
fn made(n: i64) -> bytes {
    var xs: [i64] = []
    for i in 0..n {
        xs.push(i % 200)
    }
    bytes_of(xs)
}
```

## `L-LIB-CALL-C.gr`

```grasm
//! tag: L-LIB-CALL-C
//! expect: output
//! 5 true true
//! 8.0 7
//! 4660 18446744073709551615 1.5
// `a.call_c(Sig, ..)` calls the code at an address by the C convention, with
// the parameters and result of the function type `Sig` names. Arguments
// cross as they cross to an `extern fn`: a `str` as a NUL-terminated copy,
// `bytes` as its buffer, and a narrower integer result is read at its width.
// A C function's address and a Grasm function's are called the same way.
extern fn strlen(s: str) -> i64
extern fn memcmp(a: bytes, b: bytes, n: i64) -> i32
extern fn pow(x: f64, y: f64) -> f64
extern fn abs(n: i32) -> i32

type Measure = fn(str) -> i64
type Compare = fn(bytes, bytes, i64) -> i32
type Power = fn(f64, f64) -> f64
type Small = fn(i32) -> i32
type Pack = fn(u8, u8) -> u16
type Huge = fn(u64) -> u64
type Half = fn(f64) -> f64

fn pack(hi: u8, lo: u8) -> u16 {
    hi.as_u16().shl(8) + lo.as_u16()
}

fn flip(n: u64) -> u64 {
    n.bit_not()
}

fn half(x: f64) -> f64 {
    x / 2.0
}

fn main() {
    unsafe {
        let a = bytes_of([1, 2, 3])
        let b = bytes_of([1, 2, 4])
        let cmp = mem_fn_addr(memcmp)
        print("{mem_fn_addr(strlen).call_c(Measure, "hello")} {cmp.call_c(Compare, a, b, 3) < 0} {cmp.call_c(Compare, a, a, 3) == 0}")
        let n: i32 = -7
        print("{mem_fn_addr(pow).call_c(Power, 2.0, 3.0)} {mem_fn_addr(abs).call_c(Small, n)}")
        let zero: u64 = 0
        print("{mem_fn_addr(pack).call_c(Pack, 18, 52)} {mem_fn_addr(flip).call_c(Huge, zero)} {mem_fn_addr(half).call_c(Half, 3.0)}")
    }
}
```

## `L-LIB-CLOCK.gr`

```grasm
//! tag: L-LIB-CLOCK
//! expect: output
//! monotonic never went backwards: true
//! a wait of 50ms took at least that: true
// `now_millis` is the wall clock and may move backwards; `monotonic_millis`
// names no calendar moment and only increases, which is the only thing a
// duration can be measured with.
fn main() / {io} {
    var previous = monotonic_millis()
    var ordered = true
    for _i in 0..500 {
        let now = monotonic_millis()
        if now < previous {
            ordered = false
        }
        previous = now
    }
    print("monotonic never went backwards: {ordered}")
    let before = monotonic_millis()
    sleep_millis(50)
    print("a wait of 50ms took at least that: {monotonic_millis() - before >= 50}")
}
```

## `L-LIB-CMP.gr`

```grasm
//! tag: L-LIB-CMP
//! expect: output
//! -1 0 1
//! -1 1 -1
//! -1 0 1
//! -1 1
//! [(a, 3), (b, 3), (c, 1)]
// The total order sorting uses ([L-LIB-SORT-NAN]), as a number.
//
// It was specified, used internally by `sort`, and callable by nothing — so a
// program needing it reimplemented it with `<` and `>`, got NaN wrong, and
// scrambled a column exactly as that clause warns. A language that names the
// hazard should hand over the fix.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

/// What `cmp` makes writable: a comparator sort, with per-key direction
/// decided at run time rather than baked into a key expression.
fn sort_desc_by_count(rows: [(str, i64)]) -> [(str, i64)] {
    var out: [(str, i64)] = []
    for r in rows {
        var at = out.len()
        var i = 0
        for o in out {
            // Count descending, then name ascending.
            let by_count = r.1.cmp(o.1)
            let better = if by_count != 0 { by_count > 0 } else { r.0.cmp(o.0) < 0 }
            if better and i < at {
                at = i
            }
            i += 1
        }
        var next: [(str, i64)] = []
        var j = 0
        for o in out {
            if j == at {
                next.push(r)
            }
            next.push(o)
            j += 1
        }
        if at >= out.len() {
            next.push(r)
        }
        out = next
    }
    out
}

fn main() {
    print("{(1).cmp(2)} {(2).cmp(2)} {(3).cmp(2)}")
    // NaN sorts after every number, which is the part hand-rolling gets wrong.
    print("{(1.0).cmp(2.0)} {nan().cmp(1.0)} {(1.0).cmp(nan())}")
    print("{"a".cmp("b")} {"b".cmp("b")} {"c".cmp("b")}")
    print("{(1, 2).cmp((1, 3))} {(2, 0).cmp((1, 9))}")
    print("{sort_desc_by_count([("c", 1), ("a", 3), ("b", 3)])}")
}
```

## `L-LIB-CODES.gr`

```grasm
//! tag: L-LIB-CODES
//! expect: panic
//! 104 233
//! [104, 233, 108, 108, 111]
//! hé
//! héllo
//! 615042
//! non-bytes: 1 [8364]
//! last code point: 111
//! code_at 5 out of bounds (len 5)
fn main() {
    // Code points, counted in characters — `é` is one character, two bytes.
    print("{"héllo".code_at(0)} {"héllo".code_at(1)}")
    print("{"héllo".codes()}")
    // And back again.
    print("{104.to_char()}{233.to_char()}")
    // A full round trip through integers.
    var back = ""
    for c in "héllo".codes() {
        back = back + c.to_char()
    }
    print("{back}")
    // What the pair makes possible: arithmetic over text.
    print("{hash("grasm")}")
    // Characters outside ASCII are one code point, not their byte count.
    print("non-bytes: {"€".len()} {"€".codes()}")

    // The clause says `code_at` panics out of range, and the index worth
    // testing is the first one that is: exactly `len`. Nothing here tested it,
    // and changing the check from `i >= n` to `i > n` left every gate green
    // while the compiled build read past the end of the string and died with
    // SIGABRT where `grasm run` panicked. Reading one character too far is the
    // only mistake a bounds check can make.
    print("last code point: {"héllo".code_at("héllo".len() - 1)}")
    print("{"héllo".code_at("héllo".len())}")
}

/// A string hash, which cannot be written without code points at all.
fn hash(s: str) -> i64 {
    var h = 0
    for c in s.codes() {
        h = (h * 31 + c) % 1000007
    }
    h
}
```

## `L-LIB-EPRINT.gr`

```grasm
//! tag: L-LIB-EPRINT
//! expect: output
//! one
//! two
// `eprint` writes to standard error; `print` writes to standard output.
//
// The conformance harness compares stdout, so the two `eprint` lines below must
// NOT appear in the expected output above. That absence is the whole test: if
// `eprint` ever wrote to stdout, this program would fail with four lines where
// two were declared.
fn main() / {io} {
    print("one")
    eprint("this is a diagnostic and must not reach stdout")
    print("two")
    let n = 21
    eprint("interpolation works here too: {n * 2}")
}
```

## `L-LIB-EXIT.gr`

```grasm
//! tag: L-LIB-EXIT
//! expect: output-failing
//! about to fail
// `exit(code)` ends the program with the low eight bits of `code`.
//
// Clamping to 0..=255 instead reported *success* for `exit(-1)`, which is the
// most common spelling of "fail" in a script: the shell saw 0, and the three
// engines agreed on it.
fn main() {
    print("about to fail")
    exit(-1)
}
```

## `L-LIB-FIXED.gr`

```grasm
//! tag: L-LIB-FIXED
//! expect: output
//! 3 4 1 -1
//! 0.13 0.38 -0.13
//! 2.67 9.99 1.00
//! 100 0.01
//! 289.22 289 289.22222
//! -1.5 0.000
//! 0.3333
//! [      3.50]
//! inf -inf NaN
//! 6.875 2.292
fn main() {
    // Ties go away from zero, matching `round` ([L-LIB-ROUND]).
    print("{(2.5).to_fixed(0)} {(3.5).to_fixed(0)} {(0.5).to_fixed(0)} {(0.0 - 0.5).to_fixed(0)}")
    print("{(0.125).to_fixed(2)} {(0.375).to_fixed(2)} {(0.0 - 0.125).to_fixed(2)}")
    // A tie is a property of the stored value: 2.675 is really 2.67499...
    print("{(2.675).to_fixed(2)} {(9.995).to_fixed(2)} {(1.005).to_fixed(2)}")
    // Carrying past the point.
    print("{(99.5).to_fixed(0)} {(0.005).to_fixed(2)}")
    let mean = 289.2222222222222
    print("{mean.to_fixed(2)} {mean.to_fixed(0)} {mean.to_fixed(5)}")
    // Negatives and zero keep the requested width.
    print("{(0.0 - 1.5).to_fixed(1)} {0.0.to_fixed(3)}")
    print("{(1.0 / 3.0).to_fixed(4)}")
    // It composes with the padding a column needs.
    print("[{(3.5).to_fixed(2).pad_start(10, " ")}]")
    // Not-finite values ignore the precision, as [L-DISPLAY] renders them.
    let z = 0.0
    print("{(1.0 / z).to_fixed(2)} {((0.0 - 1.0) / z).to_fixed(0)} {(z / z).to_fixed(3)}")
    var total = 0.0
    for x in [1.5, 2.25, 3.125] {
        total = total + x
    }
    print("{total.to_fixed(3)} {(total / 3.0).to_fixed(3)}")
}
```

## `L-LIB-FREE.gr`

```grasm
//! tag: L-LIB-FREE
//! expect: output
//! 0 true
fn main() {
    // `args()` is empty with no `--` arguments; a fresh map has no entries.
    // Annotated because `map()` is an empty literal like `[]`, and one whose
    // type nothing determines is refused ([T-EMPTY-LIT]).
    var m: Map[str, i64] = map()
    print("{args().len()} {m.is_empty()}")
    assert(1 + 1 == 2)
    assert_eq(2 * 2, 4)
}
```

## `L-LIB-FS.gr`

```grasm
//! tag: L-LIB-FS
//! expect: output
//! listing is sorted: [a.txt, b.txt, c.txt]
//! exists before: true
//! exists after: false
//! listing after removal: [b.txt, c.txt]
//! removing an absent file reports NotFound
//! listing an absent directory reports NotFound
// Files can be removed, and a directory can be read ([L-LIB-FS]).
fn main() / {io} {
    // Under `target/`, which git ignores: there is no way to remove a
    // directory, so whatever this makes stays, and the gates run every
    // conformance program from the repository root.
    let dir = "target/grasm_fs_conformance"
    // The program makes its own working directory, so it is self-contained.
    // `make_dir` creates the whole path and succeeds if it already exists.
    let _mk = make_dir(dir) ?? ()
    // Written out of order on purpose: `list_dir` sorts, so the order below is
    // the sorted one and not the order the filesystem happens to hold.
    let _b = write_file(dir + "/b.txt", "b") ?? ()
    let _c = write_file(dir + "/c.txt", "c") ?? ()
    let _a = write_file(dir + "/a.txt", "a") ?? ()

    match list_dir(dir) {
        Ok(names) -> print("listing is sorted: {names}")
        Err(e) -> print("could not list: {e}")
    }
    print("exists before: {file_exists(dir + "/a.txt")}")
    match remove_file(dir + "/a.txt") {
        Ok(_) -> {}
        Err(e) -> print("could not remove: {e}")
    }
    print("exists after: {file_exists(dir + "/a.txt")}")
    match list_dir(dir) {
        Ok(names) -> print("listing after removal: {names}")
        Err(e) -> print("could not list: {e}")
    }
    match remove_file(dir + "/a.txt") {
        Ok(_) -> print("removed a file that was not there")
        Err(e) -> print("removing an absent file reports NotFound")
    }
    match list_dir(dir + "/nope") {
        Ok(_) -> print("listed a directory that was not there")
        Err(e) -> print("listing an absent directory reports NotFound")
    }
    // Leave nothing behind: running the suite twice is the same as once.
    let _rb = remove_file(dir + "/b.txt") ?? ()
    let _rc = remove_file(dir + "/c.txt") ?? ()
}
```

## `L-LIB-INT-CONV.gr`

```grasm
//! tag: L-LIB-INT-CONV
//! expect: output
//! 4464 None Some(200) 70000
//! 255 0 -128
//! None Some(254) 255 0
// Conversions between integer widths, and the operations that report or
// clamp an out-of-range result instead of wrapping it.
fn main() {
    let big = 70000
    let small = 200
    let a: u16 = big.as_u16()
    print("{a} {big.try_u16()} {small.try_u8()} {big.as_i64()}")
    let f = 300.7
    let g = 0.0 / 0.0
    let h = -1000.5
    print("{f.as_u8()} {g.as_u8()} {h.as_i8()}")
    let b: u8 = 250
    let one: u8 = 1
    print("{b.checked_add(10)} {b.checked_add(4)} {b.saturating_add(10)} {one.saturating_sub(5)}")
}
```

## `L-LIB-IO.gr`

```grasm
//! tag: L-LIB-IO
//! expect: output
//! missing: NotFound(grasm_absent_9f3c.txt)
//! exists: false
//! env: unset
//! types: ok
// The io builtins report absence in three different ways, on purpose — and
// nothing pinned which was which, so `read_file` sat documented as returning
// `Option[str]` while it was fallible.
//
// Only the failing paths are exercised, because those are the ones that need
// no filesystem state and give the same answer on every machine. The writing
// side is still *typechecked*, in a branch that never runs.
fn main() {
    // Fallible: the error says why.
    match read_file("grasm_absent_9f3c.txt") {
        Ok(s) -> print("missing: read {s.len()}")
        Err(e) -> print("missing: {e}")
    }
    // bool: the question was already yes-or-no.
    print("exists: {file_exists("grasm_absent_9f3c.txt")}")
    // Option: an unset variable has no reason to give.
    print("env: {env("GRASM_A_VARIABLE_NOBODY_SETS_9F3C") ?? "unset"}")
    // Typechecked, never run — enough to pin the signatures.
    if file_exists("grasm_absent_9f3c.txt") {
        // A write can fail, and its failure is not discardable: the shape
        // that printed "wrote the report" with no file on disk was exactly
        // this call as a bare statement.
        match write_file("grasm_absent_9f3c.txt", "unused") {
            Ok(_) -> print("wrote")
            Err(e) -> print("{e}")
        }
        let line = input()
        let ms = now_millis()
        print("{line} {ms}")
    }
    print("types: ok")
}
```

## `L-LIB-LIST.gr`

```grasm
//! tag: L-LIB-LIST
//! expect: output
//! 3 false 3 -1 3 2
//! [1, 2, 3] 4 [3, 2, 1] [1, 2] true 1-2-3
//! 6 1 3
//! [2, 4, 6] [2, 3, 4] [2, 3]
//! true true 2
//! 6 6 123
fn main() {
    var xs = [3, 1, 2]
    print("{xs.len()} {xs.is_empty()} {xs[0]} {xs.get(9) ?? 0 - 1} {xs.first() ?? 0} {xs.last() ?? 0}")
    xs.push(4)
    let popped = xs.pop() ?? 0
    xs.sort()
    // `join` takes [str], so numbers are mapped to text first.
    print("{xs} {popped} {xs.rev()} {xs.slice(0, 2)} {xs.contains(2)} {xs.map(|v| v.to_str()).join("-")}")
    print("{xs.sum()} {xs.min() ?? 0} {xs.max() ?? 0}")
    print("{xs.map(|v| v * 2)} {xs.par_map(|v| v + 1)} {xs.filter(|v| v > 1)}")
    print("{xs.any(|v| v > 2)} {xs.all(|v| v > 0)} {xs.find(|v| v > 1) ?? 0}")

    // A left fold, whose accumulator type comes from the initial value —
    // `[i64].fold("", ..)` produces a `str`.
    print("{xs.fold(0, |a, b| a + b)} {xs.fold(1, |a, b| a * b)} {xs.fold("", |a, b| a + b.to_str())}")
}
```

## `L-LIB-MAP.gr`

```grasm
//! tag: L-LIB-MAP
//! expect: output
//! 2 false 1 true [a, b] [1, 2]
//! [(a, 1), (b, 2)]
//! a=1
//! b=2
//! true 1
//! 3 3 1 3
//! [ax1, bx2, cx3]
//! 2 1
//! 22 2
//! 1 1 2
fn main() {
    var m = map()
    m.set("b", 2)
    m.set("a", 1)
    print("{m.len()} {m.is_empty()} {m.get("a") ?? 0} {m.has("b")} {m.keys()} {m.values()}")

    // Pairs, in key order. Without this the only way to walk a map was to
    // iterate `keys()` and look each one up again — a probe per entry, and a
    // `?? default` for a key that certainly exists.
    print("{m.entries()}")
    for e in m.entries() {
        match e {
            (k, v) -> print("{k}={v}")
        }
    }

    let gone = m.delete("b")
    print("{gone} {m.len()}")

    // A key built on the spot and used only to look up by. The text never has
    // to become a string of its own: it can be hashed where it was written and
    // a key made only when one is genuinely new. What must hold is that the
    // table ends up with exactly the keys the program wrote, whatever route
    // they took to get there.
    let tags = ["a", "b", "c"]
    var counts = map()
    for r in 0..3 {
        for i in 0..3 {
            let k = tags[i] + "x{i + 1}"
            counts.set(k, (counts.get(k) ?? 0) + 1)
        }
    }
    print("{counts.len()} {counts.get("bx2") ?? 0} {counts.get("zz") ?? 1} {counts.values().sum() / 3}")
    print("{counts.keys()}")

    // A key longer than a value holds inline, so the table has to own a copy
    // rather than borrow the buffer it was hashed from.
    var wide = map()
    for i in 0..2 {
        let long = "a-key-far-longer-than-fifteen-bytes-{i}"
        wide.set(long, i + 1)
    }
    print("{wide.len()} {wide.get("a-key-far-longer-than-fifteen-bytes-0") ?? 0}")

    // The same name read as a string as well as used as a key: it is an
    // ordinary string and behaves like one.
    var mixed = map()
    let both = "kk" + "{2}"
    mixed.set(both, 22)
    print("{mixed.get(both) ?? 0} {both.len() - 1}")

    // Leaving the block early, which the built form is not eligible for.
    var early = map()
    for i in 0..3 {
        let e = "e{i}"
        early.set(e, i)
        if i == 0 {
            continue
        }
    }
    print("{early.len() - 2} {early.get("e1") ?? 9} {early.get("e2") ?? 9 - 1}")
}
```

## `L-LIB-MATH.gr`

```grasm
//! tag: L-LIB-MATH
//! expect: output
//! 3.141593 2.718282
//! 0.0000 1.0000 0.785398
//! 2.718282 1.000000 3.0000 3.0000
//! 5.00 3.00 4.0000 1.047198
//! 3.141593 180.00 1.1752 0.7616
//! 99.0
// The math library, and the reason it is a library at all.
//
// Reaching libm through `extern fn cos` works, and it costs a capability: a
// declared C function performs `io` ([T-EXTERN-VALUE]), so a package declaring
// `capabilities = []` could not compute a cosine. A pure function should not
// cost a capability.
fn shadowed() -> f64 {
    // `pi` and `e` are the only builtins that yield to a function of the same
    // name. `e` is common enough as an identifier that taking it outright
    // would break working programs.
    99.0
}

fn main() {
    print("{pi().to_fixed(6)} {e().to_fixed(6)}")
    print("{(0.0).sin().to_fixed(4)} {(0.0).cos().to_fixed(4)} {(1.0).atan().to_fixed(6)}")
    print("{(1.0).exp().to_fixed(6)} {e().ln().to_fixed(6)} {(8.0).log2().to_fixed(4)} {(1000.0).log10().to_fixed(4)}")
    print("{(3.0).hypot(4.0).to_fixed(2)} {(27.0).cbrt().to_fixed(2)} {(81.0).log(3.0).to_fixed(4)} {(0.5).acos().to_fixed(6)}")
    print("{(180.0).to_radians().to_fixed(6)} {pi().to_degrees().to_fixed(2)} {(1.0).sinh().to_fixed(4)} {(1.0).tanh().to_fixed(4)}")
    print("{shadowed()}")
}
```

## `L-LIB-MEM.gr`

```grasm
//! tag: L-LIB-MEM
//! expect: output
//! 0 0 0.0
//! -2 254 -2 65534 -2 4294967294 -2 18446744073709551614
//! 1.5 true 0x123456789abcdef 0x89abcdef 0x67 0x1
//! 1 2 4 8 8 8 | 1 2 4 8 8 8
//! 24 -24 true true
//! 0 true
// A fresh allocation is zeroed. Each layout type is stored and read back at
// its width, a narrow one read signed or not as its type says, and memory is
// little-endian, so the low byte of a word is at its address.
fn main() / {io, unsafe, mem} {
    unsafe {
        let a = mem_alloc(40, 8)
        print("{a.load_i64()} {a.offset(8).load_u8()} {a.offset(16).load_f64()}")
        a.store_i8(-2)
        a.offset(2).store_i16(-2)
        a.offset(4).store_i32(-2)
        a.offset(8).store_i64(-2)
        print("{a.load_i8()} {a.load_u8()} {a.offset(2).load_i16()} {a.offset(2).load_u16()} {a.offset(4).load_i32()} {a.offset(4).load_u32()} {a.offset(8).load_i64()} {a.offset(8).load_u64()}")
        a.offset(16).store_f64(1.5)
        a.offset(24).store_addr(a)
        let w: u64 = 0x0123_4567_89ab_cdef
        a.offset(32).store_u64(w)
        print("{a.offset(16).load_f64()} {a.offset(24).load_addr() == a} 0x{a.offset(32).load_u64().to_base(16)} 0x{a.offset(32).load_u32().to_base(16)} 0x{a.offset(36).load_u8().to_base(16)} 0x{a.offset(39).load_u8().to_base(16)}")
        print("{mem_size_of(u8)} {mem_size_of(i16)} {mem_size_of(u32)} {mem_size_of(i64)} {mem_size_of(f64)} {mem_size_of(addr)} | {mem_align_of(i8)} {mem_align_of(u16)} {mem_align_of(i32)} {mem_align_of(u64)} {mem_align_of(f64)} {mem_align_of(addr)}")
        let end = a.offset(32)
        print("{end.diff(a.offset(8))} {a.offset(8).diff(end)} {mem_addr(a.as_i64()) == a} {end.offset(-32) == a}")
        mem_free(a)
        // Zero bytes is an allocation too, aligned as asked, and freed the same.
        let z = mem_alloc(0, 4096)
        print("{z.as_i64() % 4096} {z.offset(0) == z}")
        mem_free(z)
    }
}
```

## `L-LIB-NET-PEER.gr`

```grasm
//! tag: L-LIB-NET-PEER
//! expect: output
//! listening on a named address
//! the accepted side sees the peer at 127.0.0.1: true
//! the peer's port is a real port: true
//! the client side sees the listener's own port: true
//! a listener has no peer, and that is an Err
//! a handle that names nothing has no peer either
//! monotonic_nanos never went backwards: true
// Who is at the other end of a connection, a listen on a named address, and a
// clock fine enough to time one request.
//
// The port is chosen by the system and the client's port is ephemeral, so the
// transcript prints what is true of them rather than the numbers themselves.
fn main() / {io, net} {
    match tcp_listen_on("127.0.0.1", 0) {
        Err(e) -> print("could not listen")
        Ok(listener) -> {
            print("listening on a named address")
            let port = net_port(listener) ?? 0
            match tcp_connect("127.0.0.1", port) {
                Err(e) -> print("could not dial the listener")
                Ok(client) -> {
                    match tcp_accept(listener) {
                        Err(e) -> print("could not accept")
                        Ok(conn) -> {
                            // The accepted side's peer is the client: the
                            // loopback address and the ephemeral port the
                            // system gave the dial.
                            match tcp_peer(conn) {
                                Err(e) -> print("the accepted side has no peer, which it must")
                                Ok(peer) -> {
                                    let (host, their_port) = peer
                                    print("the accepted side sees the peer at 127.0.0.1: {host == "127.0.0.1"}")
                                    print("the peer's port is a real port: {their_port > 0 and their_port <= 65535}")
                                }
                            }
                            // And the client's peer is the listener, on the
                            // port that was read back from it.
                            match tcp_peer(client) {
                                Err(e) -> print("the client side has no peer, which it must")
                                Ok(peer) -> {
                                    let (_, their_port) = peer
                                    print("the client side sees the listener's own port: {their_port == port}")
                                }
                            }
                            net_close(conn)
                        }
                    }
                    net_close(client)
                }
            }

            // A listener is not connected to anything, so there is no answer
            // to give — an ordinary failure, not a made-up address.
            match tcp_peer(listener) {
                Ok(_) -> print("a listener reported a peer, which it must not")
                Err(e) -> print("a listener has no peer, and that is an Err")
            }
            net_close(listener)
            match tcp_peer(4242) {
                Ok(_) -> print("a handle that names nothing reported a peer, which it must not")
                Err(e) -> print("a handle that names nothing has no peer either")
            }
        }
    }

    // The same only-increasing clock as `monotonic_millis`, read in
    // nanoseconds. Equal readings are allowed — a coarse clock may not tick
    // between two calls — and a smaller one never is.
    let first = monotonic_nanos()
    let second = monotonic_nanos()
    print("monotonic_nanos never went backwards: {second >= first}")
}
```

## `L-LIB-NET-WORKERS.gr`

```grasm
//! tag: L-LIB-NET-WORKERS
//! expect: output
//! results in index order: [0, 10, 20, 30]
//! no workers: 0 results
//! fewer than none: 0 results
//! every worker talked to itself: [true, true, true]
//! a worker's listener is open on the main thread too: true
//! the main thread's listener is open in every worker: [true, true]
//! one waiting connection, accepted once: 1
// `net_workers(n, f)` runs `f(0)`, ..., `f(n - 1)` — on a thread each in a
// compiled build, one after another in index order under `grasm run` — and
// answers their results in index order. Every line here is the same under
// both schedules, which is the property the clause promises.

/// Opens a listener of its own, dials it, and passes a message through:
/// everything a worker can do with sockets nobody else has.
fn talk_to_self(i: i64) -> bool / {net} {
    match tcp_listen_on("127.0.0.1", 0) {
        Err(_) -> false
        Ok(l) -> {
            let port = net_port(l) ?? 0
            var ok = false
            match tcp_connect("127.0.0.1", port) {
                Err(_) -> {}
                Ok(client) -> {
                    match tcp_accept(l) {
                        Err(_) -> {}
                        Ok(conn) -> {
                            let sent = tcp_write(client, "worker {i}") ?? ()
                            ok = (tcp_read(conn) ?? "") == "worker {i}"
                            net_close(conn)
                        }
                    }
                    net_close(client)
                }
            }
            net_close(l)
            ok
        }
    }
}

/// A listener left open for the main thread to look at.
fn open_one(i: i64) -> i64 / {net} {
    tcp_listen_on("127.0.0.1", 0) ?? -1
}

/// 1 if this worker took the waiting connection, 0 if it found none or another
/// worker took it first. A `tcp_accept` after `net_wait` reported the listener
/// ready does not wait, so the worker that lost the race answers rather than
/// waiting for a client that will never come.
fn take(listener: i64) -> i64 / {net} {
    let ready = net_wait([listener], 200) ?? []
    if ready.is_empty() {
        return 0
    }
    match tcp_accept(listener) {
        Err(_) -> 0
        Ok(c) -> {
            net_close(c)
            1
        }
    }
}

fn main() / {io, net} {
    let step = 10
    print("results in index order: {net_workers(4, |i| i * step)}")
    print("no workers: {net_workers(0, |i| i).len()} results")
    print("fewer than none: {net_workers(-3, |i| i).len()} results")
    print("every worker talked to itself: {net_workers(3, talk_to_self)}")

    // One table for the whole program: a handle means the same socket on
    // every thread.
    let opened = net_workers(1, open_one)
    let h = opened[0]
    print("a worker's listener is open on the main thread too: {(net_port(h) ?? 0) > 0}")
    net_close(h)

    match tcp_listen_on("127.0.0.1", 0) {
        Err(_) -> print("could not listen")
        Ok(listener) -> {
            let port = net_port(listener) ?? 0
            print("the main thread's listener is open in every worker: {net_workers(2, |i| (net_port(listener) ?? -1) == port)}")
            match tcp_connect("127.0.0.1", port) {
                Err(_) -> print("could not dial")
                Ok(client) -> {
                    let took = net_workers(2, |i| take(listener))
                    print("one waiting connection, accepted once: {took[0] + took[1]}")
                    net_close(client)
                }
            }
            net_close(listener)
        }
    }
}
```

## `L-LIB-NET.gr`

```grasm
//! tag: L-LIB-NET
//! expect: output
//! listening
//! a chosen port is in range
//! empty wait set: 0 ready
//! a handle that names nothing is an Err
//! before dial: nothing ready
//! after dial: the listener
//! before send: nothing ready
//! after send: the connection
//! read: ping
//! after hangup: the connection
//! read at end: 0 bytes
//! closed twice is fine
//! a closed handle reports rather than panics
//! nothing is listening there, and that is an Err
//! an unresolvable name is an Err too
//! as text, 0xff 0xfe became: [239, 191, 189, 239, 191, 189, 65, 66, 0, 67]
//! as bytes, they survived: [255, 254, 65, 66, 0, 67]
//! unread data: the connection
//! a new connection on the freed handle: nothing ready
//! once it sends: the connection
//! left out of the set: nothing ready
//! put back: the connection
// The `net` capability: handles are integers, payloads are text.
//
// Port 0 asks the system to choose, so this program can run anywhere without
// colliding with whatever else holds a port. It does not accept a connection —
// that would need a second process — and the shapes it does check are the ones
// with an answer that does not depend on the machine.
/// Names the ready handles by their role, so the transcript does not depend on
/// which numbers this machine's socket table happened to hand out.
fn report(step: str, ready: [i64], listener: i64, conn: i64) / {io} {
    var out = ""
    for h in ready {
        if h == listener {
            out = out + "the listener "
        } else if h == conn {
            out = out + "the connection "
        } else {
            out = out + "handle {h} "
        }
    }
    if out == "" {
        print("{step}: nothing ready")
    } else {
        print("{step}: {out.trim()}")
    }
}

fn main() / {io, net} {
    match tcp_listen(0) {
        Err(e) -> print("could not listen")
        Ok(listener) -> {
            print("listening")

            // Port 0 asked the system to choose. Reading the choice back is
            // what makes that spelling usable: the rest of this program dials
            // the listener it just opened, on a machine where the port it
            // would otherwise have picked may already be held.
            let port = net_port(listener) ?? 0
            if port > 0 and port <= 65535 {
                print("a chosen port is in range")
            } else {
                print("a chosen port is NOT in range: {port}")
            }

            // --- readiness --------------------------------------------------
            //
            // `net_wait` is what lets one thread hold many conversations.
            // Each step checks readiness *before* the event that causes it as
            // well as after: a `net_wait` that always answered "ready" would
            // pass a test that only looked at the second half, and would make
            // every server written against it busy-spin.

            // An empty wait set is answered immediately and is not an error —
            // a server that has just closed its last connection asks exactly
            // this on its next turn.
            match net_wait([], 0) {
                Ok(r) -> print("empty wait set: {r.len()} ready")
                Err(e) -> print("empty wait set: FAILED")
            }

            // A handle that names nothing is an ordinary failure, the same as
            // using a connection after closing it — never a panic.
            match net_wait([4242], 0) {
                Ok(r) -> print("a handle that names nothing was accepted, which it must not")
                Err(e) -> print("a handle that names nothing is an Err")
            }

            report("before dial", net_wait([listener], 0) ?? [], listener, -1)

            match tcp_connect("127.0.0.1", port) {
                Err(e) -> print("could not dial the listener")
                Ok(client) -> {
                    // The handshake completed, so accepting will not block.
                    report("after dial", net_wait([listener], 1000) ?? [], listener, -1)

                    match tcp_accept(listener) {
                        Err(e) -> print("could not accept")
                        Ok(conn) -> {
                            // Accepted, but the client has said nothing yet.
                            report("before send", net_wait([conn], 0) ?? [], listener, conn)

                            match tcp_write(client, "ping") {
                                Ok(_) -> {}
                                Err(e) -> print("could not write")
                            }

                            // Both handles in one call, so the answer also
                            // shows the ready ones being picked out rather
                            // than the whole set coming back.
                            report("after send", net_wait([listener, conn], 1000) ?? [], listener, conn)
                            print("read: {tcp_read(conn) ?? "(nothing)"}")

                            // A hung-up peer is ready: the read that follows
                            // returns nothing, which is how a caller learns
                            // the conversation ended. Reporting it as
                            // not-ready would leave a dead connection in the
                            // wait set forever.
                            net_close(client)
                            report("after hangup", net_wait([conn], 1000) ?? [], listener, conn)
                            print("read at end: {(tcp_read(conn) ?? "").len()} bytes")
                            net_close(conn)
                        }
                    }
                }
            }

            // Closing twice is not an error, for the same reason releasing a
            // `ref` twice is not: cleanup order is not the caller's to track.
            net_close(listener)
            net_close(listener)
            print("closed twice is fine")
            // Using a closed handle is an ordinary failure, never a panic and
            // never a wrong answer.
            match tcp_read(listener) {
                Ok(_) -> print("a closed handle answered, which it must not")
                Err(e) -> print("a closed handle reports rather than panics")
            }
            // `tcp_connect` is the client half. Port 1 is privileged and
            // nothing listens there, so this is a refusal rather than a
            // machine-dependent answer.
            match tcp_connect("127.0.0.1", 1) {
                Ok(_) -> print("something answered on port 1, which it must not")
                Err(e) -> print("nothing is listening there, and that is an Err")
            }
            match tcp_connect("no.such.host.invalid", 80) {
                Ok(_) -> print("an invalid name resolved, which it must not")
                Err(e) -> print("an unresolvable name is an Err too")
            }

            // Text and bytes are two reads, not one with a flag, and the
            // difference is visible: `tcp_read` replaces what is not valid
            // UTF-8, which is right for a protocol made of text and destroys a
            // JPEG. The same six bytes are sent twice and read both ways.
            binary_round_trip()
        }
    }
}

/// The same payload read as text and as bytes. Sent twice over two fresh
/// connections so neither read can see the other's data.
fn binary_round_trip() / {net, io} {
    let payload = [255, 254, 65, 66, 0, 67]
    print("as text, 0xff 0xfe became: {echo_as(payload, true)}")
    print("as bytes, they survived: {echo_as(payload, false)}")
    changing_set()
}

/// Sends `payload` to a listener on this machine and reads it back, as text
/// when `as_text` and as bytes otherwise. Answers with the byte values seen.
fn echo_as(payload: [i64], as_text: bool) -> [i64] / {net} {
    match tcp_listen(0) {
        Err(e) -> []
        Ok(srv) -> {
            var seen: [i64] = []
            match net_port(srv) {
                Err(e) -> {}
                Ok(port) -> match tcp_connect("127.0.0.1", port) {
                    Err(e) -> {}
                    Ok(cli) -> {
                        match tcp_write_bytes(cli, bytes_of(payload)) {
                            Err(e) -> {}
                            Ok(_) -> match tcp_accept(srv) {
                                Err(e) -> {}
                                Ok(conn) -> {
                                    if as_text {
                                        seen = (tcp_read(conn) ?? "").to_bytes().to_list()
                                    } else {
                                        seen = (tcp_read_bytes(conn) ?? bytes_of([])).to_list()
                                    }
                                    net_close(conn)
                                }
                            }
                        }
                        net_close(cli)
                    }
                }
            }
            net_close(srv)
            seen
        }
    }
}

/// Writes, reporting only a failure.
fn send(h: i64, text: str) / {io, net} {
    match tcp_write(h, text) {
        Ok(_) -> {}
        Err(e) -> print("could not write")
    }
}

/// A wait set that changes from one wait to the next, as a server's does: a
/// connection closed while it still had something to read, its handle taken
/// by a new connection that has said nothing yet, and a ready connection left
/// out of the set and then put back. Each answer must be about the set asked
/// for now, never about one asked for earlier.
fn changing_set() / {io, net} {
    match tcp_listen(0) {
        Err(e) -> print("could not listen")
        Ok(srv) -> {
            let port = net_port(srv) ?? 0
            let a = tcp_connect("127.0.0.1", port) ?? -1
            let a_in = tcp_accept(srv) ?? -1
            send(a, "x")
            report("unread data", net_wait([srv, a_in], 1000) ?? [], srv, a_in)
            net_close(a_in)
            net_close(a)
            let b = tcp_connect("127.0.0.1", port) ?? -1
            let b_in = tcp_accept(srv) ?? -1
            report("a new connection on the freed handle", net_wait([srv, b_in], 0) ?? [], srv, b_in)
            send(b, "y")
            report("once it sends", net_wait([srv, b_in], 1000) ?? [], srv, b_in)
            report("left out of the set", net_wait([srv], 0) ?? [], srv, b_in)
            report("put back", net_wait([srv, b_in], 1000) ?? [], srv, b_in)
            net_close(b_in)
            net_close(b)
            net_close(srv)
        }
    }
}
```

## `L-LIB-NUM.gr`

```grasm
//! tag: L-LIB-NUM
//! expect: output
//! 4 2.0 3.0 3.0 3.0
//! 3 5 7 3 2.0
fn main() {
    print("{(0 - 4).abs()} {(2.5).floor()} {(2.1).ceil()} {(2.6).round()} {(9.0).sqrt()}")
    // Conversion is by method: as_f64 widens, as_i64 truncates.
    print("{(3).min(5)} {(3).max(5)} {(7).to_str()} {(3.7).as_i64()} {(2).as_f64()}")
}
```

## `L-LIB-OPTION.gr`

```grasm
//! tag: L-LIB-OPTION
//! expect: output
//! false 3 true 0
//! 3
fn main() {
    let xs = [3, 1]
    let o = xs.get(0)
    print("{o.is_none()} {o.unwrap_or(0)} {xs.get(99).is_none()} {xs.get(99) ?? 0}")
    // For when absence is a bug rather than a case: the message is required,
    // because a panic nobody can read is not better than the `??` it replaced.
    print("{xs.first().expect("a two-element list has a first")}")
}
```

## `L-LIB-PAIRS.gr`

```grasm
//! tag: L-LIB-PAIRS
//! expect: output
//! [(0, a), (1, b), (2, c)]
//! [(1, x), (2, y)]
//! [] []
//! a0 b1 c2
//! 1x 2y
fn main() {
    let ws = ["a", "b", "c"]
    print("{ws.enumerate()}")
    // `zip` stops at the shorter of the two — pairing past the end of one
    // would have to invent an element.
    print("{[1, 2, 3].zip(["x", "y"])}")
    // Empty on either side gives empty.
    let none: [i64] = []
    print("{none.enumerate()} {none.zip(ws)}")
    // What they are for: naming both halves in the loop header.
    var out: [str] = []
    for (i, w) in ws.enumerate() {
        out.push("{w}{i}")
    }
    print("{out.join(" ")}")
    var pairs: [str] = []
    for (n, w) in [1, 2].zip(["x", "y"]) {
        pairs.push("{n}{w}")
    }
    print("{pairs.join(" ")}")
}
```

## `L-LIB-PARSE.gr`

```grasm
//! tag: L-LIB-PARSE
//! expect: output
//! inf NaN inf -inf
//! none none
//! 42 -7 none none
//! [1.5, -2.5] true
//! cdef |  |
// What a parser does with input that is nearly a number. Untrusted data
// reaches these, and every surprise here becomes a surprise somewhere else:
// `"1e400"` parsing to `inf` is what put an infinity into `as_i64` and into a
// sort, so both of those had to be specified too.
fn show(o: Option[f64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn showi(o: Option[i64]) -> str {
    match o {
        Some(v) -> "{v}"
        None -> "none"
    }
}

fn main() {
    // Infinities and NaN are accepted, and overflow gives an infinity.
    print("{show("inf".parse_float())} {show("nan".parse_float())} {show("1e400".parse_float())} {show("-inf".parse_float())}")
    // Text that is not a number is `None`, not zero.
    print("{show("abc".parse_float())} {showi("abc".parse_int())}")
    // Space and a sign are fine; source-literal spellings are not.
    print("{showi(" +42 ".parse_int())} {showi("-7".parse_int())} {showi("1_000".parse_int())} {showi("0xFF".parse_int())}")
    // Which is why a program reading untrusted numbers can ask.
    var kept: [f64] = []
    for c in ["1.5", "1e400", "nan", "-2.5"] {
        let v = c.parse_float() ?? 0.0
        if v.is_finite() {
            kept.push(v)
        }
    }
    print("{kept} {(0.0 / 0.0).is_nan()}")
    // `substr` clamps rather than panicking, unlike indexing.
    print("{"abcdef".substr(2, 100)} | {"abc".substr(5, 2)} |")
}
```

## `L-LIB-PROC.gr`

```grasm
//! tag: L-LIB-PROC
//! expect: output
//! echo: code 0 out "hello world" err empty
//! exit status is carried: 3
//! stderr is separate from stdout: to-err
//! arguments do not go through a shell: a b;c
//! a program that cannot be started is an Err: true
//! and that is different from one that ran and failed: 1
//! a pipeline feeds each stage into the next: a b c
//! input reaches the first stage: hello
//! the status is the last stage's: 4
//! an empty pipeline is an Err: true
//! a stage that cannot start is an Err: true
// Running another program, under a capability of its own.
//
// `proc` is not `io` and not `net`. It is strictly larger than either: a
// program that can run `curl` has the network whether or not it declared
// `net`. The tools used here are the ones POSIX guarantees.
fn main() / {io, proc} {
    match proc_run("echo", ["hello", "world"]) {
        Err(e) -> print("echo failed")
        Ok(r) -> print("echo: code {r.0} out \"{r.1.trim()}\" err {if r.2.len() == 0 { "empty" } else { r.2 }}")
    }

    match proc_run("sh", ["-c", "exit 3"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("exit status is carried: {r.0}")
    }

    // The two streams do not run together, which is the reason they are
    // returned separately rather than joined.
    match proc_run("sh", ["-c", "echo to-err 1>&2"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("stderr is separate from stdout: {r.2.trim()}{r.1.trim()}")
    }

    // Arguments are passed as a list, never through a shell, so a `;` in one of
    // them is a character and not a command separator.
    match proc_run("echo", ["a", "b;c"]) {
        Err(e) -> print("echo failed")
        Ok(r) -> print("arguments do not go through a shell: {r.1.trim()}")
    }

    // Not being able to start a program is a different thing from a program
    // that ran and returned a failure, and the two are not confusable.
    let missing = match proc_run("no-such-program-anywhere", []) {
        Err(e) -> true
        Ok(r) -> false
    }
    print("a program that cannot be started is an Err: {missing}")
    match proc_run("sh", ["-c", "exit 1"]) {
        Err(e) -> print("sh failed")
        Ok(r) -> print("and that is different from one that ran and failed: {r.0}")
    }

    // A pipeline: one child's stdout *is* the next one's stdin, which is the
    // thing `proc_run` cannot express because it waits.
    //
    // `sort` and `uniq` and not `wc`: `wc -l` right-aligns its count on the
    // BSD tools and does not on the GNU ones, and a gate whose answer depends
    // on which machine ran it is the defect F18 records.
    match proc_pipeline([("sort", []), ("uniq", [])], "b\na\nb\nc\na\n") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("a pipeline feeds each stage into the next: {r.1.trim().lines().join(" ")}")
    }

    // `input` feeds the first stage. A filter with nothing to filter is not a
    // filter, and `proc_run` had no way to supply it.
    match proc_pipeline([("cat", [])], "hello\n") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("input reaches the first stage: {r.1.trim()}")
    }

    // The answer is the *last* stage's, which is what every shell means.
    match proc_pipeline([("echo", ["ignored"]), ("sh", ["-c", "cat > /dev/null; exit 4"])], "") {
        Err(e) -> print("pipeline failed")
        Ok(r) -> print("the status is the last stage's: {r.0}")
    }

    let empty = match proc_pipeline([], "") {
        Err(e) -> true
        Ok(r) -> false
    }
    print("an empty pipeline is an Err: {empty}")

    let missing = match proc_pipeline([("no-such-program-anywhere", [])], "") {
        Err(e) -> true
        Ok(r) -> false
    }
    print("a stage that cannot start is an Err: {missing}")
}
```

## `L-LIB-RANDOM.gr`

```grasm
//! tag: L-LIB-RANDOM
//! expect: output
//! 32 bytes asked for, 32 given
//! every byte is a byte: true
//! two calls differ: true
//! zero is empty, not an error: 0
//! negative is refused
//! past the cap is refused
//! a byte in 0..5 without bias, 200 draws, every value seen: true
// `random_bytes(n)` — the only source of entropy in the language, and the
// primitive everything else is built from.
//
// Bytes rather than an integer or a float, because the conversions are where
// the mistakes are: a number in a range wants rejection sampling, not a modulo,
// and a language shipping `random_int(lo, hi)` with a modulo in it would have
// shipped a bias. The last check below builds one correctly, in Grasm, to show
// what the primitive is for.
//
// Charged `io`: it reaches outside the program and two calls do not agree.

/// A number in `0..n` with no modulo bias, by rejection.
///
/// A byte is 0..255. Taking `b % n` makes the low values more likely whenever
/// `n` does not divide 256 — for `n` of 5, the values 0 and 1 come up 52 times
/// in 256 and the rest 51. So the top of the range is cut to a multiple of `n`
/// and anything above it is drawn again.
fn below(n: i64, source: bytes, inout at: i64) -> i64 {
    let limit = 256 - (256 % n)
    var going = true
    var answer = 0
    while going {
        if at >= source.len() {
            // Out of entropy: the caller gave a buffer too small. Answering
            // anything here would be inventing a number.
            going = false
            answer = -1
        } else {
            let b = source.at(at)
            at = at + 1
            if b < limit {
                answer = b % n
                going = false
            }
        }
    }
    answer
}

fn main() / {io} {
    match random_bytes(32) {
        Err(e) -> print("random_bytes failed: {e}")
        Ok(b) -> {
            print("32 bytes asked for, {b.len()} given")
            var all_bytes = true
            for v in b.to_list() {
                if v < 0 or v > 255 {
                    all_bytes = false
                }
            }
            print("every byte is a byte: {all_bytes}")
        }
    }

    // Two draws of 32 bytes being equal has probability 2^-256. If this ever
    // prints false, the entropy source is broken and that is worth failing on.
    let a = random_bytes(32) ?? bytes_of([])
    let c = random_bytes(32) ?? bytes_of([])
    print("two calls differ: {a.to_list() != c.to_list()}")

    print("zero is empty, not an error: {(random_bytes(0) ?? bytes_of([9])).len()}")

    match random_bytes(-1) {
        Ok(_) -> print("negative was accepted, which it must not be")
        Err(e) -> print("negative is refused")
    }
    match random_bytes(2000000) {
        Ok(_) -> print("past the cap was accepted, which it must not be")
        Err(e) -> print("past the cap is refused")
    }

    // What the primitive is for: a fair number in a range, built here rather
    // than shipped with a bias in it.
    let pool = random_bytes(512) ?? bytes_of([])
    var seen = [false, false, false, false, false]
    var at = 0
    var draws = 0
    while draws < 200 {
        let v = below(5, pool, at)
        if v >= 0 {
            seen[v] = true
        }
        draws = draws + 1
    }
    var every = true
    for s in seen {
        if not s {
            every = false
        }
    }
    print("a byte in 0..5 without bias, 200 draws, every value seen: {every}")
}
```

## `L-LIB-RANGE-OPS.gr`

```grasm
//! tag: L-LIB-RANGE-OPS
//! expect: output
//! [0, 1, 2, 0, 1]
//! 10
//! [0, 2, 4, 6, 8]
//! [0, 1, 2, 3]
//! 7
//! 165
//! 0..3
//! 1..=3
//! true
//! false
//! 3
//! [0, 1, 2]
//! 0 1 2 
//! w 0..4 6
//! 4 [0, 10]
//! 4 [2, 3]
//! 0 [7] 0
//! bound 1
//! bound 4
//! [true, true, true] [9, 9, 9] [0, 2, 4, 6]
// A range answers the list combinators, producing a list of the same elements
// `for i in a..b` walks.
//
// Building a list from a count needed `var xs: [i64] = []`, a `for`, a `push`
// and two closing braces — four lines where `(0..n).map(f)` says the same
// thing, in every program that builds a list from a count. [G11] budgets Grasm
// at or below Python's token count; that pattern was most of the gap.
//
// There is one definition of what a range contains and not two: every engine
// builds that list and reuses the list method. So the three agree by
// construction rather than by three implementations happening to match.
//
// A range is also a value. It prints as its bounds rather than as its elements
// — `0..2000000` is two words and a flag, and printing two million integers
// would be a different value — and two compare by their bounds, so `0..0` is
// not equal to `5..5` even though both are empty.
type Window = { span: range, label: str }

fn window(n: i64) -> range {
    0..n
}

fn total(r: range) -> i64 {
    r.sum()
}

fn main() {
    print("{(0..5).map(|i| i % 3)}")
    print("{(1..=4).sum()}")
    print("{(0..10).filter(|i| i % 2 == 0)}")
    print("{(0..4).to_list()}")
    print("{(0..7).len()}")
    // Composes with the list surface, since what comes back *is* a list.
    print("{(0..10).map(|i| i * i).filter(|n| n % 2 == 1).sum()}")

    // Held as a value: printed by its bounds, compared by them, and walked.
    let r = 0..3
    print("{r}")
    print("{1..=3}")
    print("{(0..3) == (0..3)}")
    print("{(0..3) == (0..=3)}")
    print("{r.len()}")
    print("{r.to_list()}")
    var line = ""
    for i in r {
        line = line + "{i} "
    }
    print(line)

    // Declared by name. `range` displayed in the language's error messages
    // before it was in its syntax, so a range could be bound and never
    // declared -- not a parameter, not a result, not a field.
    let w = Window{ span: window(4), label: "w" }
    print("{w.label} {w.span} {total(w.span)}")

    // And reached from shapes that are not a name: the test for "does this
    // hold a range" recognised a bare identifier and nothing else.
    let rs: [range] = [0..2, 5..9]
    print("{rs[1].len()} {rs[0].map(|v| v * 10)}")
    let f = || 2..6
    print("{f().len()} {window(4).filter(|v| v > 1)}")

    // Mapping a range to one constant is a fill rather than a loop, and the
    // shapes around that have to keep answering the same way: an empty or
    // backwards range is still a list, and each bound is computed exactly
    // once no matter which lowering runs.
    print("{(5..2).map(|_i| 0).len()} {(3..=3).map(|_i| 7)} {(0..0).map(|_i| 1.5).len()}")
    let filled = (bound(1)..bound(4)).map(|_i| true)
    // Not a literal, so still a loop: a binding, and a body that reads the
    // element.
    var k = 9
    print("{filled} {(0..3).map(|_i| k)} {(0..4).map(|i| i * 2)}")
}

// Announces itself, so a bound evaluated twice is visible in the output.
fn bound(x: i64) -> i64 / {io} {
    print("bound {x}")
    return x
}
```

## `L-LIB-REMOVE.gr`

```grasm
//! tag: L-LIB-REMOVE
//! expect: panic
//! 10 30 [20, 40]
//! b [a, c]
//! x 1
//! [3, 1, 2]
//! the last element can be removed: 2
//! remove 1 out of bounds (len 1)
// Taking an element out by index, which is what draining a queue from the
// front needs.
type Job = { id: i64, name: str }

fn main() {
    var q = [10, 20, 30, 40]
    let first = q.remove(0)
    let mid = q.remove(1)
    print("{first} {mid} {q}")

    // Boxed elements too.
    var names = ["a", "b", "c"]
    let b = names.remove(1)
    print("{b} {names}")
    var jobs = [Job{ id: 1, name: "x" }, Job{ id: 2, name: "y" }]
    let j = jobs.remove(0)
    print("{j.name} {jobs.len()}")

    // The shape it exists for: a queue drained in order.
    var ready = [3, 1, 2]
    var order: [i64] = []
    while ready.len() > 0 {
        order.push(ready.remove(0))
    }
    print("{order}")

    // The clause says `remove` panics out of range "like indexing", and the
    // index worth testing is the first one that is: exactly `len`. Removing
    // the *last* element must work, and removing one past it must not — the
    // pair is the whole of what a bounds check can get wrong.
    var last = [1, 2]
    let two = last.remove(last.len() - 1)
    print("the last element can be removed: {two}")
    let _gone = last.remove(last.len())
    print("unreachable {_gone}")
}
```

## `L-LIB-ROUND.gr`

```grasm
//! tag: L-LIB-ROUND
//! expect: output
//! 3.0 4.0 -3.0 -1.0
//! 2.0 3.0 -2.0 -0.0
//! inf -inf NaN
//! -9223372036854775808
fn main() {
    // Ties go away from zero, not to even.
    print("{(2.5).round()} {(3.5).round()} {(0.0 - 2.5).round()} {(0.0 - 0.5).round()}")
    // Non-ties are unremarkable, and `-0.4` rounds to `-0.0`, not `0.0`.
    print("{(2.4).round()} {(2.6).round()} {(0.0 - 2.4).round()} {(0.0 - 0.4).round()}")
    // Division by zero on floats is not a panic; it is the IEEE answer.
    let z = 0.0
    print("{1.0 / z} {(0.0 - 1.0) / z} {z / z}")
    // The one integer with no positive counterpart wraps to itself.
    let least = 0 - 9223372036854775807 - 1
    print("{least.abs()}")
}
```

## `L-LIB-SORT-BY.gr`

```grasm
//! tag: L-LIB-SORT-BY
//! expect: output
//! al 25
//! cy 30
//! bo 30
//! [3, 2, 1]
//! [a, bb, ccc]
//! y
//! z
//! [7] []
type P = { name: str, age: i64 }

fn main() {
    // A list of structs has no order of its own; the key does.
    var ps = [P{ name: "cy", age: 30 }, P{ name: "al", age: 25 }, P{ name: "bo", age: 30 }]
    ps.sort_by(|p| p.age)
    // Stable: `cy` was written before `bo` and both are 30, so it stays first.
    for p in ps {
        print("{p.name} {p.age}")
    }
    // Any i64/f64/str key works, including a computed one.
    var ns = [3, 1, 2]
    ns.sort_by(|n| 0 - n)
    print("{ns}")
    var ws = ["ccc", "a", "bb"]
    ws.sort_by(|w| w.len())
    print("{ws}")
    // A key extracted from a named function works the same as a closure.
    var ps2 = [P{ name: "z", age: 1 }, P{ name: "y", age: 0 }]
    ps2.sort_by(age_of)
    for p in ps2 {
        print("{p.name}")
    }
    // An empty list and a single element are both no-ops, not errors.
    var one = [7]
    one.sort_by(|n| n)
    var none: [i64] = []
    none.sort_by(|n| n)
    print("{one} {none}")
}

fn age_of(p: P) -> i64 {
    p.age
}
```

## `L-LIB-SORT-NAN.gr`

```grasm
//! tag: L-LIB-SORT-NAN
//! expect: output
//! [1.0, 2.0, 3.0, 4.0, 5.0, NaN]
//! [-inf, -1.0, 0.0, 1.0, inf, NaN]
//! [(1.0, a), (2.0, b), (NaN, c)]
// A NaN in a list of floats used to leave the whole list unsorted, not just
// the NaN misplaced: calling every NaN comparison "equal" is not an order,
// and a sort given a non-order returns whatever it happens to return. One
// `nan` cell in an otherwise clean file was enough to scramble a column.
fn nan() -> f64 {
    "nan".parse_float() ?? 0.0
}

fn main() {
    var xs = [5.0, 4.0, 3.0, nan(), 2.0, 1.0]
    xs.sort()
    print("{xs}")
    // NaN sorts last; the infinities sit at the ends where they belong.
    var ys = [1.0, nan(), 0.0 - 1.0, 1.0 / 0.0, 0.0, (0.0 - 1.0) / 0.0]
    ys.sort()
    print("{ys}")
    // And the same order is what a float sort key uses.
    var ps = [(nan(), "c"), (2.0, "b"), (1.0, "a")]
    ps.sort_by(|p| p.0)
    print("{ps}")
}
```

## `L-LIB-STR.gr`

```grasm
//! tag: L-LIB-STR
//! expect: output
//! 11 false
//! ell abc 2
//! true true true 6
//! HeLLo,WorLd x HELLO,WORLD hello,world abab
//! chars 1 3 3 | bytes 4 4 3
//! 007 7.. 2 5 2.5
fn main() {
    let s = "Hello,World"
    print("{s.len()} {s.is_empty()}")
    // substr is (start, length) — not (start, end).
    print("{s.substr(1, 3)} {"abcdef".substr(0, 3)} {s.split(",").len()}")
    print("{s.contains("World")} {s.starts_with("He")} {s.ends_with("ld")} {s.index_of("W") ?? 0 - 1}")
    print("{s.replace("l", "L")} {"  x ".trim()} {s.to_upper()} {s.to_lower()} {"ab".repeat(2)}")
    // Characters and bytes are the same number only for ASCII, and a length
    // that goes on the wire has to be the second one.
    print("chars {"🙂".len()} {"añb".len()} {"abc".len()} | bytes {"🙂".byte_len()} {"añb".byte_len()} {"abc".byte_len()}")
    print("{"7".pad_start(3, "0")} {"7".pad_end(3, ".")} {"a\nb".lines().len()} {"5".parse_int() ?? 0} {"2.5".parse_float() ?? 0.0}")
}
```

## `L-LIB-SUM.gr`

```grasm
//! tag: L-LIB-SUM
//! expect: output
//! 6 0
//! 6.0 3.0
//! 0
//! 0.0 0.0000
//! 0.0
// The sum of an empty list is the zero of the element type.
//
// The last two lines were the deviation this file used to work around. The
// interpreter read int-versus-float off the elements, and an empty list has
// none, so `[f64].sum()` was `0`: it printed without a point and then panicked
// `no method to_fixed on 0` when the caller used the `f64` the checker had
// promised. This program asserted only what both engines agreed on, so it
// passed throughout. The deviation was recorded on the grounds that fixing it
// needed the interpreter to carry element types; it needed the checker to
// mention what it already knew.
fn total(xs: [f64]) -> f64 {
    xs.sum()
}

fn main() {
    // Integers, empty and not.
    let some: [i64] = [1, 2, 3]
    let none: [i64] = []
    print("{some.sum()} {none.sum()}")

    // Floats with elements are unambiguous in both engines.
    let fs: [f64] = [1.0, 2.0, 3.0]
    print("{fs.sum()} {fs.filter(|x| x < 3.5).sum() - 3.0}")

    // A filter that removes everything still has the element type.
    let gone = some.filter(|x| x > 99)
    print("{gone.sum()}")

    // An empty float list, directly and through a function that declares the
    // return type — the shape that used to panic.
    let empty: [f64] = []
    print("{empty.sum()} {total(empty).to_fixed(4)}")
    // And an emptied one, where the element type comes from the source list.
    print("{fs.filter(|x| x > 99.0).sum()}")
}
```

## `L-LIB-TO-STR.gr`

```grasm
//! tag: L-LIB-TO-STR
//! expect: output
//! 1.0-2.5-0.0-1000000000000000.0
//! 0.0 3.0 0.0
//! 7 -12
// `to_str` gives exactly what `print` gives.
//
// The interpreter handed the job to its host language, which drops the point
// on a whole float. So `(1.0).to_str()` was `"1"` under `grasm run` and `"1.0"`
// compiled, and the documented idiom below printed a `[f64]` as a list of
// integers — the one thing [L-DISPLAY] exists to prevent, in the engine this
// specification calls normative.
fn main() {
    let xs: [f64] = [1.0, 2.5, 0.0, 1.0e15]
    print("{xs.map(|v| v.to_str()).join("-")}")
    // Through a conversion, and through a sum with nothing to add.
    let empty: [f64] = []
    print("{(0 - 0.0).to_str()} {(3).as_f64().to_str()} {empty.sum().to_str()}")
    // An integer is still an integer.
    print("{(7).to_str()} {(0 - 12).to_str()}")
}
```

## `L-LIB-TRIM-SIDES.gr`

```grasm
//! tag: L-LIB-TRIM-SIDES
//! expect: output
//! [  hi] [hi  ] [hi]
//! [    let x = 1]
// Trimming one side. `trim` alone loses the distinction, and a line's leading
// indentation is often exactly what you are trying to keep.
fn main() {
    let s = "  hi  "
    print("[{s.trim_end()}] [{s.trim_start()}] [{s.trim()}]")
    // The case it exists for: dropping the trailing newline and spaces a file
    // leaves on a line without flattening the indentation that carries meaning.
    let line = "    let x = 1   "
    print("[{line.trim_end()}]")
}
```

## `L-LIB-TUPLE.gr`

```grasm
//! tag: L-LIB-TUPLE
//! expect: output
//! 1 2 3 4
//! 1.5 1000.0 10
// Tuple components, including nested ones.
//
// A number directly after `.` is a component, not the start of a float:
// `t.1.1.0` used to lex as `t` `.` `1.1` `.` `0` and report "expected an
// identifier but found float `1.1`", so a nested component could only be
// written with brackets. Ordinary floats and ranges are unaffected.
fn main() {
    let t = (1, (2, (3, 4)))
    print("{t.0} {t.1.0} {t.1.1.0} {t.1.1.1}")
    let f = 1.5
    let g = 1.0e3
    var n = 0
    for i in 1..5 {
        n = n + i
    }
    print("{f} {g} {n}")
}
```

## `L-LIB-UDP.gr`

```grasm
//! tag: L-LIB-UDP
//! expect: output
//! bound two sockets
//! sent 5 bytes
//! ready: 1 of 1
//! got 5 bytes: 1 2 255 0 65
//! a stream operation on a datagram socket is an Err
//! a datagram operation on a listener is an Err
//! nothing more arrived
// Datagrams: whole messages, to an address named at send time, carrying bytes.
//
// Both sockets ask for port 0 and read the choice back with `net_port`, so the
// program runs on a machine where the ports it would otherwise have picked are
// already held.
fn spell(b: bytes) -> str {
    var out = ""
    for i in 0..b.len() {
        out = out + "{b.at(i)} "
    }
    out.trim()
}

fn main() / {io, net} {
    match udp_bind(0) {
        Err(e) -> print("could not bind")
        Ok(server) -> {
            match udp_bind(0) {
                Err(e) -> print("could not bind a second time")
                Ok(client) -> {
                    print("bound two sockets")
                    let port = net_port(server) ?? 0

                    // A packed binary payload, which is the reason the payload
                    // type is `bytes` and not `str`: 255 and 0 are not text and
                    // a lossy UTF-8 conversion would not return them.
                    let msg = bytes_of([1, 2, 255, 0, 65])
                    match udp_send(client, "127.0.0.1", port, msg) {
                        Err(e) -> print("could not send")
                        Ok(n) -> print("sent {n} bytes")
                    }

                    // Readiness works on a datagram handle exactly as it does
                    // on a stream one, because they share the table.
                    let ready = net_wait([server], 1000) ?? []
                    print("ready: {ready.len()} of 1")

                    match udp_recv(server) {
                        Err(e) -> print("could not receive")
                        Ok(got) -> print("got {got.len()} bytes: {spell(got)}")
                    }

                    // The kinds do not silently substitute for one another.
                    match tcp_read(server) {
                        Ok(_) -> print("a stream read on a datagram socket answered, which it must not")
                        Err(e) -> print("a stream operation on a datagram socket is an Err")
                    }
                    match tcp_listen(0) {
                        Err(e) -> print("could not listen")
                        Ok(listener) -> {
                            match udp_recv(listener) {
                                Ok(_) -> print("a datagram read on a listener answered, which it must not")
                                Err(e) -> print("a datagram operation on a listener is an Err")
                            }
                            net_close(listener)
                        }
                    }

                    // One datagram was sent and one was received, so nothing is
                    // left: a second `net_wait` finds the socket quiet.
                    let again = net_wait([server], 100) ?? []
                    if again.len() == 0 {
                        print("nothing more arrived")
                    } else {
                        print("something arrived that was never sent")
                    }
                    net_close(client)
                }
            }
            net_close(server)
        }
    }
}
```

## `L-LIB-WORDS.gr`

```grasm
//! tag: L-LIB-WORDS
//! expect: output
//! [2026-08-09T10:00:00Z, INFO, server, started]
//! 10 4
//! [] [] [a, b, c]
//! [a, , b] [a, b]
fn main() {
    // A line of aligned columns: what `split(" ")` cannot read.
    let line = "2026-08-09T10:00:00Z INFO     server   started"
    print("{line.words()}")
    print("{line.split(" ").len()} {line.words().len()}")
    // Empty, all-whitespace, and mixed whitespace kinds.
    print("{"".words()} {"   ".words()} {"\ta\nb  c ".words()}")
    // The two are different on purpose: `split` keeps the empty, `words` does
    // not, and a CSV needs the first while a log line needs the second.
    print("{"a,,b".split(",")} {"a  b".words()}")
}
```

## `L-MAXMUNCH.gr`

```grasm
//! tag: L-MAXMUNCH
//! expect: output
//! true
fn f() -> i64 {
    1
}
fn main() {
    // `->` is one token; `<=` is one token.
    print("{f() <= 1}")
}
```

## `L-NEWLINE.gr`

```grasm
//! tag: L-NEWLINE
//! expect: output
//! 6
fn main() {
    let a = 1


    let b = 2
    // Newlines are insignificant inside ( ) and [ ].
    let xs = [
        a,
        b,
        3,
    ]
    print("{xs.sum()}")
}
```

## `L-RAW-STRING.gr`

```grasm
//! tag: L-RAW-STRING
//! expect: output
//! 3
//! [{"users": [{"name": "ada"}], "ok": true}]
//! [a	tab and "quotes" kept]
//! [a \n that is two characters]
//! 4 [97, 92, 110, 98]
//! interpolation still works: 2
//! empty:[] one:[x]
// A raw block: every character literal, newlines included.
let DOC = """
{"users": [{"name": "ada"}], "ok": true}
a	tab and "quotes" kept
a \n that is two characters
"""

let ESCAPES = """a\nb"""
let EMPTY = """"""
let ONE = """x"""

fn main() {
    print("{DOC.lines().len()}")
    for l in DOC.lines() {
        print("[{l}]")
    }
    // `\n` inside a raw block is a backslash and an `n`, not a newline.
    print("{ESCAPES.len()} {ESCAPES.codes()}")
    // Ordinary strings are unaffected.
    print("interpolation still works: {1 + 1}")
    print("empty:[{EMPTY}] one:[{ONE}]")
}
```

## `L-STRING.gr`

```grasm
//! tag: L-STRING
//! expect: error
//! unterminated string
fn main() {
    let s = "no closing quote
}
```

## `L-UFCS.gr`

```grasm
//! tag: L-UFCS
//! expect: output
//! 10
//! 10
//! 7
//! 6
//! 3
// A free function whose first parameter accepts the receiver may be called in
// method position. The two spellings are the same call.
fn double(n: i64) -> i64 {
    n * 2
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn total(xs: [i64]) -> i64 {
    xs.sum()
}

/// A user function named `len` does not shadow the built-in `len` method:
/// built-ins are resolved first.
fn len(s: str) -> i64 {
    999
}

fn main() {
    print("{double(5)}")
    print("{5.double()}")
    print("{3.add(4)}")
    print("{[1, 2, 3].total()}")
    print("{"abc".len()}")
}
```

## `L-WHITESPACE.gr`

```grasm
//! tag: L-WHITESPACE
//! expect: output
//! 3
fn main() {
	let    a	=	1
	let  b  =  2
	print("{a+b}")
}
```
