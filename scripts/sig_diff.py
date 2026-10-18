"""Signal transforms, windows and splines.

- `dct`: types I of the DCT and DST against gonum's dsp/fourier (FFTPACK's
  cost and sint); types II to IV, plain and orthonormal, against their
  sums written out in Python, scipy's conventions.
- `windowfn`: every window against gonum's dsp/window; Kaiser against its
  Bessel series in Python.
- `spline`: natural, clamped, not-a-knot, Akima and Fritsch-Butland fits
  against gonum's interp, values and derivatives, inside the data and past
  both ends.
- `dwt`: Haar and Daubechies-4 against the orthogonal transform matrix
  built in Python from the filter, LeGall 5/3 against JPEG 2000's integer
  lifting written out in Python (and exact reconstruction), CDF 9/7
  against its lifting steps and reconstruction to rounding.
"""
import json
import math
import os
import random
import struct
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle as gomods  # noqa: E402

rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="sigdiff")
harness = os.path.join(work, "sigtest")
built = subprocess.run([GRASM, "build", "packages/sigtest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
go = gomods.build(os.path.join(work, "gomods"))


def b(x):
    return str(struct.unpack("<q", struct.pack("<d", x))[0])


def f(s):
    # Go writes the bits unsigned, Grasm signed; both are the same 64 bits.
    return struct.unpack("<d", struct.pack("<Q", int(s) & 0xFFFFFFFFFFFFFFFF))[0]


def bits(v):
    return ",".join(b(x) for x in v)


cases, go_reqs = [], []


def close(got, want, scale, rel=1e-12):
    g = [f(x) for x in got.split(",")] if got else []
    return len(g) == len(want) and all(abs(a - c) <= rel * scale + 1e-300 for a, c in zip(g, want))


def check_vec(want, scale, rel=1e-12):
    return lambda got: None if close(got, want, scale, rel) else f"want {want[:4]}..."


def via_go(op, a, bb, req, rel, scale):
    cases.append([op, a, bb, None, len(go_reqs), rel, scale])
    go_reqs.append(req)


def signal(n):
    return [rng.choice([rng.uniform(-1, 1), rng.uniform(-1000, 1000), float(rng.randrange(-5, 5))]) for _ in range(n)]


for _ in range(150):
    x = signal(rng.randrange(2, 40))
    norm = sum(abs(v) for v in x) + 1
    via_go("dct1", bits(x), "", {"op": "dct1", "nums": x}, 1e-12, 2 * norm)
    via_go("dst1", bits(x), "", {"op": "dst1", "nums": x}, 1e-12, 2 * norm)
    n = len(x)
    for ortho in (False, True):
        d2 = [2 * sum(x[j] * math.cos(math.pi * (2 * j + 1) * k / (2 * n)) for j in range(n)) for k in range(n)]
        d3 = [x[0] + 2 * sum(x[j] * math.cos(math.pi * j * (2 * k + 1) / (2 * n)) for j in range(1, n)) for k in range(n)]
        d4 = [2 * sum(x[j] * math.cos(math.pi * (2 * j + 1) * (2 * k + 1) / (4 * n)) for j in range(n)) for k in range(n)]
        if ortho:
            d2 = [v * (math.sqrt(1 / (4 * n)) if k == 0 else math.sqrt(1 / (2 * n))) for k, v in enumerate(d2)]
            d3 = [x[0] / math.sqrt(n) + math.sqrt(2 / n) * sum(x[j] * math.cos(math.pi * j * (2 * k + 1) / (2 * n))
                                                             for j in range(1, n)) for k in range(n)]
            d4 = [v / 2 * math.sqrt(2 / n) for v in d4]
        tag = "ortho" if ortho else "plain"
        for op, w in (("dct2", d2), ("dct3", d3), ("dct4", d4)):
            cases.append([op, bits(x), tag, check_vec(w, 2 * norm), None, 0, 0])

WINDOWS = ["rectangular", "sine", "lanczos", "triangular", "hann", "bartlett_hann", "hamming", "blackman",
           "blackman_harris", "nuttall", "blackman_nuttall", "flat_top"]
for _ in range(10):
    for name in WINDOWS + ["gaussian", "tukey"]:
        n = rng.randrange(2, 80)
        p = rng.choice([0.1, 0.3, 0.5, 0.75, 1.0, 1.5, -0.1, 0.0])
        via_go("win", name, bits([n, p]), {"op": "win", "args": [name], "nums": [n, p]}, 1e-14, 1)


def i0(x):
    s, t, k = 1.0, 1.0, 1
    while t > 1e-17 * s and k < 500:
        t *= (x * x / 4) / (k * k)
        s += t
        k += 1
    return s


for _ in range(120):
    n, beta = rng.randrange(2, 80), rng.choice([0.0, 0.5, 5.0, 8.6, 14.0, rng.uniform(0, 20)])
    want = [i0(beta * math.sqrt(max(0.0, 1 - (2 * i / (n - 1) - 1) ** 2))) / i0(beta) for i in range(n)]
    cases.append(["win", "kaiser", bits([n, beta]), check_vec(want, 1, 1e-13), None, 0, 0])

for _ in range(80):
    for kind in ["natural", "clamped", "notaknot", "akima", "fritschbutland"]:
        n = rng.choice([2, 3, 4, 5, 8, 15])
        xs = sorted(rng.sample(range(-200, 200), n))
        xs = [v / rng.choice([1, 3, 7]) for v in xs]
        if rng.random() < 0.05:
            xs[1] = xs[0]
        ys = [rng.choice([rng.uniform(-10, 10), float(i), float(i * i), 1.0]) for i in range(n)]
        if rng.random() < 0.2:
            ys = sorted(ys)
        qs = [rng.uniform(xs[0] - 5, xs[-1] + 5) for _ in range(8)] + [xs[0], xs[-1], xs[n // 2]]
        nums = [float(n)] + xs + ys + qs
        scale = max(1.0, max(abs(v) for v in ys)) * max(1.0, max(abs(v) for v in xs))
        via_go("spline", kind, bits(nums), {"op": "spline", "args": [kind], "nums": nums}, 1e-9, scale)


def orth_matrix(h, n):
    g = [(-1) ** j * h[len(h) - 1 - j] for j in range(len(h))]
    rows = []
    for filt in (h, g):
        for k in range(n // 2):
            row = [0.0] * n
            for j in range(len(filt)):
                row[(2 * k + j) % n] += filt[j]
            rows.append(row)
    return rows


H = {"haar": [math.sqrt(0.5)] * 2,
     "d4": [(1 + math.sqrt(3)) / (4 * math.sqrt(2)), (3 + math.sqrt(3)) / (4 * math.sqrt(2)),
            (3 - math.sqrt(3)) / (4 * math.sqrt(2)), (1 - math.sqrt(3)) / (4 * math.sqrt(2))]}


def mirror(i, n):
    if n == 1:
        return 0
    p = 2 * (n - 1)
    r = i % p
    return r if r < n else p - r


def legall(x):
    n, y = len(x), list(x)
    if n == 1:  # JPEG 2000 F.3.7: a single sample is its own low band
        return list(x), []
    for i in range(1, n, 2):
        y[i] = x[i] - (x[mirror(i - 1, n)] + x[mirror(i + 1, n)]) // 2
    z = list(y)
    for i in range(0, n, 2):
        z[i] = y[i] + (y[mirror(i - 1, n)] + y[mirror(i + 1, n)] + 2) // 4
    return z[0::2], z[1::2]


def lift97(y, start, c):
    n, out = len(y), list(y)
    for i in range(start, n, 2):
        out[i] = y[i] + c * (y[mirror(i - 1, n)] + y[mirror(i + 1, n)])
    return out


for _ in range(120):
    for name in ("haar", "d4"):
        n = 8 * rng.randrange(1, 8)
        x = signal(n)
        a, parts = x, []
        for _ in range(3):
            if len(a) < 2 or len(a) % 2:
                break
            m = orth_matrix(H[name], len(a))
            out = [sum(r[j] * a[j] for j in range(len(a))) for r in m]
            a, d = out[:len(a) // 2], out[len(a) // 2:]
            parts.insert(0, d)
        parts.insert(0, a)
        norm = sum(abs(v) for v in x) + 1
        want = parts

        def chk(got, want=want, x=x, norm=norm):
            coeffs, back = got.split("|")
            got_parts = coeffs.split(";")
            if len(got_parts) != len(want) or not all(close(g, w, norm) for g, w in zip(got_parts, want)):
                return "coefficients differ"
            return None if close(back, x, norm) else "not reconstructed"
        cases.append(["dwt", name, bits(x), chk, None, 0, 0])
    n = rng.randrange(1, 40)
    xi = [rng.randrange(-300, 300) for _ in range(n)]
    lo, hi = legall(xi)
    cases.append(["dwt", "legall", ",".join(map(str, xi)),
                  (lambda w: lambda got: None if got == w else f"want {w}")(
                      ",".join(map(str, lo)) + "|" + ",".join(map(str, hi)) + "|" + ",".join(map(str, xi))), None, 0, 0])
    xf = signal(rng.randrange(2, 40))
    y = lift97(xf, 1, -1.586134342059924)
    y = lift97(y, 0, -0.052980118572961)
    y = lift97(y, 1, 0.882911075530934)
    y = lift97(y, 0, 0.443506852043971)
    k = 1.230174104914001
    low, high = [v / k for v in y[0::2]], [v * k for v in y[1::2]]
    normf = sum(abs(v) for v in xf) + 1

    def chk97(got, low=low, high=high, xf=xf, normf=normf):
        l, h, back = got.split("|")
        if not close(l, low, normf) or not close(h, high, normf):
            return "coefficients differ"
        return None if close(back, xf, normf, 1e-11) else "not reconstructed"
    cases.append(["dwt", "cdf97", bits(xf), chk97, None, 0, 0])

ran = subprocess.run([go], input=json.dumps(go_reqs), capture_output=True, text=True, check=True)
go_ans = json.loads(ran.stdout)
for c in cases:
    if c[3] is None:
        want = go_ans[c[4]]
        rel, scale = c[5], c[6]
        c[3] = (lambda w, r, s: lambda got: (None if got == "ERR" else "gonum refused") if w == "ERR" else
                (None if close(got, [f(v) for v in w.split(",")] if w else [], s, r) else f"want {w[:80]}"))(want, rel, scale)

argv = []
for c in cases:
    argv += [c[0], c[1], c[2]]
lines, start = [], 0
while start < len(argv):
    out = subprocess.run([harness] + argv[start:start + 900], capture_output=True, text=True, timeout=300)
    if out.returncode != 0:
        print(out.stderr[-2000:]); sys.exit(1)
    lines += out.stdout.split("\n")[:-1]
    start += 900
families, agree, differ = {}, 0, 0
for i, c in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    try:
        why = c[3](mine)
    except Exception as e:
        why = f"unreadable ({e})"
    key = c[0] if c[0] not in ("win", "dwt", "spline") else f"{c[0]}"
    fam = families.setdefault(key, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 10:
            print(f"  {c[0]} {c[1][:40]} {c[2][:60]}: {why}")
            print(f"    grasm {mine[:160]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 8:
    print(f"too few cases to mean anything: {thin}"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
