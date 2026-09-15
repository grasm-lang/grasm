"""inet_aton, netdb, mail dates, Maildir and ftplib, against Python.

Service lookups are checked against this machine's own /etc/services. Built
natively.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from net3_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

work = tempfile.mkdtemp()
harness = os.path.join(work, "nettest3")
built = subprocess.run([GRASM, "build", "packages/nettest3/src/main.gr",
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


agree = differ = 0
shown = {}
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1
        what = labels[i].split()[0]
        shown[what] = shown.get(what, 0) + 1
        if shown[what] <= 2:
            print(f"  {labels[i]}")
            print(f"    python {readable(want)[:80]!r}")
            print(f"    grasm  {readable(mine)[:80]!r}")
for what, n in sorted(shown.items()):
    print(f"  {what}: {n} differing")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
