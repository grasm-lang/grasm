"""`shlex`, `configparser`, `pytokenize`, `querystring` and `cformat` against
Python, Node and Perl.

Built natively. Every answer comes from running the implementation being
matched, on this machine.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lang2_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

work = tempfile.mkdtemp()
harness = os.path.join(work, "langtest2")
built = subprocess.run([GRASM, "build", "packages/langtest2/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:])
    sys.exit(1)

argv, expected, labels = build()
if not expected:
    print("no language runtime to compare against — nothing was checked")
    sys.exit(1)

# Chunked by the *size* of the arguments rather than their number: a case
# here can be a whole Python file in hexadecimal, and a few hundred of those
# is past what `execve` will take — which arrives as "Argument list too long"
# rather than as anything about the test.
LIMIT = 200_000
lines = []
start = 0
while start < len(argv):
    end = start
    used = 0
    while end < len(argv) and (used < LIMIT or end == start):
        used += len(argv[end]) + len(argv[end + 1]) + len(argv[end + 2]) + 3
        end += 3
    ran = subprocess.run([harness] + argv[start:end],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:])
        sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]
    start = end


def readable(text):
    try:
        return bytes.fromhex(text).decode("utf-8", "replace")
    except Exception:
        return text


def sorted_pairs(text):
    return "\x01".join(sorted(text.split("\x01")))


def same(want, mine, label):
    if want == mine:
        return True
    # JavaScript objects list integer-like keys first, whatever order they
    # were inserted in, so `Object.keys(qs.parse("a&2"))` is ["2", "a"]. That
    # is a property of JS objects and not of the query format, which has an
    # order and keeps it — so the pairs are compared as sets.
    if label.startswith("qpar"):
        return sorted_pairs(readable(want)) == sorted_pairs(readable(mine))
    if label.startswith("qstr"):
        return (sorted(readable(want).split("&")) ==
                sorted(readable(mine).split("&")))
    # A refusal on both sides counts as agreement: Python names an exception
    # class and these packages say what is wrong in a sentence. What is
    # compared is *which* inputs are refused, which is the part a caller can
    # depend on.
    theirs, ours = readable(want), readable(mine)
    if theirs.startswith("?") and ours.startswith("?"):
        return True
    return False


agree = differ = 0
shown = {}
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if same(want, mine, labels[i]):
        agree += 1
    else:
        differ += 1
        what = labels[i].split()[0]
        shown[what] = shown.get(what, 0) + 1
        if shown[what] <= 3:
            print(f"  {labels[i][:160]}")
            print(f"    theirs {readable(want)[:160]!r}")
            print(f"    grasm  {readable(mine)[:160]!r}")
for what, n in sorted(shown.items()):
    print(f"  {what}: {n} differing")
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
