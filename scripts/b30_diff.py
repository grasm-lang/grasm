"""ML-DSA (FIPS 204) and SLH-DSA (FIPS 205).

Two references for each. NIST's ACVP vectors, as circl ships them in its
module: for ML-DSA every key generation, signature (deterministic and
hedged, through the internal interface the vectors use) and verification;
for SLH-DSA every verification (pure, pre-hashed and internal, all twelve
parameter sets), key generation for every fast set and one of each small
set, and signatures for two cases of each fast-set group and one per
small set, those being the ones that take millions of hashes. Then circl
itself on random keys, contexts and messages: keys and deterministic
signatures must be byte-identical, its signatures must verify here, and
tampered ones must not.

Pre-hashing with SHA-224 or SHA-512/224 is refused by the package, so the
handful of vectors that use them are left out.
"""
import gzip
import json
import os
import random
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle  # noqa: E402

rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b30")
harness = os.path.join(work, "b30test")
built = subprocess.run([GRASM, "build", "packages/b30test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
go = oracle.build(os.path.join(work, "gomods"))
cache = subprocess.run(["go", "env", "GOMODCACHE"], capture_output=True, text=True).stdout.strip()
CIRCL = os.path.join(cache, "github.com/cloudflare/circl@v1.6.5/sign")


def ask(reqs):
    out = subprocess.run([go], input=json.dumps(reqs), capture_output=True, text=True)
    if out.returncode != 0:
        print(out.stderr[-2000:]); sys.exit(1)
    return json.loads(out.stdout)


def load(path):
    return json.load(gzip.open(os.path.join(CIRCL, path)))


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


cases = []   # [op, a, fields, want, go request]


def add(op, a, fields, want=None, req=None):
    cases.append([op, a, "|".join(fields), want, req])


# --- ML-DSA, ACVP -----------------------------------------------------------------
kp, kr = load("mldsa/testdata/ML-DSA-keyGen-FIPS204/prompt.json.gz"), load("mldsa/testdata/ML-DSA-keyGen-FIPS204/expectedResults.json.gz")
res = {t["tcId"]: t for g in kr["testGroups"] for t in g["tests"]}
for g in kp["testGroups"]:
    for t in g["tests"]:
        r = res[t["tcId"]]
        add("mldsa.keygen", bytes.fromhex(t["seed"]), [g["parameterSet"]], want=(r["pk"] + "|" + r["sk"]).lower())
sp, sr = load("mldsa/testdata/ML-DSA-sigGen-FIPS204/prompt.json.gz"), load("mldsa/testdata/ML-DSA-sigGen-FIPS204/expectedResults.json.gz")
res = {t["tcId"]: t for g in sr["testGroups"] for t in g["tests"]}
for g in sp["testGroups"]:
    for t in g["tests"]:
        rnd = "00" * 32 if g["deterministic"] else t["rnd"]
        add("mldsa.sign", bytes.fromhex(t["sk"] + t["message"]), [g["parameterSet"], "internal", "", rnd.lower()],
            want=res[t["tcId"]]["signature"].lower())
vp, vr = load("mldsa/testdata/ML-DSA-sigVer-FIPS204/prompt.json.gz"), load("mldsa/testdata/ML-DSA-sigVer-FIPS204/expectedResults.json.gz")
res = {t["tcId"]: t for g in vr["testGroups"] for t in g["tests"]}
for g in vp["testGroups"]:
    for t in g["tests"]:
        add("mldsa.verify", bytes.fromhex(g["pk"] + t["signature"] + t["message"]), [g["parameterSet"], "internal", ""],
            want="true" if res[t["tcId"]]["testPassed"] else "false")

# --- ML-DSA, circl -----------------------------------------------------------------
ML = ["ML-DSA-44", "ML-DSA-65", "ML-DSA-87"]
mkeys = []
for _ in range(60):
    name, seed = rng.choice(ML), rb(32)
    add("mldsa.keygen", seed, [name], req={"op": "mldsa.keygen", "args": [name], "hex": seed.hex()})
    mkeys.append((name, seed))
gen = ask([{"op": "mldsa.keygen", "args": [n], "hex": s.hex()} for n, s in mkeys])
msigs = []
for (name, _), pair in zip(mkeys, gen):
    pk, sk = (bytes.fromhex(x) for x in pair.split("|"))
    msg, ctx = rb(rng.choice([0, 1, 33, 500])), rb(rng.choice([0, 0, 8, 255]))
    msigs.append((name, pk, sk, msg, ctx))
    add("mldsa.sign", sk + msg, [name, "pure", ctx.hex(), "00" * 32],
        req={"op": "mldsa.sign", "args": [name, ctx.hex()], "hex": sk.hex() + "|" + msg.hex()})
sigs = ask([{"op": "mldsa.sign", "args": [n, c.hex()], "hex": s.hex() + "|" + m.hex()} for n, p, s, m, c in msigs])
for (name, pk, sk, msg, ctx), sig in zip(msigs, sigs):
    sig = bytes.fromhex(sig)
    add("mldsa.verify", pk + sig + msg, [name, "pure", ctx.hex()], want="true")
    for bad_sig, bad_msg, bad_ctx in [(sig[:5] + bytes([sig[5] ^ 1]) + sig[6:], msg, ctx), (sig, msg + b"x", ctx),
                                      (sig, msg, ctx + b"x" if len(ctx) < 255 else ctx[:-1]),
                                      (sig[:-1] + bytes([sig[-1] ^ 1]), msg, ctx)]:
        add("mldsa.verify", pk + bad_sig + bad_msg, [name, "pure", bad_ctx.hex()],
            req={"op": "mldsa.verify", "args": [name, bad_ctx.hex()], "hex": f"{pk.hex()}|{bad_msg.hex()}|{bad_sig.hex()}"})

# --- SLH-DSA, ACVP -------------------------------------------------------------------
UNSUPPORTED = {"SHA2-224", "SHA2-512/224"}
kp, kr = load("slhdsa/testdata/keyGen_prompt.json.gz"), load("slhdsa/testdata/keyGen_results.json.gz")
res = {t["tcId"]: t for g in kr["testGroups"] for t in g["tests"]}
for g in kp["testGroups"]:
    fast = g["parameterSet"].endswith("f")
    for t in g["tests"][: (len(g["tests"]) if fast else 1)]:
        r = res[t["tcId"]]
        add("slhdsa.keygen", bytes.fromhex(t["skSeed"] + t["skPrf"] + t["pkSeed"]), [g["parameterSet"]],
            want=(r["pk"] + "|" + r["sk"]).lower())
sp, sr = load("slhdsa/testdata/sigGen_prompt.json.gz"), load("slhdsa/testdata/sigGen_results.json.gz")
res = {t["tcId"]: t for g in sr["testGroups"] for t in g["tests"]}
small_done = set()
for g in sp["testGroups"]:
    name = g["parameterSet"]
    fast = name.endswith("f")
    if not fast and name in small_done:
        continue
    mode = "internal" if g.get("signatureInterface") == "internal" else ("prehash" if g.get("preHash") == "preHash" else "pure")
    picked = [t for t in g["tests"] if t.get("hashAlg") not in UNSUPPORTED][: (2 if fast else 1)]
    for t in picked:
        rnd = "" if g["deterministic"] else t["additionalRandomness"]
        add("slhdsa.sign", bytes.fromhex(t["sk"] + t["message"]),
            [name, mode, t.get("context", ""), rnd.lower(), t.get("hashAlg", "")], want=res[t["tcId"]]["signature"].lower())
        if not fast:
            small_done.add(name)
vp, vr = load("slhdsa/testdata/verify_prompt.json.gz"), load("slhdsa/testdata/verify_results.json.gz")
res = {t["tcId"]: t for g in vr["testGroups"] for t in g["tests"]}
for g in vp["testGroups"]:
    mode = "internal" if g.get("signatureInterface") == "internal" else ("prehash" if g.get("preHash") == "preHash" else "pure")
    for t in g["tests"]:
        if t.get("hashAlg") in UNSUPPORTED:
            continue
        add("slhdsa.verify", bytes.fromhex(t["pk"] + t["signature"] + t["message"]),
            [g["parameterSet"], mode, t.get("context", ""), "", t.get("hashAlg", "")],
            want="true" if res[t["tcId"]]["testPassed"] else "false")

# --- SLH-DSA, circl --------------------------------------------------------------------
FAST = [f"SLH-DSA-{h}-{b}f" for h in ("SHA2", "SHAKE") for b in ("128", "192", "256")]
N = {"128": 16, "192": 24, "256": 32}
skeys = []
for name in FAST * 3:
    seeds = rb(3 * N[name[-4:-1]])
    skeys.append((name, seeds))
    add("slhdsa.keygen", seeds, [name], req={"op": "slhdsa.keygen", "args": [name], "hex": seeds.hex()})
gen = ask([{"op": "slhdsa.keygen", "args": [n], "hex": s.hex()} for n, s in skeys])
ssigs = []
for (name, _), pair in zip(skeys[:12], gen[:12]):
    pk, sk = (bytes.fromhex(x) for x in pair.split("|"))
    msg, ctx = rb(rng.choice([0, 1, 100])), rb(rng.choice([0, 7, 255]))
    ssigs.append((name, pk, sk, msg, ctx))
    add("slhdsa.sign", sk + msg, [name, "pure", ctx.hex(), "", ""],
        req={"op": "slhdsa.sign", "args": [name, ctx.hex()], "hex": sk.hex() + "|" + msg.hex()})
sigs = ask([{"op": "slhdsa.sign", "args": [n, c.hex()], "hex": s.hex() + "|" + m.hex()} for n, p, s, m, c in ssigs])
for (name, pk, sk, msg, ctx), sig in zip(ssigs, sigs):
    sig = bytes.fromhex(sig)
    add("slhdsa.verify", pk + sig + msg, [name, "pure", ctx.hex(), "", ""], want="true")
    for _ in range(6):
        k = rng.randrange(len(sig))
        bad = sig[:k] + bytes([sig[k] ^ (1 << rng.randrange(8))]) + sig[k + 1:]
        add("slhdsa.verify", pk + bad + msg, [name, "pure", ctx.hex(), "", ""],
            req={"op": "slhdsa.verify", "args": [name, ctx.hex()], "hex": f"{pk.hex()}|{msg.hex()}|{bad.hex()}"})

# --- run -----------------------------------------------------------------------------------
idx = [i for i, c in enumerate(cases) if c[4] is not None]
for i, ans in zip(idx, ask([cases[i][4] for i in idx])):
    cases[i][3] = "ERR" if ans.startswith("ERR") else ans


def batches():
    out, cur, size = [], [], 0
    for i, c in enumerate(cases):
        heavy = c[0] == "slhdsa.sign" or c[0] == "slhdsa.keygen" and not c[2].split("|")[0].endswith("f")
        cur.append(i)
        size += 2 * (len(c[1]) + len(c[2])) + 30
        if size > 400000 or len(cur) >= (4 if heavy else 200):
            out.append(cur)
            cur, size = [], 0
    if cur:
        out.append(cur)
    return out


def run(batch):
    argv = []
    for i in batch:
        c = cases[i]
        argv += [c[0], "x" + c[1].hex(), "x" + c[2].encode().hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
    if ran.returncode != 0:
        return [("CRASH " + ran.stderr[-300:])] * len(batch)
    return ran.stdout.split("\n")[:-1]


answers = {}
with ThreadPoolExecutor(int(os.environ.get("JOBS", "4"))) as pool:
    for batch, got in zip(batches(), pool.map(run, batches())):
        for i, a in zip(batch, got):
            answers[i] = a

families, agree, differ = {}, 0, 0
only = os.environ.get("ONLY", "")
for i, c in enumerate(cases):
    op, a, f, want, _ = c
    mine = answers.get(i, "<missing>")
    fam_key = op + ":" + f.split("|")[0][:12]
    fam = families.setdefault(op, [0, 0])
    if mine == want:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} {f[:80]}")
            print(f"    want {str(want)[:120]}")
            print(f"    grasm {mine[:120]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 50]
if thin or len(families) < 6:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
