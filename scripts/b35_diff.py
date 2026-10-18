"""Key files against OpenSSL 3.6.

A pool of keys from `openssl genpkey` (RSA of several sizes, three-prime
RSA, e = 3, and EC on five curves), converted by `openssl rsa`, `ec`, `pkey`
and `pkcs8`, then:

- `pkcs1`, `sec1`: every structure read and written back byte for byte, and
  converted to and from PKCS#8 and SubjectPublicKeyInfo exactly as OpenSSL
  converts it; SEC1 with explicit curve parameters is refused.
- `encpkcs8` and `pbes`: keys OpenSSL encrypted (PBES2 with five PRFs and
  eleven ciphers, scrypt, and the ten PBES1 and PKCS#12 schemes) opened
  here; keys encrypted here, with algorithms written by `pbes`'s builders,
  opened by OpenSSL; and wrong passwords refused by both.
- `pemcrypt`: traditional keys OpenSSL encrypted with eleven ciphers
  opened here, and keys encrypted here opened by OpenSSL.
- `pbkdf2`: against OpenSSL's PBKDF2 over six hashes, in-process.
"""
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib"))
import evp  # noqa: E402

rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b35")
harness = os.path.join(work, "b35test")
built = subprocess.run([GRASM, "build", "packages/b35test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
LEGACY = ["-provider", "legacy", "-provider", "default"]


def ossl(args, data=b""):
    r = subprocess.run(["openssl"] + args, input=data, capture_output=True)
    return r.stdout if r.returncode == 0 else None


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def run_harness(cases):
    out, batch, size = [], [], 0

    def flush():
        nonlocal batch, size
        if not batch:
            return
        argv = []
        for op, fields in batch:
            argv += [op, "x", "x" + fields.encode().hex()]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=1800)
        if ran.returncode != 0:
            for op, fields in batch:
                one = subprocess.run([harness, op, "x", "x" + fields.encode().hex()], capture_output=True, text=True)
                if one.returncode != 0:
                    print(f"harness crashed on {op} {fields[:300]}")
                    break
            print(ran.stderr[-2000:]); sys.exit(1)
        out.extend(ran.stdout.split("\n")[:-1])
        batch, size = [], 0
    for c in cases:
        batch.append(c)
        size += 2 * len(c[1]) + 30
        if size > 300000 or len(batch) >= 200:
            flush()
    flush()
    return out


# --- the key pool ---------------------------------------------------------------------------
def to_pkcs8(der):
    return ossl(["pkcs8", "-topk8", "-nocrypt", "-inform", "DER", "-outform", "DER"], der)


rsa_specs = [["rsa_keygen_bits:512"], ["rsa_keygen_bits:768"], ["rsa_keygen_bits:1024"], ["rsa_keygen_bits:1024"],
             ["rsa_keygen_bits:2048"], ["rsa_keygen_bits:1024", "rsa_keygen_primes:3"], ["rsa_keygen_bits:768", "rsa_keygen_pubexp:3"]]
rsa = []
for spec in rsa_specs:
    args = ["genpkey", "-algorithm", "RSA", "-outform", "DER"]
    for o in spec:
        args += ["-pkeyopt", o]
    # genpkey's DER is the traditional form for RSA and EC; make it PKCS#8.
    p8 = to_pkcs8(ossl(args))
    rsa.append({"p8": p8, "p1": ossl(["rsa", "-inform", "DER", "-traditional", "-outform", "DER"], p8),
                "pub1": ossl(["rsa", "-inform", "DER", "-RSAPublicKey_out", "-outform", "DER"], p8),
                "spki": ossl(["pkey", "-inform", "DER", "-pubout", "-outform", "DER"], p8)})
ec = []
for curve in ["P-256", "P-384", "P-521", "secp256k1", "brainpoolP256r1", "P-256", "P-384"]:
    p8 = to_pkcs8(ossl(["genpkey", "-algorithm", "EC", "-pkeyopt", f"ec_paramgen_curve:{curve}", "-outform", "DER"]))
    ec.append({"p8": p8, "sec1": ossl(["ec", "-inform", "DER", "-outform", "DER"], p8),
               "nopub": ossl(["ec", "-inform", "DER", "-no_public", "-outform", "DER"], p8),
               "compressed": ossl(["ec", "-inform", "DER", "-conv_form", "compressed", "-outform", "DER"], p8),
               "explicit": ossl(["ec", "-inform", "DER", "-param_enc", "explicit", "-outform", "DER"], p8)})
others = [to_pkcs8(ossl(["genpkey", "-algorithm", a, "-outform", "DER"])) for a in ["ED25519", "X25519", "ED448"]]
all_p8 = [k["p8"] for k in rsa] + [k["p8"] for k in ec] + others

cases, wants = [], []


def add(op, fields, want):
    cases.append((op, "|".join(fields)))
    wants.append("ERR" if want is None else want.hex())


H = bytes.hex
for k in rsa:
    add("pkcs1.round", [H(k["p1"])], k["p1"])
    add("pkcs1.public", [H(k["p1"])], k["pub1"])
    add("pkcs1.pubround", [H(k["pub1"])], k["pub1"])
    add("pkcs1.to8", [H(k["p1"])], k["p8"])
    add("pkcs1.from8", [H(k["p8"])], k["p1"])
    add("pkcs1.tospki", [H(k["pub1"])], k["spki"])
    add("pkcs1.fromspki", [H(k["spki"])], k["pub1"])
for k in ec:
    for form in ["sec1", "nopub", "compressed"]:
        add("sec1.round", [H(k[form])], k[form])
    add("sec1.to8", [H(k["sec1"])], k["p8"])
    add("sec1.from8", [H(k["p8"])], k["sec1"])
    add("sec1.round", [H(k["explicit"])], None)
    add("pkcs1.from8", [H(k["p8"])], None)
for p8 in others:
    add("sec1.from8", [H(p8)], None)

# --- OpenSSL encrypts, Grasm opens ----------------------------------------------------------
PBES2_CIPHERS = ["aes-128-cbc", "aes-192-cbc", "aes-256-cbc", "des-ede3-cbc", "des-cbc", "camellia-128-cbc", "camellia-192-cbc",
                 "camellia-256-cbc", "aria-128-cbc", "aria-192-cbc", "aria-256-cbc"]
PRFS = ["hmacWithSHA1", "hmacWithSHA256", "hmacWithSHA384", "hmacWithSHA512", "hmacWithSHA512-256"]
V1 = {"PBE-MD5-DES": "pbeWithMD5AndDES-CBC", "PBE-SHA1-DES": "pbeWithSHA1AndDES-CBC", "PBE-MD5-RC2-64": "pbeWithMD5AndRC2-CBC",
      "PBE-SHA1-RC2-64": "pbeWithSHA1AndRC2-CBC", "PBE-SHA1-RC4-128": "pbeWithSHAAnd128BitRC4",
      "PBE-SHA1-RC4-40": "pbeWithSHAAnd40BitRC4", "PBE-SHA1-3DES": "pbeWithSHAAnd3-KeyTripleDES-CBC",
      "PBE-SHA1-2DES": "pbeWithSHAAnd2-KeyTripleDES-CBC", "PBE-SHA1-RC2-128": "pbeWithSHAAnd128BitRC2-CBC",
      "PBE-SHA1-RC2-40": "pbeWithSHAAnd40BitRC2-CBC"}
PASSWORDS = ["password", "correct horse battery staple", "pässwörd", "p", "x" * 80]
for i in range(120):
    p8 = rng.choice(all_p8)
    pw = rng.choice(PASSWORDS)
    kind = rng.random()
    if kind < 0.6:
        opts = ["-v2", rng.choice(PBES2_CIPHERS), "-v2prf", rng.choice(PRFS), "-iter", str(rng.choice([1, 2, 100, 2048]))]
    elif kind < 0.7:
        opts = ["-scrypt", "-scrypt_N", str(rng.choice([16, 1024])), "-scrypt_r", str(rng.choice([1, 8])), "-scrypt_p", "1",
                "-v2", rng.choice(["aes-256-cbc", "aes-128-cbc"])]
    else:
        opts = ["-v1", rng.choice(list(V1)), "-iter", str(rng.choice([1, 100, 2048]))]
    enc = ossl(["pkcs8", "-topk8", "-inform", "DER", "-outform", "DER", "-passout", "pass:" + pw] + opts + LEGACY, p8)
    if enc is None:
        print("openssl could not encrypt with", opts); sys.exit(1)
    add("enc8.dec", [H(enc), H(pw.encode())], p8)
    add("enc8.dec", [H(enc), H((pw + "!").encode())], None)

# Traditional PEM, encrypted by OpenSSL.
PEM_CIPHERS = {"aes128": "AES-128-CBC", "aes192": "AES-192-CBC", "aes256": "AES-256-CBC", "des3": "DES-EDE3-CBC",
               "des": "DES-CBC", "camellia128": "CAMELLIA-128-CBC", "camellia192": "CAMELLIA-192-CBC",
               "camellia256": "CAMELLIA-256-CBC", "aria128": "ARIA-128-CBC", "aria192": "ARIA-192-CBC", "aria256": "ARIA-256-CBC"}
for i in range(80):
    pw = rng.choice(PASSWORDS)
    flag = rng.choice(list(PEM_CIPHERS))
    if rng.random() < 0.5:
        k = rng.choice(rsa)
        pem = ossl(["rsa", "-inform", "DER", "-traditional", "-" + flag, "-passout", "pass:" + pw] + LEGACY, k["p8"])
        plain = k["p1"]
    else:
        k = rng.choice(ec)
        pem = ossl(["ec", "-inform", "DER", "-" + flag, "-passout", "pass:" + pw] + LEGACY, k["p8"])
        plain = k["sec1"]
    if pem is None:
        print("openssl could not write", flag); sys.exit(1)
    add("pem.dec", [H(pem), H(pw.encode())], plain)
    # Many wrong passwords, so that some leave valid padding behind.
    for w in range(12):
        add("pem.dec", [H(pem), H(f"wrong{w}".encode())], None)

# PBKDF2, in-process.
MACS = {"sha1": "SHA1", "sha256": "SHA256", "sha384": "SHA384", "sha512": "SHA512", "sha3-256": "SHA3-256", "sm3": "SM3"}
for _ in range(200):
    name = rng.choice(list(MACS))
    pw, salt = rb(rng.choice([0, 1, 8, 64, 65, 129, rng.randrange(0, 200)])), rb(rng.choice([0, 8, 16, rng.randrange(0, 40)]))
    it, n = rng.choice([1, 2, 10, 1000]), rng.choice([1, 16, 20, 32, 64, 65, 100])
    add(f"pbkdf2.{name}", [H(pw), H(salt), str(it), str(n)],
        evp.kdf("PBKDF2", n, [("digest", MACS[name]), ("pass", pw), ("salt", salt), ("iter", it), ("pkcs5", True)]))

# --- Grasm encrypts, OpenSSL opens -----------------------------------------------------------
# Round 1: algorithms from pbes's builders.
build = []
for i in range(120):
    pw = rng.choice(PASSWORDS)
    p8 = rng.choice(all_p8)
    kind = rng.random()
    if kind < 0.6:
        c = rng.choice(PBES2_CIPHERS)
        bs = 8 if c.startswith("des") else 16
        build.append(("pbes2.build", [c.encode().hex(), rng.choice(PRFS).encode().hex(), H(rb(rng.choice([8, 16]))),
                                      str(rng.choice([1, 100, 2048])), H(rb(bs))], pw, p8))
    elif kind < 0.7:
        build.append(("pbes2s.build", ["aes-256-cbc".encode().hex(), H(rb(16)), str(rng.choice([16, 1024])), "8", "1", H(rb(16))], pw, p8))
    else:
        build.append(("pbe.build", [rng.choice(list(V1.values())).encode().hex(), H(rb(8)), str(rng.choice([1, 100, 2048]))], pw, p8))
algs = run_harness([(op, "|".join(f)) for op, f, _, _ in build])
enc_cases = []
for (op, f, pw, p8), alg in zip(build, algs):
    if alg == "ERR":
        print("a builder refused", op, f); sys.exit(1)
    enc_cases.append(("enc8.enc", "|".join([H(p8), H(pw.encode()), alg])))
pem_build = []
for i in range(80):
    pw = rng.choice(PASSWORDS)
    cipher = rng.choice(list(PEM_CIPHERS.values()))
    iv = rb(8 if cipher.startswith("DES") else 16)
    if rng.random() < 0.5:
        k = rng.choice(rsa)
        pem_build.append(("RSA PRIVATE KEY", k["p1"], pw, cipher, iv, "rsa"))
    else:
        k = rng.choice(ec)
        pem_build.append(("EC PRIVATE KEY", k["sec1"], pw, cipher, iv, "ec"))
    enc_cases.append(("pem.enc", "|".join([pem_build[-1][0].encode().hex(), H(pem_build[-1][1]), H(pw.encode()),
                                            cipher.encode().hex(), H(iv)])))
encrypted = run_harness(enc_cases)
for (op, f, pw, p8), out in zip(build, encrypted[:len(build)]):
    got = None if out == "ERR" else ossl(["pkcs8", "-inform", "DER", "-outform", "DER", "-passin", "pass:" + pw] + LEGACY,
                                         bytes.fromhex(out))
    # Decrypting, `openssl pkcs8` writes the traditional form back out.
    got = to_pkcs8(got) if got else None
    cases.append(("enc8.opened", op))
    wants.append("ok" if got == p8 else f"OpenSSL could not open it ({op})")
for (label, der, pw, cipher, iv, tool), out in zip(pem_build, encrypted[len(build):]):
    got = None if out == "ERR" else ossl([tool, "-passin", "pass:" + pw, "-outform", "DER"] + (["-traditional"] if tool == "rsa" else [])
                                         + LEGACY, bytes.fromhex(out))
    cases.append(("pem.opened", cipher))
    wants.append("ok" if got == der else f"OpenSSL could not open it ({cipher})")

# --- compare ----------------------------------------------------------------------------------
real = [(i, c) for i, c in enumerate(cases) if not c[0].endswith(".opened")]
answers = run_harness([c for _, c in real])
mine = ["ok"] * len(cases)
for (i, _), a in zip(real, answers):
    mine[i] = a
families, differ = {}, 0
only = os.environ.get("ONLY", "")
for i, (op, fields) in enumerate(cases):
    fam = families.setdefault(op.split(".")[0] + "." + op.split(".")[1] if "." in op else op, [0, 0])
    if mine[i] == wants[i]:
        fam[0] += 1
    else:
        fam[1] += 1
        differ += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} {fields[:160]}")
            print(f"    want  {wants[i][:160]}")
            print(f"    grasm {mine[i][:160]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 7]
if thin or len(families) < 14:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
