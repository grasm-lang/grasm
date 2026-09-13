"""Format specifications against Python's own `format()`.

Specs are a seeded random walk over the grammar plus a curated list of the
awkward ones, and every spec is tried against every value — including the
combinations Python refuses, which have to be refused here too.
"""
import os
import random
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

random.seed(17)
ALIGN = ["", "<", ">", "^", "=", "*<", "*>", "*^", "0=", ".>"]
SIGN = ["", "+", " ", "-"]
ALTERNATE = ["", "#"]
WIDTH = ["", "1", "5", "8", "12"]
GROUPING = ["", ",", "_"]
KIND = ["", "d", "b", "o", "x", "X", "n", "c", "s"]

specs = set()
for _ in range(700):
    specs.add(random.choice(ALIGN) + random.choice(SIGN)
              + random.choice(ALTERNATE) + random.choice(WIDTH)
              + random.choice(GROUPING) + random.choice(KIND))
specs |= {"08", "012,", "08x", "#010x", "#012_b", ".3", ".0", "5.2", "<5.2",
          "^9.3", "*^11,d", "0", "", "099", "x", ",", "_", "=8", "+08d",
          " 08d", "#b", "#o", "#x", "#X", "010b", "020_x", "+", "-", " ",
          "#", "^", "<", ">", "=", "20", "020", "0<8", "0>8", "0^8", ",d",
          "_x", "_b", "_o", "#_x", ".5s", "10.3s", "<10s", ">10s", "^10s",
          "c", "5c", ".2", "0=8", "#n", "#12n", ",n", "_n", "1_0"}
# The float presentation types are out of scope: Python converts an integer
# to a float for `.1f`, and correctly-rounded decimal conversion is a
# separate piece of work the package says it does not do.
SPECS = sorted(specs)

INTEGERS = [0, 1, -1, 7, 42, -42, 255, -255, 1234, 1234567, -1234567,
            2 ** 31, -(2 ** 31), 2 ** 62, -(2 ** 62)]
STRINGS = ["", "a", "hello", "café", "日本語", "a longer piece of text"]

cases = []
for value in INTEGERS:
    for spec in SPECS:
        try:
            want = format(value, spec)
        except Exception:
            want = "error"
        cases.append(("int", value, spec, want))
for value in STRINGS:
    for spec in SPECS:
        try:
            want = format(value, spec)
        except Exception:
            want = "error"
        cases.append(("str", value, spec, want))

argv = []
for kind, value, spec, _ in cases:
    if kind == "int":
        argv += ["int", str(value), ":" + spec.encode().hex()]
    else:
        argv += ["str", ":" + value.encode().hex(),
                 ":" + spec.encode().hex()]

# Chunked in whole cases: the argument list is far past what one exec takes,
# and a split inside a three-argument group would silently lose alignment.
CHUNK = 999
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([GRASM, "run", "packages/fstest/src/main.gr", "--"]
                         + argv[start:start + CHUNK], cwd=ROOT,
                        capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]

agree = differ = shown = 0
for i, (kind, value, spec, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    if want == "error":
        readable = mine
        matched = mine == "error"
    else:
        try:
            readable = bytes.fromhex(mine).decode("utf-8", "replace")
        except Exception:
            readable = mine
        matched = readable == want
    if matched:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  format({value!r}, {spec!r})")
            print(f"    python {want!r}")
            print(f"    grasm  {readable!r}")
print(f"specs: {len(SPECS)}   cases: {len(cases)}   agreeing: {agree}   "
      f"differing: {differ}")
sys.exit(1 if differ else 0)
