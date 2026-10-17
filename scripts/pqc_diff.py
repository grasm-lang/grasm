"""Post-quantum packages against OpenSSL 3.6, byte for byte.

ML-KEM at all three sizes: keys generated from the same 64-byte seed
(`genpkey -pkeyopt hexseed:`), encapsulation with the same 32-byte message
(`pkeyutl -encap -pkeyopt hexikme:`), and decapsulation -- of good
ciphertexts, and of damaged ones, where both sides must agree on FIPS 203's
implicit-rejection key rather than merely both fail.
"""
import concurrent.futures
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)
work = tempfile.mkdtemp()
harness = os.path.join(work, "pqctest")

if b"ML-KEM-768" not in subprocess.run(["openssl", "list", "-kem-algorithms"],
                                       capture_output=True).stdout:
    print("this OpenSSL has no ML-KEM, so nothing was checked")
    sys.exit(1)
built = subprocess.run([GRASM, "build", "packages/pqctest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

counter = [0]


def fresh(suffix):
    counter[0] += 1
    return os.path.join(work, f"f{counter[0]}{suffix}")


def openssl(args):
    ran = subprocess.run(["openssl"] + args, capture_output=True)
    return ran.returncode, ran.stdout


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


EK = {"ML-KEM-512": 800, "ML-KEM-768": 1184, "ML-KEM-1024": 1568}


def mlkem_expect(variant, seed, m):
    key = fresh(".pem")
    if openssl(["genpkey", "-algorithm", variant, "-pkeyopt",
                f"hexseed:{seed.hex()}", "-out", key])[0] != 0:
        return "ERR", None
    pub = fresh(".pem")
    openssl(["pkey", "-in", key, "-pubout", "-out", pub])
    code, der = openssl(["pkey", "-in", key, "-pubout", "-outform", "DER"])
    ek = der[-EK[variant]:]
    ct, ss = fresh(".ct"), fresh(".ss")
    openssl(["pkeyutl", "-encap", "-pubin", "-inkey", pub, "-pkeyopt",
             f"hexikme:{m.hex()}", "-out", ct, "-secret", ss])
    c, s = open(ct, "rb").read(), open(ss, "rb").read()
    back = fresh(".ss")
    openssl(["pkeyutl", "-decap", "-inkey", key, "-in", ct, "-out", back])
    d = open(back, "rb").read()
    return f"{ek.hex()}|{s.hex()}|{c.hex()}|{d.hex()}", key


def decap_expect(key, ct):
    path = fresh(".ct")
    open(path, "wb").write(ct)
    out = fresh(".ss")
    code, _ = openssl(["pkeyutl", "-decap", "-inkey", key, "-in", path, "-out", out])
    return open(out, "rb").read().hex() if code == 0 else "ERR"


cases, expected = [], []
seeds = []
for variant in EK:
    for _ in range(12):
        seeds.append((variant, rb(64), rb(32)))
with concurrent.futures.ThreadPoolExecutor(12) as pool:
    first = list(pool.map(lambda s: mlkem_expect(*s), seeds))

damaged = []
for (variant, seed, m), (want, key) in zip(seeds, first):
    cases.append(("mlk", variant, seed + m))
    expected.append(want)
    if want == "ERR":
        continue
    ct = bytes.fromhex(want.split("|")[2])
    for _ in range(3):
        bad = bytearray(ct)
        bad[rng.randrange(len(bad))] ^= 1 << rng.randrange(8)
        damaged.append((variant, seed, bytes(bad), key))
    # A ciphertext of the wrong length is refused by both.
    damaged.append((variant, seed, ct[:-1], key))
with concurrent.futures.ThreadPoolExecutor(12) as pool:
    rejected = list(pool.map(lambda d: decap_expect(d[3], d[2]), damaged))
for (variant, seed, ct, _), want in zip(damaged, rejected):
    cases.append(("mlkdec", variant, seed + ct))
    expected.append(want)

# An encapsulation key holding a coefficient of q or more fails FIPS 203's
# modulus check; OpenSSL will not even import it.
for (variant, seed, m), (want, _) in zip(seeds[::4], first[::4]):
    ek = bytearray(bytes.fromhex(want.split("|")[0]))
    ek[0], ek[1] = 0xFF, ek[1] | 0x0F
    cases.append(("mlkenc", variant, bytes(ek) + m))
    expected.append("ERR")

argv = []
for op, text, data in cases:
    argv += [op, text, "x" + data.hex()]
got = []
for start in range(0, len(argv), 3 * 40):
    ran = subprocess.run([harness] + argv[start:start + 120],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    got += ran.stdout.split("\n")[:-1]

families = {}
agree = differ = 0
for i, (op, text, data) in enumerate(cases):
    want, mine = expected[i], (got[i] if i < len(got) else "<missing>")
    fam = families.setdefault(f"{op}:{text}", [0, 0])
    if want == mine:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 6:
            print(f"  {op} {text}")
            for label, v in [("openssl", want), ("grasm", mine)]:
                print(f"    {label:8}" + " ".join(p[:24] for p in v.split("|")))
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 3]
if thin or len(families) < 9:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
