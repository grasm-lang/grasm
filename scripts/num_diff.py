"""Number packages against Python.

- `numtheory` against brute force over small moduli (the smallest square
  root, order, primitive root and discrete logarithm found by trying every
  candidate; CRT by scanning the whole range) and Python's own big
  integers for 62-bit moduli.
- `contfrac` against `fractions.Fraction` -- terms by repeated floor and
  reciprocal, `limit_denominator` itself -- and square-root expansions
  against `decimal` at 600 digits.
- `ntt` against schoolbook polynomial products modulo 998244353.
- `bitops` against bit-by-bit definitions on Python integers.
- `float16` and `bfloat16` against `struct`'s 'e' and 'f' formats; the
  bfloat16 rounding against an exact nearest-value search.
- `specialfn` and `complexfn` against `math` and `cmath`, within the
  stated tolerances, with C99's values where Python raises.

Doubles cross as their 64 bits, so nothing is lost on the way.
"""
import cmath
import math
import os
import random
import struct
import subprocess
import sys
import tempfile
from decimal import Decimal, getcontext
from fractions import Fraction

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="numdiff")
harness = os.path.join(work, "numtest")
built = subprocess.run([GRASM, "build", "packages/numtest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
M64 = (1 << 64) - 1
cases = []


def s64(x):
    x &= M64
    return x - (1 << 64) if x >> 63 else x


def bits(x):
    return s64(struct.unpack("<Q", struct.pack("<d", x))[0])


def dbl(b):
    return struct.unpack("<d", struct.pack("<Q", b & M64))[0]


def add(op, args, want, b=""):
    cases.append((op, ",".join(map(str, args)), b, lambda got: None if got == want else f"want {want[:200]}"))


def add_check(op, args, check, b=""):
    cases.append((op, ",".join(map(str, args)), b, check))


# --- numtheory ------------------------------------------------------------------------
def factor(n):
    out, d = [], 2
    while d * d <= n:
        while n % d == 0:
            out.append(d)
            n //= d
        d += 1
    if n > 1:
        out.append(n)
    return out


def check_egcd(a, b):
    def check(got):
        g, x, y = map(int, got.split(","))
        if g != math.gcd(a, b) or a * x + b * y != g:
            return f"not a Bezout identity for gcd {math.gcd(a, b)}"
        if g and (abs(x) > max(1, abs(b) // g) or abs(y) > max(1, abs(a) // g)):
            return "coefficients larger than Euclid gives"
        return None
    return check


for _ in range(150):
    a, b = rng.randrange(-10 ** 6, 10 ** 6), rng.randrange(-10 ** 6, 10 ** 6)
    add_check("nt.egcd", [a, b], check_egcd(a, b))
for _ in range(150):
    m = rng.choice([rng.randrange(1, 1000), rng.randrange(1, 1 << 31), rng.randrange(1, 1 << 62)])
    a, e = rng.randrange(-(1 << 62), 1 << 62), rng.randrange(0, 1 << 62)
    add("nt.powmod", [a, e, m], str(pow(a, e, m)))
    add("nt.mulmod", [a, rng.randrange(-(1 << 62), 1 << 62), m], None)
    cases[-1] = (cases[-1][0], cases[-1][1], "", (lambda x, y, mm: lambda got: None if int(got) == (x * y) % mm else f"want {(x * y) % mm}")(
        a, int(cases[-1][1].split(",")[1]), m))
for _ in range(150):
    m = rng.choice([rng.randrange(1, 100), rng.randrange(2, 1 << 40)])
    a = rng.randrange(-m, 2 * m)
    try:
        want = str(pow(a, -1, m))
    except ValueError:
        want = "none"
    add("nt.inverse", [a, m], want)


def brute_crt(rs, ms):
    lcm = 1
    for m in ms:
        lcm = lcm * m // math.gcd(lcm, m)
    for x in range(lcm):
        if all(x % m == r % m for r, m in zip(rs, ms)):
            return f"{x},{lcm}"
    return "none"


for _ in range(150):
    k = rng.randrange(1, 4)
    ms = [rng.randrange(1, 40) for _ in range(k)]
    rs = [rng.randrange(-50, 50) for _ in range(k)]
    add("nt.crt", rs + ms, brute_crt(rs, ms))


def legendre(a, p):
    r = pow(a, (p - 1) // 2, p)
    return -1 if r == p - 1 else r


def jacobi_by_factors(a, n):
    out = 1
    for p in factor(n):
        out *= legendre(a % p, p)
    return out


SMALL_PRIMES = [p for p in range(3, 3000) if all(p % d for d in range(2, int(p ** 0.5) + 1))]
for _ in range(150):
    n = rng.randrange(1, 4000) * 2 + 1
    a = rng.randrange(-5000, 5000)
    add("nt.jacobi", [a, n], str(jacobi_by_factors(a, n)))
for _ in range(150):
    p = rng.choice(SMALL_PRIMES + [2])
    a = rng.randrange(0, 3 * p)
    roots = [x for x in range(p) if x * x % p == a % p]
    add("nt.sqrtmod", [a, p], str(min(roots)) if roots else "none")


def order(a, n):
    if math.gcd(a % n, n) != 1:
        return 0
    k, v = 1, a % n
    while v != 1 % n:
        v = v * a % n
        k += 1
    return k


def phi(n):
    return sum(1 for k in range(1, n + 1) if math.gcd(k, n) == 1)


for _ in range(150):
    n = rng.randrange(1, 3000)
    add("nt.order", [rng.randrange(-n, 2 * n), n], None)
    a = int(cases[-1][1].split(",")[0])
    cases[-1] = (cases[-1][0], cases[-1][1], "", (lambda w: lambda got: None if got == w else f"want {w}")(str(order(a, n))))
for _ in range(120):
    n = rng.randrange(1, 1500)
    ph = phi(n)
    root = next((g for g in range(1, n) if math.gcd(g, n) == 1 and order(g, n) == ph), -1) if n > 1 else 0
    add("nt.root", [n], str(root))
for _ in range(150):
    m = rng.randrange(1, 2500)
    a, t = rng.randrange(0, 2 * m), rng.randrange(0, 2 * m)
    x = next((x for x in range(2 * m + 2) if pow(a, x, m) == t % m), -1)
    add("nt.dlog", [a, t, m], str(x))
# Bases of small order, where several baby steps share a value and the
# smallest answer depends on which one the table keeps.
for _ in range(120):
    m = rng.choice(SMALL_PRIMES[5:300])
    k = rng.choice([2, 3, 4, 6])
    g = next(g for g in range(2, m) if order(g, m) == m - 1)
    a = pow(g, (m - 1) // k, m) if (m - 1) % k == 0 else m - 1
    t = pow(a, rng.randrange(0, 12), m)
    x = next((x for x in range(2 * m + 2) if pow(a, x, m) == t % m), -1)
    add("nt.dlog", [a, t, m], str(x))
for _ in range(120):
    n = rng.randrange(1, 2000)
    lam = 1
    for g in range(1, n + 1):
        if math.gcd(g, n) == 1:
            o = order(g, n)
            lam = lam * o // math.gcd(lam, o)
    add("nt.carmichael", [n], str(lam))
for _ in range(120):
    n = rng.randrange(1, 10 ** 7)
    f = factor(n)
    add("nt.mobius", [n], str(0 if len(set(f)) != len(f) else (-1) ** len(f)))
    add("nt.divisors", [n % 100000 + 1], ",".join(str(d) for d in range(1, n % 100000 + 2) if (n % 100000 + 1) % d == 0))
for _ in range(150):
    n = rng.choice([rng.randrange(0, 1000), rng.randrange(0, 1 << 62), (1 << 62) - 1, rng.randrange(0, 1 << 40) ** 1])
    add("nt.isqrt", [n], str(math.isqrt(n)))
    k = rng.randrange(1, 8)
    r = int(round(n ** (1 / k))) if n else 0
    while r ** k > n:
        r -= 1
    while (r + 1) ** k <= n:
        r += 1
    add("nt.iroot", [n, k], str(r))
    q = rng.choice([n, rng.randrange(2, 3000) ** rng.randrange(2, 5), rng.randrange(2, 100) ** 2 + 1])
    perfect = any(round(q ** (1 / e)) ** e == q or (round(q ** (1 / e)) + 1) ** e == q or (round(q ** (1 / e)) - 1) ** e == q
                  for e in range(2, 63) if q >= 4)
    add("nt.perfect", [q], "true" if perfect and q >= 4 else "false")


# --- continued fractions ------------------------------------------------------------------
def terms_of(fr):
    out = []
    while True:
        a = math.floor(fr)
        out.append(a)
        if fr == a:
            return out
        fr = 1 / (fr - a)


for _ in range(200):
    num, den = rng.randrange(-10 ** 9, 10 ** 9), rng.choice([1, -1]) * rng.randrange(1, 10 ** 9)
    t = terms_of(Fraction(num, den))
    conv = [Fraction(0)] * 0
    convs = []
    for i in range(len(t)):
        v = Fraction(t[i])
        for a in reversed(t[:i]):
            v = a + 1 / v
        convs.append(f"{v.numerator},{v.denominator}")
    ev = Fraction(num, den)
    add("cf.of", [num, den], ",".join(map(str, t)) + "/" + ";".join(convs) + f"/{ev.numerator},{ev.denominator}")
for _ in range(200):
    num, den = rng.randrange(-10 ** 12, 10 ** 12), rng.choice([1, -1]) * rng.randrange(1, 10 ** 12)
    lim = rng.choice([1, 2, 10, 113, 1000, rng.randrange(1, 10 ** 6)])
    r = Fraction(num, den).limit_denominator(lim)
    add("cf.limit", [num, den, lim], f"{r.numerator},{r.denominator}")
# Ties between the two candidates: Python keeps the convergent.
for _ in range(120):
    lim = rng.randrange(1, 30)
    q = rng.randrange(1, lim + 1)
    num, den = 2 * rng.randrange(-50, 50) + 1, 2 * q
    if rng.random() < 0.5:
        num, den = rng.randrange(-200, 200), rng.randrange(1, 200)
    r = Fraction(num, den).limit_denominator(lim)
    add("cf.limit", [num, den, lim], f"{r.numerator},{r.denominator}")
getcontext().prec = 600
for _ in range(120):
    n = rng.randrange(1, 4000)
    a0 = math.isqrt(n)
    if a0 * a0 == n:
        add("cf.sqrt", [n], f"{a0}/")
        continue
    x = Decimal(n).sqrt()
    terms = []
    for _ in range(2 * int(2 * math.sqrt(n)) + 4):
        a = int(x)
        terms.append(a)
        x = 1 / (x - a)
    period = []
    for t in terms[1:]:
        period.append(t)
        if t == 2 * a0:
            break
    add("cf.sqrt", [n], f"{a0}/" + ",".join(map(str, period)))


# --- NTT -------------------------------------------------------------------------------------
P = 998244353
for _ in range(150):
    a = [rng.randrange(-P, 2 * P) for _ in range(rng.randrange(0, 40))]
    b = [rng.randrange(0, P) for _ in range(rng.randrange(0, 40))]
    prod = [0] * (len(a) + len(b) - 1) if a and b else []
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            prod[i + j] = (prod[i + j] + x * y) % P
    add("ntt.mul", [",".join(map(str, a))], ",".join(map(str, prod)), ",".join(map(str, b)))
    cases[-1] = (cases[-1][0], ",".join(map(str, a)), cases[-1][2], cases[-1][3])
    n = rng.choice([1, 2, 4, 8, 16, 32])
    x = [rng.randrange(0, P) for _ in range(n)]
    y = [rng.randrange(0, P) for _ in range(n)]
    cyc = [sum(x[j] * y[(i - j) % n] for j in range(n)) % P for i in range(n)]
    cases.append(("ntt.cyc", ",".join(map(str, x)), ",".join(map(str, y)),
                  (lambda w: lambda got: None if got == w else f"want {w[:200]}")(",".join(map(str, cyc)))))
    cases.append(("ntt.tr", ",".join(map(str, x)), "",
                  (lambda w: lambda got: None if got == w else f"want {w[:200]}")(",".join(map(str, x)))))


# --- bit operations ---------------------------------------------------------------------------
def word():
    return rng.choice([0, 1, M64, 1 << 63, (1 << 63) - 1, rng.getrandbits(64), rng.getrandbits(rng.randrange(1, 64))])


for _ in range(250):
    u = word()
    x = s64(u)
    pop = bin(u).count("1")
    add("bit.pop", [x], f"{pop},{pop % 2},{64 - u.bit_length()},{(u & -u).bit_length() - 1 if u else 64},{u.bit_length()}")
    n = rng.randrange(-200, 200)
    k = n % 64
    rl = ((u << k) | (u >> (64 - k))) & M64 if k else u
    k2 = (-n) % 64
    rr = ((u << k2) | (u >> (64 - k2))) & M64 if k2 else u
    add("bit.rot", [x, n], f"{s64(rl)},{s64(rr)}")
    w = rng.randrange(1, 65)
    rev = int(format(u & ((1 << w) - 1), f"0{w}b")[::-1], 2)
    sw = [int.from_bytes((u & ((1 << (8 * k)) - 1)).to_bytes(k, "little"), "big") for k in (2, 4, 8)]
    add("bit.rev", [x, w], f"{s64(rev)},{s64(sw[0])},{s64(sw[1])},{s64(sw[2])}")
    npow = 1 if u <= 1 else (1 << (u - 1).bit_length()) if u <= 1 << 63 else 0
    add("bit.pow2", [x], f"{s64(npow)},{'true' if u and u & (u - 1) == 0 else 'false'},{s64(u & -u)},"
                         f"{s64(1 << (u.bit_length() - 1)) if u else 0}")
    g = u
    dec = 0
    while g:
        dec ^= g
        g >>= 1
    add("bit.gray", [x], f"{s64(u ^ (u >> 1))},{s64(dec)}")
    mask = word()
    dep, ext, k = 0, 0, 0
    for i in range(64):
        if mask >> i & 1:
            if u >> k & 1:
                dep |= 1 << i
            if u >> i & 1:
                ext |= 1 << k
            k += 1
    add("bit.pdep", [x, s64(mask)], f"{s64(dep)},{s64(ext)}")
    y = word()
    z = 0
    for i in range(32):
        z |= ((u >> i) & 1) << (2 * i) | ((y >> i) & 1) << (2 * i + 1)
    ev = sum(((u >> (2 * i)) & 1) << i for i in range(32))
    od = sum(((u >> (2 * i + 1)) & 1) << i for i in range(32))
    add("bit.morton", [x, s64(y)], f"{s64(z)},{ev},{od}")
    add("bit.cmp", [x, s64(y)], str((u > y) - (u < y)))


# --- half precision and bfloat16 ------------------------------------------------------------------
def interesting_double():
    return rng.choice([rng.uniform(-70000, 70000), rng.uniform(-1, 1), rng.uniform(-1e-4, 1e-4),
                       math.ldexp(rng.uniform(1, 2), rng.randrange(-30, 18)) * rng.choice([1, -1]),
                       65504.0, 65519.99, 65520.0, 2 ** -24, 2 ** -25, 3 * 2 ** -26, 0.0, -0.0, math.inf, -math.inf,
                       float("nan"), 1e300, -1e-300, 1 + 2 ** -11, 1 + 3 * 2 ** -11,
                       math.ldexp(rng.uniform(1, 2), rng.randrange(-150, 130)), 3.4028235677973366e38,
                       3.4028235e38 * 1.0000001, 1.401298464324817e-45, 7e-46])


def f16_bits(v):
    try:
        return struct.unpack("<H", struct.pack("<e", v))[0]
    except OverflowError:
        return 0xFC00 if v < 0 else 0x7C00


def f32_bits(v):
    try:
        return struct.unpack("<I", struct.pack("<f", v))[0]
    except OverflowError:
        return 0xFF800000 if v < 0 else 0x7F800000


def bf16_round(b):
    if (b & 0x7F800000) == 0x7F800000 and b & 0x7FFFFF:
        return (b >> 16) | 0x40
    low = b >> 16
    value = struct.unpack("<f", struct.pack("<I", b))[0]
    candidates = [low, low + 1]

    def val(h):
        if (h & 0x7FFF) == 0x7F80:
            return math.copysign(2.0 ** 128, value if value else 1.0)
        return struct.unpack("<f", struct.pack("<I", (h & 0xFFFF) << 16))[0]
    if b & 0xFFFF == 0:
        return low
    d0, d1 = abs(val(low) - value), abs(val(low + 1) - value)
    if d0 < d1:
        return low
    if d1 < d0:
        return (low + 1) & 0xFFFF
    return low if low % 2 == 0 else (low + 1) & 0xFFFF


for _ in range(400):
    v = interesting_double()
    add("f16.from", [bits(v)], str(f16_bits(v)))
    h = rng.randrange(0, 65536)
    if (h & 0x7C00) != 0x7C00 or not h & 0x3FF:
        add("f16.to", [h], str(bits(struct.unpack("<e", struct.pack("<H", h))[0])))
    add("bf.f32", [bits(v)], str(f32_bits(v)))
    b32 = rng.choice([rng.getrandbits(32), f32_bits(v), 0x7F7FFFFF, 0x7F7F8000, 0x3F808000, 0x3F818000, 0x7FC00001,
                      0xFF800001, 0x00008000])
    add("bf.from", [b32], str(bf16_round(b32)))
    if (b32 & 0x7F800000) != 0x7F800000 or not b32 & 0x7FFFFF:
        add("bf.f32back", [b32], str(bits(struct.unpack("<f", struct.pack("<I", b32))[0])))
    hb = rng.randrange(0, 65536)
    if (hb & 0x7F80) != 0x7F80 or not hb & 0x7F:
        add("bf.to", [hb], str(bits(struct.unpack("<f", struct.pack("<I", hb << 16))[0])))


# --- special functions -----------------------------------------------------------------------------
def c99(name, x, y=None):
    try:
        if name == "beta":
            if x == int(x) and y == int(y) and 0 < x < 60 and 0 < y < 60:
                fr = Fraction(math.factorial(int(x) - 1) * math.factorial(int(y) - 1), math.factorial(int(x + y) - 1))
                return float(fr)
            g = math.gamma(x) * math.gamma(y) / math.gamma(x + y) if x + y < 171 else \
                math.exp(math.lgamma(x) + math.lgamma(y) - math.lgamma(x + y))
            return g
        return getattr(math, name)(x)
    except OverflowError:
        if name == "lgamma":
            return math.inf
        return math.copysign(math.inf, 1.0 if name != "gamma" or x > 0 else 1.0)
    except ValueError:
        if name == "gamma":
            return math.copysign(math.inf, x) if x == 0 else math.nan
        if name == "lgamma":
            return math.inf
        if name == "log1p":
            return -math.inf if x == -1 else math.nan
        return math.nan


TOL = {"gamma": 5e-13, "lgamma": 2e-13, "erf": 2e-15, "erfc": 2e-14, "expm1": 2e-15, "log1p": 2e-15, "asinh": 2e-15,
       "exp2": 4e-15, "beta": 5e-11}


def close(got_bits, want, tol, absolute=0.0):
    g = dbl(int(got_bits))
    if math.isnan(want) or math.isnan(g):
        return math.isnan(want) and math.isnan(g)
    if math.isinf(want) or math.isinf(g):
        return g == want
    return abs(g - want) <= tol * abs(want) + absolute


def sf_values(name):
    base = [rng.uniform(-6, 6), rng.uniform(-30, 30), rng.uniform(0, 1e-3), rng.uniform(-1e-3, 1e-3),
            float(rng.randrange(-5, 10)), rng.randrange(-5, 10) + 0.5, rng.uniform(0, 175), rng.uniform(-170, -0.5),
            1.0, 2.0, 0.0, -0.0, math.inf, -math.inf, float("nan"), 1e-300, rng.uniform(1.4, 1.6), 30.0, 27.0]
    if name in ("expm1", "exp2"):
        base += [rng.uniform(-750, 1100), 1e-10, -1e-10]
    if name == "log1p":
        base += [-1.0, -2.0, rng.uniform(-1, 0), 1e300]
    return base


for name in ["gamma", "lgamma", "erf", "erfc", "expm1", "log1p", "asinh", "exp2"]:
    for _ in range(140):
        x = rng.choice(sf_values(name))
        want = c99(name, x)
        absolute = 2e-15 if name == "lgamma" else 0.0
        if name == "gamma" and x < 0:
            tol = 1e-10
        elif name == "lgamma" and x < 0:
            tol, absolute = 1e-10, 1e-12
        else:
            tol = TOL[name]
        add_check("sf." + name, [bits(x)], (lambda w, t, ab: lambda got: None if close(got, w, t, ab) else f"want {w!r}, got {dbl(int(got))!r}")(want, tol, absolute))
# The continued-fraction range of erf and erfc, densely: where x^2 is
# large, an error in it is magnified most.
for _ in range(300):
    name = rng.choice(["erf", "erfc"])
    x = rng.choice([1, -1]) * rng.uniform(1.5, 27.2)
    want = c99(name, x)
    add_check("sf." + name, [bits(x)], (lambda w: lambda got: None if close(got, w, TOL["erfc"]) else f"want {w!r}, got {dbl(int(got))!r}")(want))
for _ in range(150):
    x, y = rng.choice([rng.uniform(0.01, 20), float(rng.randrange(1, 30)), rng.uniform(50, 300)]), \
        rng.choice([rng.uniform(0.01, 20), float(rng.randrange(1, 30))])
    want = c99("beta", x, y)
    add_check("sf.beta", [bits(x), bits(y)], (lambda w: lambda got: None if close(got, w, TOL["beta"]) else f"want {w!r}, got {dbl(int(got))!r}")(want))


# --- complex functions -------------------------------------------------------------------------------
def czs():
    return rng.choice([complex(rng.uniform(-3, 3), rng.uniform(-3, 3)), complex(rng.uniform(-30, 30), rng.uniform(-30, 30)),
                       complex(rng.uniform(-3, 3), 0.0), complex(0.0, rng.uniform(-3, 3)), complex(rng.uniform(-1e-3, 1e-3),
                       rng.uniform(-1e-3, 1e-3)), complex(-2.0, 0.0), complex(2.0, 0.0), complex(0.5, 0.0),
                       complex(-0.5, 0.0), complex(0.0, 2.0), complex(0.0, -2.0), complex(1.0, 1.0), complex(-4.0, 0.0)])


def close_c(got, want, tol=2e-12):
    parts = got.split(",")
    g = complex(dbl(int(parts[0])), dbl(int(parts[1])))
    return abs(g - want) <= tol * max(abs(want), 1e-300) + 1e-300


for name in ["sqrt", "log", "log10", "exp", "sin", "cos", "tan", "sinh", "cosh", "tanh", "asin", "acos", "atan", "asinh",
             "acosh", "atanh"]:
    for _ in range(110):
        z = czs()
        try:
            want = getattr(cmath, name)(z)
        except (ValueError, OverflowError):
            continue
        add_check("cx." + name, [bits(z.real), bits(z.imag)],
                  (lambda w: lambda got: None if close_c(got, w) else f"want {w!r}, got {got}")(want))
for _ in range(110):
    z, base = czs(), czs()
    try:
        want = cmath.log(z, base)
    except (ValueError, ZeroDivisionError):
        continue
    add_check("cx.logb", [bits(z.real), bits(z.imag), bits(base.real), bits(base.imag)],
              (lambda w: lambda got: None if close_c(got, w, 1e-11) else f"want {w!r}")(want))
    r, p = cmath.polar(z)
    add_check("cx.polar", [bits(z.real), bits(z.imag)],
              (lambda w: lambda got: None if close_c(got, w) else f"want {w!r}")(complex(r, p)))
    add_check("cx.rect", [bits(abs(z.real)), bits(z.imag)],
              (lambda w: lambda got: None if close_c(got, w) else f"want {w!r}")(cmath.rect(abs(z.real), z.imag)))


def run(picked):
    argv = []
    for op, a, b, _ in picked:
        argv += [op, a, b]
    lines, start = [], 0
    while start < len(argv):
        chunk = argv[start:start + 1500]
        start += 1500
        try:
            out = subprocess.run([harness] + chunk, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("the harness did not finish in 300 s"); sys.exit(1)
        if out.returncode != 0:
            print(out.stderr[-2000:]); sys.exit(1)
        lines += out.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, a, b, check) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    try:
        why = check(mine) if mine != "<missing>" else "missing"
    except Exception as e:  # a malformed answer is a disagreement, not a crash
        why = f"unreadable answer ({e})"
    fam = families.setdefault(op.split(".")[0], [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        only = os.environ.get("ONLY", "")
        if op.startswith(only) and (differ <= 12 or only):
            print(f"  {op} {a[:120]} {b[:60]}")
            print(f"    grasm {mine[:200]}")
            print(f"    {why[:300]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 8:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
