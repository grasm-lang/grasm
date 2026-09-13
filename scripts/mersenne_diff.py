"""MT19937 against Python's `random`, which is the same generator.

Seeds are every value an `i64` can hold, because that is what the package
takes; Python accepts integers of any size and those are out of range here
rather than quietly different.
"""
import os
import random
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

SEEDS = [0, 1, 2, 12345, 42, 2 ** 31, 2 ** 32, 2 ** 32 + 1, 2 ** 62,
         2 ** 63 - 1, -7, -2 ** 62, 999999937, 123456789012345]
DRAWS = [
    ("bits32", lambda r: r.getrandbits(32)),
    ("bits7", lambda r: r.getrandbits(7)),
    ("bits53", lambda r: r.getrandbits(53)),
    ("below", lambda r: r.randrange(1000)),
    # Compared as the 53-bit integer the double came from, so the comparison
    # does not depend on how either side prints a float.
    ("double", lambda r: round(r.random() * 9007199254740992)),
]

argv = []
expected = []
labels = []
for seed in SEEDS:
    for name, draw in DRAWS:
        generator = random.Random(seed)
        expected.append(",".join(str(draw(generator)) for _ in range(20)))
        argv += [name, str(seed), "20"]
        labels.append(f"{name} seed {seed}")

ran = subprocess.run([GRASM, "run", "packages/mttest/src/main.gr", "--"]
                     + argv, cwd=ROOT, capture_output=True, text=True)
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
            print(f"  {labels[i]}")
            print(f"    python {want[:70]}")
            print(f"    grasm  {mine[:70]}")
print(f"streams: {len(expected)}   numbers: {len(expected) * 20}   "
      f"agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
