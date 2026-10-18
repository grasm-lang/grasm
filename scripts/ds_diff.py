"""Data structures against Python models.

Each structure runs a random script of operations -- updates and queries
interleaved, with the edge cases (empty ranges, positions past the end,
stale keys, deadlines already passed or beyond the wheel) mixed in -- and
every operation's answer is compared with a model: a plain Python list,
set or dict doing the same thing the slow, obvious way. A sum is
`sum(a[lo:hi])`, a quantile is `sorted(a[lo:hi])[k]`, a stabbing query is
a filter over every interval, and a timer is due when its deadline has
passed.
"""
import os
import random
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="dsdiff")
harness = os.path.join(work, "dstest")
built = subprocess.run([GRASM, "build", "packages/dstest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

cases = []
I64_MAX, I64_MIN = (1 << 63) - 1, -(1 << 63)


def add(op, setup, cmds, want):
    cases.append((op, setup, ";".join(" ".join(map(str, c)) for c in cmds), " ".join(want)))


def b(x):
    return "true" if x else "false"


# --- Fenwick ------------------------------------------------------------------
for k in range(220):
    n = rng.randrange(0, 40)
    nonneg = k % 2 == 0
    a = [rng.randrange(0, 50) if nonneg else rng.randrange(-50, 50) for _ in range(n)]
    setup = ("L" if rng.random() < 0.5 else "U") + ",".join(map(str, a))
    cmds, want = [], []
    for _ in range(40):
        op = rng.randrange(6 if nonneg else 5)
        if op <= 1 and n == 0:
            op = 2
        if op == 0:
            i, d = rng.randrange(n), rng.randrange(0, 9) if nonneg else rng.randrange(-9, 9)
            a[i] += d
            cmds.append((0, i, d)); want.append("0")
        elif op == 1:
            i, v = rng.randrange(n), rng.randrange(0, 60) if nonneg else rng.randrange(-60, 60)
            a[i] = v
            cmds.append((1, i, v)); want.append("0")
        elif op == 2:
            i = rng.randrange(0, n + 1)
            cmds.append((2, i)); want.append(str(sum(a[:i])))
        elif op == 3:
            lo, hi = rng.randrange(0, n + 1), rng.randrange(0, n + 1)
            cmds.append((3, lo, hi)); want.append(str(sum(a[lo:hi]) if hi > lo else 0))
        elif op == 4:
            i = rng.randrange(n) if n else 0
            if n == 0:
                cmds.append((2, 0)); want.append("0"); continue
            cmds.append((4, i)); want.append(str(a[i]))
        else:
            t = rng.randrange(-2, sum(a) + 5)
            pos = next((i for i in range(n) if sum(a[:i + 1]) >= t), n)
            cmds.append((5, t)); want.append(str(pos))
    add("fenwick", setup, cmds, want)


# --- segment trees ------------------------------------------------------------
P = 1000003


def affine(x, y):
    a1, b1, a2, b2 = x >> 20, x & 1048575, y >> 20, y & 1048575
    return ((a1 * a2) % P) << 20 | ((a2 * b1 + b2) % P)


for k in range(240):
    kind = "ASNX"[k % 4]
    n = rng.randrange(0, 30)
    if kind == "A":
        a = [(rng.randrange(P) << 20) | rng.randrange(P) for _ in range(n)]
        op, unit = affine, 1 << 20
    else:
        a = [rng.randrange(-100, 100) for _ in range(n)]
        op, unit = {"S": (lambda x, y: x + y, 0), "N": (min, I64_MAX), "X": (max, I64_MIN)}[kind]
    setup = kind + ",".join(map(str, a))
    cmds, want = [], []
    for _ in range(40):
        if n and rng.random() < 0.4:
            i = rng.randrange(n)
            v = (rng.randrange(P) << 20) | rng.randrange(P) if kind == "A" else rng.randrange(-100, 100)
            a[i] = v
            cmds.append((0, i, v)); want.append("-")
        else:
            lo = rng.randrange(0, n + 1)
            hi = rng.randrange(lo, n + 1)
            acc = unit
            for x in a[lo:hi]:
                acc = op(acc, x)
            cmds.append((1, lo, hi)); want.append(str(acc))
    add("segtree", setup, cmds, want)

for _ in range(160):
    n = rng.randrange(1, 30)
    a = [rng.randrange(-100, 100) for _ in range(n)]
    setup = ",".join(map(str, a))
    cmds, want = [], []
    for _ in range(40):
        lo = rng.randrange(0, n)
        hi = rng.randrange(lo + 1, n + 1)
        if rng.random() < 0.5:
            d = rng.randrange(-20, 20)
            for i in range(lo, hi):
                a[i] += d
            cmds.append((0, lo, hi, d)); want.append("-")
        else:
            part = a[lo:hi]
            cmds.append((1, lo, hi)); want.append(f"{sum(part)}/{min(part)}/{max(part)}")
    add("lazyseg", setup, cmds, want)


# --- interval tree --------------------------------------------------------------
def fmt(items):
    return "[" + ",".join(f"{s}:{e}:{i}" for s, e, i in sorted(items)) + "]"


for _ in range(220):
    held = set()
    cmds, want = [], []
    for _ in range(60):
        r = rng.random()
        if r < 0.45:
            s = rng.randrange(0, 60)
            e = s + rng.randrange(-2, 15)
            i = rng.randrange(0, 5)
            ok = s <= e and (s, e, i) not in held
            if ok:
                held.add((s, e, i))
            cmds.append((0, s, e, i)); want.append(b(ok))
        elif r < 0.6:
            pick = rng.choice(sorted(held)) if held and rng.random() < 0.8 else (rng.randrange(60), rng.randrange(60), 0)
            ok = pick in held
            held.discard(pick)
            cmds.append((1,) + pick); want.append(b(ok))
        elif r < 0.75:
            p = rng.randrange(-2, 75)
            cmds.append((2, p)); want.append(fmt(x for x in held if x[0] <= p < x[1]))
        elif r < 0.95:
            lo, hi = rng.randrange(-2, 75), rng.randrange(-2, 75)
            got = [x for x in held if lo < hi and x[0] < x[1] and x[0] < hi and x[1] > lo]
            cmds.append((3, lo, hi)); want.append(fmt(got))
        else:
            cmds.append((4,)); want.append(fmt(held) + f"#{len(held)}")
    add("intervals", "-", cmds, want)


# --- rank and select -------------------------------------------------------------
for k in range(200):
    n = rng.choice([0, 1, 63, 64, 65, 127, 128, 129]) if k < 40 else rng.randrange(0, 400)
    density = rng.random()
    bits = [rng.random() < density for _ in range(n)]
    ones = [i for i in range(n) if bits[i]]
    zeros = [i for i in range(n) if not bits[i]]
    cmds, want = [], []
    for _ in range(50):
        op = rng.randrange(5)
        if op == 4 and n == 0:
            op = 0
        if op in (0, 1):
            i = rng.randrange(-2, n + 3)
            c = min(max(i, 0), n)
            r1 = sum(bits[:c])
            cmds.append((op, i)); want.append(str(r1 if op == 0 else c - r1))
        elif op in (2, 3):
            src = ones if op == 2 else zeros
            j = rng.randrange(-1, len(src) + 2)
            cmds.append((op, j)); want.append(str(src[j] if 0 <= j < len(src) else -1))
        else:
            i = rng.randrange(n)
            cmds.append((4, i)); want.append("1" if bits[i] else "0")
    add("rankselect", "".join("1" if x else "0" for x in bits), cmds, want + [str(len(ones))])


# --- wavelet matrix --------------------------------------------------------------
for k in range(200):
    n = rng.randrange(0, 60)
    top = rng.choice([1, 2, 7, 100, 1 << 20, 1 << 40])
    vals = [rng.randrange(top) for _ in range(n)]
    if k % 25 == 0 and n:
        vals[rng.randrange(n)] = -1
        add("wavelet", ",".join(map(str, vals)), [(0, 0)], ["ERR"])
        continue
    cmds, want = [], []
    pool = vals + [rng.randrange(top + 2) for _ in range(3)] + [top * 4]
    for _ in range(40):
        op = rng.randrange(5)
        if op == 0 and n == 0:
            op = 1
        if op == 0:
            i = rng.randrange(n)
            cmds.append((0, i)); want.append(str(vals[i]))
        elif op == 1:
            v, i = rng.choice(pool), rng.randrange(-1, n + 2)
            c = min(max(i, 0), n)
            cmds.append((1, v, i)); want.append(str(vals[:c].count(v)))
        elif op == 2:
            v = rng.choice(pool)
            where = [i for i in range(n) if vals[i] == v]
            j = rng.randrange(-1, len(where) + 2)
            cmds.append((2, v, j)); want.append(str(where[j] if 0 <= j < len(where) else -1))
        elif op == 3:
            lo, hi = rng.randrange(-2, n + 2), rng.randrange(-2, n + 3)
            a, z = max(lo, 0), min(hi, n)
            part = sorted(vals[a:z]) if a < z else []
            j = rng.randrange(-1, len(part) + 2)
            cmds.append((3, lo, hi, j)); want.append(str(part[j] if 0 <= j < len(part) else -1))
        else:
            lo, hi = rng.randrange(-2, n + 2), rng.randrange(-2, n + 3)
            low, high = sorted([rng.choice(pool + [0, -5]), rng.choice(pool + [top * 8])])
            if rng.random() < 0.1:
                low, high = high, low
            a, z = max(lo, 0), min(hi, n)
            got = sum(1 for v in vals[a:z] if low <= v < high) if a < z and low < high else 0
            cmds.append((4, lo, hi, low, high)); want.append(str(got))
    add("wavelet", ",".join(map(str, vals)), cmds, want)


# --- sparse set --------------------------------------------------------------------
for _ in range(200):
    u = rng.randrange(0, 40)
    dense, count = [], 0
    cmds, want = [], []
    for _ in range(60):
        op = rng.choices([0, 1, 2, 3, 4], [5, 3, 3, 1, 2])[0]
        x = rng.randrange(-1, u + 2)
        if op == 0:
            ok = 0 <= x < u and x not in dense
            if ok:
                dense.append(x)
            cmds.append((0, x)); want.append(b(ok))
        elif op == 1:
            ok = x in dense
            if ok:
                at = dense.index(x)
                dense[at] = dense[-1]
                dense.pop()
            cmds.append((1, x)); want.append(b(ok))
        elif op == 2:
            cmds.append((2, x)); want.append(b(x in dense))
        elif op == 3:
            dense = []
            cmds.append((3,)); want.append("-")
        else:
            cmds.append((4,)); want.append("[" + ",".join(map(str, dense)) + f"]#{len(dense)}")
    add("sparseset", str(u), cmds, want)


# --- slot map ---------------------------------------------------------------------
for _ in range(200):
    values, gens, free = [], [], []
    seen = []
    cmds, want = [], []
    for _ in range(60):
        op = rng.choices([0, 1, 2, 3, 4], [4, 3, 3, 2, 1])[0]
        if op == 0:
            v = rng.randrange(1000)
            if free:
                i = free.pop()
                values[i] = v
            else:
                values.append(v)
                gens.append(0)
                i = len(values) - 1
            seen.append((i, gens[i]))
            cmds.append((0, v)); want.append(f"{i}.{gens[i]}")
            continue
        if op == 4:
            live = [f"{i}.{gens[i]}" for i in range(len(values)) if values[i] is not None]
            cmds.append((4,)); want.append(",".join(live) + f"#{len(live)}")
            continue
        i, g = rng.choice(seen) if seen and rng.random() < 0.85 else (rng.randrange(-1, 5), rng.randrange(0, 3))
        live = 0 <= i < len(values) and gens[i] == g and values[i] is not None
        if op == 1:
            cmds.append((1, i, g)); want.append(str(values[i]) if live else "none")
        elif op == 2:
            want.append(str(values[i]) if live else "none")
            if live:
                values[i] = None
                gens[i] += 1
                free.append(i)
            cmds.append((2, i, g))
        else:
            v = rng.randrange(1000)
            if live:
                values[i] = v
            cmds.append((3, i, g, v)); want.append(b(live))
    add("slotmap", "-", cmds, want)


# --- timing wheel ------------------------------------------------------------------
for k in range(220):
    now = rng.choice([0, 63, 64, 4095, 1 << 30, (1 << 36) - 3, rng.randrange(1 << 40)])
    start = now
    pending = {}
    next_id = 0
    cmds, want = [], []
    for _ in range(50):
        r = rng.random()
        if r < 0.5:
            reach = rng.choice([0, 3, 64, 200, 5000, 1 << 20, 1 << 36, (1 << 37) + 5])
            d = now + rng.randrange(-5, reach + 1)
            pending[next_id] = d
            cmds.append((0, d)); want.append(str(next_id))
            next_id += 1
        elif r < 0.65:
            i = rng.randrange(-1, next_id + 2)
            ok = i in pending
            pending.pop(i, None)
            cmds.append((1, i)); want.append(b(ok))
        else:
            step = rng.choice([0, 1, 5, 63, 64, 65, 300, 4096, 1 << 20, 1 << 31, (1 << 36) + 7])
            t = now + rng.randrange(-3, step + 1)
            now = max(now, t)
            due = sorted((d, i) for i, d in pending.items() if d <= now)
            for _, i in due:
                del pending[i]
            cmds.append((2, t)); want.append("[" + ",".join(str(i) for _, i in due) + f"]@{now}#{len(pending)}")
    add("timerwheel", str(start), cmds, want)


def run(picked):
    argv = []
    for op, setup, script, _ in picked:
        argv += [op, setup, script]
    lines = []
    for start in range(0, len(argv), 3 * 200):
        ran = subprocess.run([harness] + argv[start:start + 600], capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, setup, script, want) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    fam = families.setdefault(op, [0, 0])
    if mine == want:
        agree += 1
        fam[0] += 1
        continue
    differ += 1
    fam[1] += 1
    if differ <= 6:
        w, m, c = want.split(" "), mine.split(" "), script.split(";")
        at = next((j for j in range(min(len(w), len(m))) if w[j] != m[j]), min(len(w), len(m)))
        print(f"  {op} setup={setup[:60]} first difference at operation {at}: {c[at] if at < len(c) else ''}")
        print(f"    model {' '.join(w[max(0, at - 1):at + 2])[:200]}")
        print(f"    grasm {' '.join(m[max(0, at - 1):at + 2])[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 9:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
