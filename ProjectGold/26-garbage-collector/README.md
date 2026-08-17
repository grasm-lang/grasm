# 26 — Automated Garbage Collector
**Tests:** heap modelling, reference counting, mark-and-sweep. **Result:** works, all three engines agree.

## What it exercises

Both algorithms run over the **same heap**, built twice, and the point of the
project is the case where they disagree: a cycle. Reference counting cannot
collect one and mark-and-sweep can. That is the single most important fact about
the two techniques and it is invisible in a program that only builds trees — so
this one builds cycles on purpose.

| case | refcount frees | mark-sweep frees | leaked |
|---|---|---|---|
| self cycle | 0 | 1 | **1** |
| two-node cycle | 0 | 2 | **2** |
| five-node ring | 0 | 5 | **5** |
| garbage cycle beside live data | 0 | 2 | **2** |

On trees and shared structure the two agree exactly, including the case a naive
refcount gets wrong: two roots pointing at one object, which must survive
dropping either and die only when both are gone.

Refcounting here cascades — freeing a list head must free its tail — which is
why it is a fixpoint loop rather than a single pass.

This is also why Grasm's own `ref` is **generational** rather than counted: a
generation check catches a stale handle without needing to know whether anything
still points at the slot ([T-REF-GENERATIONAL]), which sidesteps the entire
column above.

## Findings
None.
