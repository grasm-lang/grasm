"""Z85, string.Template, expandvars, format strings and ndiff, against Python.

Format-string parsing is checked exhaustively over every string of up to
five characters from the ones the parser cares about. Built natively.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from text2_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

work = tempfile.mkdtemp()
harness = os.path.join(work, "texttest2")
built = subprocess.run([GRASM, "build", "packages/texttest2/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

argv, expected, labels = build()
# Every command takes exactly three words, so a chunk that is a multiple of
# three never splits one.
CHUNK = 3000
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([harness] + argv[start:start + CHUNK],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]


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
            print(f"    python {readable(want)[:80]!r}")
            print(f"    grasm  {readable(mine)[:80]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
