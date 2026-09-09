"""RPM version ordering against `rpm`'s own `tests/rpmvercmp.at`.

The suite is the standard's expectations, caret cases and all: 91 pairs
that between them cover every branch of `rpmvercmp`, including the two
separators that are not separators.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/rpmtest/src/main.gr")
SUITE = os.path.join(ROOT, "scripts/fixtures/rpmvercmp.at")

def main():
    suite = []
    for line in open(SUITE):
        found = re.match(r"^RPMVERCMP\((.*?),\s*(.*?),\s*(-?\d+)\)", line.strip())
        if found:
            suite.append((found.group(1), found.group(2), int(found.group(3))))
    if not suite:
        print("the vendored rpmvercmp suite is empty")
        return 1
    argv = []
    for left, right, _ in suite:
        argv += [left, right]
    got = subprocess.run([GRASM, "run", HARNESS, "--"] + argv,
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:])
        return 1
    lines = got.stdout.split("\n")
    agree = differ = 0
    for i, (left, right, want) in enumerate(suite):
        mine = int(lines[i]) if lines[i].lstrip("-").isdigit() else None
        if mine == want:
            agree += 1
        else:
            differ += 1
            if differ <= 8:
                print(f"  {left!r} vs {right!r}: rpm {want}, grasm {mine}")
    print(f"pairs: {len(suite)}   agreeing: {agree}   differing: {differ}")
    return 1 if differ else 0

if __name__ == "__main__":
    sys.exit(main())
