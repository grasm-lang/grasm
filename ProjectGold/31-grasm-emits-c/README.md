# 31 — A compiler written in Grasm that emits C

**Tests:** whether Grasm can host a compiler back end. **Result:** **yes, via C.**

## Why this project exists

The previous conclusion was that Grasm cannot host its own back end, because it
cannot write the byte `0xC7`:

```
wanted:  48 c7 c0 01 00 00 00   (7 bytes of x86)
on disk: 48 c3 87 c3 80 01 00 00 00   (9 bytes)
```

`str` is UTF-8, so any byte above 127 is re-encoded and the machine code is
corrupt. That is true, and it is only true of **direct machine-code emission**.

Nim, Vala, Haxe, Eiffel and the original Go frontend all take the other route:
emit C, and let a C compiler generate code. Under that model a back end only has
to produce **text** — which is what this language demonstrably does well
(projects 12, 13, 21 and 29 are all text producers).

## What was measured

A small expression language — variables, arithmetic, precedence, `print` — with
a lexer, a recursive-descent parser, an AST over `ref Node`, **and two back
ends**: a direct interpreter, and a C emitter.

```
$ grasm run main.gr        # emits out.c, and says what the answers should be
$ cc -O2 -Wall -Wextra -o out out.c
$ ./out
14
5
71
49
```

* `cc` accepted the generated C with `-Wall -Wextra` and **zero warnings**.
* The resulting **arm64 Mach-O binary**, 33,424 bytes, produced output
  **identical** to the Grasm interpreter's.
* The interpreter is in the same file precisely so the emitter has an oracle —
  the same discipline `scripts/difftest.sh` applies to the three real engines.

Expressions are emitted fully parenthesised, so C's precedence cannot disagree
with the tree the compiler already built. Getting that wrong is the classic
transpiler bug and parentheses cost nothing.

## What it proves, and what it does not

**Proves:** a compiler written in Grasm can produce a working native executable
today, with no byte type, no pointer FFI, and no Rust in the back end.

**Does not prove:** that the *runtime* can be Grasm. `libgrasm_rt.a` allocates
memory, starts threads and calls the OS — none of which Grasm can express. In a
C-backend world that becomes hand-written C rather than Rust, which is what Nim
does (`nimbase.h`).

**And it is not dependency-free.** It trades a dependency on Rust and Cranelift
for a dependency on a C compiler. That is a much better trade — a C compiler
exists on every platform that exists, and it is a standard rather than one
vendor's library — but it is a trade, not an escape.

## Findings

None new. This project *un-blocks* a conclusion rather than producing a finding:
F9 (no byte type) blocks direct code generation and does **not** block
self-hosting, which is the opposite of what was recorded before it was written.
