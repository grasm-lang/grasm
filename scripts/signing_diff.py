"""Private keys, allowed signers and commit messages, against ssh-keygen
and git."""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import signing_corpus

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

for tool in ("ssh-keygen", "git"):
    if subprocess.run([tool, "--version"], capture_output=True).returncode \
            not in (0, 1):
        print(f"no {tool} on this machine; skipped")
        sys.exit(0)

work = tempfile.mkdtemp()
try:
    argv, expected, labels = signing_corpus.build(work)
    ran = subprocess.run([GRASM, "run", "packages/signtest/src/main.gr",
                          "--"] + argv, cwd=ROOT, capture_output=True,
                         text=True)
finally:
    signing_corpus.clean(work)
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
            print(f"    want {readable(want)[:90]!r}")
            print(f"    got  {readable(mine)[:90]!r}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
