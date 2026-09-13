"""Pickles against Python's own `pickle`, at every protocol.

Every value is written by Python at protocols 0 to 5 and read back here.
Floats are compared by value rather than by text: Python's `repr` prints the
shortest decimal that round-trips and `picklefile` does not claim to.
"""
import datetime
import os
import pickle
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

shared = [1, 2, 3]
cyclic_list = [1]
cyclic_list.append(cyclic_list)
cyclic_dict = {}
cyclic_dict["self"] = cyclic_dict

VALUES = [
    None, True, False, 0, 1, -1, 127, 128, 255, 256, 65535, 65536,
    2 ** 31 - 1, -2 ** 31, 2 ** 62, -(2 ** 62), 2 ** 63 - 1, -(2 ** 63),
    0.0, 1.5, -2.25, 1e10, 1e-10, 3.141592653589793, 1e300, 5e-324,
    "", "hello", "café", "日本語", "a'b\"c", "line\nbreak\ttab",
    b"", b"bytes", bytes(range(256)),
    [], [1, 2, 3], [1, "two", 3.0, None, True],
    (), (1,), (1, 2), (1, 2, 3), (1, 2, 3, 4),
    {}, {"a": 1}, {"a": 1, "b": [1, 2], "c": {"d": None}},
    set(), {1, 2, 3}, frozenset([1, 2]),
    [[1, [2, [3, [4]]]]],
    {"nested": {"list": [1, (2, 3), {"k": b"v"}]}},
    list(range(300)),
    # The same list twice, which is what the memo is for.
    [shared, shared],
]
# Values that must be refused: ones that need a callable, and ones that
# contain themselves.
REFUSED = [datetime.date(2020, 1, 1), complex(1, 2), object, cyclic_list,
           cyclic_dict]

cases = []
for value in VALUES:
    for protocol in range(6):
        try:
            raw = pickle.dumps(value, protocol=protocol)
        except Exception:
            continue
        cases.append((repr(value)[:40], protocol, raw, repr(value),
                      isinstance(value, float)))
for value in REFUSED:
    for protocol in range(6):
        try:
            raw = pickle.dumps(value, protocol=protocol)
        except Exception:
            continue
        cases.append((f"refuse {value!r}"[:40], protocol, raw, "error",
                      False))
cases.append(("not a pickle", 0, b"hello there", "error", False))

argv = []
for _, _, raw, _, _ in cases:
    argv += ["render", ":" + raw.hex()]
ran = subprocess.run([GRASM, "run", "packages/pktest/src/main.gr", "--"]
                     + argv, cwd=ROOT, capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")

agree = differ = shown = 0
for i, (label, protocol, _, want, is_float) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    if want == "error":
        matched = mine == "error"
        readable = mine
    else:
        try:
            readable = bytes.fromhex(mine).decode("utf-8", "replace")
        except Exception:
            readable = mine
        if is_float:
            try:
                matched = float(readable) == float(want)
            except ValueError:
                matched = False
        else:
            matched = readable == want
    if matched:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  protocol {protocol} {label}")
            print(f"    python {want[:70]}")
            print(f"    grasm  {readable[:70]}")
print(f"values: {len(VALUES) + len(REFUSED)}   cases: {len(cases)}   "
      f"agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
