"""Patch ids and SSH signatures, against git and ssh-keygen.

The harness is built natively: the patch corpus is this repository's own
history and some of those diffs run to megabytes.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import patchid_corpus
import sshsig_corpus

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

if subprocess.run(["git", "--version"], capture_output=True).returncode != 0:
    print("no git on this machine; skipped")
    sys.exit(0)

work = tempfile.mkdtemp()
harness = os.path.join(work, "pidtest")
built = subprocess.run([GRASM, "build", "packages/pidtest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

patches = os.path.join(work, "patches")
os.makedirs(patches)
argv, expected, labels = patchid_corpus.build(patches)
signatures = os.path.join(work, "signatures")
os.makedirs(signatures)
more_argv, more_expected, more_labels = sshsig_corpus.build(signatures)
argv += more_argv
expected += more_expected
labels += more_labels

lines = []
try:
    # Chunked in whole cases: a split inside one would lose alignment, and
    # `sig` takes three arguments where `id` takes two.
    start = 0
    while start < len(argv):
        end = start
        count = 0
        while end < len(argv) and count < 100:
            end += 3 if argv[end] == "sig" else 2
            count += 1
        ran = subprocess.run([harness] + argv[start:end],
                             capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
        start = end
finally:
    sshsig_corpus.clean(work)

agree = differ = shown = 0
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {labels[i]}")
            print(f"    want {want}")
            print(f"    got  {mine}")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
