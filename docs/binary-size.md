# Where the 351 KB goes

`MASTERPLAN.md` §2.3 **G5** asks for a hello-world static binary of **≤300 KB**
with no runtime dependencies. It is 351 KB, and the gap has been recorded as
"binary size 350 KB against a 300 KB target" without anyone asking what is
actually in it. Measured 2026-10-07.

    fn main() { print("hello, world") }   ->   359,536 bytes

## By segment

| | bytes |
|---|---:|
| `__text` | 229,948 |
| `__eh_frame` | 31,084 |
| `__const` | 12,408 |
| `__cstring` | 6,046 |
| `__unwind_info` | 5,720 |
| `__gcc_except_tab` | 5,436 |

## By origin, which is the part that matters

Summing every text symbol by the crate it came from:

| | bytes | share |
|---|---:|---:|
| **gimli + addr2line** (DWARF reader) | **113,008** | **48%** |
| std, other | 36,032 | 15% |
| core | 26,184 | 11% |
| **rustc_demangle** | **21,156** | **9%** |
| **grasm_rt — ours** | **16,496** | **7%** |
| other | 9,528 | 4% |
| panic / unwind | 4,744 | 2% |
| std backtrace | 3,992 | 1% |
| alloc | 2,360 | 1% |

**135 KB — 59% of the code — is Rust's DWARF backtrace symboliser.** Grasm's
own runtime is 16.5 KB, 7%.

That reframes the problem. `Cargo.toml` records two experiments against the
*unwind tables*: `panic = "abort"` saved 1 KB, and `opt-level = "z"` on
`grasm_rt` made the binary 4 KB **bigger**. Both were correct and both were
aimed at the 39 KB, not the 135.

## Why it is linked

Not by us. `rt_panic` writes its message and calls `std::process::exit(101)`;
the stack trace a Grasm panic prints is Grasm's own frames, and neither
`grasm_rt` nor `grasm_native` mentions `std::backtrace` anywhere. The
symboliser arrives through std's *default panic hook*, which any `unwrap` or
slice index in Rust code can reach, and `-Wl,-dead_strip` cannot remove it
because it is genuinely reachable.

## Measured: `no_std` removes it, on stable

Before committing to the work, the hypothesis was tested. `scratchpad/nostd`
is a `no_std` staticlib with the shape `grasm_rt` has -- a global allocator
over `malloc`, a reference-counted box with an `AtomicI64`, and a
`#[panic_handler]` that writes to fd 2 and exits 101 -- linked into a C `main`
with the same `-Wl,-dead_strip` the real build uses.

    linked binary: 32.8 KB      (the real hello-world: 351.1 KB)

So the 135 KB symboliser, the 39 KB of unwind tables and the rest of std's
baggage all go, and they go **without a nightly toolchain**. The floor for a
Rust runtime that does not link std is about 33 KB; `grasm_rt`'s own logic
measured 16.5 KB in the current binary, so a `no_std` Grasm hello-world should
land somewhere under 100 KB -- not merely inside G5's 300 KB but a long way
inside it.

## What would remove it

Two paths, and the measurement above picks the second.

`-Z build-std` with `panic_immediate_abort` needs a nightly toolchain and
`rust-src`, and would make nightly a build requirement. It gets to about
216 KB.

**A `no_std` runtime** keeps the toolchain stable and gets further -- under
100 KB on the probe's evidence. The cost is the work: of `grasm_rt`'s 11,030
lines, roughly 210 uses of `std::ptr`, `slice`, `mem`, `cmp`, `cell`, `str`
and `ops` are renames to `core`, and about 95 uses of `std::io`, `fs`, `net`,
`thread`, `process` and `time` need direct syscalls. The runtime already does
exactly that in `sys_poll` and `sys_kqueue`, which declare `poll`, `kqueue`
and `mmap` as raw `extern "C"`, so the pattern is established rather than
invented. What it also needs is a `Mutex`, and a `#[panic_handler]` in place
of `rt_panic`'s current `std::process::exit`.

That is a real project touching the code under every Grasm program, and it is
gated by 511 differential programs, which is the right way to attempt it.

## What is not worth trying

Anything aimed at the 16.5 KB that is ours. It is 7% of the binary, and the
target is 51 KB away.
