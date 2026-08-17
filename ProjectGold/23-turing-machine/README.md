# 23 — Turing Machine Simulator
**Tests:** an expanding tape, transition tables, halting. **Result:** works, all three engines agree.

## What it exercises

The tape is the subject. A real machine's tape is infinite in both directions
and a list is not, so it grows on demand at **both ends** — and growing it on the
left shifts every index, which is where this is usually got wrong. The head is
kept as an absolute coordinate and translated on access, so nothing outside
`read`/`write` knows the tape moved. Checked: after ten leftward steps the tape
is 10 long with origin −9 and head −10.

Correctness is by published results:

* **binary increment** — eight cases including the all-ones carry (`11111111` →
  `100000000`);
* **a palindrome recogniser** over `{a,b}` — accepts and rejects correctly, and
  halts in a non-accepting state on an out-of-alphabet symbol;
* **the 3-state busy beaver** — halts after **14 steps having written 6 ones**,
  which is the published answer.

Non-termination is bounded by a step limit rather than hung, because a program
that hangs is not a test result.

## An honest failure

The first version used a *different* 3-state machine and compared its step count
to the published one. It scored the right six ones in thirteen steps, and the
fourteen belonged to a machine that was not being run. Rado's table is now
transcribed exactly. A known-answer test against the wrong machine is not a
known-answer test.

## Findings
None.
