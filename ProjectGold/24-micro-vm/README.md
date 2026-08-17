# 24 — Micro Virtual Machine
**Tests:** bytecode, an operand stack, an instruction pointer, call frames. **Result:** works, all three engines agree.

## What it exercises

The list asks for PUSH/ADD/POP. That is an afternoon and proves little, so this
goes as far as **calls**: frames, locals, arguments and returns, which is where a
stack machine actually gets hard.

The load-bearing invariant is that a callee must leave the operand stack exactly
as it found it apart from its own return value. A return that leaves it
unbalanced corrupts the *caller*, several instructions later, in a way that is
very hard to trace back — so it is checked on every return rather than trusted.

It immediately earned its place: the first version of the `calls` program had its
`Halt` in the wrong slot, so the caller fell straight into `square`'s body after
the second call returned. The VM reported `unbalanced stack on return: 1 value
left behind` instead of producing a plausible wrong number.

Recursion works to real depth — `factorial(10)` = 3628800 in 109 steps,
`fib(15)` = 610 in 19,730 steps.

Nine fault cases, all values and none a panic: underflow, divide by zero, bad
jump, bad local, running off the end, an infinite loop caught by the step limit,
returning from the outermost frame, an unbalanced callee, and runaway recursion
caught by a depth limit. A VM that panics on bad bytecode is a VM you cannot run
untrusted code on, which is the only reason to write one.

## Findings
None.
