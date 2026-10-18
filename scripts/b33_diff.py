"""Eight block ciphers and blockmodes against OpenSSL 3's libcrypto.

Each cipher at each key size it allows, in ECB, CBC, CFB, OFB and (where
OpenSSL has them) CFB8 and CTR, both directions, on data of every length
class, including lengths ECB and CBC must refuse. Where OpenSSL has no
such mode for a cipher (CTR and CFB8 on the 64-bit ones), the expected
answer is built in Python from OpenSSL's own ECB, so the block function is
still OpenSSL's. Blowfish, CAST5 and RC2 run at variable key lengths and
RC2 at effective key lengths from 1 to 1024 bits.

PKCS#7 padding is checked against Python's definition, and a few key
lengths outside each specification against the specification (OpenSSL
reads a fixed-size key buffer, so it cannot refuse them).

scripts/b33_diff.sh first runs `gen_cipher_tables.py --check`, which
re-derives every table from its source.
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
work = tempfile.mkdtemp(prefix="b33")
harness = os.path.join(work, "b33test")
built = subprocess.run([GRASM, "build", "packages/b33test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


# name: (block size, key lengths, OpenSSL name for (key, mode), modes OpenSSL has)
CIPHERS = {
    "sm4": (16, [16], lambda k, m: f"SM4-{m}", ["ECB", "CBC", "CFB", "OFB", "CTR"]),
    "camellia": (16, [16, 24, 32], lambda k, m: f"CAMELLIA-{len(k) * 8}-{m}", ["ECB", "CBC", "CFB", "CFB8", "OFB", "CTR"]),
    "aria": (16, [16, 24, 32], lambda k, m: f"ARIA-{len(k) * 8}-{m}", ["ECB", "CBC", "CFB", "CFB8", "OFB", "CTR"]),
    "seed": (16, [16], lambda k, m: f"SEED-{m}", ["ECB", "CBC", "CFB", "OFB"]),
    "bf": (8, list(range(1, 73)), lambda k, m: f"BF-{m}", ["ECB", "CBC", "CFB", "OFB"]),
    "cast5": (8, list(range(5, 17)), lambda k, m: f"CAST5-{m}", ["ECB", "CBC", "CFB", "OFB"]),
    "rc2": (8, list(range(1, 129)), lambda k, m: f"RC2-{m}", ["ECB", "CBC", "CFB", "OFB"]),
    "idea": (8, [16], lambda k, m: f"IDEA-{m}", ["ECB", "CBC", "CFB", "OFB"]),
}
VARIABLE = {"bf", "cast5", "rc2"}


def params(name, key, bits):
    p = []
    if name in VARIABLE:
        p.append(("keylen", len(key)))
    if name == "rc2":
        p.append(("keybits", bits))
    return p


def ossl(name, key, mode, iv, data, encrypt, bits):
    bs, _, nm, _ = CIPHERS[name]
    return evp.cipher(nm(key, mode), key, iv if mode != "ECB" else None, data, encrypt=encrypt, params=params(name, key, bits))


def block(name, key, bits, b):
    return ossl(name, key, "ECB", None, b, True, bits)


def composed(name, key, bits, mode, iv, data, encrypt):
    """CTR or CFB8 from OpenSSL's ECB, for ciphers OpenSSL has no such mode for."""
    bs = CIPHERS[name][0]
    out = bytearray()
    if mode == "CTR":
        ctr = int.from_bytes(iv, "big")
        for at in range(0, len(data), bs):
            ks = block(name, key, bits, ctr.to_bytes(bs, "big"))
            out += bytes(a ^ b for a, b in zip(data[at:at + bs], ks))
            ctr = (ctr + 1) % (1 << (8 * bs))
        return bytes(out)
    reg = bytearray(iv)
    for x in data:
        v = x ^ block(name, key, bits, bytes(reg))[0]
        out.append(v)
        reg = reg[1:] + bytes([x if not encrypt else v])
    return bytes(out)


cases = []


def add(op, fields, want):
    cases.append((op, "|".join(fields), "ERR" if want is None else want.hex()))


for name, (bs, keylens, nm, have) in CIPHERS.items():
    count = 120 if name in VARIABLE else 60
    for mode in ["ECB", "CBC", "CFB", "OFB", "CTR", "CFB8"]:
        for _ in range(count if mode in have else count // 3):
            key = rb(rng.choice(keylens))
            bits = rng.choice([8 * len(key), 40, 63, 64, 128, 1024, rng.randrange(1, 1025)]) if name == "rc2" else 0
            iv = rb(bs)
            if mode == "CTR" and rng.random() < 0.3:
                # Counters about to carry across bytes, or wrap entirely.
                iv = iv[:bs - rng.choice([1, 2, bs])] + b"\xff" * rng.choice([1, 2, bs])
                iv = iv[-bs:].rjust(bs, b"\x00")
            whole = mode in ("ECB", "CBC")
            n = rng.choice([bs, 2 * bs, 5 * bs, 0, 1, bs - 1, bs + 1, 3 * bs + 2, rng.randrange(0, 12 * bs)])
            if whole and rng.random() < 0.85:
                n -= n % bs
            data = rb(n)
            for encrypt in (True, False):
                if mode in have:
                    want = ossl(name, key, mode, iv, data, encrypt, bits)
                    if whole and n % bs:
                        want = None
                else:
                    want = composed(name, key, bits, mode, iv, data, encrypt)
                fields = [key.hex(), iv.hex(), data.hex(), str(bits)]
                add(f"{name}.{mode.lower()}.{'enc' if encrypt else 'dec'}", fields, want)

# PKCS#7 against its definition.
def pad(d, bs):
    n = bs - len(d) % bs
    return d + bytes([n]) * n


def unpad(d, bs):
    if not d or len(d) % bs:
        return None
    n = d[-1]
    if n < 1 or n > bs or d[-n:] != bytes([n]) * n:
        return None
    return d[:-n]


for _ in range(300):
    name = rng.choice(list(CIPHERS))
    bs = CIPHERS[name][0]
    key = rb(16)
    d = rb(rng.randrange(0, 40))
    add(f"{name}.pad.enc", [key.hex(), "", d.hex(), "128"], pad(d, bs))
    p = pad(d, bs) if rng.random() < 0.6 else rb(rng.choice([bs, 2 * bs, bs - 1, 0]))
    if rng.random() < 0.3 and p:
        p = p[:-1] + bytes([rng.choice([0, bs + 1, p[-1] ^ 1, 1])])
    add(f"{name}.pad.dec", [key.hex(), "", p.hex(), "128"], unpad(p, bs))

# Key lengths outside each specification.
for name, key in [("sm4", rb(15)), ("camellia", rb(20)), ("aria", rb(8)), ("seed", rb(32)), ("bf", b""), ("bf", rb(73)),
                  ("cast5", rb(4)), ("cast5", rb(17)), ("rc2", b""), ("rc2", rb(129)), ("idea", rb(15))]:
    add(f"{name}.ecb.enc", [key.hex(), "", "00" * CIPHERS[name][0], "64"], None)
add("rc2.ecb.enc", [rb(8).hex(), "", "00" * 8, "0"], None)
add("rc2.ecb.enc", [rb(8).hex(), "", "00" * 8, "1025"], None)

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

families, differ = {}, 0
only = os.environ.get("ONLY", "")
for i, (op, fields, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    fam = families.setdefault(op, [0, 0])
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
if thin or len(families) < 100:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
