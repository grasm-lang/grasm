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

## The whole conformance suite, compared (2026-10-11)

All 257 programs in `spec/conformance`, interpreted against a native build:

| | |
|---|---:|
| agree | **197** |
| disagree | **0** |
| negative tests, failing in both engines by design | 60 |
| failing natively but not interpreted | **0** |

The native stack trace is excluded from the comparison, because
`docs/northstar.md` records that a `--native` build prints none at all. That
difference accounts for seven programs -- `E-DIV-ZERO`, `E-POW-NEG`,
`E-INDEX-BOUNDS`, `E-PANIC-TRACE`, `E-TASK-PANIC`, `E-CHANNEL`, `E-DEADLOCK`
-- and nothing else differs in any of them once it is set aside.

The 60 that the native backend declines were each re-checked rather than
assumed: every one of them also fails under `grasm run`, so they are the
suite's grammar, effect and handler error tests and not a native gap. Zero
programs compile interpreted and fail natively, which is the number that
would have hidden a regression as a capability limit -- and did, earlier
today, when `readdir$INODE64` took three apps out and difftest called them
"unsupported".

Two defects came out of this comparison and are fixed: `RwLock::write`
returning a mutex guard, and exec failure being indistinguishable from exit
127. Both are in the commit log with their own regression tests.

## The whole corpus (2026-10-11)

| set | programs | agree | disagree | not runnable | fail natively only |
|---|---:|---:|---:|---:|---:|
| `spec/conformance` | 257 | 197 | 0 | 60 | **0** |
| `packages` | 838 | 257 | 0 | 581 | **0** |
| `apps`, `projects`, `benchmarks/grasm`, `examples` (difftest, four engines) | 57 | 57 | 0 | 0 | **0** |
| | **1,152** | **511** | **0** | 641 | **0** |

511 is the number of runnable programs the corpus is documented to have, which
is the check that this is the whole of it rather than most of it.

Every one of the 641 that the native backend declines was re-run under
`grasm run` and fails there too -- a library without a `main`, or one of the
suite's deliberate error tests. Zero programs compile interpreted and fail
natively. That last column is the one that matters: it is where a regression
hides as a capability limit, and it did exactly that this morning, when
`readdir$INODE64` took three apps out and difftest called them "unsupported".

### Four apparent disagreements that were not

`accounts-demo`, `crypto_hash-demo`, `observe-demo`, `password-demo`,
`postgres-demo` and `examples/parallel_map` each looked like a mismatch and
were the interpreter running out of time. `crypto_hash-demo` takes **8m15s**
interpreted against well under a second compiled; `accounts-demo` 81s;
`parallel_map` 2m18s. A 90-second cap is not generous for a corpus with
crypto in it, and a timeout presents as missing output -- which looks exactly
like a backend that stopped early.

`examples/concurrency` was a seventh, and a different cause: `exit 0 vs 137`,
SIGKILL with empty output, at load average 23 with a `cargo-fuzz` run holding
the machine. Three runs alone, all identical. Both failure modes are recorded
in memory for the next person; neither was a defect.

## The conversion cost 31% of `tree`, and the cause was one instruction (2026-10-11)

G5 was met; the question was what it cost G2. Against master, A/B with
alternating runs and processor time rather than wall:

| | baseline | candidate | |
|---|---:|---:|---|
| `tree`, before the fix | 18.1 ms | **23.9 ms** | 1.31x slower |
| `tree`, after | 18.1 ms | 18.0 ms | parity |

Confirmed three ways before being believed: `abbench` three times running,
an independent `getrusage` harness (18.15 ms best against 23.99), and the
same ratio at 60x the workload.

### What it was not

`GRASM_ALLOC_STATS` gave **196,820 allocations on both sides** -- identical
work, so the cost was per operation rather than extra operations.

Thread-local storage was the obvious candidate: `docs/mining.md` records
`tree`'s release walk making two reaches for thread state per node, and
`_tlv_get_addr` at 48 samples. Priced by removing -- a single-threaded
pointer cache, bypassing `pthread_getspecific` entirely -- it moved 1.31x to
**1.26x**. A sixth of the problem. The obvious candidate was mostly wrong,
which is what [[measure-ceiling-before-building]] says to expect.

### What it was

`sample` on a 60x run, then `otool` on the hottest address:

    0000000100007460    casalb  w9, w8, [x10]        279 samples of ~1,600

`Mutex::raw`, `RwLock::raw` and `OnceLock::get_or_init` each ran a
**compare-exchange** to check their own three-state initialiser -- on every
call, for the life of the process. On arm64 that is `casalb`, a
read-modify-write with full barrier semantics, where the question ("is this
made yet?") is answered by a load.

They now load first and branch, with the compare-exchange moved to an
`#[inline(never)]` cold path that runs once. `tree`'s hottest instruction
was a lock asking whether it had been created, several hundred thousand
times.

After: `tree` 1.01x, `wordfreq` 1.07x, `sort` 1.02x, `sieve` 1.02x, `fib`
1.03x, `matmul` 1.01x -- all nominally faster, all within the +/-10% this
machine swings, and none slower.

### Re-verified after the lock change

The CAS-to-load fix is in lock initialisation, on the path every boxed value
takes, so the corpus was compared again rather than assumed: `spec/conformance`
197 agree / 0 differ / 60 declined, `packages` 257 / 0 / 581, `examples` 24 of
24 identical across all four engines under difftest. The same numbers as
before the fix.

### G3 could not be measured

Compile speed was the next metric to check, since `panic = "abort"` now
applies to the compiler too. It is not measurable on this machine at present:
two Android NDK `clang++` processes at 99% each, plus a macOS storage scan,
put the load average at 40. The run that was attempted returned 25,061 lines
"taking" 824 ms against 175,056 lines in 742 ms, which is not a slow result
but a meaningless one, and it was discarded rather than recorded.

The generator for those files is worth keeping either way: structs, generics,
interpolation and cross-file calls at 5k, 25k, 75k and 175k lines, all four
checking clean. `as f64` is not Grasm's cast -- `.as_f64()` is -- and the
first version measured 1,430 parse errors instead of a typecheck.

## 85.7 KB, not 68.3, and the reason is a page boundary (2026-10-12)

The 68.3 KB above was measured at `0ed1d88` and is no longer the number. A
hello-world built from the branch head is **85.7 KB**. Bisected:

| commit | file | `__text` |
|---|---:|---:|
| `bd50c5a` | 69,896 | 27,808 |
| `6ddcb5a` (the CAS-to-load fix) | 87,720 | 29,712 |
| branch head | 87,808 | 29,648 |

**The code grew 1,904 bytes. The file grew 17,824.** `__TEXT`'s sections now
total 32,504 bytes, which with the Mach-O header and load commands is just
past 32,768 -- so the segment takes three 16 KB pages where it took two. The
whole 17.8 KB is page quantisation around a few hundred bytes.

Two things follow. The file size is **not** a linear measure of code at this
scale: a 500-byte change can move it 16 KB either way, so a reading taken
once is a reading of which side of a cliff the build happens to be on.
And G5 is still met with room -- 85.7 KB against 300 KB, 3.5x inside -- so
trimming the few hundred bytes to fall back under the boundary is cosmetic
and was not done.

What was tested and did **not** explain it: `#[inline(always)]` on the lock
fast paths, relaxed to `#[inline]`, left the file byte-identical and `tree`
still at parity. The 31% that fix bought costs 1.9 KB of code and nothing in
speed to make it polite.

### A smaller structure, found while looking for the 16 KB

`grasm_rt`'s scheduler tracked which waiters were really queued in a
`BTreeSet<(block, slot)>`, which pulled 29 BTree symbols out of `alloc`. It
is now one `Vec<bool>` per block: slots are dense and blocks are few, so
membership is an index rather than a tree walk -- **O(1) rather than
O(log n)**, and no BTree code at all. It was not the 17.8 KB (that was the
page), but it is the better structure on both counts, and 1,000,000 tasks
still answer correctly.

## The gates that can run on this machine (2026-10-12)

| gate | scope | result |
|---|---|---|
| `leakcheck` | 226 programs | **0 leaks** |
| `usecheck` (the `poison` build) | 197 programs | **0 differences** |
| `crosstest` x86_64 | 224 programs | **195 pass, 0 real failures, 60 declined** |
| `clippy --all-targets` | native and wasm32 | 0 |
| unit tests | every crate | 609 pass |
| `wasmtest` | — | pass |
| difftest / corpus comparison | 511 programs | 0 differences |

`leakcheck` and `usecheck` are the two that matter most for a rewritten
allocator, and no other check can replace either: a leaking program produces
exactly the right output, and a premature free makes a value *less* live, not
more. The `poison` feature also still compiles under `no_std`, which had not
been checked.

`crosstest` is more than a second compile. It builds for
`x86_64-apple-darwin`, links with `cc -arch x86_64` and **runs the result
under Rosetta**, so it exercises paths the arm64 build never reaches --
`readdir$INODE64` exists only there, and variadic arguments go in registers
on x86_64 SysV where arm64 puts them on the stack. The `open`/`fcntl` fix
earlier today was for an arm64-only defect; this confirms it did not break
the architecture that was already working.

**Two reported failures that are not.** `E-NOT-PARALLEL` and
`E-RECURSION-PARALLEL` differed during a run taken at load 38 rising to 134.
Both agree on re-run -- 3 of 3 and 2 of 2 -- and both are *parallel* tests,
which is what starves first on a saturated machine. Recorded as load
artefacts rather than quietly dropped, and the whole crosstest figure should
be taken again on a quiet machine before anyone calls it green.

Still unrun: `replaytest`, the fuzzers, and anything on Linux -- no Linux
target is installed here, so roughly 2,100 lines of `cfg(not(macos))` have
never been compiled, let alone run.

## An audit of the syscall code, since the machine was too busy to measure on

Three defect classes, each checked against every call rather than spot-read.

**Variadic declarations.** Of the 40-odd libc functions `grasm_sys` declares,
exactly two are variadic in C -- `open` and `fcntl` -- and both now carry
`...`. Nothing else in the list (`ioctl`, `openat`, the `printf` family) is
declared at all, so the class that cost a morning is closed rather than
patched.

**`EINTR`.** Nine wrappers retry. Two did not and should have:

- `File::sync` is `fsync`, which a signal can interrupt. Reporting a failure
  there tells a caller its data is not on the disk when it is about to be,
  which is the opposite of what the call is for.
- `Socket::send_to` is a single `sendto`. A signal arriving before the
  datagram left is not a failure to send it.

Both now loop, like every other call here. `close` is deliberately *not*
retried: POSIX leaves the descriptor's state unspecified after an interrupted
`close`, so retrying may close something else.

**A reported length that may exceed its buffer.** `recvfrom`,
`getsockname` and `getpeername` write back the address's *actual* size, not
how much they stored, and POSIX allows that to be larger than the buffer
given -- the address is truncated and the length still says what it would
have needed. Both call sites sliced to the reported length, which **panics**
on any address family wider than the 128 bytes reserved. Both now take the
smaller of the two. Nothing in the corpus reaches it, because nothing uses an
address family that large; a program that did would have aborted on a
perfectly ordinary call.

**Descriptor leaks on error paths.** None. `File` and `Socket` are RAII types
whose `Drop` closes, so `listen_on`'s `?` on `set_opt`, `bind` or `listen`
closes the socket on the way out; `process::run` shuts both pipe ends on every
error path; `read_dir` closes the directory on all of them.
