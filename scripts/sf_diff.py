"""HTTP structured fields against the working group's own test suite.

Each case gives a raw field value and either `must_fail`, or an expected
parse and a `canonical` spelling. Comparing canonical forms checks the
parse and the serialisation together: a parser that reads a field into the
wrong shape cannot write it back the way the suite says.
"""
import glob
import json
import os
import subprocess
import sys

def canonical_of(case):
    if case.get("must_fail"):
        return None
    # A case written as several strings is a field that arrived on several
    # header lines, which HTTP joins with a comma and a space.
    if "canonical" in case:
        return ", ".join(case["canonical"])
    return ", ".join(case["raw"])

def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    harness = os.path.join(root, "packages/sftest/src/main.gr")
    fixtures = os.path.join(root, "scripts/fixtures/structured-field-tests")
    cases = []
    for path in sorted(glob.glob(os.path.join(fixtures, "*.json"))):
        for case in json.load(open(path)):
            if case.get("header_type") not in ("item", "list", "dictionary"):
                continue
            cases.append((os.path.basename(path), case))
    argv = []
    for _file, case in cases:
        raw = ", ".join(case["raw"])
        argv += [case["header_type"], raw.encode("utf-8").hex()]
    lines = []
    STEP = 400
    for at in range(0, len(argv), STEP * 2):
        chunk = argv[at:at + STEP * 2]
        got = subprocess.run([grasm, "run", harness, "--"] + chunk,
                             capture_output=True, text=True)
        if got.returncode != 0:
            print(got.stderr[-2000:]); sys.exit(1)
        lines += got.stdout.split("\n")[:len(chunk) // 2]
    agree = differ = shown = 0
    for i, (path, case) in enumerate(cases):
        want = canonical_of(case)
        raw = lines[i]
        mine = None if raw == "error" else bytes.fromhex(raw).decode("utf-8")
        if mine == want:
            agree += 1
        else:
            differ += 1; shown += 1
            if shown <= 20:
                print(f"  {path}: {case['name']}")
                print(f"    raw   {case['raw']!r} ({case['header_type']})")
                print(f"    want  {want!r}")
                print(f"    grasm {mine!r}")
    print(f"fields: {len(cases)}   agreeing: {agree}   differing: {differ}")
    sys.exit(1 if differ else 0)

main()
