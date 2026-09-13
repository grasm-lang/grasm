"""Unified and context diffs against `difflib`, line for line.

A diff whose hunk header is off by one is rejected by `patch`, so the
comparison is the whole output rather than a summary of it.
"""
import difflib
import os
import random
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

random.seed(29)
PAIRS = [
    ("", ""), ("a\n", ""), ("", "a\n"), ("a\n", "a\n"),
    ("a\nb\nc\n", "a\nB\nc\n"),
    ("one\ntwo\nthree\nfour\nfive\n", "one\ntwo\nTHREE\nfour\nfive\n"),
    ("\n".join(str(i) for i in range(30)) + "\n",
     "\n".join(str(i) for i in range(30) if i != 15) + "\n"),
    ("\n".join(str(i) for i in range(40)) + "\n",
     "\n".join("X" if i in (3, 20, 37) else str(i)
               for i in range(40)) + "\n"),
    ("x\n" * 10, "x\n" * 10 + "y\n"),
    ("y\n" + "x\n" * 10, "x\n" * 10),
    # No trailing newline on either side, which is where a diff that assumes
    # one puts a line ending the file never had.
    ("no newline at end", "no newline at end either"),
]
for _ in range(25):
    original = [random.choice("abcde") + "\n"
                for _ in range(random.randrange(0, 25))]
    changed = list(original)
    for _ in range(random.randrange(0, 6)):
        if not changed:
            break
        where = random.randrange(len(changed))
        action = random.choice("dir")
        if action == "d":
            del changed[where]
        elif action == "i":
            changed.insert(where, random.choice("vwxyz") + "\n")
        else:
            changed[where] = random.choice("vwxyz") + "\n"
    PAIRS.append(("".join(original), "".join(changed)))

cases = []
for before, after in PAIRS:
    a = before.splitlines(keepends=True)
    b = after.splitlines(keepends=True)
    for n in (0, 1, 3, 5):
        cases.append(("unified", a, b, n, "\x01".join(
            difflib.unified_diff(a, b, "a", "b", n=n))))
        cases.append(("context", a, b, n, "\x01".join(
            difflib.context_diff(a, b, "a", "b", n=n))))
    codes = ["%s %d %d %d %d" % code for code
             in difflib.SequenceMatcher(None, a, b).get_opcodes()]
    cases.append(("opcodes", a, b, 3, "\x01".join(codes)))

argv = []
for what, a, b, n, _ in cases:
    argv += [what, ":" + "\x01".join(a).encode().hex(),
             ":" + "\x01".join(b).encode().hex(), str(n)]

# Chunked in whole cases: each one is four arguments and a split inside a
# group would silently lose alignment.
CHUNK = 1000
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([GRASM, "run", "packages/dttest/src/main.gr", "--"]
                         + argv[start:start + CHUNK], cwd=ROOT,
                        capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]

agree = differ = shown = 0
for i, (what, a, b, n, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else ""
    try:
        readable = bytes.fromhex(mine).decode("utf-8", "replace")
    except Exception:
        readable = mine
    if readable == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {what} n={n}")
            print(f"    difflib {want[:110]!r}")
            print(f"    grasm   {readable[:110]!r}")
print(f"pairs: {len(PAIRS)}   cases: {len(cases)}   agreeing: {agree}   "
      f"differing: {differ}")
sys.exit(1 if differ else 0)
