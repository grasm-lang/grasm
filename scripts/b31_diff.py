"""go.mod files, signed notes, transparency logs and coverage profiles.

- `gomod`: random go.mod files (every directive, quoted and bare tokens,
  shorthand and pseudo versions, /vN and gopkg.in paths, comments before,
  beside and inside blocks, blank lines, CRLF) and broken ones, against
  x/mod/modfile's Parse; what the file says and what Format writes must
  both match.
- `signednote`: keys from seeds, notes signed by several keys, and notes
  opened with known, unknown, duplicate and ambiguous keys, tampered or
  malformed, against x/mod/sumdb/note.
- `tlog`: logs of up to 300 records: tree hashes, stored-hash layout,
  record and tree proofs, proofs checked whole and tampered, tile paths
  and tree heads, against x/mod/sumdb/tlog.
- `coverprofile`: random profiles with repeated blocks in both modes, and
  broken lines, against x/tools/cover.
"""
import base64
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
work = tempfile.mkdtemp(prefix="b31")
harness = os.path.join(work, "b31test")
built = subprocess.run([GRASM, "build", "packages/b31test/src/main.gr", "--native", "-o", harness],
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


cases = []   # [op, a bytes, b text, want, go request, transform]


def add(op, a, b="", want=None, req=None, hexed=False):
    cases.append([op, a, b, want, req, hexed])


# --- go.mod ------------------------------------------------------------------------
PATHS = ["golang.org/x/net", "github.com/a/b", "github.com/a/b/v2", "example.com/m/v3", "gopkg.in/yaml.v2",
         "gopkg.in/check.v1", "rsc.io/quote", "my.org/x y", "github.com/a/b/v0", "example.com/v1"]
VERSIONS = ["v1.2.3", "v1.2", "v1", "v0.0.0-20230101000000-abcdef123456", "v2.0.0", "v2.1.0+incompatible", "v3.0.0-rc.1",
            "v1.2.3+build", "1.2.3", "v1.02.3", "v1.2.3-01", "\"v1.4.0\"", "v0.1.0", "v1.2.3-pre.1+meta"]


# Path and version pairs that agree on the major version, for files meant
# to parse.
GOOD = [("golang.org/x/net", ["v0.1.0", "v1.2.3", "v1.2", "v1", "v0.0.0-20230101000000-abcdef123456", "v1.2.3-pre.1+meta", "\"v1.4.0\""]),
        ("github.com/a/b/v2", ["v2.0.0", "v2.1.0", "v2.0.0-rc.1"]), ("gopkg.in/yaml.v2", ["v2.4.0", "v2.0.0"]),
        ("example.com/m/v3", ["v3.0.0-rc.1", "v3.1.0"]), ("github.com/a/b", ["v2.1.0+incompatible", "v1.0.0"]),
        ("rsc.io/quote", ["v1.5.2", "v1.5"]), ("my.org/x y", ["v1.0.0"])]
CLEAN = [False]


def tok(s):
    if " " in s or rng.random() < 0.15:
        return json.dumps(s)
    return s


def comment():
    return rng.choice(["// a note", "// indirect", "//indirect", "// indirect; for tests", "// indirectly", "// Deprecated: use v2",
                       "//", "// x // y", "// unicode é"])


def ver():
    return rng.choice(VERSIONS)


def path():
    return rng.choice(PATHS)


def pair():
    if CLEAN[0]:
        p, vs = rng.choice(GOOD)
        return tok(p), rng.choice(vs)
    return tok(path()), ver()


def line_for(verb):
    if verb == "require" or verb == "exclude":
        return "%s %s" % pair()
    if verb == "replace":
        lp, lv = pair()
        left = lp + (" " + lv if rng.random() < 0.5 else "")
        if CLEAN[0]:
            right = rng.choice(["%s %s" % pair(), "./local", "../up", "/abs/dir"])
        else:
            right = rng.choice([tok(path()) + " " + ver(), "./local", "../up", "/abs/dir", "C:\\win\\dir", tok(path()), "./dir v1.0.0",
                                "example.com/m@v1.0.0"])
        return f"{left} => {right}"
    if verb == "retract":
        return rng.choice([ver(), f"[{ver()}, {ver()}]", f"[ {ver()} , {ver()} ]", f"[{ver()}]", "(v1.0.0)"])
    if verb == "godebug":
        return rng.choice(["panicnil=1", "x509sha1=1"] if CLEAN[0] else ["panicnil=1", "x509sha1=1", "bad", "a=b=c", "k=\"v\""])
    if verb in ("tool", "ignore"):
        return tok(rng.choice(["golang.org/x/tools/cmd/stringer", "./vendor", "example.com/tool", "with space"]))
    return ""


def gomod():
    CLEAN[0] = rng.random() < 0.6
    out = []
    eol = "\r\n" if rng.random() < 0.1 else "\n"
    if rng.random() < 0.3:
        out.append(comment())
        if rng.random() < 0.5:
            out.append("")
    dep = rng.random() < 0.2
    if dep:
        out.append("// Deprecated: " + rng.choice(["use example.com/new", "gone"]))
    out.append("module " + tok(rng.choice(["example.com/m", "github.com/u/r/v2", "my mod", "example.com/m/v1"])) +
               (" " + comment() if rng.random() < 0.2 else ""))
    out.append("")
    if rng.random() < 0.9:
        out.append("go " + rng.choice(["1.21", "1.22.3", "1.23rc1", "1.21.0", "1.20beta2"] +
                                      ([] if CLEAN[0] else ["1", "1.021", "v1.21", "1.21 "])))
    if rng.random() < 0.3:
        out.append("toolchain " + rng.choice(["go1.21.3", "default", "go1.22rc1"] + ([] if CLEAN[0] else ["gccgo", "go1"])))
    for _ in range(rng.randrange(0, 6)):
        verb = rng.choice(["require", "require", "exclude", "replace", "retract", "godebug", "tool", "ignore"])
        if rng.random() < 0.5:
            head = verb + " (" + (" " + comment() if rng.random() < 0.15 else "")
            out.append(head)
            for _ in range(rng.randrange(0, 5)):
                if rng.random() < 0.2:
                    out.append("\t" + comment())
                if rng.random() < 0.15:
                    out.append("")
                out.append("\t" + line_for(verb) + (" " + comment() if rng.random() < 0.3 else ""))
            out.append(")" + (" " + comment() if rng.random() < 0.1 else ""))
        else:
            if rng.random() < 0.2:
                out.append(comment())
            out.append(verb + " " + line_for(verb) + (" " + comment() if rng.random() < 0.3 else ""))
        if rng.random() < 0.5:
            out.append("")
    if rng.random() < 0.2:
        out.append(comment())
    text = eol.join(out) + (eol if rng.random() < 0.9 else "")
    if not CLEAN[0] and rng.random() < 0.2:
        text = text + rng.choice(["unknown directive\n", "/* c */\n", "require (\n", "module \"unterminated\n", "go 1.21\n",
                                  "require x v1.0.0 )\n", "require ( x v1.0.0 )\n", "\x01\n"])
    return text


for _ in range(1400):
    text = gomod().encode()
    add("mod", text, req={"op": "modfile", "hex": text.hex()}, hexed=True)

# --- signed notes ---------------------------------------------------------------------
NAMES = ["sum.golang.org", "example.com/log", "witness-1", "a", "bad name", "x+y"]
seeds = [(rb(32), rng.choice(NAMES[:4])) for _ in range(12)]
keys = ask([{"op": "note.keygen", "args": [n], "hex": s.hex()} for s, n in seeds])
for (seed, name), k in zip(seeds, keys):
    add("note.keygen", seed, name, want=k)
for seed, name in [(rb(32), "bad name"), (rb(32), "x+y"), (rb(31), "short"), (rb(33), "long"), (rb(32), ""), (rb(32), "tab\there"),
                   (rb(32), "nbsp\u00a0x"), (b"", "empty")]:
    add("note.keygen", seed, name, want="ERR")
good = [k.split("|") for k in keys if k != "ERR"]
TEXTS = ["go.sum database tree\n1234\nxB1nDQ3uO6c3Ixa3QZ5qkEp0Du1Ae5jsCwAvbJrhEiA=\n", "hello\n", "a\nb\n", "no newline", "é ünicode\n",
         "with\ttab\n", "\n"]
for _ in range(300):
    text = rng.choice(TEXTS)
    signers = rng.sample(good, rng.randrange(1, 4))
    msg_hex = ask([{"op": "note.sign", "args": [s for s, _ in signers], "hex": text.encode().hex()}])[0]
    add("note.sign", text.encode(), "\n".join(s for s, _ in signers), want=msg_hex)
    if msg_hex == "ERR":
        continue
    msg = bytes.fromhex(msg_hex)
    r = rng.random()
    if r < 0.15:
        msg = msg.replace(text.encode()[:1], b"Z", 1)
    elif r < 0.25:
        i = msg.rindex(b" ") + 3
        msg = msg[:i] + bytes([msg[i] ^ 1 if msg[i] != ord("=") else msg[i]]) + msg[i + 1:]
    elif r < 0.33:
        msg = msg + msg[msg.index(b"\n\n") + 2:]
    elif r < 0.38:
        msg = msg.replace(b"\xe2\x80\x94 ", b"-- ", 1)
    elif r < 0.42:
        msg = msg[:msg.index(b"\n\n") + 1] + msg[msg.index(b"\n\n") + 2:]
    elif r < 0.45:
        msg = msg.replace(b"\n", b"\x01\n", 1)
    elif r < 0.48:
        msg = msg + b"\xe2\x80\x94 extra AAAAAAAAAA==\n"
    elif r < 0.56:
        # Unused bits set in the last base64 group: Go ignores them.
        i = msg.rindex(b"=\n") - 1
        A = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        msg = msg[:i] + bytes([A[A.index(msg[i]) | rng.randrange(1, 4)]]) + msg[i + 1:]
    known = rng.sample(good, rng.randrange(0, 4))
    if rng.random() < 0.5:
        known.append(rng.choice(signers))
    if rng.random() < 0.08 and known:
        known.append(known[0])
    vkeys = [v for _, v in known]
    add("note.open", msg, "\n".join(vkeys), req={"op": "note.open", "args": vkeys, "hex": msg.hex()})
# Keys that do not parse.
for k in ["PRIVATE+KEY+a+00000000+AQ==", "a+12345678+AQ==", "a+zzzzzzzz+AQ==", "nokey"]:
    add("note.open", b"x\n\n\xe2\x80\x94 a AAAAAA==\n", k, req={"op": "note.open", "args": [k], "hex": b"x\n\n\xe2\x80\x94 a AAAAAA==\n".hex()})

# --- tlog ----------------------------------------------------------------------------------
for _ in range(60):
    n = rng.choice([1, 2, 3, 7, 8, 9, 16, 31, 33, 64, 100, 257, 300])
    records = [rb(rng.randrange(0, 20)) for _ in range(n)]
    qs = []
    for _ in range(25):
        k = rng.choice(["th", "rp", "tp", "sc", "si"])
        if k == "th":
            qs.append(f"th:{rng.randrange(0, n + 1)}:0")
        elif k == "rp":
            t = rng.randrange(1, n + 1)
            qs.append(f"rp:{t}:{rng.randrange(0, t)}")
        elif k == "tp":
            t = rng.randrange(1, n + 1)
            qs.append(f"tp:{t}:{rng.randrange(1, t + 1)}")
        elif k == "sc":
            qs.append(f"sc:{rng.randrange(0, 5000)}:0")
        else:
            qs.append(f"si:{rng.randrange(0, 8)}:{rng.randrange(0, 500)}")
    rec_csv = ",".join(r.hex() for r in records)
    add("tlog", b"", rec_csv + ";" + " ".join(qs), req={"op": "tlog", "hex": rec_csv, "args": qs})
    # Proofs from Go, checked here whole and tampered.
    t = rng.randrange(1, n + 1)
    m = rng.randrange(0, t)
    m2 = rng.randrange(1, t + 1)
    got = ask([{"op": "tlog", "hex": rec_csv, "args": [f"th:{t}:0", f"rp:{t}:{m}", f"th:{m2}:0", f"tp:{t}:{m2}"]}])[0].split("|")
    th, rp, th2, tp = got
    import hashlib
    leaf = hashlib.sha256(b"\x00" + records[m]).hexdigest()
    for proof, want_ok in [(rp, True)] + ([(rp.replace(rp[0], "f" if rp[0] != "f" else "e", 1), False)] if rp else []):
        add("tlog.check", b"", f"record|{t}|{m}|{proof}|{th}|{leaf}", want="true" if want_ok else "false")
    add("tlog.check", b"", f"record|{t}|{m}|{rp}|{th}|{'00' * 32}", want="false")
    add("tlog.check", b"", f"tree|{t}|{m2}|{tp}|{th}|{th2}", want="true")
    if tp:
        add("tlog.check", b"", f"tree|{t}|{m2}|{tp + ',' + th}|{th}|{th2}", want="false")
for _ in range(300):
    h = rng.randrange(1, 9)
    qs = [f"for:{h}:{rng.randrange(0, 100000)}", f"new:{h}:{rng.randrange(0, 5000)}:{rng.randrange(5000, 70000)}"]
    p = rng.choice(["tile/8/0/x001/234.p/7", "tile/8/0/001", "tile/2/1/000.p/2", "tile/8/data/005", "tile/31/0/000", "tile/8/0/1234",
                    "tile/8/0/x000/123", "tile/8/0/012.p/256", "tile/8/0/012.p/0", "tile/0/0/000", "file/8/0/000", "tile/8/-1/000",
                    "tile/3/2/x123/x456/789", "tile/3/2/123.p/08"])
    qs.append("parse:" + p)
    add("tlog.tile", b"", " ".join(qs), req={"op": "tlog.tile", "args": qs})
for _ in range(150):
    n = rng.choice([0, 1, 31897089, 2 ** 40, rng.randrange(10 ** 9)])
    hsh = rb(32)
    text = f"go.sum database tree\n{n}\n{base64.b64encode(hsh).decode()}\n"
    r = rng.random()
    if r < 0.1:
        text = text.replace("tree", "trie")
    elif r < 0.2:
        text = text.replace(f"\n{n}\n", f"\n0{n}\n")
    elif r < 0.25:
        text = text[:-1]
    elif r < 0.3:
        text = text.replace("=", "")
    elif r < 0.35:
        text = text + "extra\n"
    elif r < 0.42:
        A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        b64 = base64.b64encode(hsh).decode()
        i = len(b64) - 2
        text = text.replace(b64, b64[:i] + A[A.index(b64[i]) | rng.randrange(1, 4)] + b64[i + 1:])
    elif r < 0.46:
        b64 = base64.b64encode(hsh).decode()
        text = text.replace(b64, b64[:10] + "\r" + b64[10:])
    add("tree", text.encode(), req={"op": "tree", "hex": text.encode().hex()})

# --- coverage profiles ---------------------------------------------------------------------
for _ in range(500):
    mode = rng.choice(["set", "count", "atomic"])
    lines = [f"mode: {mode}"]
    blocks = []
    for f in rng.sample(["a.go", "pkg/b.go", "c:/x.go", "d.go"], rng.randrange(1, 4)):
        starts = set()
        for _ in range(rng.randrange(1, 8)):
            sl, sc = rng.randrange(1, 60), rng.randrange(1, 40)
            if (sl, sc) in starts:
                continue
            starts.add((sl, sc))
            el, ec = sl + rng.randrange(0, 5), rng.randrange(1, 40)
            ns = rng.randrange(0, 6)
            cnt = rng.randrange(0, 2) if mode == "set" else rng.randrange(0, 50)
            blocks.append((f, sl, sc, el, ec, ns, cnt))
            if rng.random() < 0.3:
                blocks.append((f, sl, sc, el, ec, ns if rng.random() < 0.95 else ns + 1, rng.randrange(0, 2)))
    rng.shuffle(blocks)
    for f, sl, sc, el, ec, ns, cnt in blocks:
        lines.append(f"{f}:{sl}.{sc},{el}.{ec} {ns} {cnt}")
    r = rng.random()
    if r < 0.05:
        lines.append("bad line")
    elif r < 0.08:
        lines[0] = "mode:"
    elif r < 0.1:
        lines.append(":1.1,2.2 1 1")
    elif r < 0.12:
        lines.append("x.go:1.1,2.2 1 -1")
    elif r < 0.15:
        lines.append("x.go:+1.01,2.2 1 1")
    text = "\n".join(lines) + rng.choice(["\n", "", "\r\n"])
    add("cover", text.encode(), req={"op": "cover", "hex": text.encode().hex()}, hexed=True)

# --- run -------------------------------------------------------------------------------------
idx = [i for i, c in enumerate(cases) if c[4] is not None]
for i, ans in zip(idx, ask([cases[i][4] for i in idx])):
    c = cases[i]
    c[3] = "ERR" if ans.startswith("ERR") else (ans.encode().hex() if c[5] else ans)

lines, batch, size = [], [], 0


def flush():
    global batch, size
    if not batch:
        return
    argv = []
    for c in batch:
        argv += [c[0], "x" + c[1].hex(), "x" + c[2].encode().hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=600)
    if ran.returncode != 0:
        # Name the case that crashed.
        for c in batch:
            one = subprocess.run([harness, c[0], "x" + c[1].hex(), "x" + c[2].encode().hex()], capture_output=True, text=True)
            if one.returncode != 0:
                print(f"harness crashed on {c[0]} a={c[1][:300]!r} b={c[2][:200]!r}")
                break
        print(ran.stderr[-2000:]); sys.exit(1)
    lines.extend(ran.stdout.split("\n")[:-1])
    batch, size = [], 0


for c in cases:
    batch.append(c)
    size += 2 * (len(c[1]) + len(c[2])) + 30
    if size > 300000 or len(batch) >= 500:
        flush()
flush()


def show(op, s):
    if op in ("mod", "cover") and s not in ("ERR", "<missing>"):
        try:
            text = bytes.fromhex(s).decode()
            if "\x1f" in text:
                summ, fmt = text.split("\x1f")
                return summ.replace("\x1e", " | ") + " || " + repr(bytes.fromhex(fmt).decode())
            return repr(text)
        except Exception:
            return s
    return s


families, agree, differ = {}, 0, 0
only = os.environ.get("ONLY", "")
for i, c in enumerate(cases):
    op, a, b, want, _, _ = c
    mine = lines[i] if i < len(lines) else "<missing>"
    fam = families.setdefault(op, [0, 0])
    if mine == want:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} a={a[:300]!r} b={b[:100]!r}")
            print(f"    want  {show(op, str(want))[:500]}")
            print(f"    grasm {show(op, mine)[:500]}")
# Each family must see both sides: answers and refusals (or true and
# false, or verified and unverified), or it is only testing one path.
kinds = {}
for c in cases:
    w = str(c[3])
    k = "ERR" if w == "ERR" else w.split("|")[0] if c[0] == "note.open" else w if c[0] == "tlog.check" else "ok"
    kinds.setdefault(c[0], {}).setdefault(k, 0)
    kinds[c[0]][k] += 1
if os.environ.get("MIX"):
    print(kinds)
for fam, need in [("mod", {"ok", "ERR"}), ("note.open", {"OK", "UNVERIFIED", "ERR"}), ("tlog.check", {"true", "false"}),
                  ("tree", {"ok", "ERR"}), ("cover", {"ok", "ERR"}), ("note.keygen", {"ok", "ERR"})]:
    missing = need - {k for k, n in kinds.get(fam, {}).items() if n >= 5}
    if missing:
        print(f"{fam} never saw {sorted(missing)} (5 or more)"); sys.exit(1)
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 12]
if thin or len(families) < 9:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
