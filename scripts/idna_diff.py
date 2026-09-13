"""Internationalised domain names against Python's `encodings.idna`."""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from idna_corpus import build, build_punycode

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

argv, expected, labels, skipped = build()
ran = subprocess.run([GRASM, "run", "packages/idnatest/src/main.gr", "--"]
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
            print(f"    python {readable(want)!r}")
            print(f"    grasm  {readable(mine)!r}")
# Punycode itself, through its own harness: `idna` is the layer above it and
# a mistake in either shows up here.
puny_argv, puny_expected, puny_labels = build_punycode()
ran = subprocess.run([GRASM, "run", "packages/punytest/src/main.gr", "--"]
                     + puny_argv, cwd=ROOT, capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")
for i, want in enumerate(puny_expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {puny_labels[i]}")
            print(f"    python {readable(want)!r}")
            print(f"    grasm  {readable(mine)!r}")

print(f"cases: {len(expected) + len(puny_expected)}   agreeing: {agree}   "
      f"differing: {differ}   left out for Unicode 3.2: {skipped}")
sys.exit(1 if differ else 0)
