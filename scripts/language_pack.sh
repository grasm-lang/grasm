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
#   grasm-conformance-{grammar,types,semantics,library}.md
#                              one executable program per normative clause,
#                              split by the area its tag belongs to.
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
executable program in one of the `grasm-conformance-*.md` files, and `cargo test -p
grasm_spec` fails if a clause has no program, if a program names a clause that
does not exist, or if any program behaves differently from what it declares. So
the rules and the examples cannot disagree.
"""]
for f in ["spec/grammar.md", "spec/types.md", "spec/semantics.md", "spec/library.md"]:
    parts.append(f"\n\n<!-- ===== {f} ===== -->\n\n" + open(f).read())
parts.append("\n\n<!-- ===== docs/tutorial.md ===== -->\n\n" + open("docs/tutorial.md").read())

# The conformance programs, split by area.
#
# It was one file until it passed 200 KB, which is the size a reader can load on
# its own — the ceiling the test enforces. Splitting was the recorded answer
# rather than raising it: these are loaded when a rule is in dispute, and a rule
# in dispute is about the grammar, or the types, or the semantics, or the
# library. Nobody needs all four at once, and a ceiling on the sum would have
# been a ceiling on how many clauses the language may ever have.
#
# The prefix of a clause tag says which: `G-` grammar, `T-` types, `E-`
# semantics, `L-` library. Anything else lands in `misc` rather than being
# dropped, and the test below notices if `misc` is not empty.
AREAS = [
    ("grammar", "G", "the grammar — what is a program"),
    ("types", "T", "the type and effect system — what a program means before it runs"),
    ("semantics", "E", "evaluation — what a program does when it runs"),
    ("library", "L", "the standard library"),
]

def area_of(name):
    for slug, prefix, _ in AREAS:
        if name.startswith(prefix + "-"):
            return slug
    return "misc"

buckets = {slug: [] for slug, _, _ in AREAS}
buckets["misc"] = []
for f in sorted(glob.glob("spec/conformance/*.gr")):
    base = os.path.basename(f)
    body = f"\n## `{base}`\n\n```grasm\n" + open(f).read().rstrip() + "\n```\n"
    buckets[area_of(base)].append(body)

os.makedirs("dist", exist_ok=True)
written = [("grasm-language-pack.md", "".join(parts))]

def header(title, blurb, count):
    return f"""# Grasm — conformance programs: {title}

{blurb.capitalize()}. {count} executable program(s), one per normative clause of
`grasm-language-pack.md`. Each header states the tag it demonstrates and the
exact expected behaviour, and every one is run by `cargo test -p grasm_spec`, so
a program here cannot describe behaviour the implementation does not have.

This is the companion to the definition rather than part of it. Read the
definition to write Grasm; read this when a rule is in dispute and you want to
see it settled by a program.

The programs are split by area because together they no longer fit in one
context, and a reader with a rule in dispute needs one area rather than four.
The other parts are `grasm-conformance-{"`, `grasm-conformance-".join(s for s, _, _ in AREAS)}`.
"""

for slug, _, blurb in AREAS:
    body = buckets[slug]
    written.append((f"grasm-conformance-{slug}.md", header(slug, blurb, len(body)) + "".join(body)))
if buckets["misc"]:
    written.append(("grasm-conformance-misc.md",
                    header("misc", "clauses whose tag matches no known area", len(buckets["misc"]))
                    + "".join(buckets["misc"])))

for name, text in written:
    open(f"dist/{name}", "w").write(text)
    print(f"wrote dist/{name}: {len(text)} bytes (~{len(text)//4000}k tokens)")

# The single-file pack is gone; leaving a stale copy would let a reader load a
# definition that is months behind without noticing.
old_pack = "dist/grasm-conformance-pack.md"
if os.path.exists(old_pack):
    os.remove(old_pack)
    print(f"removed {old_pack} — split into areas")
PY
