"""Hashes, HMAC and key derivation functions against OpenSSL 3's libcrypto.

- `ripemd160`, `whirlpool`, `sm3`, `mdc2`: messages around every block and
  padding boundary.
- `hmac`: thirteen hashes (MD5 to SHA-3, SM3, RIPEMD-160, Whirlpool) with
  keys shorter than, equal to and longer than each block.
- `kbkdf`: counter and feedback modes, HMAC and AES-CMAC, every counter
  width, with and without the length field and the separator.
- `x963kdf`, `sskdf` (hash and HMAC), `sshkdf` (all six letters),
  `tls12prf` (TLS 1.2 over many hashes, and TLS 1.0's MD5/SHA-1),
  `x942kdf` (every optional field, all four key-encryption algorithms),
  `pkcs12kdf` (all three purposes), `krb5kdf` (AES-128, AES-256, Triple
  DES), `pbkdf1`.

OpenSSL ignores a parameter name it does not know, so every option above
is varied: if one were ignored, the cases that set it would disagree.
`bmp_password` and `mpint` are checked against Python's definitions.
scripts/b34_diff.sh first re-derives the hash tables from their sources.
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
work = tempfile.mkdtemp(prefix="b34")
harness = os.path.join(work, "b34test")
built = subprocess.run([GRASM, "build", "packages/b34test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

# name here: (OpenSSL name, block size, output size)
HASHES = {"md5": ("MD5", 64, 16), "sha1": ("SHA1", 64, 20), "sha256": ("SHA256", 64, 32), "sha384": ("SHA384", 128, 48),
          "sha512": ("SHA512", 128, 64), "sha512-256": ("SHA512-256", 128, 32), "sha3-224": ("SHA3-224", 144, 28),
          "sha3-256": ("SHA3-256", 136, 32), "sha3-384": ("SHA3-384", 104, 48), "sha3-512": ("SHA3-512", 72, 64),
          "sm3": ("SM3", 64, 32), "ripemd160": ("RIPEMD160", 64, 20), "whirlpool": ("WHIRLPOOL", 64, 64)}


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def msg_len(block):
    return rng.choice([0, 1, block - 9, block - 8, block - 1, block, block + 1, 2 * block, rng.randrange(0, 4 * block)])


cases = []


def add(op, fields, want):
    cases.append((op, "|".join(fields), "ERR" if want is None else want.hex()))


H = bytes.hex
for name in ["ripemd160", "whirlpool", "sm3"]:
    for _ in range(200):
        d = rb(max(0, msg_len(64)))
        add(f"hash.{name}", [H(d)], evp.digest(HASHES[name][0], d))
for _ in range(200):
    d = rb(rng.choice([0, 1, 7, 8, 9, 16, rng.randrange(0, 100)]))
    add("hash.mdc2", [H(d)], evp.digest("MDC2", d))
for name, (ossl, block, _) in HASHES.items():
    for _ in range(40):
        k = rb(rng.choice([0, 1, block - 1, block, block + 1, 2 * block + 3, rng.randrange(0, 300)]))
        d = rb(max(0, msg_len(block)))
        add(f"hmac.{name}", [H(k), H(d)], evp.mac("HMAC", k, d, [("digest", ossl)]))

KDF_HASHES = ["sha1", "sha256", "sha384", "sha512", "sha3-256", "sm3", "ripemd160", "whirlpool", "md5", "sha512-256"]
for _ in range(500):
    use_cmac = rng.random() < 0.25
    name = rng.choice(KDF_HASHES)
    key = rb(16) if use_cmac else rb(rng.choice([16, 32, 64, rng.randrange(1, 100)]))
    mode = rng.choice(["c", "f"])
    label, ctx = rb(rng.choice([0, 5, rng.randrange(0, 40)])), rb(rng.choice([0, 7, rng.randrange(0, 40)]))
    out_len = 16 if use_cmac else {"sha1": 20, "sha256": 32, "sha384": 48, "sha512": 64, "sha3-256": 32, "sm3": 32,
                                    "ripemd160": 20, "whirlpool": 64, "md5": 16, "sha512-256": 32}[name]
    # The IV is empty or one PRF output long; other lengths must be refused.
    iv = rb(rng.choice([0, out_len, out_len, out_len, 16, 32])) if mode == "f" else b""
    n = rng.choice([1, 16, 32, 42, 64, 100, rng.randrange(1, 200)])
    r = rng.choice([8, 16, 24, 32, 32])
    use_l, use_sep = rng.random() < 0.7, rng.random() < 0.7
    params = [("mode", "FEEDBACK" if mode == "f" else "COUNTER"), ("key", key), ("salt", label), ("info", ctx), ("r", r),
              ("use-l", use_l), ("use-separator", use_sep)]
    params += [("mac", "CMAC"), ("cipher", "AES-128-CBC")] if use_cmac else [("mac", "HMAC"), ("digest", HASHES[name][0])]
    if mode == "f" and iv:
        params.append(("seed", iv))
    add("kbkdf", ["cmac" if use_cmac else name, H(key), mode, H(label), H(ctx), H(iv), str(n), str(r), "1" if use_l else "0",
                  "1" if use_sep else "0"], evp.kdf("KBKDF", n, params))
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    z, info, n = rb(rng.choice([16, 32, 66, rng.randrange(1, 100)])), rb(rng.choice([0, 10, rng.randrange(0, 60)])), rng.randrange(1, 200)
    add(f"x963.{name}", [H(z), H(info), str(n)], evp.kdf("X963KDF", n, [("digest", HASHES[name][0]), ("key", z), ("info", info)]))
    add(f"sskdf.{name}", [H(z), H(info), str(n)], evp.kdf("SSKDF", n, [("digest", HASHES[name][0]), ("key", z), ("info", info)]))
    salt = rb(rng.choice([1, 16, HASHES[name][1], rng.randrange(1, 150)]))
    add(f"sskdfh.{name}", [H(salt), H(z), H(info), str(n)],
        evp.kdf("SSKDF", n, [("mac", "HMAC"), ("digest", HASHES[name][0]), ("key", z), ("info", info), ("salt", salt)]))
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    secret, label, seed = rb(rng.choice([1, 48, 47, rng.randrange(1, 100)])), rb(rng.choice([13, 0, 22])), rb(rng.choice([64, 32, 0]))
    if not label and not seed:
        # OpenSSL treats an empty seed as a missing one and refuses; RFC
        # 5246 does not, and tls12prf does not.
        seed = rb(8)
    n = rng.choice([12, 48, 104, rng.randrange(1, 300)])
    add(f"tls12.{name}", [H(secret), H(label), H(seed), str(n)],
        evp.kdf("TLS1-PRF", n, [("digest", HASHES[name][0]), ("secret", secret), ("seed", [label, seed])]))
    add("tls10", [H(secret), H(label), H(seed), str(n)],
        evp.kdf("TLS1-PRF", n, [("digest", "MD5-SHA1"), ("secret", secret), ("seed", [label, seed])]))
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    k = rb(rng.choice([5, 33, 37, 260, rng.randrange(1, 100)]))
    hsh, sid = rb(HASHES[name][2]), rb(rng.choice([HASHES[name][2], 20, 32]))
    letter = rng.randrange(65, 71)
    n = rng.choice([8, 16, 32, 64, 100, rng.randrange(1, 200)])
    add(f"ssh.{name}", [H(k), H(hsh), str(letter), H(sid), str(n)],
        evp.kdf("SSHKDF", n, [("digest", HASHES[name][0]), ("key", k), ("xcghash", hsh), ("session_id", sid), ("type", chr(letter))]))
KEKS = {"1": "AES-128-WRAP", "2": "AES-192-WRAP", "3": "AES-256-WRAP", "4": "DES3-WRAP"}
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    zz = rb(rng.choice([20, 32, 128, rng.randrange(1, 200)]))
    kek = rng.choice(list(KEKS))
    u, v, pub, priv = [rb(rng.choice([0, 0, 5, 64])) for _ in range(4)]
    use_kb = rng.random() < 0.8
    n = rng.choice([16, 24, 32, rng.randrange(1, 100)])
    params = [("digest", HASHES[name][0]), ("key", zz), ("cekalg", KEKS[kek]), ("use-keybits", use_kb)]
    for pname, val in [("partyu-info", u), ("partyv-info", v), ("supp-pubinfo", pub), ("supp-privinfo", priv)]:
        if val:
            params.append((pname, val))
    add(f"x942.{name}", [H(zz), kek, H(u), H(v), H(pub), H(priv), "1" if use_kb else "0", str(n)],
        evp.kdf("X942KDF-ASN1", n, params))
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    pw = rng.choice(["", "password", "pässwörd", "\U0001f511 key", "x" * 70]).encode("utf-16-be") + b"\0\0"
    if rng.random() < 0.1:
        pw = b""
    salt = rb(rng.choice([8, 16, 0, rng.randrange(1, 70)]))
    if not pw and not salt:
        # OpenSSL fails to allocate the empty string I that RFC 7292
        # defines for this; pkcs12kdf derives it.
        salt = rb(8)
    pid, it, n = rng.randrange(1, 4), rng.choice([1, 2, 10, 100, 2048]), rng.choice([8, 16, 20, 24, 32, rng.randrange(1, 150)])
    add(f"pkcs12.{name}", [H(pw), H(salt), str(pid), str(it), str(n)],
        evp.kdf("PKCS12KDF", n, [("digest", HASHES[name][0]), ("pass", pw), ("salt", salt), ("iter", it), ("id", pid)]))
for _ in range(300):
    c = rng.choice(["a128", "a256", "d3"])
    kl, ossl = {"a128": (16, "AES-128-CBC"), "a256": (32, "AES-256-CBC"), "d3": (24, "DES-EDE3-CBC")}[c]
    key = rb(kl)
    const = rng.choice([bytes([0, 0, 0, rng.randrange(256), rng.choice([0x99, 0xAA, 0x55])]), b"kerberos", b"\x01",
                        rb(rng.randrange(1, 40)), rb(16), rb(8)])
    add("krb5", [c, H(key), H(const)], evp.kdf("KRB5KDF", kl, [("cipher", ossl), ("key", key), ("constant", const)]))
for _ in range(300):
    name = rng.choice(KDF_HASHES)
    pw, salt = rb(rng.randrange(0, 40)), rb(rng.choice([8, 0, 16]))
    it = rng.choice([1, 2, 1000, rng.randrange(1, 200)])
    n = rng.choice([8, 16, HASHES[name][2], HASHES[name][2] + 1, rng.randrange(1, HASHES[name][2] + 1)])
    want = evp.kdf("PBKDF1", n, [("digest", HASHES[name][0]), ("pass", pw), ("salt", salt), ("iter", it)])
    add(f"pbkdf1.{name}", [H(pw), H(salt), str(it), str(n)], want)
for _ in range(100):
    s = "".join(rng.choice(["a", "é", "€", "\U0001f600", "中", " "]) for _ in range(rng.randrange(0, 12)))
    add("bmp", [s.encode().hex()], s.encode("utf-16-be") + b"\0\0")
    m = rb(rng.randrange(0, 40))
    if rng.random() < 0.3:
        m = b"\0" * rng.randrange(1, 4) + m
    if rng.random() < 0.3 and m:
        m = bytes([m[0] | 0x80]) + m[1:]
    v = m.lstrip(b"\0")
    if v and v[0] & 0x80:
        v = b"\0" + v
    add("mpint", [H(m)], len(v).to_bytes(4, "big") + v)

lines, batch, size = [], [], 0


def flush():
    global batch, size
    if not batch:
        return
    argv = []
    for op, fields, _ in batch:
        argv += [op, "x", "x" + fields.encode().hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=900)
    if ran.returncode != 0:
        for op, fields, _ in batch:
            one = subprocess.run([harness, op, "x", "x" + fields.encode().hex()], capture_output=True, text=True)
            if one.returncode != 0:
                print(f"harness crashed on {op} {fields[:300]}")
                break
        print(ran.stderr[-2000:]); sys.exit(1)
    lines.extend(ran.stdout.split("\n")[:-1])
    batch, size = [], 0


for c in cases:
    batch.append(c)
    size += 2 * len(c[1]) + 30
    if size > 400000 or len(batch) >= 300:
        flush()
flush()

families, differ = {}, 0
only = os.environ.get("ONLY", "")
for i, (op, fields, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    # Hashes and HMACs are families per digest; the KDFs one each.
    fam = families.setdefault(op if op.startswith(("hash.", "hmac.")) else op.split(".")[0], [0, 0])
    if mine == want:
        fam[0] += 1
    else:
        fam[1] += 1
        differ += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} {fields[:200]}")
            print(f"    want  {want[:160]}")
            print(f"    grasm {mine[:160]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 12]
if thin or len(families) < 30:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
