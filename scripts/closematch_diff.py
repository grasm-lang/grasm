"""Close matches against `difflib`, including the auto-junk cutoff."""
import difflib
import os
import random
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/cmtest/src/main.gr")

random.seed(3)
pairs = [("", ""), ("abc", "abc"), ("abc", ""), ("", "abc"),
         ("push", "pull"), ("push", "punt"), ("improt", "import"),
         ("kitten", "sitting"), ("abcdefg", "gfedcba"), ("aaaa", "aa"),
         ("the quick brown fox", "the quick brown cat"),
         # 199 and 200 straddle the length at which difflib starts dropping
         # over-common elements, which changes the ratio rather than only
         # the speed.
         ("a" * 199, "a" * 199), ("a" * 200, "a" * 200),
         ("a" * 201 + "b", "a" * 201 + "c"), ("x" * 250, "x" * 250),
         (" " * 300 + "hello", " " * 300 + "hella")]
for _ in range(30):
    pairs.append(("".join(random.choice("abcde")
                          for _ in range(random.randrange(0, 40))),
                  "".join(random.choice("abcde")
                          for _ in range(random.randrange(0, 40)))))
for _ in range(6):
    pairs.append(("".join(random.choice("ab ")
                          for _ in range(random.randrange(200, 320))),
                  "".join(random.choice("ab ")
                          for _ in range(random.randrange(200, 320)))))

args = []
expected = []
labels = []
for a, b in pairs:
    matcher = difflib.SequenceMatcher(None, a, b)
    args += ["matched", ":" + a.encode().hex(), ":" + b.encode().hex()]
    expected.append(str(sum(m.size for m in matcher.get_matching_blocks())))
    labels.append(f"matched {a[:20]!r} {b[:20]!r}")
    args += ["quick", ":" + a.encode().hex(), ":" + b.encode().hex()]
    expected.append(str(round(matcher.quick_ratio() * 1000000)))
    labels.append(f"quick {a[:20]!r} {b[:20]!r}")

POOL = ["import", "impart", "report", "hello", "help", "python", "status",
        "stat", "stash", "commit", "commits", "checkout", "check", "push",
        "pull", "branch", "brunch", "blame", "bisect", "clone"]
for word in ("improt", "hlelo", "pyton", "stat", "xyz", "commit", "chekout",
             "push", "a", "", "branchh"):
    for cutoff, count in ((60, 3), (60, 1), (50, 5), (80, 3), (30, 20),
                          (0, 3)):
        args += ["close", "%d,%d" % (cutoff, count),
                 ":" + word.encode().hex(),
                 ":" + "\t".join(POOL).encode().hex()]
        expected.append(",".join(
            difflib.get_close_matches(word, POOL, count, cutoff / 100)))
        labels.append(f"close {word!r} cutoff {cutoff} n {count}")

ran = subprocess.run([GRASM, "run", HARNESS, "--"] + args,
                     capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")
agree = differ = shown = 0
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print("  case:", labels[i])
            print("    difflib", want)
            print("    grasm  ", mine)
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
