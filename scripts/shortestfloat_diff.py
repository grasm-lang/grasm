"""The shortest round-tripping decimal against Python's `repr`.

Built natively: every candidate length is an exact big-integer division, and
the corpus is twenty thousand doubles.
"""
import os
import random
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))


def bits_of(value):
    return struct.unpack("<q", struct.pack("<d", value))[0]


random.seed(101)
values = [0.1, 0.3, 0.1 + 0.2, 1.0, -1.0, 100.0, 1e16, 1e15, 1e17, 1e-4,
          1e-5, 5e-324, 1.7976931348623157e308, 0.0, -0.0, -2.5, 1 / 3,
          1e-3, 123456789.0,
          # The smallest normal, the largest subnormal, and the pair of
          # consecutive doubles either side of 2^53.
          2.2250738585072014e-308, 2.2250738585072011e-308,
          9007199254740992.0, 9007199254740993.0,
          0.5, 0.25, 1e22, 1e23, 8.98846567431158e307, 5e-320, 4.9e-324,
          1.0000000000000002]
for exponent in range(-320, 309):
    try:
        values.append(float("1e%d" % exponent))
    except Exception:
        pass
for _ in range(18000):
    values.append(struct.unpack("<d", struct.pack("<Q",
                                                  random.getrandbits(64)))[0])
for _ in range(1000):
    values.append(random.uniform(-1e6, 1e6))
for _ in range(1000):
    values.append(random.uniform(0, 1) * (10 ** random.randrange(-300, 300)))
values = [v for v in values if v == v and abs(v) != float("inf")]

cases = []
argv = []
for value in values:
    argv += ["repr", str(bits_of(value))]
    cases.append(("repr", value, repr(value)))
for value in values[:400]:
    for places in (0, 1, 2, 6, 17):
        argv += ["fixed:%d" % places, str(bits_of(value))]
        cases.append(("fixed%d" % places, value, "%.*f" % (places, value)))

work = tempfile.mkdtemp()
harness = os.path.join(work, "sftest2")
built = subprocess.run([GRASM, "build", "packages/sftest2/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

CHUNK = 1000
lines = []
for start in range(0, len(argv), CHUNK):
    ran = subprocess.run([harness] + argv[start:start + CHUNK],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines += ran.stdout.split("\n")[:-1]

agree = differ = shown = 0
for i, (what, value, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {what} of {value!r}")
            print(f"    python {want}")
            print(f"    grasm  {mine}")
print(f"doubles: {len(values)}   cases: {len(cases)}   agreeing: {agree}   "
      f"differing: {differ}")
sys.exit(1 if differ else 0)
