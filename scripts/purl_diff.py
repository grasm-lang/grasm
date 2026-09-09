"""Package URLs against the purl-spec project's own test suite.

Two of the 586 cases contradict the specification and the suite's own
maven file, and are listed below rather than quietly passed: the
specification says a qualifier key "must be lowercased", and
`maven-test.json` parses `?repositorY_url=...` into `repository_url`,
while `gem-test.json` and `rpm-test.json` expect `?Platform=` and
`?Arch=` to be refused outright. This follows the specification. The two
cases are counted and named, and any *other* disagreement fails the gate.
"""
import collections
import glob
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/purltest/src/main.gr")
FIXTURES = os.path.join(ROOT, "scripts/fixtures/purl")
suite = []
for path in sorted(glob.glob(os.path.join(FIXTURES, "*.json"))):
    payload = json.load(open(path))
    tests = payload["tests"] if isinstance(payload, dict) else payload
    for test in tests:
        test["file"] = path.split("/")[-1]
        suite.append(test)
requests = []
for test in suite:
    if test["test_type"] == "parse" or test["test_type"] == "validate":
        requests.append("parse\x01" + str(test["input"]))
    else:
        one = test["input"]
        pairs = one.get("qualifiers") or {}
        requests.append("build\x01" + (one.get("type") or "") + "\x01" +
                        (one.get("namespace") or "") + "\x01" +
                        (one.get("name") or "") + "\x01" +
                        (one.get("version") or "") + "\x01" +
                        "&".join(f"{k}={v}" for k, v in pairs.items()) +
                        "\x01" + (one.get("subpath") or ""))
lines = []
STEP = 200
for at in range(0, len(requests), STEP):
    chunk = requests[at:at + STEP]
    got = subprocess.run([GRASM, "run", HARNESS, "--"] +
                         [r.encode().hex() for r in chunk],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    lines += got.stdout.split("\n")[:len(chunk)]
CONTRADICTS_THE_SPECIFICATION = {
    ("gem-test.json", "pkg:gem/jruby-launcher@1.1.2?Platform=java"),
    ("rpm-test.json",
     "pkg:Rpm/fedora/curl@7.50.3-1.fc25?Arch=i386&Distro=fedora-25"),
}
agree = differ = known = 0
shown = 0
by_file = collections.Counter()
for i, test in enumerate(suite):
    raw = lines[i]
    mine = None if raw == "error" else bytes.fromhex(raw).decode().split("\x02")
    if test["expected_failure"]:
        ok = mine is None
        want = "error"
        got_text = "parsed" if mine else "error"
    elif test["test_type"] == "parse":
        want_fields = test["expected_output"]
        pairs = want_fields.get("qualifiers") or {}
        want = [want_fields.get("type") or "", want_fields.get("namespace") or "",
                want_fields.get("name") or "", want_fields.get("version") or "",
                "&".join(f"{k}={v}" for k, v in sorted(pairs.items())),
                want_fields.get("subpath") or ""]
        ok = mine is not None and mine[:6] == want
        got_text = mine[:6] if mine else "error"
    else:
        want = test["expected_output"]
        ok = mine is not None and mine[6] == want
        got_text = mine[6] if mine else "error"
    if ok:
        agree += 1
    elif (test["file"], test["input"]) in CONTRADICTS_THE_SPECIFICATION:
        known += 1
        print(f"  known: {test['file']} keeps a qualifier key's case; the "
              f"specification lowercases it -- {test['input']!r}")
    else:
        differ += 1
        by_file[test["file"]] += 1
        shown += 1
        if shown <= 40:
            print(f"  [{test['file']} {test['test_type']}] "
                  f"{json.dumps(test['input'])[:70]}\n    want {want}\n"
                  f"    got  {got_text}")
print(f"cases: {len(suite)}   agreeing: {agree}   differing: {differ}   "
      f"contradicting the specification: {known}")
sys.exit(1 if differ else 0)
if by_file:
    print("  by file:", dict(by_file.most_common()))
