"""PKCS#12 against OpenSSL 3.6 and JKS against Java 17's keytool.

Identities are made fresh each run: a CA and a leaf certificate signed by
it, for RSA, EC and Ed25519 keys.

- `pkcs12`: files OpenSSL exported (keys under PBES2 with AES or Camellia
  and under the PKCS#12 3DES, 2DES, RC2 and RC4 schemes; certificates
  plain, under AES, or under RC2 and 3DES; MACs over SHA-1, SHA-256,
  SHA-384 and SHA-512, or none) read here: the key, both certificates,
  the friendly name and the local key id must be what went in; wrong
  passwords refused. Files built here read by OpenSSL, which must find the
  same key, certificates and name.
- `jks`: keystores keytool converted from PKCS#12 (with a trusted
  certificate added) read here; keystores built here listed by keytool and
  converted back to PKCS#12, whose key and certificates must be what went
  in. Keys are compared after `openssl pkey` normalises them, because Java
  re-encodes them; certificates byte for byte. Passwords here are ASCII:
  Java 17 refuses any other for PBES2 key protection ("Password is not
  ASCII"), which is Java's limit, not the format's.
"""
import hashlib
import os
import random
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b36")
harness = os.path.join(work, "b36test")
built = subprocess.run([GRASM, "build", "packages/b36test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
LEGACY = ["-provider", "legacy", "-provider", "default"]


def run(cmd, data=b""):
    r = subprocess.run(cmd, input=data, capture_output=True)
    return r.stdout if r.returncode == 0 else None


def ossl(args, data=b""):
    return run(["openssl"] + args, data)


def path(name):
    return os.path.join(work, name)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def harness_answers(cases):
    out = []
    for i in range(0, len(cases), 50):
        argv = []
        for op, fields in cases[i:i + 50]:
            argv += [op, "x", "x" + fields.encode().hex()]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=1800)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        out.extend(ran.stdout.split("\n")[:-1])
    return out


def norm_key(der):
    return ossl(["pkey", "-inform", "DER", "-outform", "DER"], der)


def pem_blocks(text):
    import base64
    return [base64.b64decode("".join(b.split())) for b in re.findall(r"-----BEGIN [^-]+-----(.*?)-----END", text, re.S)]


# --- identities -------------------------------------------------------------------------------
ossl(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-keyout", path("ca.key"), "-out", path("ca.pem"),
      "-subj", "/CN=Grasm Test CA", "-days", "30", "-nodes"])
ca_der = ossl(["x509", "-in", path("ca.pem"), "-outform", "DER"])
ids = []
for i, spec in enumerate([["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:1024"], ["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"],
                          ["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-384"], ["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"],
                          ["-algorithm", "ED25519"]]):
    kp = path(f"k{i}.pem")
    ossl(["genpkey"] + spec + ["-out", kp])
    csr = ossl(["req", "-new", "-key", kp, "-subj", f"/CN=leaf {i}"])
    ossl(["x509", "-req", "-CA", path("ca.pem"), "-CAkey", path("ca.key"), "-days", "30", "-out", path(f"l{i}.pem")], csr)
    ids.append({"key_pem": kp, "leaf_pem": path(f"l{i}.pem"), "p8": ossl(["pkcs8", "-topk8", "-nocrypt", "-in", kp, "-outform", "DER"]),
                "leaf": ossl(["x509", "-in", path(f"l{i}.pem"), "-outform", "DER"]), "jks_ok": "ED25519" not in spec})

cases, wants = [], []


def summary(entries):
    return ";".join(sorted(entries)).encode().hex()


H = bytes.hex
NAMES = ["server", "Søren's key", "x", "代码签名"]
PASSWORDS = ["changeit", "pässword", "correct horse"]

# --- OpenSSL exports, Grasm reads ---------------------------------------------------------------
KEYPBE = ["AES-256-CBC", "AES-128-CBC", "CAMELLIA-256-CBC", "PBE-SHA1-3DES", "PBE-SHA1-2DES", "PBE-SHA1-RC2-40", "PBE-SHA1-RC2-128",
          "PBE-SHA1-RC4-128"]
CERTPBE = ["NONE", "AES-256-CBC", "PBE-SHA1-RC2-40", "PBE-SHA1-3DES"]
for n in range(60):
    ident = rng.choice(ids)
    name, pw = rng.choice(NAMES), rng.choice(PASSWORDS)
    opts = ["-keypbe", rng.choice(KEYPBE), "-certpbe", rng.choice(CERTPBE), "-iter", str(rng.choice([1, 100, 2048]))]
    nomac = rng.random() < 0.1
    opts += ["-nomac"] if nomac else ["-macalg", rng.choice(["sha1", "sha256", "sha384", "sha512"])]
    p12 = ossl(["pkcs12", "-export", "-inkey", ident["key_pem"], "-in", ident["leaf_pem"], "-certfile", path("ca.pem"), "-name", name,
                "-passout", "pass:" + pw] + opts + LEGACY)
    if p12 is None:
        print("openssl could not export with", opts); sys.exit(1)
    kid = hashlib.sha1(ident["leaf"]).digest()
    want = summary([f"k:{H(ident['p8'])}:{H(name.encode())}:{H(kid)}", f"c:{H(ident['leaf'])}:{H(name.encode())}:{H(kid)}",
                    f"c:{H(ca_der)}::"])
    cases.append(("pfx.parse", f"{H(p12)}|{H(pw.encode())}"))
    wants.append(want)
    cases.append(("pfx.parse", f"{H(p12)}|{H((pw + 'x').encode())}"))
    wants.append("ERR")

# --- Grasm builds, OpenSSL reads ------------------------------------------------------------------
PRFS = ["hmacWithSHA1", "hmacWithSHA256", "hmacWithSHA512"]
SCHEMES = ["pbeWithSHAAnd3-KeyTripleDES-CBC", "pbeWithSHAAnd2-KeyTripleDES-CBC", "pbeWithSHAAnd40BitRC2-CBC",
           "pbeWithSHAAnd128BitRC2-CBC", "pbeWithSHAAnd128BitRC4"]


def spec(allow_none):
    r = rng.random()
    if allow_none and r < 0.25:
        return ""
    if r < 0.65:
        c = rng.choice(["aes-256-cbc", "aes-128-cbc", "des-ede3-cbc", "camellia-128-cbc"])
        return f"pbes2:{c}:{rng.choice(PRFS)}:{H(rb(16))}:{rng.choice([1, 100, 2048])}:{H(rb(8 if c.startswith('des') else 16))}"
    return f"pbe:{rng.choice(SCHEMES)}:{H(rb(8))}:{rng.choice([1, 100, 2048])}"


builds = []
for n in range(50):
    ident = rng.choice(ids)
    name, pw = rng.choice(NAMES), rng.choice(PASSWORDS)
    kid = hashlib.sha1(ident["leaf"]).digest()
    fields = [H(ident["p8"]), H(ident["leaf"]) + "," + H(ca_der), H(name.encode()), H(kid), H(pw.encode()), spec(False), spec(True),
              rng.choice(["sha1", "sha256", "sha384", "sha512"]), H(rb(rng.choice([8, 16, 20]))), str(rng.choice([1, 100, 2048]))]
    builds.append((ident, name, pw, "|".join(fields)))
outs = harness_answers([("pfx.build", b[3]) for b in builds])
for (ident, name, pw, _), out in zip(builds, outs):
    ok = "missing"
    if out != "ERR":
        p12 = bytes.fromhex(out)
        key_pem = ossl(["pkcs12", "-passin", "pass:" + pw, "-nodes", "-nocerts"] + LEGACY, p12)
        certs = ossl(["pkcs12", "-passin", "pass:" + pw, "-nokeys"] + LEGACY, p12)
        if key_pem is None or certs is None:
            ok = "OpenSSL could not read it"
        else:
            got_key = ossl(["pkey", "-outform", "DER"], key_pem)
            # OpenSSL prints a non-ASCII friendlyName byte by byte, not as
            # UTF-8, so the name is checked here only when it is ASCII (the
            # other direction checks every name exactly).
            text = certs.decode("latin-1")
            named = (not name.isascii()) or f"friendlyName: {name}" in text
            ok = "ok" if (got_key == norm_key(ident["p8"]) and sorted(pem_blocks(text)) == sorted([ident["leaf"], ca_der])
                          and named) else "OpenSSL read something else"
    cases.append(("pfx.opened", ""))
    wants.append(ok)

# --- keytool writes JKS, Grasm reads ----------------------------------------------------------------
KT = ["keytool"]
jks_ids = [i for i in ids if i["jks_ok"]]
for n in range(12):
    ident = jks_ids[n % len(jks_ids)]
    name, pw = rng.choice(["server", "Signing Key", "tomcat"]), rng.choice(["changeit", "s3cret pass"])
    p12 = ossl(["pkcs12", "-export", "-inkey", ident["key_pem"], "-in", ident["leaf_pem"], "-certfile", path("ca.pem"), "-name", name,
                "-passout", "pass:" + pw])
    open(path("in.p12"), "wb").write(p12)
    js = path(f"ks{n}.jks")
    if os.path.exists(js):
        os.remove(js)
    run(KT + ["-importkeystore", "-noprompt", "-srckeystore", path("in.p12"), "-srcstoretype", "PKCS12", "-srcstorepass", pw,
              "-destkeystore", js, "-deststoretype", "JKS", "-deststorepass", pw, "-destkeypass", pw])
    open(path("ca.der"), "wb").write(ca_der)
    run(KT + ["-importcert", "-noprompt", "-alias", "Root CA", "-file", path("ca.der"), "-keystore", js, "-storetype", "JKS", "-storepass", pw])
    data = open(js, "rb").read()
    cases.append(("jks.parse", f"{H(data)}|{H(pw.encode())}|{H(pw.encode())}"))
    wants.append(("jks", ident, name, ca_der))
    cases.append(("jks.parse", f"{H(data)}|{H(b'wrong')}|{H(pw.encode())}"))
    wants.append("ERR")

# --- Grasm writes JKS, keytool reads ----------------------------------------------------------------
jbuilds = []
for n in range(12):
    ident = jks_ids[n % len(jks_ids)]
    alias, pw = rng.choice(["server", "signing key", "tomcat"]), rng.choice(["changeit", "s3cret pass"])
    entries = [f"{H(alias.encode())}~k~{H(ident['p8'])}~{H(ident['leaf'])}+{H(ca_der)}~{1760000000000 + n}",
               f"{H(b'root ca')}~c~~{H(ca_der)}~1760000000000"]
    jbuilds.append((ident, alias, pw, f"{';'.join(entries)}|{H(pw.encode())}|{H(pw.encode())}|{H(rb(20))}"))
jouts = harness_answers([("jks.build", b[3]) for b in jbuilds])
for (ident, alias, pw, _), out in zip(jbuilds, jouts):
    ok = "missing"
    if out != "ERR":
        js = path("built.jks")
        open(js, "wb").write(bytes.fromhex(out))
        listed = run(KT + ["-list", "-keystore", js, "-storetype", "JKS", "-storepass", pw])
        p12p = path("back.p12")
        if os.path.exists(p12p):
            os.remove(p12p)
        run(KT + ["-importkeystore", "-noprompt", "-srckeystore", js, "-srcstoretype", "JKS", "-srcstorepass", pw, "-srcalias", alias,
                  "-destkeystore", p12p, "-deststoretype", "PKCS12", "-deststorepass", pw, "-destkeypass", pw])
        back = open(p12p, "rb").read() if os.path.exists(p12p) else b""
        key_pem = ossl(["pkcs12", "-passin", "pass:" + pw, "-nodes", "-nocerts"], back)
        certs = ossl(["pkcs12", "-passin", "pass:" + pw, "-nokeys"], back)
        if listed is None or key_pem is None or certs is None:
            ok = "keytool could not read it"
        else:
            ok = "ok" if (ossl(["pkey", "-outform", "DER"], key_pem) == norm_key(ident["p8"]) and
                          sorted(pem_blocks(certs.decode("latin-1"))) == sorted([ident["leaf"], ca_der])
                          and alias in listed.decode("utf-8", "replace")) else "keytool read something else"
    cases.append(("jks.opened", ""))
    wants.append(ok)

# --- compare ------------------------------------------------------------------------------------------
real = [i for i, c in enumerate(cases) if not c[0].endswith(".opened")]
answers = harness_answers([cases[i] for i in real])
mine = ["ok"] * len(cases)
for i, a in zip(real, answers):
    mine[i] = a
families, differ = {}, 0
for i, (op, fields) in enumerate(cases):
    want = wants[i]
    if isinstance(want, tuple):
        # A JKS read: keys compared once normalised, certificates exactly.
        _, ident, name, ca = want
        got = bytes.fromhex(mine[i]).decode() if mine[i] != "ERR" else ""
        entries = sorted(got.split(";")) if got else []
        keys = [e for e in entries if e.startswith("k:")]
        trusted = [e for e in entries if e.startswith("c:")]
        good = (len(keys) == 1 and len(trusted) == 1 and bytes.fromhex(keys[0].split(":")[1]).decode() == name.lower()
                and norm_key(bytes.fromhex(keys[0].split(":")[2])) == norm_key(ident["p8"])
                and keys[0].split(":")[3] == H(ident["leaf"]) + "," + H(ca)
                and trusted[0] == f"c:{H(b'root ca')}::{H(ca)}")
        mine[i], want = ("ok" if good else mine[i][:80]), "ok"
    fam = families.setdefault(op, [0, 0])
    if mine[i] == want:
        fam[0] += 1
    else:
        fam[1] += 1
        differ += 1
        if differ <= 10:
            print(f"  {op} {fields[:120]}")
            print(f"    want  {str(want)[:200]}")
            print(f"    grasm {mine[i][:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
if len(families) < 4 or any(a + d < 12 for a, d in families.values()):
    print("too few cases to mean anything", families); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
