"""SASLprep, source-literal escapes and account files, against Python.

SASLprep is compared against RFC 4013 implemented over the tables Python's
`stringprep` ships; escapes against the `unicode_escape` codec; and the
account files against `pwd` and `grp` reading the same machine's files.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from prep_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

argv, expected, labels, skipped = build()
ran = subprocess.run([GRASM, "run", "packages/preptest/src/main.gr", "--"]
                     + argv, cwd=ROOT, capture_output=True, text=True)
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
            print(f"    python {readable(want)[:90]!r}")
            print(f"    grasm  {readable(mine)[:90]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}   "
      f"left out for Unicode 3.2 or lone surrogates: {skipped}")
sys.exit(1 if differ else 0)
