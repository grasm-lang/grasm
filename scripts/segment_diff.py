"""Grapheme cluster, word and sentence boundaries, against the Unicode
Consortium's own test files.

Each line of those files is a string written as `÷ 0061 × 0300 ÷`, where a
division sign is a boundary and a multiplication sign is not. The boundary
positions are what a segmenter has to agree about.
"""
import os
import subprocess
import sys


def load(path):
    cases = []
    for line in open(path, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if not line:
            continue
        codes, breaks = [], [0]
        for piece in line.replace("÷", " D ").replace("×", " X ").split():
            if piece == "D":
                if codes:
                    breaks.append(len(codes))
            elif piece != "X":
                codes.append(int(piece, 16))
        cases.append((codes, sorted(set(breaks))))
    return cases


def run(grasm, root, harness, fixture, label):
    cases = load(fixture)
    argv = [",".join(f"{code:x}" for code in codes) for codes, _ in cases]
    got = subprocess.run([grasm, "run", harness, "--"] + argv, cwd=root,
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-1500:])
        sys.exit(1)
    lines = got.stdout.split("\n")
    agree = differ = shown = 0
    for i, (codes, want) in enumerate(cases):
        mine = [int(x) for x in lines[i].split(",") if x != ""]
        if mine == want:
            agree += 1
        else:
            differ += 1
            shown += 1
            if shown <= 15:
                print("  " + " ".join(f"{code:04X}" for code in codes))
                print(f"    unicode {want}")
                print(f"    grasm   {mine}")
    print(f"{label}: {len(cases)}   agreeing: {agree}   differing: {differ}")
    return differ


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    fixtures = os.path.join(root, "scripts/fixtures")
    bad = run(grasm, root, os.path.join(root, "packages/gbtest/src/main.gr"),
              os.path.join(fixtures, "GraphemeBreakTest.txt"), "clusters")
    bad += run(grasm, root, os.path.join(root, "packages/wbtest/src/main.gr"),
               os.path.join(fixtures, "WordBreakTest.txt"), "words")
    bad += run(grasm, root, os.path.join(root, "packages/sbtest/src/main.gr"),
               os.path.join(fixtures, "SentenceBreakTest.txt"), "sentences")
    sys.exit(1 if bad else 0)


main()
