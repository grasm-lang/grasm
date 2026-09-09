"""URI templates, against the suite the RFC's authors publish.

The suite gives a list of acceptable answers wherever RFC 6570 leaves the
order open — an associative array has no order, so every permutation of its
members is correct — and `false` where a template is malformed and must be
refused rather than guessed at.
"""
import json
import os
import subprocess
import sys


def hexs(text):
    return text.encode("utf-8").hex()


def render(value):
    """One variable, in the harness's argument spelling. `None` to skip."""
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, list):
        return "l:{}:" + "|".join(hexs(str(x)) for x in value)
    if isinstance(value, dict):
        return "p:{}:" + "|".join(hexs(k) + "," + hexs(str(v))
                                  for k, v in value.items())
    text = repr(value) if isinstance(value, float) else str(value)
    return "t:{}:" + hexs(text)


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    harness = os.path.join(root, "packages/uritdiff/src/main.gr")
    fixtures = os.path.join(root, "scripts/fixtures")
    files = ["uritemplate-spec-examples.json",
             "uritemplate-spec-examples-by-section.json",
             "uritemplate-extended-tests.json",
             "uritemplate-negative-tests.json"]
    cases = agree = differ = shown = 0
    for name in files:
        suite = json.load(open(os.path.join(fixtures, name)))
        for group, body in suite.items():
            argv = []
            for variable, value in body.get("variables", {}).items():
                shape = render(value)
                if shape is not None:
                    argv.append(shape.format(hexs(variable)))
            templates = [t for t, _ in body["testcases"]]
            argv += ["x:" + hexs(t) for t in templates]
            got = subprocess.run([grasm, "run", harness, "--"] + argv,
                                 cwd=root, capture_output=True, text=True)
            if got.returncode != 0:
                print(got.stderr[-1500:])
                sys.exit(1)
            lines = got.stdout.split("\n")
            for i, (template, want) in enumerate(body["testcases"]):
                cases += 1
                raw = lines[i] if i < len(lines) else "<missing>"
                mine = False if raw.startswith("error/") else \
                    bytes.fromhex(raw).decode("utf-8")
                if want is False:
                    ok = mine is False
                elif isinstance(want, list):
                    ok = mine in want
                else:
                    ok = mine == want
                if ok:
                    agree += 1
                else:
                    differ += 1
                    shown += 1
                    if shown <= 20:
                        print(f"  {group}: {template!r}")
                        print(f"    want: {want!r}")
                        print(f"    got:  {mine!r}")
    print(f"templates: {cases}   agreeing: {agree}   differing: {differ}")
    sys.exit(1 if differ else 0)


main()
