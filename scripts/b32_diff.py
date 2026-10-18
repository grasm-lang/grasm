"""AES modes and MACs against OpenSSL 3's libcrypto, called in-process.

- `cmac`: AES-CMAC at all three key sizes and Triple-DES CMAC, over
  messages around every block boundary.
- `gmac`: IVs of 1 to 40 bytes (12 is the common one; every other length
  goes through GHASH first), messages of every length class.
- `kmac`: KMAC128/256 and their XOFs, keys, messages and customisation
  strings across the sponge's rate boundaries, outputs of many lengths.
- `ccm`, `ocb`, `gcmsiv`, `aessiv`: sealing compared byte for byte, and
  opening compared on good, tampered and truncated inputs, at every nonce
  and tag length each allows, and lengths each refuses.
- `keywrap`: KW and KWP wraps, and unwraps of good and damaged values.
- `xts`: sectors of 16 to 100 bytes (ciphertext stealing), and keys with
  equal halves, which both sides must refuse.
- `cbccts`: CS1, CS2 and CS3 on whole and partial final blocks.

A refusal on one side must be a refusal on the other.
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
work = tempfile.mkdtemp(prefix="b32")
harness = os.path.join(work, "b32test")
built = subprocess.run([GRASM, "build", "packages/b32test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def length():
    return rng.choice([0, 1, 15, 16, 17, 31, 32, 33, 47, 48, 64, rng.randrange(0, 100), rng.randrange(0, 300)])


def aes_key():
    return rb(rng.choice([16, 24, 32]))


def name(key, mode):
    return f"AES-{len(key) * 8}-{mode}"


def h(b):
    return b.hex()


cases = []   # (op, fields, want)


def add(op, fields, want):
    cases.append((op, "|".join(fields), "ERR" if want is None else want.hex()))


def tamper(b):
    if not b:
        return b"\x00"
    i = rng.randrange(len(b))
    return b[:i] + bytes([b[i] ^ (1 << rng.randrange(8))]) + b[i + 1:]


# --- MACs --------------------------------------------------------------------------------
for _ in range(400):
    k, m = aes_key(), rb(length())
    add("cmac.aes", [h(k), h(m)], evp.mac("CMAC", k, m, [("cipher", name(k, "CBC"))]))
for _ in range(150):
    k, m = rb(24), rb(length())
    add("cmac.tdes", [h(k), h(m)], evp.mac("CMAC", k, m, [("cipher", "DES-EDE3-CBC")]))
for k in [rb(15), rb(20), b""]:
    add("cmac.aes", [h(k), "00"], None)
for _ in range(400):
    k, m = aes_key(), rb(length())
    iv = rb(rng.choice([12, 12, 1, 8, 16, 13, rng.randrange(1, 41)]))
    add("gmac", [h(k), h(iv), h(m)], evp.mac("GMAC", k, m, [("cipher", name(k, "GCM")), ("iv", iv)]))
for _ in range(500):
    op = rng.choice(["kmac128", "kmac256", "kmacxof128", "kmacxof256"])
    k = rb(rng.choice([4, 16, 32, rng.randrange(4, 200)]))
    m = rb(rng.choice([0, 1, 135, 136, 137, 167, 168, 169, rng.randrange(0, 400)]))
    custom = rb(rng.choice([0, 0, 1, 10, rng.randrange(0, 80)]))
    n = rng.choice([4, 16, 32, 64, 100, 168, rng.randrange(4, 300)])
    want = evp.mac("KMAC" + op[-3:], k, m, [("custom", custom)] if custom else [], size=n, xof=op.startswith("kmacxof"))
    add(op, [h(k), h(m), h(custom), str(n)], want)

# --- AEADs ---------------------------------------------------------------------------------
for _ in range(500):
    k, aad, pt = aes_key(), rb(length()), rb(length())
    nonce = rb(rng.choice([7, 8, 11, 12, 13, 13, 6, 14]))
    t = rng.choice([4, 6, 8, 10, 12, 14, 16, 16, 3, 5, 18, 2])
    ok = 7 <= len(nonce) <= 13 and 4 <= t <= 16 and t % 2 == 0
    r = evp.aead_seal(name(k, "CCM"), k, nonce, [aad], pt, tag_len=t, ccm=True) if ok else None
    add("ccm.seal", [h(k), h(nonce), h(aad), h(pt), str(t)], None if r is None else r[0] + r[1])
    if r:
        sealed = r[0] + r[1]
        if rng.random() < 0.5:
            sealed = rng.choice([tamper(sealed), sealed[:-1], sealed[1:]])
        if len(sealed) >= t:
            want = evp.aead_open(name(k, "CCM"), k, nonce, [aad], sealed[:-t], sealed[-t:], ccm=True)
        else:
            want = None
        add("ccm.open", [h(k), h(nonce), h(aad), h(sealed), str(t)], want)
# Associated data long enough for the 6-byte length framing.
for _ in range(4):
    k, aad, pt, nonce = aes_key(), rb(65280 + rng.randrange(0, 50)), rb(20), rb(12)
    r = evp.aead_seal(name(k, "CCM"), k, nonce, [aad], pt, tag_len=16, ccm=True)
    add("ccm.seal", [h(k), h(nonce), h(aad), h(pt), "16"], r[0] + r[1])
for _ in range(500):
    k, aad, pt = aes_key(), rb(length()), rb(length())
    nonce = rb(rng.choice([12, 12, 1, 8, 15, rng.randrange(1, 16)]))
    t = rng.choice([16, 16, 12, 8, rng.randrange(1, 17)])
    r = evp.aead_seal(name(k, "OCB"), k, nonce, [aad], pt, tag_len=t)
    add("ocb.seal", [h(k), h(nonce), h(aad), h(pt), str(t)], None if r is None else r[0] + r[1])
    if r:
        sealed = r[0] + r[1]
        if rng.random() < 0.5:
            sealed = rng.choice([tamper(sealed), sealed[:-1]])
        want = evp.aead_open(name(k, "OCB"), k, nonce, [aad], sealed[:-t], sealed[-t:]) if len(sealed) >= t else None
        add("ocb.open", [h(k), h(nonce), h(aad), h(sealed), str(t)], want)
for k, nonce, t in [(rb(16), b"", 16), (rb(16), rb(16), 16), (rb(16), rb(12), 0), (rb(16), rb(12), 17)]:
    add("ocb.seal", [h(k), h(nonce), "", "00", str(t)], None)
for _ in range(400):
    k, aad, pt = rb(rng.choice([16, 32])), rb(length()), rb(length())
    nonce = rb(12)
    r = evp.aead_seal(name(k, "GCM-SIV"), k, nonce, [aad], pt)
    add("gcmsiv.seal", [h(k), h(nonce), h(aad), h(pt)], r[0] + r[1])
    sealed = r[0] + r[1]
    if rng.random() < 0.5:
        sealed = rng.choice([tamper(sealed), sealed[:-1]])
    want = evp.aead_open(name(k, "GCM-SIV"), k, nonce, [aad], sealed[:-16], sealed[-16:]) if len(sealed) >= 16 else None
    add("gcmsiv.open", [h(k), h(nonce), h(aad), h(sealed)], want)
for k, nonce in [(rb(24), rb(12)), (rb(16), rb(11)), (rb(16), rb(16))]:
    add("gcmsiv.seal", [h(k), h(nonce), "", "00"], None)
for _ in range(400):
    k = rb(rng.choice([32, 48, 64]))
    ads = [rb(rng.choice([0, 1, 16, rng.randrange(0, 40)])) for _ in range(rng.randrange(0, 5))]
    pt = rb(length())
    r = evp.aead_seal(f"AES-{len(k) * 4}-SIV", k, None, ads, pt, fixed_nonce=True)
    sealed = r[1] + r[0]
    adf = ",".join("x" + a.hex() for a in ads)
    add("siv.seal", [h(k), h(pt), adf], sealed)
    if rng.random() < 0.5:
        sealed = rng.choice([tamper(sealed), sealed[:-1] if len(sealed) > 16 else sealed])
    want = evp.aead_open(f"AES-{len(k) * 4}-SIV", k, None, ads, sealed[16:], sealed[:16], fixed_nonce=True)
    add("siv.open", [h(k), h(sealed), adf], want)
add("siv.seal", [h(rb(16)), "00", ""], None)

# --- Key wrap, XTS, CTS ---------------------------------------------------------------------
# EVP "wraps" an empty input to an empty output, because no update runs;
# RFC 3394 and 5649 refuse it, and so does keywrap. That quirk is not copied.
def wrapped(mode, kek, data, encrypt=True):
    return None if not data else evp.cipher(name(kek, mode), kek, None, data, encrypt=encrypt)


for _ in range(300):
    kek = aes_key()
    key = rb(rng.choice([16, 24, 32, 40, 64, 8, 0, 20, rng.randrange(0, 70)]))
    w = wrapped("WRAP", kek, key)
    add("kw.wrap", [h(kek), h(key)], w)
    if w:
        bad = rng.choice([w, w, tamper(w), w[:-8]])
        add("kw.unwrap", [h(kek), h(bad)], evp.cipher(name(kek, "WRAP"), kek, None, bad, encrypt=False))
    w = wrapped("WRAP-PAD", kek, key)
    add("kwp.wrap", [h(kek), h(key)], w)
    if w:
        bad = rng.choice([w, w, tamper(w), w[:-8]])
        add("kwp.unwrap", [h(kek), h(bad)], evp.cipher(name(kek, "WRAP-PAD"), kek, None, bad, encrypt=False))
# Unwraps of values no wrap produced: right lengths, wrong check values.
for _ in range(60):
    kek = aes_key()
    for op, mode in [("kw.unwrap", "WRAP"), ("kwp.unwrap", "WRAP-PAD")]:
        c = rb(rng.choice([16, 24, 32, 40]))
        add(op, [h(kek), h(c)], evp.cipher(name(kek, mode), kek, None, c, encrypt=False))
for _ in range(400):
    k = rb(rng.choice([32, 64]))
    if rng.random() < 0.05:
        k = k[:len(k) // 2] * 2
    tweak = rb(16)
    data = rb(rng.choice([16, 17, 31, 32, 33, 48, 15, rng.randrange(16, 120)]))
    nm = f"AES-{len(k) * 4}-XTS"
    add("xts.enc", [h(k), h(tweak), h(data)], evp.cipher(nm, k, tweak, data))
    add("xts.dec", [h(k), h(tweak), h(data)], evp.cipher(nm, k, tweak, data, encrypt=False))
for _ in range(400):
    k, iv = aes_key(), rb(16)
    data = rb(rng.choice([16, 17, 31, 32, 33, 48, 64, 15, rng.randrange(16, 120)]))
    v = rng.choice(["1", "2", "3"])
    nm = name(k, "CBC-CTS")
    add("cts.enc", [h(k), h(iv), h(data), v], evp.cipher(nm, k, iv, data, params=[("cts_mode", "CS" + v)]))
    add("cts.dec", [h(k), h(iv), h(data), v], evp.cipher(nm, k, iv, data, encrypt=False, params=[("cts_mode", "CS" + v)]))

# --- run ----------------------------------------------------------------------------------
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
    if size > 400000 or len(batch) >= 400:
        flush()
flush()

families, kinds, differ = {}, {}, 0
only = os.environ.get("ONLY", "")
for i, (op, fields, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    fam = families.setdefault(op, [0, 0])
    kinds.setdefault(op, set()).add("ERR" if want == "ERR" else "ok")
    if mine == want:
        fam[0] += 1
    else:
        fam[1] += 1
        differ += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} {fields[:240]}")
            print(f"    want  {want[:200]}")
            print(f"    grasm {mine[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 12]
if thin or len(families) < 22:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
# Opening and unwrapping must have seen both verdicts.
one_sided = [k for k in families if (k.endswith("open") or k.endswith("unwrap") or k.endswith("dec")) and len(kinds[k]) < 2]
if one_sided:
    print(f"only one verdict seen: {one_sided}"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
