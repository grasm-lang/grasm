"""Index, attributes and mailmap against git itself.

Every input is produced by git — real `.git/index` files at each version
including one stopped mid-merge — and every expectation is what `git
ls-files --stage`, `git check-attr` and `git check-mailmap` print.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from git_corpus import build, clean

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

if subprocess.run(["git", "--version"], capture_output=True).returncode != 0:
    print("no git on this machine; skipped")
    sys.exit(0)

argv, expected, labels, work = build()
try:
    ran = subprocess.run([GRASM, "run", "packages/gitest/src/main.gr", "--"]
                         + argv, cwd=ROOT, capture_output=True, text=True)
finally:
    clean(work)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")


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
            print(f"    git   {readable(want)[:90]!r}")
            print(f"    grasm {readable(mine)[:90]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
