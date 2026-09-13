"""UTF-7, UTF-32, ISO-2022-KR and HZ against Python's codecs."""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from encoding5_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

argv, expected, labels = build()

# Chunked in whole cases: each is two arguments and a split inside a pair
# would silently lose alignment.
CHUNK = 1000
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([GRASM, "run", "packages/enctest5/src/main.gr",
                          "--"] + argv[start:start + CHUNK], cwd=ROOT,
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
            print(f"    python {readable(want)[:70]!r}")
            print(f"    grasm  {readable(mine)[:70]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
