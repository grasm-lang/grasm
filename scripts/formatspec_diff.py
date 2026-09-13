"""Format specifications against Python's own `format()`.

Specs are a seeded random walk over the grammar plus a curated list of the
awkward ones, and every spec is tried against every value — including the
combinations Python refuses, which have to be refused here too.

Built natively: formatting a float is an exact big-integer division per
candidate length, and there are twenty thousand cases.
"""
import os
import random
import struct
import subprocess
import sys
import tempfile

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
# The float presentation types, which `format_int` refuses and
# `format_float` handles.
FLOAT_SPECS = ["", "f", "e", "E", "g", "G", "%", "n", ".0f", ".2f", ".6f",
               ".17f", ".0e", ".3e", ".1g", ".2g", ".3g", ".6g", ".10g",
               "#g", "#.3g", "10.2f", "<10.2f", ">10.2f", "^10.2f",
               "=10.2f", "010.2f", "+.2f", " .2f", "-.2f", ",.2f", "_.2f",
               ",f", "012,.2f", ".2%", "10.1%", "#.0f", "#.0e", "#.0g",
               ".0", ".3", "20", "020", "*^12.3f", "+e", "0=+12.3e"]
FLOATS = [0.0, -0.0, 1.0, -1.0, 0.5, 1.5, -2.25, 3.141592653589793, 1 / 3,
          99.9, 0.1, 0.0001, 1e-5, 1e16, 1e15, 123456.789, -123456.789,
          1e300, 5e-324, 2.5, 3.5, 1234.5, 0.125, float("inf"),
          float("-inf"), float("nan"), 1e100, 1.7976931348623157e308]
for _ in range(40):
    FLOATS.append(random.uniform(-1e6, 1e6))
for _ in range(20):
    FLOATS.append(random.uniform(0, 1) * 10 ** random.randrange(-20, 20))
SPECS = sorted(specs)

INTEGERS = [0, 1, -1, 7, 42, -42, 255, -255, 1234, 1234567, -1234567,
            2 ** 31, -(2 ** 31), 2 ** 62, -(2 ** 62)]
STRINGS = ["", "a", "hello", "café", "日本語", "a longer piece of text"]

cases = []
for value in FLOATS:
    for spec in FLOAT_SPECS:
        try:
            want = format(value, spec)
        except Exception:
            want = "error"
        cases.append(("float", value, spec, want))
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
    if kind == "float":
        # Floats go in as their raw bit pattern, so nothing is lost on the
        # way to the harness.
        argv += ["float",
                 str(struct.unpack("<q", struct.pack("<d", value))[0]),
                 ":" + spec.encode().hex()]
    elif kind == "int":
        argv += ["int", str(value), ":" + spec.encode().hex()]
    else:
        argv += ["str", ":" + value.encode().hex(),
                 ":" + spec.encode().hex()]

# Chunked in whole cases: the argument list is far past what one exec takes,
# and a split inside a three-argument group would silently lose alignment.
work = tempfile.mkdtemp()
harness = os.path.join(work, "fstest")
built = subprocess.run([GRASM, "build", "packages/fstest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

# Every case is three arguments, so the chunk stays a multiple of three.
CHUNK = 999
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([harness] + argv[start:start + CHUNK],
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
print(f"specs: {len(SPECS) + len(FLOAT_SPECS)}   cases: {len(cases)}   "
      f"agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
