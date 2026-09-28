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
    let ch = channel()
    concurrent {
        go {
            for i in 0..3 {
                ch.send(i * i)
            }
            ch.close()
        }
        go {
            var total = 0
            while true {
                match ch.recv() {
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
- `channel()` makes a `Chan[T]`, `T` inferred from use. `ch.send(v)` hands a
  value over; `ch.recv()` is `Some(v)`, or `None` once the channel is closed and
  drained; `ch.close()` says no more will be sent. `channel(n)` buffers `n`
  values; `channel()` is a rendezvous — a send waits for its receiver.
- The operations perform `async` -- the effect `parallel` and `spawn` already
  perform ([T-EFFECT]) -- so a function that can block says so in its row, and
  one declared `/ {}` cannot. Every rule that rests on rows
  — `par_map`'s purity, a package's capability ceiling — keeps working.

## What it means

**Tasks switch only at a channel operation.** Between two operations a task
runs alone, so everything it does in between is atomic with respect to the
others. That is cooperative scheduling, and it is what makes the rest of this
tractable: the only choices the scheduler ever makes are at `send`, `recv` and
`close`, and the end of a task.

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
`grasm_task_go` hands it to the runtime scheduler with its captured
environment. A channel operation is a runtime call that may switch fibers. The
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

1. `grasm_fiber`: stacks and the context switch, with unit tests that switch
   thousands of fibers and check every register survives.
2. Syntax and checking: `concurrent`, `go`, `channel`, the `async` effect, the
   no-outer-writes rule. Spec clauses, terse — the specification is at its page
   budget, so history moves out first.
3. Interpreter: scheduler, channels, deadlock report. Conformance programs.
4. Native: lifted task bodies, runtime scheduler and channels, stack floors.
   Both engines held to the same output.
5. `grasm test --schedules N`.
6. Measurement: tasks per second and memory per task, against the MASTERPLAN
   figure of 1M lightweight tasks on a node.

## Not in this design

Actors (planned on top of channels), `select` over several channels, timeouts,
and I/O that parks a fiber instead of blocking the thread — `net_wait` already
covers concurrent sockets on one thread, and bringing it under the scheduler is
its own step.
