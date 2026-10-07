# G8, measured: what a journal can and cannot bring back

`MASTERPLAN.md` §2.3 **G8** asks that **≥99% of production failures replay
locally and deterministically**. The machinery shipped -- `GRASM_RECORD`,
`GRASM_REPLAY`, `grasm journal` -- and the percentage had never been measured.
This is 2026-10-07.

## Every builtin is classified, and the classification is complete

From `crates/grasm_sema/src/builtins.rs`, every builtin carries a `Journal`
kind. There are three, and no builtin is left out:

| | count | |
|---|---:|---|
| `Recorded` | 39 | every answer the program takes from outside itself |
| `Repeated` | 3 | `print`, `eprint`, `net_workers` -- re-run rather than recorded |
| `Untouched` | 10 | `assert`, `panic`, `exit`, `chan_*`, `bytes_of` -- nothing to record |

`Repeated` is right for the first two: a replay whose output you cannot see is
not worth running, and output is not an input. `replaytest` holds every
`Recorded` builtin to actually being hooked, by running under an empty journal
and requiring it to stop rather than reach the world.

## What was tested

**Thirteen panicking conformance programs**: recorded, then replayed. All
thirteen reproduce byte-identically with the same exit code. They are the easy
case -- deterministic panics with no external input -- and they are the
baseline.

**A crash caused by the outside world**, which is the claim that matters. A
program reads `/tmp/g8probe.txt`, parses it, and indexes a three-element list
with it. Recorded with the file holding `9`, it panics with `index 9 out of
bounds (len 3)` and exits 101.

Then the world was changed underneath it:

- probe rewritten to `1` -- the program now exits 0 and prints `element 20`;
  **the replay still panics identically, exit 101**;
- probe **deleted entirely** -- the replay still panics identically, exit 101;
- the same journal handed to the **native build** -- identical output, exit
  101, from a journal the interpreter recorded.

That last one is worth stating plainly: a journal crosses engines.

**`file_exists` and the monotonic clock** were tested the same way, because a
first reading of the source suggested they might not be recorded. They are:
the journal holds `file_exists true` and `monotonic_millis 0`, and a crash
depending on the file's existence replays after the file is gone.

## The gap, named

**`net_workers` is `Repeated`, not `Recorded`.** It starts OS threads, and
re-running it on replay starts them again -- so the interleaving is not the
one that was recorded. A crash that depends on which of 32 workers won a race
is **not** reproducible from a journal.

Everything else is: Grasm's own task scheduler is deterministic by
[E-SCHEDULE], so `concurrent` and `go` need no recording, and every external
input has a `Recorded` builtin behind it.

So the honest statement of G8 is not a percentage, because "crashes in
production" is not a population anyone here has sampled. It is this: **every
external input is recorded, and the single unrecorded source of nondeterminism
is OS-thread interleaving under `net_workers`.** Closing that would mean
recording the scheduling decisions of real threads, which is the same problem
the project declined when it chose a deterministic scheduler for `go`.
