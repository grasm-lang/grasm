"""ecc, ecdh, ecdsa, dsa, rfc6979 and modarith against OpenSSL and Python.

The curve packages are checked on eight curves:

Every expected answer comes from the `openssl` command line: public keys
from `pkey -pubout`, shared secrets from `pkeyutl -derive`, signatures from
`pkeyutl -sign -pkeyopt nonce-type:1` -- RFC 6979's deterministic nonces, so
a signature is compared byte for byte -- and verdicts from `pkeyutl
-verify`, including on signatures and keys that have been damaged.

SM2 is checked for public keys only: OpenSSL treats an SM2 key as the SM2
algorithm, whose key exchange and signature are different protocols.

DSA is checked the same way at four sizes of parameters OpenSSL generates
here -- public keys, deterministic signatures with SHA-1 and SHA-2, and
verdicts on damaged ones -- and `rfc6979` through both. `modarith` is
checked against Python's own `pow`, including `pow(a, -1, m)`.
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
harness = os.path.join(work, "pubkeytest")

stale = subprocess.run([sys.executable, "scripts/gen_curves.py", "--check",
                        "packages/ecc/src/main.gr"], cwd=ROOT,
                       capture_output=True, text=True)
if stale.returncode != 0:
    print(stale.stdout + stale.stderr); sys.exit(1)
built = subprocess.run([GRASM, "build", "packages/pubkeytest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

# name: (OpenSSL OID, order, scalar bytes)
OIDS = {"P-256": "2a8648ce3d030107", "P-384": "2b81040022",
        "P-521": "2b81040023", "secp256k1": "2b8104000a",
        "brainpoolP256r1": "2b2403030208010107",
        "brainpoolP384r1": "2b240303020801010b",
        "brainpoolP512r1": "2b240303020801010d", "SM2": "2a811ccf5501822d"}
ORDERS = {}
for name, ossl in [("P-256", "prime256v1"), ("P-384", "secp384r1"),
                   ("P-521", "secp521r1"), ("secp256k1", "secp256k1"),
                   ("brainpoolP256r1", "brainpoolP256r1"),
                   ("brainpoolP384r1", "brainpoolP384r1"),
                   ("brainpoolP512r1", "brainpoolP512r1"), ("SM2", "SM2")]:
    text = subprocess.run(["openssl", "ecparam", "-name", ossl, "-param_enc",
                           "explicit", "-text", "-noout"], capture_output=True,
                          text=True).stdout
    order = text.split("Order:")[1].split("Cofactor")[0]
    ORDERS[name] = int(order.replace(":", "").replace("\n", "").replace(" ", ""), 16)


def size(name):
    return (ORDERS[name].bit_length() + 7) // 8


def tlv(tag, body):
    n = len(body)
    if n < 128:
        head = bytes([n])
    elif n < 256:
        head = bytes([0x81, n])
    else:
        head = bytes([0x82, n >> 8, n & 255])
    return bytes([tag]) + head + body


def private_der(name, k):
    oid = tlv(6, bytes.fromhex(OIDS[name]))
    return tlv(0x30, tlv(2, b"\x01") + tlv(4, k) + tlv(0xA0, oid))


def public_der(name, point):
    alg = tlv(0x30, tlv(6, bytes.fromhex("2a8648ce3d0201")) +
              tlv(6, bytes.fromhex(OIDS[name])))
    return tlv(0x30, alg + tlv(3, b"\x00" + point))


counter = [0]


def path(suffix, data):
    counter[0] += 1
    p = os.path.join(work, f"f{counter[0]}{suffix}")
    open(p, "wb").write(data)
    return p


def openssl(args):
    ran = subprocess.run(["openssl"] + args, capture_output=True)
    return ran.returncode, ran.stdout


def read_tlv(der, at):
    """(tag, content start, content end) of the element at `at`."""
    tag, n = der[at], der[at + 1]
    at += 2
    if n & 0x80:
        width = n & 0x7F
        n = int.from_bytes(der[at:at + width], "big")
        at += width
    return tag, at, at + n


def spki_point(der):
    tag, body, end = read_tlv(der, 0)
    _, _, alg_end = read_tlv(der, body)
    tag, start, stop = read_tlv(der, alg_end)
    if tag != 3 or der[start] != 0:
        raise ValueError("not a SubjectPublicKeyInfo")
    return der[start + 1:stop]


def ossl_public(name, k):
    key = path(".der", private_der(name, k))
    out = []
    for form in ["uncompressed", "compressed"]:
        # `ec`, not `pkey`: `pkey -ec_conv_form` refuses an SM2 key.
        code, der = openssl(["ec", "-inform", "DER", "-in", key, "-pubout",
                             "-outform", "DER", "-conv_form", form])
        if code != 0 or not der:
            return "ERR|ERR"
        out.append(spki_point(der).hex())
    return "|".join(out)


def scalar(name, kind=None):
    n = ORDERS[name]
    kind = kind or rng.choice(["rand", "rand", "rand", "small", "top"])
    if kind == "small":
        v = rng.randrange(1, 1000)
    elif kind == "top":
        v = n - rng.randrange(1, 1000)
    else:
        v = rng.randrange(1, n)
    return v.to_bytes(size(name), "big")


cases = []      # (op, text, bytes)
jobs = []       # a function computing the expected answer


def add(op, text, data, job):
    cases.append((op, text, data))
    jobs.append(job)


def pub_job(name, k):
    return lambda: ossl_public(name, k)


for name in OIDS:
    for _ in range(14):
        k = scalar(name)
        add("pub", name, k, pub_job(name, k))
for name, bad in [("P-256", b"\x00" * 32), ("P-256", ORDERS["P-256"].to_bytes(32, "big")),
                  ("P-256", b"\x01" * 31), ("secp256k1", b"\xff" * 32)]:
    add("pub", name, bad, lambda: "ERR|ERR")

DH_CURVES = [c for c in OIDS if c != "SM2"]
points = {}


def dh_job(name, k, peer_point):
    def run():
        key = path(".der", private_der(name, k))
        peer = path(".der", public_der(name, peer_point))
        code, out = openssl(["pkeyutl", "-derive", "-inkey", key, "-keyform",
                             "DER", "-peerkey", peer, "-peerform", "DER"])
        return out.hex() if code == 0 else "ERR"
    return run


# Peers' points come from OpenSSL, then the exchange is asked of both.
peer_keys = [(name, scalar(name)) for name in DH_CURVES for _ in range(10)]
with concurrent.futures.ThreadPoolExecutor(16) as pool:
    peer_points = list(pool.map(lambda nk: ossl_public(*nk), peer_keys))
for (name, pk), pts in zip(peer_keys, peer_points):
    unc, comp = pts.split("|")
    k = scalar(name)
    point = bytes.fromhex(unc)
    add("dh", name, k + point, dh_job(name, k, point))
    # A compressed peer is the same peer.
    add("dh", name, k + bytes.fromhex(comp), dh_job(name, k, point))
    # A point moved off the curve must be refused by both.
    off = bytearray(point)
    off[-1] ^= 1
    add("dh", name, k + bytes(off), dh_job(name, k, bytes(off)))
    add("valid", name, bytes(off), lambda: "no")
    add("valid", name, point, lambda: "yes")

HASHES = {"SHA-256": ("sha256", 32), "SHA-384": ("sha384", 48),
          "SHA-512": ("sha512", 64)}


def sign_job(name, k, hname, digest):
    def run():
        key = path(".der", private_der(name, k))
        h = path(".bin", digest)
        code, out = openssl(["pkeyutl", "-sign", "-inkey", key, "-keyform",
                             "DER", "-in", h, "-pkeyopt", "nonce-type:1",
                             "-pkeyopt", f"digest:{HASHES[hname][0]}"])
        return out.hex() if code == 0 else "ERR"
    return run


def verify_job(name, point, digest, der):
    def run():
        pub = path(".der", public_der(name, point))
        h = path(".bin", digest)
        sig = path(".sig", der)
        code, _ = openssl(["pkeyutl", "-verify", "-pubin", "-inkey", pub,
                           "-keyform", "DER", "-in", h, "-sigfile", sig])
        return "yes" if code == 0 else "no"
    return run


sign_cases = []
for name in DH_CURVES:
    for hname in HASHES:
        for _ in range(6):
            k = scalar(name)
            digest = os.urandom(HASHES[hname][1])
            sign_cases.append((name, k, hname, digest))
            add("sign", f"{name},{hname}", k + digest,
                sign_job(name, k, hname, digest))

# Verdicts: real signatures from OpenSSL, and the same damaged.
verify_inputs = []
for name, k, hname, digest in sign_cases[::2]:
    verify_inputs.append((name, k, hname, digest))
with concurrent.futures.ThreadPoolExecutor(16) as pool:
    made = list(pool.map(lambda c: (sign_job(c[0], c[1], c[2], c[3])(),
                                    ossl_public(c[0], c[1]).split("|")[0]),
                         verify_inputs))
for (name, k, hname, digest), (sig_hex, pub_hex) in zip(verify_inputs, made):
    point = bytes.fromhex(pub_hex)
    der = bytes.fromhex(sig_hex)
    variants = [der]
    d = bytearray(der)
    d[-1] ^= 1
    variants.append(bytes(d))
    d = bytearray(der)
    d[2] = d[2] + 1 if d[2] < 127 else d[2]
    variants.append(bytes(d))
    variants.append(der + b"\x00")
    other = bytearray(digest)
    other[0] ^= 0x80
    for v in variants:
        add("verify", f"{name},{len(point)},{len(digest)}", point + digest + v,
            verify_job(name, point, digest, v))
    add("verify", f"{name},{len(point)},{len(digest)}", point + bytes(other) + der,
        verify_job(name, point, bytes(other), der))

# --- DSA ---------------------------------------------------------------


def der_ints(der):
    out, at = [], 0
    tag, at, end = read_tlv(der, 0)
    while at < end:
        tag, start, stop = read_tlv(der, at)
        out.append(int.from_bytes(der[start:stop], "big"))
        at = stop
    return out


def dsa_params(bits, qbits):
    code, der = openssl(["genpkey", "-genparam", "-algorithm", "DSA",
                         "-pkeyopt", f"dsa_paramgen_bits:{bits}",
                         "-pkeyopt", f"dsa_paramgen_q_bits:{qbits}",
                         "-outform", "DER"])
    if code != 0:
        print(f"OpenSSL could not make {bits}/{qbits} DSA parameters")
        sys.exit(1)
    # `-genparam` writes PEM whatever `-outform` says.
    if der.startswith(b"-----"):
        import base64
        body = b"".join(l for l in der.splitlines() if not l.startswith(b"-----"))
        der = base64.b64decode(body)
    return der_ints(der)[:3]


def int_bytes(v, n=None):
    n = n or max(1, (v.bit_length() + 7) // 8)
    return v.to_bytes(n, "big")


def packed(*parts):
    return b"".join(len(p).to_bytes(2, "big") + p for p in parts)


def dsa_private_der(p, q, g, x):
    y = pow(g, x, p)
    ints = [0, p, q, g, y, x]
    body = b"".join(tlv(2, (v.to_bytes((v.bit_length() + 8) // 8, "big")
                            if v else b"\x00")) for v in ints)
    return tlv(0x30, body)


DSA_HASH = {"SHA-1": ("sha1", 20), "SHA-256": ("sha256", 32),
            "SHA-384": ("sha384", 48), "SHA-512": ("sha512", 64)}


def dsa_sign_job(key_der, hname, digest):
    def run():
        key = path(".der", key_der)
        h = path(".bin", digest)
        code, out = openssl(["pkeyutl", "-sign", "-inkey", key, "-keyform",
                             "DER", "-in", h, "-pkeyopt", "nonce-type:1",
                             "-pkeyopt", f"digest:{DSA_HASH[hname][0]}"])
        return out.hex() if code == 0 else "ERR"
    return run


def dsa_verify_job(key_der, digest, sig):
    def run():
        key = path(".der", key_der)
        h = path(".bin", digest)
        sf = path(".sig", sig)
        code, _ = openssl(["pkeyutl", "-verify", "-inkey", key, "-keyform",
                           "DER", "-in", h, "-sigfile", sf])
        return "yes" if code == 0 else "no"
    return run


with concurrent.futures.ThreadPoolExecutor(4) as pool:
    dsa_sets = list(pool.map(lambda s: dsa_params(*s),
                             [(1024, 160), (2048, 224), (2048, 256), (3072, 256)]))
for p_, q_, g_ in dsa_sets:
    qb = (q_.bit_length() + 7) // 8
    base = [int_bytes(p_), int_bytes(q_), int_bytes(g_)]
    hashes = ["SHA-1", "SHA-256"] if q_.bit_length() == 160 else ["SHA-256", "SHA-384", "SHA-512"]
    for _ in range(4):
        x = rng.randrange(1, q_)
        y = pow(g_, x, p_)
        add("dsapub", "", packed(*base, int_bytes(x, qb)),
            (lambda y=y, n=len(base[0]): int_bytes(y, n).hex()))
        key_der = dsa_private_der(p_, q_, g_, x)
        for hname in hashes:
            digest = os.urandom(DSA_HASH[hname][1])
            add("dsasign", hname, packed(*base, int_bytes(x, qb), digest),
                dsa_sign_job(key_der, hname, digest))
            sig = bytes.fromhex(dsa_sign_job(key_der, hname, digest)())
            yb = int_bytes(y, len(base[0]))
            bad = bytearray(sig)
            bad[-1] ^= 1
            other = bytearray(digest)
            other[0] ^= 1
            for d, s_ in [(digest, sig), (digest, bytes(bad)), (bytes(other), sig)]:
                add("dsaverify", "", packed(*base, yb, d, s_),
                    dsa_verify_job(key_der, d, s_))
    # A private key of zero or of q is refused by both.
    for bad_x in [0, q_]:
        add("dsapub", "", packed(*base, int_bytes(bad_x, qb)), lambda: "ERR")

# --- modarith, against Python ---------------------------------------------
for _ in range(400):
    bits = rng.choice([3, 8, 26, 27, 52, 64, 100, 255, 256, 384, 521, 1024, 2048])
    m = rng.randrange(1 << (bits - 1), 1 << bits) | 1
    a = rng.randrange(0, m * rng.choice([1, 1, 3, 1000]))
    b = rng.randrange(0, m)
    e = rng.randrange(0, 1 << rng.choice([1, 8, 64, 300]))
    mb = int_bytes(m)
    want_pow = pow(a, e, m)
    add("modpow", "", packed(int_bytes(a), int_bytes(e), mb),
        (lambda v=want_pow, n=len(mb): int_bytes(v, n).hex()))
    eb = int_bytes(e)
    add("modpowsecret", "", packed(int_bytes(a), eb, mb),
        (lambda v=pow(a, e, m), n=len(mb): int_bytes(v, n).hex()))
    add("modmul", "", packed(int_bytes(a), int_bytes(b), mb),
        (lambda v=(a * b) % m, n=len(mb): int_bytes(v, n).hex()))
    try:
        inv = pow(a, -1, m)
        want_inv = int_bytes(inv, len(mb)).hex()
    except ValueError:
        want_inv = "NONE"
    add("modinv", "", packed(int_bytes(a), mb), (lambda v=want_inv: v))
for bad_m in [b"\x00", b"\x01", b"\x10", b"\x00\x04"]:
    add("modpow", "", packed(b"\x02", b"\x03", bad_m), lambda: "ERR")

with concurrent.futures.ThreadPoolExecutor(16) as pool:
    expected = list(pool.map(lambda j: j(), jobs))

argv = []
for op, text, data in cases:
    argv += [op, text, "x" + data.hex()]
got = []
for start in range(0, len(argv), 3 * 100):
    ran = subprocess.run([harness] + argv[start:start + 300],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    got += ran.stdout.split("\n")[:-1]

families = {}
agree = differ = 0
for i, (op, text, data) in enumerate(cases):
    want, mine = expected[i], (got[i] if i < len(got) else "<missing>")
    fam = families.setdefault(f"{op}:{text.split(',')[0]}", [0, 0])
    if want == mine:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 8:
            print(f"  {op} {text} {data.hex()[:80]}")
            print(f"    openssl {want[:120]}")
            print(f"    grasm   {mine[:120]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 4]
if thin or len(families) < 30:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
