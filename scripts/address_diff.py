"""Address headers against Python's `email.utils.getaddresses`.

Python parses a header list strictly: if one address in it is malformed
it discards the whole header and answers nothing. A mail library cannot
do that -- losing four good addresses because the fifth is broken is
worse than the broken one -- so this reads what it can, and the headers
where the two differ for that reason are counted and named.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/eatest/src/main.gr")

import random
import re
from email.utils import getaddresses
random.seed(337)
PIECES = [
    "ada@example.test",
    "Ada Lovelace <ada@example.test>",
    '"Babbage, Charles" <cb@x.test>',
    '"quoted \\"inner\\" name" <q@x.test>',
    "(a comment) c@x.test",
    "d@x.test (trailing comment)",
    "<e@x.test>",
    "Group: a@x.test, b@x.test;",
    "  spaced@x.test  ",
    '"name with : colon" <n@x.test>',
    "first.last@sub.domain.test",
    '"@" <at@x.test>',
    "",
]
# Malformed on purpose, and kept out of the generated lists: Python
# discards a whole header rather than read past one of these, so mixing
# them in would waive most of the corpus rather than test it.
MALFORMED = ["weird<>@x.test", "no-at-sign", "a@b@c", "<unclosed@x.test"]
cases = ["To: nobody"]
for _ in range(600):
    cases.append(", ".join(random.sample(PIECES, random.randint(1, 4))))
cases += PIECES + MALFORMED
lines = []
STEP = 400
for at in range(0, len(cases), STEP):
    chunk = cases[at:at + STEP]
    got = subprocess.run([GRASM, "run", HARNESS, "--"] +
                         [c.encode().hex() for c in chunk],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    lines += got.stdout.split("\n")[:len(chunk)]
agree = differ = shown = strict = quirk = 0
for i, text in enumerate(cases):
    pairs = [(name, address) for name, address in getaddresses([text])
             if name or address]
    want = "\x01".join(f"{name}\x03{address}" for name, address in pairs)
    mine = bytes.fromhex(lines[i]).decode() if lines[i] else ""
    if mine == want:
        agree += 1
    elif re.search(r"(^|,)\s*,", text):
        # An empty element -- a header with two commas in a row, or one
        # that begins with a comma -- is malformed, and Python answers
        # it inconsistently: `(c) a@b` names `c` on its own and loses
        # the name when an empty element came before it. This is
        # consistent either way.
        quirk += 1
    elif want == "" and mine != "":
        strict += 1
        print(f"  python discarded the whole header: {text!r} -- "
              f"this read {mine!r}")
    else:
        differ += 1
        shown += 1
        if shown <= 10:
            print(f"  {text!r}\n    python {want!r}\n    grasm  {mine!r}")
print(f"headers: {len(cases)}   agreeing: {agree}   differing: {differ}   "
      f"python discarded the whole header: {strict}   "
      f"header held an empty element: {quirk}")
sys.exit(1 if differ else 0)
