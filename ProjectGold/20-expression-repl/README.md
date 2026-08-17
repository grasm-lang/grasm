# 20 — Expression Evaluation REPL
**Tests:** read-eval-print architecture, persistent scope, error recovery. **Result:** works, all three engines agree.

What makes this more than project 01 with a prompt is **state**. A session has
variables; they persist between lines; a line can refer to one defined three
lines ago, or fail because it never was. The environment is threaded as `inout`.

Every error — parse, name, arithmetic — is a value that ends the *line* and
leaves the session intact. A REPL that dies on a typo is not a REPL. Four
consecutive failures (`z`, `1 / 0`, `2 +`, `4 $ 5`) are followed by `x = x + 5`
working correctly, reading the old `x` before writing the new one.

`ans` is pre-defined and updated after every successful line, so `ans + 1` works
the way it does in a real shell. Two meta-commands (`:vars`, `:count`) read
session state without evaluating anything.

## Findings
None.
