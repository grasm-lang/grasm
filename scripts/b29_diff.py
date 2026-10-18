"""X448, Ed448, HPKE and age.

- `x448`: random private keys against random u-coordinates (non-canonical
  ones of p or more among them, and the small-order points 0 and 1, whose
  all-zero secrets both sides must flag), and RFC 7748's vectors.
- `ed448`: public keys and signatures (Ed448 and Ed448ph, contexts of 0
  to 255 bytes) must equal circl's byte for byte, since both are
  deterministic; circl's signatures must verify here, and refuse once a
  bit of the signature, message, key or context changes or S is pushed
  past the group order.
- `hpke`: every KEM (P-256, P-384, P-521, X25519, X448), KDF and AEAD in
  all four modes, from fixed key material:
  circl's encapsulated key, ciphertexts and exported secrets must equal
  the package's, ciphertexts must open here, and tampered ones not.
- `age`: files circl's sibling metacubex/age writes (X25519 recipients,
  passphrases, armor, payloads around the 64 KiB chunk size) decrypted
  here; files written here decrypted there; damaged files and wrong keys
  refused by both; and identities it generates turned into the same
  recipients.
"""
import json
import os
import random
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle  # noqa: E402

rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b29")
harness = os.path.join(work, "b29test")
built = subprocess.run([GRASM, "build", "packages/b29test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
go = oracle.build(os.path.join(work, "gomods"))


def ask(reqs):
    out = subprocess.run([go], input=json.dumps(reqs), capture_output=True, text=True)
    if out.returncode != 0:
        print(out.stderr[-2000:]); sys.exit(1)
    return json.loads(out.stdout)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


cases = []   # [op, a, b, want, go request, then]


def add(op, a, b=b"", want=None, req=None, then=None):
    cases.append([op, a, b if isinstance(b, bytes) else b.encode(), want, req, then])


# --- X448 ------------------------------------------------------------------------
P448 = 2 ** 448 - 2 ** 224 - 1
for _ in range(250):
    sk = rb(56)
    r = rng.random()
    if r < 0.1:
        u = (P448 + rng.randrange(0, 2 ** 224)).to_bytes(56, "little")
    elif r < 0.15:
        u = rng.choice([0, 1]).to_bytes(56, "little")
    else:
        u = rb(56)
    add("x448", sk + u, req={"op": "x448", "hex": sk.hex() + "|" + u.hex()})
    add("x448.pub", sk, req={"op": "x448.pub", "hex": sk.hex()})

# --- Ed448 -----------------------------------------------------------------------
L448 = 2 ** 446 - 13818066809895115352007386748515426880336692474882178609894547503885
eds = []
for _ in range(110):
    seed, msg = rb(57), rb(rng.choice([0, 1, 32, 100, rng.randrange(0, 400)]))
    ctx = rng.choice([b"", b"", rb(rng.randrange(1, 20)), rb(255)])
    mode = rng.choice(["", "", "ph"])
    eds.append((seed, msg, ctx, mode))
    f = f"{ctx.hex()}|{mode}"
    add("ed448.pub", seed, f, req={"op": "ed448.pub", "hex": seed.hex()})
    add("ed448.sign", seed + msg, f, req={"op": "ed448.sign", "hex": seed.hex() + "|" + msg.hex(), "args": [ctx.hex(), mode]})
pubs = ask([{"op": "ed448.pub", "hex": s.hex()} for s, _, _, _ in eds])
sigs = ask([{"op": "ed448.sign", "hex": s.hex() + "|" + m.hex(), "args": [c.hex(), md]} for s, m, c, md in eds])
for (seed, msg, ctx, mode), pk, sg in zip(eds, pubs, sigs):
    pk, sg = bytes.fromhex(pk), bytes.fromhex(sg)
    f = f"{ctx.hex()}|{mode}"
    add("ed448.verify", pk + sg + msg, f, want="true")
    variants = []
    bad = bytearray(sg)
    bad[rng.randrange(114)] ^= 1 << rng.randrange(8)
    variants.append((pk, bytes(bad), msg, ctx))
    variants.append((pk, sg, msg + b"x", ctx))
    variants.append((pk, sg, msg, ctx + b"x" if len(ctx) < 255 else ctx[:-1]))
    s_big = (int.from_bytes(sg[57:], "little") + L448).to_bytes(57, "little")
    variants.append((pk, sg[:57] + s_big, msg, ctx))
    badpk = bytearray(pk)
    badpk[rng.randrange(56)] ^= 1 << rng.randrange(8)
    variants.append((bytes(badpk), sg, msg, ctx))
    for vpk, vsg, vmsg, vctx in variants:
        add("ed448.verify", vpk + vsg + vmsg, f"{vctx.hex()}|{mode}",
            req={"op": "ed448.verify", "hex": f"{vpk.hex()}|{vmsg.hex()}|{vsg.hex()}", "args": [vctx.hex(), mode]})

# --- HPKE ------------------------------------------------------------------------
NSK = {16: 32, 17: 48, 18: 66, 32: 32, 33: 56}
for _ in range(330):
    kem = rng.choice([16, 17, 18, 32, 33, 32, 32])
    kdf = rng.choice([1, 2, 3])
    aead = rng.choice([1, 2, 3])
    mode = rng.randrange(4)
    ikm_r, ikm_e, ikm_s = rb(NSK[kem]), rb(NSK[kem]), rb(NSK[kem])
    info, aad, pt, expctx = rb(rng.randrange(0, 20)), rb(rng.randrange(0, 20)), rb(rng.randrange(0, 80)), rb(rng.randrange(0, 10))
    psk, pskid = (rb(32), rb(rng.randrange(1, 12))) if mode in (1, 3) else (b"", b"")
    # An inconsistent PSK only where circl's API can be handed one.
    if mode in (1, 3) and rng.random() < 0.08:
        psk, pskid = (rb(32), b"") if rng.random() < 0.5 else (b"", b"")
    explen = rng.choice([0, 16, 32, 64])
    fields = [ikm_r, ikm_e, ikm_s, info, psk, pskid, aad, pt, expctx]
    req = {"op": "hpke.seal", "hex": "|".join(x.hex() for x in fields), "args": [str(kem), str(kdf), str(aead), str(mode), str(explen)]}
    b = "|".join([str(kem), str(kdf), str(aead), str(mode), str(explen)] + [x.hex() for x in fields])
    add("hpke.seal", b"", b, req=req, then=("hpke", kem, kdf, aead, mode, ikm_r, ikm_s, info, psk, pskid, aad, pt))

# --- age -------------------------------------------------------------------------
keys = ask([{"op": "age.keygen"} for _ in range(40)])
keys = [k.split("|") for k in keys]
for ident, recip in keys:
    add("age.key", b"", ident, want=f"{recip}|{ident}")
SIZES = [0, 1, 100, 65535, 65536, 65537, 131072, 131073, 200000]
files = []
for _ in range(120):
    pt = rb(rng.choice(SIZES)) if rng.random() < 0.5 else rb(rng.randrange(0, 500))
    armor = rng.choice(["armor", "binary"])
    if rng.random() < 0.3:
        pw = "".join(rng.choice("abcxyz0123 ") for _ in range(rng.randrange(1, 16))).strip() or "pw"
        log_n = rng.randrange(1, 11)
        recips, ids = [f"scrypt:{pw}:{log_n}"], [f"scrypt:{pw}"]
        wrong = [f"scrypt:{pw}x"]
    else:
        chosen = rng.sample(keys, rng.randrange(1, 4))
        recips, ids = [r for _, r in chosen], [rng.choice(chosen)[0]]
        wrong = [rng.choice([k for k in keys if k not in chosen])[0]]
    files.append((pt, armor, recips, ids, wrong))
enc = ask([{"op": "age.encrypt", "hex": pt.hex(), "args": [armor] + recips} for pt, armor, recips, _, _ in files])
for (pt, armor, recips, ids, wrong), hx in zip(files, enc):
    if hx.startswith("ERR"):
        print("age refused to encrypt:", hx, recips); sys.exit(1)
    data = bytes.fromhex(hx)
    add("age.decrypt", data, "\n".join(ids), want=pt.hex())
    add("age.decrypt", data, "\n".join(wrong), want="ERR")
    if armor == "binary":
        r = rng.random()
        hdr_end = data.index(b"\n---") + 1
        if r < 0.25:
            bad = bytearray(data)
            bad[rng.randrange(hdr_end)] ^= 1
        elif r < 0.5:
            bad = bytearray(data)
            bad[rng.randrange(len(bad) - 16, len(bad))] ^= 1
        elif r < 0.65 and len(pt) > 65536:
            bad = data[:data.index(b"\n", hdr_end + 4) + 1 + 16 + 65536 + 16]
        elif r < 0.8:
            bad = data + rb(rng.randrange(1, 20))
        else:
            bad = data[:rng.randrange(len(data))]
        bad = bytes(bad)
        add("age.decrypt", bad, "\n".join(ids), req={"op": "age.decrypt", "hex": bad.hex(), "args": ids})
# Written here, read there.
mine = []
for _ in range(90):
    pt = rb(rng.choice(SIZES)) if rng.random() < 0.4 else rb(rng.randrange(0, 300))
    armor = rng.choice(["armor", "binary"])
    if rng.random() < 0.3:
        pw = "".join(rng.choice("abcxyz0123") for _ in range(rng.randrange(1, 12)))
        recips, ids = [f"scrypt:{pw}:{rng.randrange(1, 11)}"], [f"scrypt:{pw}"]
    else:
        chosen = rng.sample(keys, rng.randrange(1, 4))
        recips, ids = [r for _, r in chosen], [rng.choice(chosen)[0]]
    random_bytes = rb(16 + 32 * len(recips) + 16 + 16)
    add("age.encrypt", pt, "\n".join([f"{armor}|{random_bytes.hex()}"] + recips), then=("age", ids, pt))

# --- run -------------------------------------------------------------------------
idx = [i for i, c in enumerate(cases) if c[4] is not None]
for i, ans in zip(idx, ask([cases[i][4] for i in idx])):
    # Go's refusals carry their reason; the harness says only ERR.
    cases[i][3] = "ERR" if ans.startswith("ERR") else ans

lines, batch, size = [], [], 0


def flush():
    global batch, size
    if not batch:
        return
    argv = []
    for c in batch:
        argv += [c[0], "x" + c[1].hex(), "x" + c[2].hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=900)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines.extend(ran.stdout.split("\n")[:-1])
    batch, size = [], 0


for c in cases:
    batch.append(c)
    size += 2 * (len(c[1]) + len(c[2])) + 30
    if size > 400000 or len(batch) >= 300:
        flush()
flush()

# Second round: what was written here, read by circl and age.
second, where = [], []
for i, c in enumerate(cases):
    then = c[5]
    mine_ans = lines[i] if i < len(lines) else "ERR"
    if not then:
        continue
    if then[0] == "hpke" and mine_ans != "ERR":
        kem, kdf, aead, mode, ikm_r, ikm_s, info, psk, pskid, aad, pt = then[1:]
        parts = mine_ans.split("|")
        if parts[2] != "-":
            ct = bytes.fromhex(parts[2])
            fields = [ikm_r, ikm_s, bytes.fromhex(parts[1]), info, psk, pskid, aad, ct]
            second.append({"op": "hpke.open", "hex": "|".join(x.hex() for x in fields), "args": [str(kem), str(kdf), str(aead), str(mode)]})
            where.append((i, "open"))
            # And a tampered one, opened here.
            bad = bytearray(ct)
            bad[rng.randrange(len(bad))] ^= 1
            add_case = ["hpke.open", b"", "|".join([str(kem), str(kdf), str(aead), str(mode)] +
                                                    [x.hex() for x in [ikm_r, ikm_s, bytes.fromhex(parts[1]), info, psk, pskid, aad, bytes(bad)]]).encode(),
                        "ERR", None, None]
            cases.append(add_case)
            cases.append(["hpke.open", b"", "|".join([str(kem), str(kdf), str(aead), str(mode)] +
                                                      [x.hex() for x in [ikm_r, ikm_s, bytes.fromhex(parts[1]), info, psk, pskid, aad, ct]]).encode(),
                          pt.hex(), None, None])
    elif then[0] == "age" and mine_ans != "ERR":
        second.append({"op": "age.decrypt", "hex": mine_ans, "args": then[1]})
        where.append((i, "age"))
back = {}
for (i, kind), ans in zip(where, ask(second)):
    back[i] = ans
# Run the cases added in the second round.
extra_from = len(lines)
batch = cases[extra_from:]
size = 0
flush()

families, agree, differ = {}, 0, 0
only = os.environ.get("ONLY", "")
for i, c in enumerate(cases):
    op, a, b, want, req, then = c
    mine_ans = lines[i] if i < len(lines) else "<missing>"
    if then and then[0] == "hpke":
        same = mine_ans == want
        opened = want == "ERR" or back.get(i) == then[11].hex()
        diffs = [k for k, (x, y) in enumerate(zip(mine_ans.split("|"), str(want).split("|"))) if x != y]
        why = None if same and opened else f"same={same} opened={opened} differing fields={diffs}"
    elif then and then[0] == "age":
        why = None if back.get(i) == then[2].hex() else f"age decrypts ours as {str(back.get(i))[:60]}"
    else:
        why = None if mine_ans == want else f"want {str(want)[:200]!r}"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 12 or (only and fam[1] <= 12)):
            print(f"  {op} a={a.hex()[:80]} b={(b if only else b[:120])!r}")
            print(f"    {why}")
            print(f"    grasm {mine_ans[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 30]
if thin or len(families) < 10:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
