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

## What would remove it

`-Z build-std` with `panic_immediate_abort`, which is what the `Cargo.toml`
note already names. That needs a nightly toolchain and the `rust-src`
component; this machine has `stable` and `1.98.0` and no `rust-src`, so it
could not be tried here. On the evidence above it is worth trying: 351 KB
minus 135 would be about **216 KB, comfortably inside G5**, and it would be a
decision about the project's toolchain requirement rather than a code change.

The alternative, a `no_std` runtime, is the other path the note names and is a
far larger piece of work for the same outcome.

## What is not worth trying

Anything aimed at the 16.5 KB that is ours. It is 7% of the binary, and the
target is 51 KB away.
