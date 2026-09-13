"""Refs, reflogs, submodules and dates, against git itself.

Every input is produced by git — a repository with packed refs, tags and a
reflog, and commits made at chosen instants in chosen zones — and every
expectation is what `git show-ref`, `git reflog`, `git config --list` and
`git log --date=` print.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import repo_corpus

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

if subprocess.run(["git", "--version"], capture_output=True).returncode != 0:
    print("no git on this machine; skipped")
    sys.exit(0)

# The commands that take three arguments rather than two.
THREE_ARGUMENT = ("iso", "isostrict", "rfc", "short", "raw", "unix",
                  "abbrev")

work = tempfile.mkdtemp()
try:
    argv, expected, labels = repo_corpus.build(work)
    # Chunked on whole cases: some commands take three arguments and some
    # take two, so a fixed chunk size would split a group and every result
    # after it would line up against the wrong expectation.
    lines = []
    start = 0
    while start < len(argv):
        end = start
        count = 0
        while end < len(argv) and count < 40:
            end += 3 if argv[end] in THREE_ARGUMENT else 2
            count += 1
        ran = subprocess.run([GRASM, "run", "packages/repotest/src/main.gr",
                              "--"] + argv[start:end], cwd=ROOT,
                             capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
        start = end
finally:
    repo_corpus.clean(work)


def readable(text):
    try:
        return bytes.fromhex(text).decode("utf-8", "replace")
    except Exception:
        return text


agree = differ = shown = 0
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {labels[i]}")
            print(f"    git   {readable(want)[:100]!r}")
            print(f"    grasm {readable(mine)[:100]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
