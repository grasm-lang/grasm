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

## The conversion landed, and the binary did not move (2026-10-11)

`grasm_rt` compiles under `#![no_std]`: 793 errors down to zero, and the whole
workspace builds. A hello-world built with it:

    350.0 KB      against 351.1 before

**No change.** The reason, found by looking rather than guessing: `nm` still
shows eleven `gimli`/`addr2line`/`rust_panic` symbols in the binary.
`grasm_rt` depends on `grasm_journal` and `grasm_fiber`, both of which are
still `std` crates, so `std` is still linked and brings its symboliser with
it. Converting one crate of three cannot drop a dependency all three share.

So the 32.8 KB the probe measured remains the right target and the work so far
remains necessary, but it was not sufficient, and the honest reading is that
the payoff is still unrealised.

What is left is much smaller than what is done: the two crates are 1,144 lines
with 37 `std` uses between them, against `grasm_rt`'s 11,030 and 295. Most are
`cell`, `ptr`, `mem`, `hint`, `any`, `ffi` and `rc`, which move to `core` and
`alloc`. The real work is `process` 6, `fs` 6, `env` 3 and `io` 1 -- all of
which now have a `no_std` implementation in `grasm_rt` to call -- plus
`std::panic` 5, which is the one thing with no `no_std` equivalent and needs
reading before it is replaced.

## 68.3 KB, measured (2026-10-11)

All four crates a compiled program links are `no_std` -- `grasm_sys`,
`grasm_journal`, `grasm_fiber`, `grasm_rt` -- and the archive they link
through, `grasm_rt_abi`, supplies the allocator and the panic handler.

| | |
|---|---:|
| hello-world, before | 351.1 KB |
| hello-world, `no_std` | **68.3 KB** |
| G5's target | 300 KB |

`nm` on the archive: 315 `grasm_*` entry points present, **zero**
`gimli`/`addr2line`/`rust_panic` symbols. The archive itself went from 21 MB
to 4.7 MB. The probe predicted "under 100 KB"; the measurement is 68.3.

**Checked so far:** all 17 `examples/` programs give byte-identical output
compiled and interpreted (`parallel_map` needs 2m18s interpreted, so a short
timeout makes it look like a disagreement). difftest over the full corpus has
not completed yet.

**What this does not yet say.** 17 programs ran correctly. Roughly 2,100 lines
of new syscall code -- `fs`, `net`, `process`, `thread`, `io`, `tls`, `sync`
-- have not been through the gates. Every hazard that code was written around
is one that fails under load or on large input rather than at compile time, so
the number above is a measurement and not yet a result.

### The four things that only appeared at the end

- **A `#[panic_handler]` cannot live in `grasm_rt`**, because `grasm_interp`
  links it as an rlib and `std` already supplies one. Hence `grasm_rt_abi`: a
  staticlib-only crate, never linked into anything with `std`, holding the two
  items a program must have exactly one of. `grasm_rt` is an rlib now and
  `find_runtime` looks for `libgrasm_rt_abi.a` first, still accepting the old
  name.
- **`panic = "abort"` is now required rather than chosen.** The note in
  `Cargo.toml` recording that it saved only 1 KB was measured while `std` was
  still linked; a `no_std` crate has no unwinder and the compiler refuses to
  build one that could unwind.
- **`core` has no floating-point maths.** `sqrt`, `powf`, `floor` and 19 more
  are libm calls that only `std` declares. `grasm_sys::math` declares them and
  puts them on `f64` as a trait, so no call site changed. libm's `round` is
  halves-away-from-zero, which is what `std` did and what [L-LIB-ROUND]
  requires -- a different rule there would move every money column in the
  corpus.
- **`rust_eh_personality` has to exist.** `core` ships precompiled with
  `panic = "unwind"`, so a few of its paths still name the landing-pad
  personality routine even in a build that cannot unwind -- which is what the
  compiler's "rebuilding your crate with panic=abort may not be enough" means.
  A no-op suffices, since no landing pad is ever entered. **Found by a
  compiled program failing to link, not by a warning**: hello-world does not
  reach those paths and linked fine, so 16 of the 17 examples failed to build
  while the one measurement taken looked perfect. A single-program check was
  not enough to notice.
- **The allocator must honour over-aligned requests.** `malloc` promises 16
  bytes; anything stricter goes through `posix_memalign`, and a `realloc` of
  such a block copies by hand because `realloc` cannot promise the alignment.

## The Linux half is compiled nowhere local (2026-10-11)

Every `#[cfg(not(target_os = "macos"))]` branch in `grasm_sys` -- about 2,100
lines of syscall code -- has never been compiled on this machine: only
`aarch64-apple-darwin` and `x86_64-apple-darwin` are installed. The x86_64
cross build does work (`libgrasm_rt_abi.a`, 4.8 MB, `lipo` confirms the
architecture), so the two Apple targets are covered and CI's ubuntu runner
will catch anything that does not *compile* on Linux.

What CI will not catch is a constant with the wrong **value**, which compiles
perfectly. So all seventeen platform-dependent pairs were read against the
headers rather than trusted. One was wrong:

**`AF_INET6` was 30 with no `cfg`** -- macOS's value, where Linux is 10. The
failure mode is the quiet kind: an IPv6 peer's family would not match, so
`Addr::parts` would read the address as IPv4, taking four bytes from offset 4
instead of sixteen from offset 8 and handing `inet_ntop` the wrong family.
`peer_addr` would return wrong text and report success.

The other sixteen are correct: `pthread_key_t` (u64/u32), `pthread_t`
(pointer/usize), `EAGAIN` (35/11), `EILSEQ` (92/84), `SOL_SOCKET`
(0xffff/1), `SO_REUSEADDR` (4/2), `O_CREAT` (512/64), `O_TRUNC` (1024/512),
`O_APPEND` (8/1024), `O_NONBLOCK` (4/2048), `CLOCK_MONOTONIC` (6/1),
`_SC_NPROCESSORS_ONLN` (58/84), and the `Dirent` and `addrinfo` layouts.
`EINTR`, `ENOENT`, `EEXIST`, `AF_INET`, `IPPROTO_TCP`, `TCP_NODELAY`,
`CLOCK_REALTIME`, `POLLIN` and `SIGKILL` are the same on both.

## Two defects difftest found that reading the code had not (2026-10-11)

### `open` is variadic, and arm64 passes variadic arguments differently

`int open(const char *, int, ...)` takes its mode as a **variadic** argument.
arm64 Apple's ABI passes variadic arguments on the stack where a fixed
parameter goes in a register. Declared as `fn open(path, flags, mode)`, the
mode went into x2 and `open` read whatever happened to be on the stack.

The symptom was not a crash. A created file got a garbage mode, so the *first*
`append_bytes` to a path succeeded and the *second* failed with "Permission
denied" -- and `file_size` then reported 5 where the interpreter said 10. Three
of `apps/broker`'s assertions failed while thirty-two other programs passed.

`fcntl` has the same shape and had the same defect, in two places. Nothing
had failed visibly there yet: a wrong `F_SETFL` argument makes
`set_nonblocking` quietly do something else.

Both are now declared with `...`, which is what they are.

### `readdir$INODE64` does not exist on arm64

The suffixed symbol is an x86_64-only compatibility alias. On arm64 plain
`readdir` already returns a 64-bit inode and no alias exists, so asking for
one fails at **link** time -- `apps/broker`, `apps/grlint` and `apps/logpipe`
all stopped compiling natively. The cfg was per-OS where it had to be
per-architecture.

Both found by `difftest`, neither by reading the code, and neither by
compiling it. After them: **57 programs identical across all four engines, 0
mismatched, 0 unsupported.**
