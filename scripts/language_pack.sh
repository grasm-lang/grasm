#!/usr/bin/env bash
# Regenerates the two distribution packs under dist/.
#
# A new language's hardest adoption barrier is now that AI assistants have
# never seen it. The answer is not to wait for training data: the whole
# definition, plus an executable program for every rule, fits in a context
# window.
#
# It is two files rather than one because it outgrew one. At 321 KB the single
# pack failed its own size gate, and that gate says what to do about it:
# "split the conformance programs into a second file, not to move this". So:
#
#   grasm-language-pack.md     the definition — grammar, types, semantics,
#                              library, tutorial. ~40k tokens. This is what a
#                              reader needs to write correct Grasm.
#   grasm-conformance-pack.md  one executable program per normative clause.
#                              ~37k tokens. This is the evidence that the
#                              definition is true, and it is only needed when
#                              a rule is in dispute.
#
# The split is not only bookkeeping: the definition alone now costs half of
# what the combined pack did, and the part that grows with every new clause is
# the part a reader can leave behind.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import glob, os

parts = ["""# Grasm — complete language definition

Everything needed to write correct Grasm, in one file. No prior exposure
required: the normative rules, then a guided tour.

Each `[TAG]` below is a normative clause. Every clause has a corresponding
executable program in `grasm-conformance-pack.md`, and `cargo test -p
grasm_spec` fails if a clause has no program, if a program names a clause that
does not exist, or if any program behaves differently from what it declares. So
the rules and the examples cannot disagree.
"""]
for f in ["spec/grammar.md", "spec/types.md", "spec/semantics.md", "spec/library.md"]:
    parts.append(f"\n\n<!-- ===== {f} ===== -->\n\n" + open(f).read())
parts.append("\n\n<!-- ===== docs/tutorial.md ===== -->\n\n" + open("docs/tutorial.md").read())

conf = ["""# Grasm — conformance programs

One executable program per normative clause of `grasm-language-pack.md`. Each
header states the tag it demonstrates and the exact expected behaviour, and
every one of them is run by `cargo test -p grasm_spec`, so a program here
cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.
"""]
for f in sorted(glob.glob("spec/conformance/*.gr")):
    conf.append(f"\n## `{os.path.basename(f)}`\n\n```grasm\n" + open(f).read().rstrip() + "\n```\n")

os.makedirs("dist", exist_ok=True)
for name, text in [("grasm-language-pack.md", "".join(parts)),
                   ("grasm-conformance-pack.md", "".join(conf))]:
    open(f"dist/{name}", "w").write(text)
    print(f"wrote dist/{name}: {len(text)} bytes (~{len(text)//4000}k tokens)")
PY
