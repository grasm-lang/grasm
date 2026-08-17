# 15 — Sorting Algorithm Benchmark
**Tests:** array manipulation, timing, algorithmic complexity. **Result:** works, all three engines agree.

A benchmark that does not verify its answers measures how fast a program
produces the wrong result. So every sort is checked twice before any timing:
the output must be ordered **and** a permutation of the input. The second is
what catches a quicksort that silently drops elements equal to the pivot — which
is why the partition here is three-way, and why "all equal" and "many
duplicates" are in the input set.

Five implementations (bubble, insertion, quick, merge, library) must produce not
merely sorted output but the *identical* list. Merge sort's stability is checked
by encoding `value*10 + index` and confirming indices still ascend within an
equal group.

Timings are reported as the **shape of the curve**, not milliseconds: bubble at
least 3× slower when n doubles, merge under 3×. Measured, the raw numbers were
350/1417/5640 and 15/32/69 — textbook O(n²) and O(n log n).

## Findings
**F7** — there is no way to write to stderr. A benchmark's timings are
diagnostics, not results; with one stream they land in stdout and make the output
machine-dependent. Hence the derived-facts workaround above.
