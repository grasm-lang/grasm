# Compile-speed generators

`MASTERPLAN.md` §2.3 G3 asks for 500k LOC/s in the frontend and a 50 ms
edit-to-diagnostic. Measuring that needs single compilation units of a known
size, which the corpus does not have -- its largest file is 3,489 lines.

    python3 gen.py 1000   > big.gr     # 25,052 lines
    python3 genrs.py 1000 > big.rs     # 21,046 lines, the same shapes

Both write realistic code rather than one line repeated: a record type, a sum
type, and three functions per unit doing a `match`, a loop with a branch, and
a struct literal with interpolation.

    grasm check big.gr                 # frontend: parse + typecheck
    grasm build --native big.gr -o b   # whole pipeline
    rustc -O -o r big.rs               # the comparison

Results and what they found are in `docs/compile-speed.md`. Time the best of
five; the first run of a freshly written binary pays a one-off macOS cost.
