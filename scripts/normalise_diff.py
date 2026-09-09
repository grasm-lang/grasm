"""The four normalisation forms, against NormalizationTest.txt.

Each line of that file is five columns, and the file's own header states
what must hold between them:

    c2 == NFC(c1)  == NFC(c2)  == NFC(c3)     c4 == NFC(c4)  == NFC(c5)
    c3 == NFD(c1)  == NFD(c2)  == NFD(c3)     c5 == NFD(c4)  == NFD(c5)
    c4 == NFKC(c1) == NFKC(c2) == ... == NFKC(c5)
    c5 == NFKD(c1) == NFKD(c2) == ... == NFKD(c5)

All of it is checked, which is about four hundred thousand comparisons and
takes under a minute — the arguments go to the harness in batches because a
process can only be handed so many.
"""
import os
import subprocess
import sys

BATCH = 1500


def load(path):
    cases = []
    for line in open(path, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if not line or line.startswith("@"):
            continue
        columns = [c.strip() for c in line.split(";") if c.strip()]
        if len(columns) < 5:
            continue
        cases.append([[int(x, 16) for x in c.split()] for c in columns[:5]])
    return cases


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    harness = os.path.join(root, "packages/normtest/src/main.gr")
    fixture = os.path.join(root, "scripts/fixtures/NormalizationTest.txt")

    checks = []
    for c1, c2, c3, c4, c5 in load(fixture):
        for source in (c1, c2, c3):
            checks.append(("nfc", source, c2))
            checks.append(("nfd", source, c3))
        for source in (c4, c5):
            checks.append(("nfc", source, c4))
            checks.append(("nfd", source, c5))
        for source in (c1, c2, c3, c4, c5):
            checks.append(("nfkc", source, c4))
            checks.append(("nfkd", source, c5))

    argv = []
    for form, source, _want in checks:
        argv += [form, ",".join(f"{code:x}" for code in source)]
    lines = []
    for at in range(0, len(argv), BATCH * 2):
        chunk = argv[at:at + BATCH * 2]
        got = subprocess.run([grasm, "run", harness, "--"] + chunk, cwd=root,
                             capture_output=True, text=True)
        if got.returncode != 0:
            print(got.stderr[-1500:])
            sys.exit(1)
        lines += got.stdout.split("\n")[:len(chunk) // 2]

    agree = differ = shown = 0
    for i, (form, source, want) in enumerate(checks):
        mine = [int(x, 16) for x in lines[i].split(",") if x]
        if mine == want:
            agree += 1
        else:
            differ += 1
            shown += 1
            if shown <= 15:
                print(f"  {form} of " +
                      " ".join(f"{code:04X}" for code in source))
                print("    unicode " + " ".join(f"{c:04X}" for c in want))
                print("    grasm   " + " ".join(f"{c:04X}" for c in mine))
    print(f"forms: {len(checks)}   agreeing: {agree}   differing: {differ}")
    sys.exit(1 if differ else 0)


main()
