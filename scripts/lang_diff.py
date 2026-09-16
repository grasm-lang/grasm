"""`jsnumber`, `tcllist`, `luaformat`, `rubymarshal` and `perlpack` against
node, tclsh, lua, ruby and perl.

Built natively. Every answer comes from running the language on this machine.
"""
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lang_corpus import build

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

work = tempfile.mkdtemp()
harness = os.path.join(work, "langtest")
built = subprocess.run([GRASM, "build", "packages/langtest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:])
    sys.exit(1)

argv, expected, labels = build()
if not expected:
    print("no language runtime to compare against — nothing was checked")
    sys.exit(1)

# Every command is exactly three words, so a chunk that is a multiple of three
# never splits one.
CHUNK = 1500
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([harness] + argv[start:start + CHUNK],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:])
        sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]


def readable(text):
    try:
        return bytes.fromhex(text).decode("utf-8", "replace")
    except Exception:
        return text


def same(want, mine, label):
    if want == mine:
        return True
    # A refusal on both sides counts as agreement for the Tcl list reader:
    # the corpus records Tcl's "?" and this package says why in its own
    # words. What is compared is which strings are refused.
    if label.startswith("tsplt"):
        return want == "3f" and readable(mine).startswith("?")
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
