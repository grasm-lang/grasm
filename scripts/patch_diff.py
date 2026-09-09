"""Both JSON patch formats, against the suites their specifications publish.

RFC 6902 has a test suite the format's authors maintain; RFC 7386 has a
table in its own appendix, which is committed here as the RFC printed it
and read out of that text rather than retyped.
"""
import json
import os
import subprocess
import sys


def hexs(text):
    return text.encode("utf-8").hex()


def merge_cases(path):
    """Appendix A's three columns.

    The column boundaries are found by looking for the runs of space every
    line shares rather than by counting characters: the appendix's header
    row is one column off from its own data.
    """
    lines = open(path, encoding="utf-8").read().split("\n")[2:]
    width = max(len(line) for line in lines)
    lines = [line.ljust(width) for line in lines]
    blank = [all(line[i] == " " for line in lines if line.strip())
             for i in range(width)]
    runs, i = [], 0
    while i < width:
        if blank[i]:
            j = i
            while j < width and blank[j]:
                j += 1
            if j - i >= 3:
                runs.append((i, j))
            i = j
        else:
            i += 1
    starts = [0] + [b for a, b in runs if a > 0 and b < width]
    bounds = list(zip(starts, starts[1:] + [width]))[:3]
    out, cur = [], ["", "", ""]
    for line in lines:
        if not line.strip():
            if any(part.strip() for part in cur):
                out.append(cur)
                cur = ["", "", ""]
            continue
        for k, (a, b) in enumerate(bounds):
            cur[k] += " " + line[a:b]
    if any(part.strip() for part in cur):
        out.append(cur)
    return [[json.loads(part) for part in record] for record in out]


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    harness = os.path.join(root, "packages/patchdiff/src/main.gr")
    fixtures = os.path.join(root, "scripts/fixtures")
    cases = []
    for name in ["jsonpatch-tests.json", "jsonpatch-spec-tests.json"]:
        for case in json.load(open(os.path.join(fixtures, name))):
            if case.get("disabled") or "doc" not in case or \
                    "patch" not in case:
                continue
            want = None if "error" in case else \
                case.get("expected", case["doc"])
            cases.append(("6902", case["doc"], case["patch"], want,
                          case.get("comment", "")))
    for before, patch, after in merge_cases(
            os.path.join(fixtures, "rfc7386-appendix-a.txt")):
        cases.append(("7386", before, patch, after, "RFC 7386 appendix A"))

    argv = []
    for kind, doc, patch, _want, _comment in cases:
        argv += [kind, hexs(json.dumps(doc)), hexs(json.dumps(patch))]
    got = subprocess.run([grasm, "run", harness, "--"] + argv, cwd=root,
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-1500:])
        sys.exit(1)
    lines = got.stdout.split("\n")

    agree = differ = shown = 0
    for i, (kind, doc, patch, want, comment) in enumerate(cases):
        raw = lines[i] if i < len(lines) else "<missing>"
        mine = None if raw.startswith("error") else \
            json.loads(bytes.fromhex(raw).decode("utf-8"))
        ok = (mine is None) if want is None else \
            (mine is not None and mine == want)
        if ok:
            agree += 1
        else:
            differ += 1
            shown += 1
            if shown <= 20:
                print(f"  RFC {kind}: {comment!r}")
                print(f"    doc:   {json.dumps(doc)}")
                print(f"    patch: {json.dumps(patch)}")
                print("    want:  " + ("<error>" if want is None
                                       else json.dumps(want)))
                print("    got:   " + ("<error>" if mine is None
                                       else json.dumps(mine)))
    print(f"patches: {len(cases)}   agreeing: {agree}   differing: {differ}")
    sys.exit(1 if differ else 0)


main()
