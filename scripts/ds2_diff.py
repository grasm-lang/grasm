"""Caches, consistent hashing, MinHash and filters against references.

- `lfu` and `arccache` run random scripts of reads and writes against
  Python models: LFU as "evict the least (uses, last use)", ARC as the
  paper's cases I-IV on four Python lists.
- `jumphash` against the loop printed in Lamping and Veach's paper.
- `rendezvous` against dgryski/go-rendezvous with cespare/xxhash, from the
  Go module cache (scripts/lib/gomods), adding and removing nodes.
- `minhash` against a Python implementation of its stated hash family,
  and the estimate against the true Jaccard similarity.
- `xorfilter` by its guarantees: every key it was built from is a
  member, and strangers are let in at about 1 in 256.
- `quotientfilter` against a multiset of fingerprints -- it is exact on
  fingerprints -- including dense tables whose clusters wrap past the end.
"""
import json
import os
import random
import subprocess
import sys
import tempfile
from collections import Counter

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="ds2diff")
M64 = (1 << 64) - 1


def signed(x):
    x &= M64
    return x - (1 << 64) if x >> 63 else x


harness = os.path.join(work, "dstest2")
built = subprocess.run([GRASM, "build", "packages/dstest2/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle as gomods  # noqa: E402
oracle = gomods.build(os.path.join(work, "gomods"))

cases = []


def add(op, setup, cmds, check):
    cases.append((op, setup, ";".join(cmds), check))


def exact(want):
    return lambda got: None if got == want else f"want {want[:300]}"


def fnv64a(data):
    h = 0xcbf29ce484222325
    for c in data:
        h ^= c
        h = (h * 0x100000001b3) & M64
    return h


# --- LFU -------------------------------------------------------------------------
for _ in range(200):
    cap = rng.randrange(1, 6)
    d, tick = {}, 0
    cmds, want = [], []
    keys = [f"k{i}" for i in range(rng.randrange(2, 10))]
    for _ in range(60):
        r = rng.random()
        k = rng.choice(keys)
        if r < 0.45:
            v = rng.randrange(1000)
            if k in d:
                d[k] = [v, d[k][1] + 1, tick]
                ev = "-"
            else:
                ev = "-"
                if len(d) >= cap:
                    ev = min(d, key=lambda x: (d[x][1], d[x][2]))
                    del d[ev]
                d[k] = [v, 1, tick]
            tick += 1
            cmds.append(f"set {k} {v}"); want.append(ev)
        elif r < 0.75:
            if k in d:
                d[k][1] += 1
                d[k][2] = tick
                tick += 1
                want.append(str(d[k][0]))
            else:
                want.append("none")
            cmds.append(f"get {k}")
        elif r < 0.82:
            want.append("true" if k in d else "false")
            d.pop(k, None)
            cmds.append(f"del {k}")
        elif r < 0.92:
            cmds.append(f"freq {k}"); want.append(str(d[k][1] if k in d else 0))
        else:
            order = sorted(d, key=lambda x: (d[x][1], d[x][2]))
            cmds.append("order"); want.append(",".join(order) + f"#{len(d)}")
    add("lfu", str(cap), cmds, exact(" ".join(want)))


# --- ARC -------------------------------------------------------------------------
class Arc:
    def __init__(self, c):
        self.c, self.p = c, 0
        self.t1, self.t2, self.b1, self.b2, self.v = [], [], [], [], {}

    def replace(self, hit_b2):
        if self.t1 and ((hit_b2 and len(self.t1) == self.p) or len(self.t1) > self.p):
            x = self.t1.pop(0)
            self.b1.append(x)
        else:
            x = self.t2.pop(0)
            self.b2.append(x)
        del self.v[x]
        return x

    def get(self, k):
        if k not in self.v:
            return "none"
        for lst in (self.t1, self.t2):
            if k in lst:
                lst.remove(k)
        self.t2.append(k)
        return str(self.v[k])

    def set(self, k, val):
        if k in self.v:
            self.v[k] = val
            for lst in (self.t1, self.t2):
                if k in lst:
                    lst.remove(k)
            self.t2.append(k)
            return "-"
        if k in self.b1 or k in self.b2:
            from_b2 = k in self.b2
            n1, n2 = len(self.b1), len(self.b2)
            if from_b2:
                self.p = max(self.p - (1 if n2 >= n1 else n1 // n2), 0)
            else:
                self.p = min(self.p + (1 if n1 >= n2 else n2 // n1), self.c)
            ev = self.replace(from_b2)
            (self.b2 if from_b2 else self.b1).remove(k)
            self.t2.append(k)
            self.v[k] = val
            return ev
        ev = "-"
        total = len(self.t1) + len(self.t2) + len(self.b1) + len(self.b2)
        if len(self.t1) + len(self.b1) == self.c:
            if len(self.t1) < self.c:
                self.b1.pop(0)
                ev = self.replace(False)
            else:
                ev = self.t1.pop(0)
                del self.v[ev]
        elif total >= self.c:
            if total == 2 * self.c:
                self.b2.pop(0)
            ev = self.replace(False)
        self.t1.append(k)
        self.v[k] = val
        return ev

    def lists(self):
        return "|".join(",".join(x) for x in (self.t1, self.t2, self.b1, self.b2)) + f"|{self.p}"


for _ in range(200):
    cap = rng.randrange(1, 6)
    a = Arc(cap)
    keys = [f"k{i}" for i in range(rng.randrange(2, 3 * cap + 4))]
    cmds, want = [], []
    for _ in range(80):
        r = rng.random()
        k = rng.choice(keys)
        if r < 0.5:
            v = rng.randrange(1000)
            cmds.append(f"set {k} {v}"); want.append(a.set(k, v))
        elif r < 0.85:
            cmds.append(f"get {k}"); want.append(a.get(k))
        else:
            cmds.append("lists"); want.append(a.lists())
        assert len(a.t1) + len(a.t2) <= a.c and len(a.t1) + len(a.b1) <= a.c
    add("arc", str(cap), cmds, exact(" ".join(want)))


# --- jump consistent hashing -----------------------------------------------------
def jump(key, n):
    if n < 1:
        return -1
    key &= M64
    b, j = -1, 0
    while j < n:
        b = j
        key = (key * 2862933555777941757 + 1) & M64
        j = int((b + 1) * (float(1 << 31) / float((key >> 33) + 1)))
    return b


for _ in range(150):
    cmds, want = [], []
    for _ in range(40):
        n = rng.choice([0, 1, 2, 3, 10, 100, 1000, 1 << 20, (1 << 31) - 1])
        if rng.random() < 0.3:
            s = "".join(rng.choice("abcxyz09") for _ in range(rng.randrange(0, 12))) or "e"
            cmds.append(f"s {s} {n}"); want.append(str(jump(fnv64a(s.encode()), n)))
        else:
            key = rng.getrandbits(64)
            cmds.append(f"k {signed(key)} {n}"); want.append(str(jump(key, n)))
    add("jump", "-", cmds, exact(" ".join(want)))
# Growing from n to n + 1 buckets moves only keys that land in bucket n.
moved_wrong = 0
for _ in range(2000):
    key, n = rng.getrandbits(64), rng.randrange(1, 50)
    if jump(key, n + 1) not in (jump(key, n), n):
        moved_wrong += 1
if moved_wrong:
    print(f"the reference itself broke monotonicity {moved_wrong} times"); sys.exit(1)


# --- rendezvous -------------------------------------------------------------------
rdv_requests, rdv_slots = [], []
for _ in range(150):
    nodes = [f"node{rng.randrange(20)}" for _ in range(rng.randrange(0, 7))]
    setup = ",".join(nodes)
    cmds, slots = [], []
    for _ in range(25):
        r = rng.random()
        if r < 0.15:
            n = f"node{rng.randrange(20)}"
            nodes.append(n)
            cmds.append(f"add {n}")
        elif r < 0.25 and nodes:
            n = rng.choice(nodes)
            nodes.remove(n)
            cmds.append(f"rm {n}")
        else:
            k = "".join(rng.choice("abcdefgh123") for _ in range(rng.randrange(1, 10)))
            cmds.append(f"look {k}")
            slots.append(len(rdv_requests))
            rdv_requests.append({"op": "rdv", "nodes": list(nodes), "keys": [k]})
    rdv_slots.append(slots)
    cases.append(("rdv", setup, ";".join(cmds), None))
ran = subprocess.run([oracle], input=json.dumps(rdv_requests), capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-1500:]); sys.exit(1)
go_answers = json.loads(ran.stdout)
k = 0
for i, c in enumerate(cases):
    if c[0] == "rdv" and c[3] is None:
        want = " ".join(go_answers[s] for s in rdv_slots[k])
        cases[i] = (c[0], c[1], c[2], exact(want))
        k += 1


# --- MinHash ------------------------------------------------------------------------
P31 = (1 << 31) - 1
GAMMA = 0x9E3779B97F4A7C15


def mix(z):
    z ^= z >> 30
    z = (z * 0xBF58476D1CE4E5B9) & M64
    z ^= z >> 27
    z = (z * 0x94D049BB133111EB) & M64
    return z ^ (z >> 31)


def hasher(k, seed):
    a, b, state = [], [], seed & M64
    for _ in range(k):
        state = (state + GAMMA) & M64
        a.append(1 + (mix(state) >> 1) % (P31 - 1))
        state = (state + GAMMA) & M64
        b.append((mix(state) >> 1) % P31)
    return a, b


def signature(ab, elements):
    a, b = ab
    sig = [P31] * len(a)
    for e in elements:
        x = (fnv64a(e.encode()) >> 1) % P31
        for i in range(len(a)):
            sig[i] = min(sig[i], (a[i] * x + b[i]) % P31)
    return sig


far_off = 0
for _ in range(120):
    k, seed = rng.choice([1, 4, 16, 64, 128, 256]), rng.randrange(0, 1 << 40)
    ab = hasher(k, seed)
    universe = [f"w{i}" for i in range(rng.randrange(1, 60))]
    cmds, want, last = [], [], []
    for _ in range(6):
        elements = rng.sample(universe, rng.randrange(0, len(universe) + 1))
        bands = rng.randrange(1, 9)
        rows = rng.randrange(1, max(2, k // bands + 2))
        sig = signature(ab, elements)
        hashes = []
        for band in range(bands):
            part = ",".join(str(sig[at]) for at in range(band * rows, band * rows + rows) if at < len(sig))
            hashes.append(signed(fnv64a(f"{band}:".encode() + part.encode())))
        same = sum(1 for x, y in zip(last, sig) if x == y) / k if len(last) == len(sig) else 0.0
        scaled = int(same * 1000000.0 + 0.5)
        cmds.append(f"set {','.join(elements) or '_'} {bands} {rows}")
        want.append(",".join(map(str, sig)) + "/" + ",".join(map(str, hashes)) + f"/{scaled}")
        last = sig
    add("minhash", f"{k},{seed}", cmds, exact(" ".join(want)))
# The scheme, not just the code: with 256 positions the estimate is close.
ab = hasher(256, 7)
for _ in range(40):
    u = [f"t{i}" for i in range(200)]
    s1, s2 = set(rng.sample(u, 120)), set(rng.sample(u, 120))
    est = sum(1 for x, y in zip(signature(ab, s1), signature(ab, s2)) if x == y) / 256
    if abs(est - len(s1 & s2) / len(s1 | s2)) > 0.12:
        far_off += 1
if far_off > 2:
    print(f"MinHash estimates far from the Jaccard similarity {far_off} times in 40"); sys.exit(1)


# --- xor filter ---------------------------------------------------------------------
xor_stats = [0, 0]


def check_xor(keys, queries, members, got):
    if got == "ERR":
        return "refused to build"
    bits, size = got.split(" ")
    unique = len(set(keys))
    want_size = (32 + -(-123 * unique // 100)) // 3 * 3
    if int(size) != want_size:
        return f"size {size}, want {want_size}"
    for q, bit in zip(queries, bits):
        if q in members and bit != "1":
            return f"member {q} missing"
        if q not in members:
            xor_stats[0] += 1
            xor_stats[1] += bit == "1"
    return None


for _ in range(150):
    n = rng.choice([0, 1, 2, 10, 100, 1000, 5000])
    keys = [signed(rng.getrandbits(64)) for _ in range(n)]
    if keys and rng.random() < 0.3:
        keys += rng.sample(keys, min(len(keys), 10))
    members = set(keys)
    queries = list(members)[:300] + [signed(rng.getrandbits(64)) for _ in range(300)]
    rng.shuffle(queries)
    cases.append(("xor", ",".join(map(str, keys)), ";".join(map(str, queries)),
                  lambda got, a=(keys, queries, members): check_xor(*a, got)))


# --- quotient filter ------------------------------------------------------------------
def fmix(key):
    h = key & M64
    h ^= h >> 33
    h = (h * 0xff51afd7ed558ccd) & M64
    h ^= h >> 33
    h = (h * 0xc4ceb9fe1a85ec53) & M64
    return h ^ (h >> 33)


for _ in range(220):
    q, r = rng.randrange(1, 7), rng.randrange(1, 5)
    mask, cap = (1 << (q + r)) - 1, (1 << q) - 1
    bag = Counter()
    hot = [rng.randrange(1 << (q + r)) for _ in range(4)]
    cmds, want = [], []
    for _ in range(80):
        op = rng.choices(["ins", "del", "has", "kins", "khas", "fp"], [5, 3, 3, 1, 1, 1])[0]
        if op in ("ins", "del", "has"):
            fp = rng.choice(hot) if rng.random() < 0.4 else rng.randrange(1 << (q + r))
            if rng.random() < 0.3:
                fp = ((cap) << r) | rng.randrange(1 << r)
            cmds.append(f"{op} {fp}")
        else:
            key = signed(rng.getrandbits(64))
            fp = fmix(key) & mask
            cmds.append(f"{op} {key}")
        if op in ("ins", "kins"):
            ok = sum(bag.values()) < cap
            if ok:
                bag[fp] += 1
            want.append("true" if ok else "false")
        elif op == "del":
            ok = bag[fp] > 0
            if ok:
                bag[fp] -= 1
            want.append("true" if ok else "false")
        elif op in ("has", "khas"):
            want.append("true" if bag[fp] > 0 else "false")
        else:
            want.append(f"{fp}#{sum(bag.values())}")
    add("qf", f"{q},{r}", cmds, exact(" ".join(want)))


def run(picked):
    argv = []
    for op, setup, script, _ in picked:
        argv += [op, setup, script]
    lines, start = [], 0
    while start < len(argv):
        chunk, size = [], 0
        while start < len(argv) and (size < 200000 or not chunk):
            chunk += argv[start:start + 3]
            size += sum(len(x) for x in argv[start:start + 3])
            start += 3
        try:
            ran = subprocess.run([harness] + chunk, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("the harness did not finish in 300 s -- a loop that never ends"); sys.exit(1)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, setup, script, check) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    why = check(mine) if mine != "<missing>" else "missing"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 8:
            print(f"  {op} setup={setup[:60]} script={script[:120]}")
            print(f"    grasm {mine[:240]}")
            print(f"    {why[:240]}")
strangers, let_in = xor_stats
if strangers < 10000 or let_in / strangers > 0.01:
    print(f"xor filter let in {let_in} of {strangers} strangers")
    differ += 1
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 7:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}   "
      f"xor strangers let in: {let_in}/{strangers}")
sys.exit(1 if differ else 0)
