# G6, measured: the task scheduler is quadratic, and why the obvious fix does not work

`MASTERPLAN.md` §2.3 **G6** asks for **1M+ tasks per node**. It had never been
tested. Measured 2026-10-07.

## What it does today

N tasks each send once on a channel, with one receiver draining it:

| tasks | wall | peak RSS |
|---:|---:|---:|
| 1,000 | 0.06 s | 17 MB |
| 2,000 | 0.25 s | 33 MB |
| 4,000 | 1.20 s | 65 MB |
| 10,000 | 18.2 s | 160 MB |
| 100,000 | did not finish in 180 s | 1,573 MB |

**Memory is linear and fine** -- about 16 KB a task, which is one page.
`TASK_STACK` reserves a stack but the pages are untouched until used, so the
footprint is a page per task, not a megabyte.

**Time is quadratic.** Doubling the tasks quadruples the wall clock. That is
what stops 1M, not the memory: at this rate it is hours.

## Where the quadratic is

`run_tasks` resumes **every live task every round**. With a channel that lets
one value through at a time, only one task gets anywhere per round, so N tasks
need N rounds of N resumes.

## The fix that looked obvious, and does not work

Built and measured before keeping: give each channel a version, have a task
record the version it blocked on, and let the scheduler skip a task whose
channel has not changed. Three wait sites, a field on `Chan`, a field on the
task tuple.

It made no difference -- 4,000 tasks went 1.20 s to 1.30 s, inside the noise.
The reason is a **thundering herd**: the version is per channel, not per
waiter, so when the receiver takes one value every blocked sender becomes
eligible at once. One of them succeeds and the rest block again, so a
successful send still costs O(N) resumes. Reverted.

## What would actually work, and what it costs

A per-channel **FIFO wait queue**: a blocked task joins the queue, and a send
or receive wakes exactly one. That turns a round from O(live) into
O(runnable) for real.

### The corpus cost, counted (2026-10-11)

"A corpus-wide diff" above was an estimate, and it is wrong. Counted across
`apps`, `examples`, `packages`, `projects`, `spec/conformance` and
`benchmarks`: **eleven programs use `concurrent` at all, and three have more
than one `go`**. Those three are `E-SCHEDULE.gr`, `E-DEADLOCK.gr` and
`E-CONCURRENT.gr` -- the conformance tests whose job is to pin the rule a
queue would change.

No application, package, example or benchmark anywhere has two tasks. So the
blast radius is three tests that exist to encode the behaviour, which is
exactly what a spec change should have to update, and nothing else.

`E-SCHEDULE` is also listed **experimental** in `spec/stability.md`, so
changing it is permitted by the project's own policy rather than a break.

What stands is the spec question -- the clause says "the next task, in the
order the `go`s ran", and a queue would make it the order they blocked -- plus
the 16 KB a task that puts 1M at 16 GB. Neither is a corpus problem.

The reason it is not done here is that it changes **which** task proceeds.
Today the round-robin order decides; a queue would decide by arrival. Both are
deterministic, so [E-SCHEDULE] survives either way -- but the *observable
interleaving changes*, and the output of any program with more than one waiter
on a channel changes with it. That is a corpus-wide diff and a spec question
about what `[E-SCHEDULE]` promises, not a local fix.

So G6 is reachable, and it is a scheduler redesign with a spec change attached
rather than an optimisation. 16 KB a task also puts 1M at 16 GB, so the stack
would want shrinking in the same pass.

## Built, 2026-10-12: the queue, and what it moved

Per-channel wait queues, approved as a change to `[E-SCHEDULE]`. Interpreter
first, since it is the engine the others are diffed against.

| tasks | before | after | before, per doubling | after |
|---:|---:|---:|---:|---:|
| 1,000 | 0.12 s | 0.03 s | — | — |
| 2,000 | 0.41 s | 0.05 s | 3.4x | 1.8x |
| 4,000 | 1.78 s | 0.10 s | 4.4x | 2.1x |
| 10,000 | 15.65 s | 0.28 s | 8.8x | 2.9x |
| 40,000 | >240 s | 0.98 s | — | 3.5x |
| 100,000 | >240 s | 2.33 s | — | 2.4x |
| 200,000 | — | 6.53 s | — | 2.8x |

Quadratic to linear, and every row's answer checked, not just its clock.
**56x at 10,000 tasks**, and 200,000 now runs where 40,000 did not finish.

### Two queues, not one

The first version had one queue per channel and reported a **deadlock on a
program that has none**. A send woke whichever slot had waited longest, which
could be another *sender*: it found the buffer just as full, waited again, and
the receiver that could have gone on was never reached.

So a channel has two queues -- tasks wanting room, tasks wanting a value. A
send wakes a receiver, a receive wakes a sender, and closing wakes both.

A second defect in the same first version: a blocked operation retries in a
loop, so a task re-joins a queue every time it gives way, and queues fill with
entries for slots since woken or finished. A wake spent on one of those is a
wake nobody gets. `waiting[slot]` now says whether a slot is really in a
queue, and `wake_one` skips the stale ones rather than stopping at the first.

### What the corpus did

Nothing. All 257 conformance programs give the same output as before, with one
exception that is not one: `E-PANIC-TRACE` elides 53,950 frames where it
elided 67,165, and its own comment records that depth as
implementation-defined ([E-RECURSION-DEPTH]). The three tests predicted to
change -- `E-SCHEDULE`, `E-DEADLOCK`, `E-CONCURRENT` -- did not, because at
most one task waits per channel in them, so arrival order and `go` order
coincide.

### The limit is now memory, and it is not 1M yet

400,000 tasks fails, and not on time: `could not map a fiber stack of
470827008 bytes`. `TASK_STACK` is `STACK_BUDGET + 1 MB`, so **449 MB of
address space is reserved per task**, and 400,000 of those exceeds the
128 TB a process gets. Pages stay untouched, so the resident cost is about
15 KB a task -- 2.97 GB at 200,000, which matches the 16 KB this document
measured at the start.

So G6 is no longer blocked by the scheduler. Reaching 1M needs the per-task
stack reservation cut by roughly 4x for the address space, and 1M x 15 KB is
15 GB resident, which is a machine question rather than a code one. Both are
separate from the quadratic, which is fixed.
