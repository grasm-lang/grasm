# Concurrent tasks and channels

G6 step 2 (MASTERPLAN §8.6). Decided 2026-09-28: stackful green threads with a
seeded, replayable scheduler. This is the design, written before the code so
the code can be held to it.

Step 1 is already in: a `parallel` block of independent tasks runs them on
several cores where no program could tell ([E-NOT-PARALLEL]). That is
parallelism, not concurrency — tasks there cannot talk to each other. This is
the part where they can.

## What a program looks like

```grasm
fn main() / {io, async} {
    let ch = chan_new()
    concurrent {
        go {
            for i in 0..3 {
                chan_send(ch, i * i)
            }
            chan_close(ch)
        }
        go {
            var total = 0
            while true {
                match chan_recv(ch) {
                    Some(v) -> total += v
                    None -> break
                }
            }
            print("{total}")
        }
    }
}
```

- `concurrent { … }` is a scope. It does not end until every task started in
  it has ended — structured, as `parallel` is: no task outlives the block that
  started it.
- `go { … }` starts a task. Its body sees the enclosing scope **by value**, as
  a closure does; it may not assign to a binding outside itself. Tasks share
  nothing mutable: they communicate by channel, which is what makes them free
  of data races by construction.
- `chan_new()` makes a `Chan[T]`, `T` inferred from use. `chan_send(ch, v)`
  hands a value over; `chan_recv(ch)` is `Some(v)`, or `None` once the channel
  is closed and drained; `chan_close(ch)` says no more will be sent.
  `chan_new(n)` buffers `n` values; `chan_new()` is a rendezvous — a send waits
  for its receiver.
- The operations are built-in functions, named like `tcp_read` and its
  family, rather than methods. Effects are inferred before types are known, so
  a method `send` could not be told from a user's own `send` on some other type
  -- and `close`, `send` and `recv` are already the names of user functions
  across the corpus. So did `channel`: the first version of this design
  named the constructor that, and the `wav` and `raytrace` packages stopped
  compiling. Every one of the four is prefixed now. A prefixed built-in is in the one table that says what
  each built-in performs, so its row is right everywhere at once.
- They perform `async` -- the effect `parallel` and `spawn` already perform
  ([T-EFFECT]) -- so a function that can block says so in its row, and one
  declared `/ {}` cannot. Every rule that rests on rows — `par_map`'s purity, a
  package's capability ceiling — keeps working.

## What it means

**Tasks switch only at a channel operation.** Between two operations a task
runs alone, so everything it does in between is atomic with respect to the
others. That is cooperative scheduling, and it is what makes the rest of this
tractable: the only choices the scheduler ever makes are at `chan_send`,
`chan_recv` and `chan_close`, and the end of a task.

**The default schedule is fixed.** Tasks start in the order their `go`s run; a
task that blocks gives way to the next runnable one, first in first out. So a
program prints the same thing every time, on every engine — which is the
property every gate in this repository is built on.

**`grasm test --schedules N`** runs each test under N other schedules, each
chosen by a seed that is printed with any failure, so a failure found once can
be replayed exactly. This is MASTERPLAN's deterministic scheduler mode: a race
between tasks is a test failure with a seed, not a heisenbug.

**When every task is waiting, the program panics**: `deadlock: every task is
waiting`, naming what each waits on. A deadlock is a bug with a deterministic
schedule that reproduces it, so it is reported rather than left to hang.

**A panic in a task ends the program**, as it does in a `parallel` block
([E-TASK-PANIC]).

## How the engines run it

**A shared fiber crate.** `grasm_fiber` owns stacks (`mmap`, with a guard page)
and a context switch written in `asm!` for aarch64 and x86_64: save the
callee-saved registers and the stack pointer, load another's. Both engines use
it, so they cannot disagree about scheduling.

**Native.** A `go` body is lifted into a function the way a closure is, and
the block hands the list of them to the runtime scheduler (`grasm_conc_run`). A channel operation is a runtime call that may switch fibers. The
stack-depth check each function makes reads a per-thread floor; switching
fibers switches that floor with the stack.

**Interpreter.** Tasks are fibers on the interpreter's own thread, one running
at a time, and **each task is its own interpreter instance** sharing only the
channels: its scopes, handlers and call depth are its own, so nothing a
suspended task holds is touched by another, and no `&mut` interpreter is ever
live on two stacks at once. A task's output goes to a buffer the scheduler
drains into the program's output after each resume, which keeps the order
every other engine prints in. The interpreter's values are `Rc`, which is not
thread-safe — and does not need to be, because nothing leaves the thread. This is why the design
uses fibers rather than one OS thread per task passing a baton: that would have
moved `Rc` values between threads and relied on a promise the type system
cannot check.

**The browser build** has no way to switch stacks in `wasm32-unknown-unknown`,
so a program using `concurrent` is skipped there with that reason, as programs
needing threads, sockets or a filesystem already are.

**Parallelism later.** Fibers first run on one OS thread. Running them across
several — an M:N scheduler with work stealing — changes nothing a program can
observe, since tasks share nothing mutable, and is a later step.

## Stages

1. *Done.* `grasm_fiber`: stacks and the context switch, with unit tests that
   switch thousands of fibers and check every register survives.
2. *Done.* Syntax and checking: `concurrent`, `go`, `channel`, the `async`
   effect, the no-outer-writes rule, and no `ref` from outside a task.
   [E-CONCURRENT], [E-CHANNEL], [E-SCHEDULE], [E-DEADLOCK].
3. *Done.* Interpreter: scheduler, channels, deadlock report, conformance
   programs and two laws.
4. *Done.* Native: a `go` body is a closure of no arguments; the block keeps
   its tasks in a list bound in its own scope, so a block left early releases
   them with everything else, and `grasm_conc_run` runs them on fibers.
   Channels are a table in the runtime, a root like the `ref` slab. Each task
   gets a stack of the same budget as `main`'s thread, and the depth check's
   floor is switched with it -- in both engines, which had called every task's
   first call too deep. Both engines are held to the same output by difftest.
5. *Done.* `grasm test --schedules N`: a unit test that started tasks runs
   again under `N` orders, each round visiting the tasks in an order drawn
   from a seed (SplitMix64, so a seed names the same orders everywhere). A
   failure prints the seed; `--schedule-seed S` runs that one order again. A
   test that started no tasks is not rerun -- there is nothing to vary.
6. *Done.* Measurement, native build, Apple M-series, 2026-09-29, at a load
   average of 20-30 (so wall times are pessimistic; CPU times are the
   steadier figure):

   | tasks | shape | wall | CPU (user + sys) | peak RSS |
   |---|---|---|---|---|
   | 1 000 000 | each sends on a buffered channel and ends | 0.95 s | 0.53 s | 289 MB |
   | 100 000 | the same | 0.09 s | 0.05 s | 33 MB |
   | 100 000 | all waiting at once on a rendezvous | 6.4 s | 2.7 s | 334 MB |
   | 1 000 000 | all waiting at once on a rendezvous | 69 s | 24.5 s | 397 MB |

   So the MASTERPLAN figure holds in both senses: a million short tasks in
   under a second, and a million alive at once on one node. The second is
   bounded by the kernel, not the scheduler -- 22 of its 24.5 CPU seconds are
   `sys`, mapping and first-touching a stack per waiting task. RSS undercounts
   there (macOS compresses idle pages); one touched page per waiting task,
   16 KB on Apple silicon, is the honest floor of a stackful design.

   The first measurement found two defects, both fixed before these numbers:
   the scheduler removed finished tasks from the middle of a list, which is
   quadratic -- 100 000 short tasks took 46 s -- and made every task's fiber up
   front, so each held a stack from the start. Now it is a run queue, a
   fiber is made on a task's first turn, and `grasm_fiber` keeps up to 64
   finished stacks for the next fiber. The interpreter had the same quadratic
   list and now rebuilds it once a round.

## Actors

MASTERPLAN §8.6: "`actor Counter { var n = 0; fn inc(inout self) }` —
exclusive by construction". Done as [E-ACTOR], lowered in the parser the way an
`impl` is: a state struct behind a `ref`, a handle holding it, a constructor,
and the methods as `impl` methods of an interface no program can name, so a
call resolves by the handle's type in both engines unchanged.

Exclusive by construction here means a method cannot wait: it may not perform
`async`, and tasks switch only where something waits, so a method runs to its
end before another task runs and no lock is needed. When tasks run on several
threads, a handle will need one; the language does not change.

Two departures from the MASTERPLAN sketch. A field declares its type
(`var n: i64 = 0`), as a struct's does. And a method call is not charged
`async` -- "messages are effects" -- because effects are inferred before types,
so `c.inc()` cannot be told from any other `inc` there; the property the
effect would have protected, exclusivity, holds without it.

In the interpreter a task is its own interpreter, and a `ref` names a slot in
one slab, so the slab is handed from interpreter to interpreter as they take
turns (`slab_hold` in `crate::tasks`) rather than each having its own.

## Select

`chan_select(chs)` is the index of the first channel in `chs` that a
`chan_recv` would not wait on -- one holding a value, or closed -- and waits
until there is one. It answers with an index rather than a value because
nothing runs between it and the `chan_recv` that follows, so the answer is
still true then, and an integer is all either runtime has to build. The
channels share an element type, being a list; channels of different types
are selected over by sending a sum type on each. Earlier channels win ties,
so the choice is as fixed as the schedule.

## Not in this design

Sending on whichever of several channels is ready, timeouts,
and I/O that parks a fiber instead of blocking the thread — `net_wait` already
covers concurrent sockets on one thread, and bringing it under the scheduler is
its own step.
