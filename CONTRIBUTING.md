# Contributing

## The one rule that matters

**The interpreter defines the language.** `crates/grasm_interp` is the reference
implementation. Where it and the native backend disagree, the interpreter is
right and the backend has a bug — unless the interpreter contradicts `spec/`,
in which case the specification is right and the interpreter has a bug.

Everything below follows from that.

## Before you open a pull request

One command runs everything below and judges each by its exit code:

```
./scripts/allgates.sh                    # every gate and generator; exits 1 if any failed
```

It reads `$?` and nothing else. That is the whole point of it: `wasmtest.sh`
prints `identical: 184   mismatched: 0` on a run that exits 1 over four stale
committed modules, and reading the summary instead of the status reported a
failing gate as green. The same mistake in another shape hid nine of eleven
`agreetest` failures behind a `tail -2`.

The individual gates, if you want to run one:

```
cargo build --release --workspace
cargo test --workspace                   # all tests
cargo clippy --workspace --all-targets   # must be silent
cargo test -p grasm_spec                 # specification conformance

./scripts/difftest.sh                    # native output must equal interpreter output
./scripts/leakcheck.sh                   # no program leaks
./scripts/agreetest.sh                   # interpreter, bundle and native agree
./scripts/nativegaps.sh                  # the documented gap list matches the compiler
./scripts/replaytest.sh                  # every journaled builtin refuses under an empty journal
./scripts/wasmtest.sh                    # the browser build agrees, and still builds
./scripts/crosstest.sh                   # the same programs, cross-compiled to another architecture
./scripts/gatecheck.sh                   # the gates themselves call helpers that exist
./scripts/findings_index.sh --check      # FINDINGS.md's index matches its entries
./ProjectGold/run.sh                     # forty-four whole programs, three engines
```

`gatecheck.sh` is there because of what it is named after. `nativegaps.sh`
defines `expect_compiled`, eleven cases were written as `expect_compiles`, and
bash reported each as a command not found on stderr and carried on — so the
script reported success having run 28 of its 39 cases (F40). A gate that can
silently skip a quarter of itself is worse than no gate, because it is believed.

Each of those builds the compiler it tests (`scripts/lib/fresh.sh`), and so do
the generators (`scripts/lib/fresh.py`) — a gate that reads a binary it did not
produce measures whatever was lying there, which is F36 and F49.

And the generators, which find what nobody thought to write down:

```
python3 scripts/fuzz_differential.py --count 150
python3 scripts/fuzz_fallible.py 100
python3 scripts/fuzz_generics.py --count 100
python3 scripts/fuzz_handlers.py --count 120
python3 scripts/fuzz_rows.py
python3 scripts/fuzz_extern.py
python3 scripts/fuzz_places.py           # every shape of place, read and written
python3 scripts/fuzz_methods.py          # every builtin method, every receiver shape
python3 scripts/fuzz_fusions.py          # the compiled shortcuts against the interpreter
python3 scripts/fuzz_captures.py         # what a closure captures, once per representation
python3 scripts/fuzz_operators.py        # every operator against every representation
python3 scripts/fuzz_panics.py           # every way a program fails, from every place it can
python3 scripts/fuzz_exits.py            # every way out of every block that owns something
python3 scripts/fuzz_laws.py             # laws the answer obeys, checked against arithmetic

`fuzz_laws.py` is the only gate here that does not compare two engines. A
bundle *is* the interpreter with the program attached, and the interpreter is
what every differential is measured against, so a wrong answer they share is
invisible to all the others. It checks against arithmetic instead, over every
method `spec/library.md` names — and a method added there without a law fails
it by name.

Every generator runs the programs it invents a second time under
`GRASM_LEAK_CHECK=1`, through the one definition in `scripts/lib/leaks.py`. A
miscounted reference produces exactly the right output while holding an
allocation for the life of the process, so comparing engines cannot see it —
`scripts/leakcheck.sh` asked that of the 226 hand-written programs and of none
of the ~1,150 generated ones (F57).
```

The last four enumerate an axis along which the native backend has more than
one implementation site. Every one of F17, F22, F23, F24 and F26 in
`ProjectGold/FINDINGS.md` was a rule added at one site and not the others, so
that axis is where the defects are.

`fuzz_captures.py` is the answer to a subtler version of the same thing. A
value's *type* is one axis and how it reached the code is another, and the
crossings that varied only the first found nothing for four rounds because
they all built their values from literals — and a closure returning a literal
never reads its environment. F50 was sitting behind that the whole time.

`.github/workflows/ci.yml` runs all of this on macOS, Linux and Windows, plus a
cross-compilation job. **It has never run**: this repository has no remote, so
no push has ever triggered it. Until that changes, the list above is the whole
of the evidence, and it is only as good as the machine you ran it on.

## Changing behaviour

If a change alters what a program prints, exits with, or refuses to compile:

1. **Write or update the clause in `spec/`.** Give it a tag. The specification
   is where a decision is recorded; a commit message is not.
2. **Add `spec/conformance/<TAG>.gr`.** Every clause needs a program, and every
   program must name a real clause — both directions are tested, so you cannot
   forget either.
3. **Record its stability level in `spec/stability.md`.** A clause with no level
   fails the suite.
4. **Check both engines.** `./scripts/difftest.sh` compiles every program in the
   repository and compares native output against the interpreter byte for byte.

The native backend currently compiles every program in the repository. If you
add something it cannot compile yet, it must say so by name —
`nope(span, "what it is")` — so difftest reports it as unsupported rather than
passing silently. A skipped program is never counted as a pass.

## Tests

Test the behaviour, not the implementation. A test named
`a_blank_line_ends_a_doc_block` says what the language does;
`test_attach_docs_2` says nothing.

Every bug fix gets a test that fails before the fix. That is not a formality:
several bugs in this repository were fixed twice because the first fix silently
did not apply.

## Comments

Comment the *why*, never the *what*. `// increment i` is noise. A comment
earns its place by explaining a decision, a constraint, or a trap:

```rust
// One scope pop for one push, on the path both branches share.
```

```rust
// `abs` would overflow on i64::MIN.
```

If code needs a comment to explain what it does, rewrite the code.

## Performance claims

Any performance claim needs a reproduction. `scripts/bench.sh` builds every
implementation, verifies its answer against the interpreter, and refuses to time
one that got a different result. Add a benchmark rather than a number in a
commit message.

## Commit messages

Say what changed and why. If a change was surprising — a bug that turned out to
be somewhere other than where it looked, a fix that had to be rethought — say
that too. The commit log is the only place that history survives.

## Scope

Small, complete changes. A pull request that adds a language feature includes
the checker rule, the interpreter, the native backend, the spec clause, the
conformance program, and the tests. One that adds only some of those leaves the
gates able to pass while the language is inconsistent.
