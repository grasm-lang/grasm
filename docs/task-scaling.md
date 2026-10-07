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

The reason it is not done here is that it changes **which** task proceeds.
Today the round-robin order decides; a queue would decide by arrival. Both are
deterministic, so [E-SCHEDULE] survives either way -- but the *observable
interleaving changes*, and the output of any program with more than one waiter
on a channel changes with it. That is a corpus-wide diff and a spec question
about what `[E-SCHEDULE]` promises, not a local fix.

So G6 is reachable, and it is a scheduler redesign with a spec change attached
rather than an optimisation. 16 KB a task also puts 1M at 16 GB, so the stack
would want shrinking in the same pass.
