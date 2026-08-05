#!/usr/bin/env bash
# Regenerates dist/grasm-language-pack.md — the entire language in one file.
#
# A new language's hardest adoption barrier is now that AI assistants have
# never seen it. The answer is not to wait for training data: the whole
# definition, plus an executable program for every rule, is ~13k tokens and
# fits in a context window.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import glob, os
parts = ["""# Grasm — complete language definition

Everything needed to write correct Grasm, in one file. No prior exposure
required: the normative rules, a guided tour, and an executable program for
every rule.

Each `[TAG]` below is a normative clause. Every clause has a corresponding
program in the conformance section, and `cargo test -p grasm_spec` fails if a
clause has no program, if a program names a clause that does not exist, or if
any program behaves differently from what it declares. So the rules and the
examples cannot disagree.
"""]
for f in ["spec/grammar.md", "spec/types.md", "spec/semantics.md", "spec/library.md"]:
    parts.append(f"\n\n<!-- ===== {f} ===== -->\n\n" + open(f).read())
parts.append("\n\n<!-- ===== docs/tutorial.md ===== -->\n\n" + open("docs/tutorial.md").read())
parts.append("\n\n# Conformance programs\n\nOne per normative clause. Each header states the tag it demonstrates and the exact expected behaviour.\n")
for f in sorted(glob.glob("spec/conformance/*.gr")):
    parts.append(f"\n## `{os.path.basename(f)}`\n\n```grasm\n" + open(f).read().rstrip() + "\n```\n")
out = "".join(parts)
os.makedirs("dist", exist_ok=True)
open("dist/grasm-language-pack.md", "w").write(out)
print(f"wrote dist/grasm-language-pack.md: {len(out)} bytes (~{len(out)//4000}k tokens)")
PY
