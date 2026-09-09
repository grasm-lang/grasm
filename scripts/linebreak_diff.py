"""Line break opportunities against UAX #14's own test file."""
import subprocess
import sys

def load(path):
    cases = []
    for line in open(path, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if not line:
            continue
        codes, breaks = [], []
        for piece in line.replace("÷", " D ").replace("×", " X ").split():
            if piece == "D":
                if codes:
                    breaks.append(len(codes))
            elif piece != "X":
                codes.append(int(piece, 16))
        cases.append((codes, sorted(set(breaks))))
    return cases

import os
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
harness = os.path.join(root, "packages/lbtest/src/main.gr")
path = os.path.join(root, "scripts/fixtures/LineBreakTest.txt")
cases = load(path)
argv = [",".join(f"{c:x}" for c in codes) for codes, _ in cases]
lines = []
STEP = 2000
for at in range(0, len(argv), STEP):
    chunk = argv[at:at + STEP]
    got = subprocess.run([grasm, "run", harness, "--"] + chunk,
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    lines += got.stdout.split("\n")[:len(chunk)]
agree = differ = shown = 0
for i, (codes, want) in enumerate(cases):
    mine = [int(x) for x in lines[i].split(",") if x != ""]
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 12:
            print("  " + " ".join(f"{c:04X}" for c in codes))
            print(f"    unicode {want}")
            print(f"    grasm   {mine}")
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
