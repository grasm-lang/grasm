"""Compiled catalogues against Python's `gettext.GNUTranslations`."""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gettext_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

argv, expected, labels = build()
ran = subprocess.run([GRASM, "run", "packages/motest/src/main.gr", "--"]
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
            print(f"  {labels[i]}: want {readable(want)!r} "
                  f"got {readable(mine)!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
