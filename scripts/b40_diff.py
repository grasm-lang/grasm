"""Other languages' file formats, each against the language that writes it.

- JSON Lines and JSON text sequences against Python's `json`, line by line.
- JAR manifests against java.util.jar.Manifest (Java 17), read and written.
- .pyc files written by `py_compile` in Pythons 3.9, 3.13 and 3.14, in all
  three invalidation modes, against what each Python's `marshal` reads; and
  `source_hash` against importlib.
- TAP against Perl's TAP::Parser: tokens, results and parse errors.
- V8 serialization against Node's `v8.serialize` and `v8.deserialize`.
- Storable against Perl's `nfreeze`, `freeze`, `nstore` and `store`.
- Java serialization streams against ObjectOutputStream and what
  ObjectInputStream reads back.
- Class files of the JDK's own modules against `javap -v -c -p`.
- YAML against Ruby's Psych (libyaml 0.2.5): parse events for random
  documents and every error; resolved values; and YAML written here, read
  back by Psych.
"""
import io
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIB = os.path.join(ROOT, "scripts/lib/b40")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b40")
harness = os.path.join(work, "b40test")
built = subprocess.run([GRASM, "build", "packages/b40test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

cases = []   # (op, path, check, family); check(answer) -> (ok, want, got)
counter = [0]


def path_for(ext):
    counter[0] += 1
    return os.path.join(work, f"f{counter[0]}.{ext}")


def write(data, ext="bin"):
    p = path_for(ext)
    with open(p, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode())
    return p


def run(cmd, data=None, **kw):
    r = subprocess.run(cmd, input=data, capture_output=True, **kw)
    if r.returncode != 0:
        raise RuntimeError(" ".join(cmd[:6]) + "\n" + r.stderr.decode(errors="replace")[-800:])
    return r.stdout.decode()


def equal(want):
    return lambda got: (got == want, want, got)


# --- JSON Lines and JSON text sequences --------------------------------------------

def json_value(d=0):
    k = rng.randrange(8 if d < 3 else 5)
    if k == 0:
        return None
    if k == 1:
        return rng.random() < 0.5
    if k == 2:
        return rng.randrange(-2 ** 52, 2 ** 52)
    if k == 3:
        return rng.choice([0.5, -1.25, 1e300, 3.141592653589793, 2.5e-308])
    if k == 4:
        return "".join(rng.choice("ab\"\\\n\té☺😀\u0001") for _ in range(rng.randrange(6)))
    if k in (5, 6):
        return [json_value(d + 1) for _ in range(rng.randrange(4))]
    return {"k" + str(i): json_value(d + 1) for i in range(rng.randrange(4))}


def json_from_render(hexes):
    return [json.loads(bytes.fromhex(h).decode()) for h in hexes.split(";") if h]


for i in range(30):
    values = [json_value() for _ in range(rng.randrange(1, 8))]
    lines = [json.dumps(v, ensure_ascii=rng.random() < 0.5, separators=rng.choice([(",", ":"), (", ", ": ")])) for v in values]
    nl = "\r\n" if i % 5 == 1 else "\n"
    if i % 6 == 2:
        lines.insert(rng.randrange(len(lines) + 1), "")
    if i % 6 == 3:
        lines.insert(rng.randrange(len(lines) + 1), rng.choice(["{", "[1,", "nul", "1 2", "'x'"]))
    text = nl.join(lines) + (nl if i % 4 else "")
    # Python's reading: each line one value, the empty piece after a last
    # line ending not a line.
    pieces = text.split("\n")
    if pieces and pieces[-1] == "":
        pieces = pieces[:-1]
    want, bad = [], None
    for n, line in enumerate(pieces):
        try:
            want.append(json.loads(line))
        except ValueError:
            bad = n + 1
            break

    def check(got, want=want, bad=bad):
        if bad is not None:
            return got == f"BAD:{bad}", f"BAD:{bad}", got
        if got.startswith("BAD") or got == "ERR":
            return False, "values", got
        return json_from_render(got) == want, str(want)[:100], str(json_from_render(got))[:100]
    cases.append(("jsonl", write(text, "jsonl"), check, "jsonlines"))

for i in range(20):
    records, want = [], []
    if i % 4 == 0:
        records.append(b"junk before")
    for _ in range(rng.randrange(1, 6)):
        v = json_value()
        body = json.dumps(v).encode()
        kind = rng.randrange(6)
        if kind == 0:
            body = body[:max(1, len(body) // 2)]          # cut short
        elif kind == 1:
            body = b"{not json"
        text = body + (b"" if kind == 0 else b"\n")
        records.append(b"\x1e" + text)
        t = text.decode(errors="replace")
        if kind == 0 or kind == 1:
            want.append(("bad", text))
        else:
            want.append(("ok", v))
    data = b"".join(records)

    def check(got, want=want):
        if got == "ERR":
            return False, "records", got
        mine = []
        for r in got.split(";") if got else []:
            kind, h = r.split(":")
            mine.append(("ok", json.loads(bytes.fromhex(h))) if kind == "ok" else ("bad", bytes.fromhex(h)))
        return mine == want, str(want)[:120], str(mine)[:120]
    cases.append(("jsonseq", write(data, "seq"), check, "json-seq"))


# --- JAR manifests --------------------------------------------------------------------

jt = os.path.join(work, "java")
os.makedirs(jt)
run(["javac", "-d", jt, os.path.join(LIB, "B40.java")])
JAVA = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "--add-opens", "java.base/java.util=ALL-UNNAMED", "-cp", jt, "B40"]


def wrap72(line):
    """A header line as Java writes it: 72 bytes, then a space and 71."""
    b = line.encode()
    out = [b[:72]]
    b = b[72:]
    while b:
        out.append(b" " + b[:71])
        b = b[71:]
    return out


def header_name():
    if rng.random() < 0.05:
        return rng.choice(["Bad Name", "Bad.Name", "", "x" * 71])
    return rng.choice(["Main-Class", "Class-Path", "Created-By", "X_Custom-1", "Automatic-Module-Name", "Multi-Release", "SHA-256-Digest", "a"])


def header_value():
    k = rng.randrange(5)
    if k == 0:
        return ""
    if k == 1:
        return "".join(rng.choice("abc é☺😀") for _ in range(rng.randrange(1, 60)))
    if k == 2:
        return "v" * rng.randrange(60, 200)
    return " leading" if k == 3 else "com.example.Main"


def manifest_bytes(i):
    nl = [b"\r\n", b"\n", b"\r"][i % 3] if i % 7 else b"\r\n"
    lines = []
    main = []
    if i % 9 != 4:
        main.append(("Signature-Version" if i % 13 == 5 else "Manifest-Version", "1.0"))
    for _ in range(rng.randrange(0, 5)):
        main.append((header_name(), header_value()))
    for n, v in main:
        lines.extend(wrap72(f"{n}: {v}") if rng.random() < 0.8 else [f"{n}: {v}".encode()])
    for s in range(rng.randrange(0, 3)):
        lines.append(b"")
        if rng.random() < 0.2:
            lines.append(b"")
        name = rng.choice(["com/example/A.class", "META-INF/x", "é/☺.txt", "dup"])
        lines.extend(wrap72("Name: " + name))
        for _ in range(rng.randrange(0, 3)):
            lines.extend(wrap72(f"{header_name()}: {header_value()}"))
    if i % 11 == 3:
        lines.append(b"Broken:NoSpace")
    if i % 17 == 6:
        lines.append(b"Long: " + b"x" * rng.choice([504, 505, 506, 507]))
    data = nl.join(lines) + (nl if i % 5 else b"")
    return data


manifests = [write(manifest_bytes(i), "mf") for i in range(60)]
java_mf = run(JAVA + ["mf"] + manifests).splitlines()
for path, want in zip(manifests, java_mf):
    def check(got, want=want):
        if want == "ERR" or got == "ERR":
            return got == want, want, got
        wm, ws, ww = want.split("|")
        gm, gs, gw = got.split("|")
        gs = ";".join(sorted(gs.split(";"))) if gs else ""
        ws = ";".join(sorted(ws.split(";"))) if ws else ""
        if (gm, gs) != (wm, ws):
            return False, wm + "|" + ws, gm + "|" + gs
        return ww == "-" or ww == gw, "written " + ww[:80], gw[:80]
    cases.append(("mf", path, check, "jarmanifest"))


# --- .pyc ------------------------------------------------------------------------------

PYTHONS = [p for p in ("python3.14", "python3.13", "/usr/bin/python3") if shutil.which(p) or os.path.exists(p)]


def py_source():
    parts = []
    for k in range(rng.randrange(1, 6)):
        kind = rng.randrange(6)
        if kind == 0:
            parts.append(f"def f{k}(a, b=1, /, c=2, *args, d, e=3, **kw):\n    x = [i * a for i in range(b)]\n    return lambda y: y + x[0] + c\n")
        elif kind == 1:
            parts.append(f"class C{k}:\n    attr = {k}\n    def m(self, *, q):\n        def inner(z):\n            return z + self.attr + q\n        return inner\n")
        elif kind == 2:
            parts.append(f"g{k} = {{i: str(i) for i in range({k})}}\nh{k} = (n for n in g{k} if n)\n")
        elif kind == 3:
            parts.append(f"async def co{k}(x):\n    async with x as y:\n        await y\n    try:\n        pass\n    except (ValueError, KeyError) as e:\n        raise RuntimeError from e\n")
        elif kind == 4:
            parts.append(f"s{k} = 'é☺' + \"\\x00\" * {k} + str(2 ** 70) + repr(1.5j)\nt{k} = (1, 2.0, None, ..., b'x', frozenset({{1}}))\n")
        else:
            parts.append(f"import os as o{k}\nfrom sys import path as p{k}\nglobal_{k} = o{k}.sep\n")
    return "".join(parts)


pyc_files = {p: [] for p in PYTHONS}
for i in range(36):
    py = PYTHONS[i % len(PYTHONS)]
    src = write(py_source(), "py")
    mode = ["TIMESTAMP", "CHECKED_HASH", "UNCHECKED_HASH"][i % 3]
    out = src + "c"
    run([py, "-c", "import py_compile, sys; py_compile.compile(sys.argv[1], sys.argv[2], doraise=True, "
         f"invalidation_mode=py_compile.PycInvalidationMode.{mode})", src, out])
    pyc_files[py].append(out)
for py, files in pyc_files.items():
    if not files:
        continue
    for line in run([py, os.path.join(LIB, "pyc.py")] + files).splitlines():
        f, want = line.split(" ", 1)
        cases.append(("pyc", f, equal(want), "pycfile"))

for i in range(12):
    src = write(py_source(), "py")
    py = PYTHONS[i % len(PYTHONS)]
    magic, want = run([py, "-c", "import importlib.util, sys; m = importlib.util.MAGIC_NUMBER; "
                       "print(m[0] | m[1] << 8, importlib.util.source_hash(open(sys.argv[1], 'rb').read()).hex())", src]).split()
    cases.append((f"pychash={magic}", src, equal(want), "pyc-source-hash"))


# --- TAP --------------------------------------------------------------------------------

def tap_text(i):
    lines = []
    v13 = i % 3 != 0
    if v13 or i % 7 == 1:
        lines.append(rng.choice(["TAP version 13", "TAP version 13", "TAP version 12", "TAP version 14", "TAP   VERSION   13"]))
    if i % 9 == 2:
        lines.insert(0, "# a comment first")
    n = rng.randrange(0, 7)
    plan_at = rng.choice(["start", "start", "end", "none", "middle", "both"])
    plan = f"1..{n}" + rng.choice(["", "", " # SKIP no network" if n == 0 else "", " todo 2 3" if not v13 else ""])
    if plan_at in ("start", "both"):
        lines.append(plan)
    if v13 and i % 8 == 3:
        lines.append("pragma +strict")
    count = n + rng.choice([0, 0, 0, 1, -1]) if n else rng.randrange(0, 2)
    for k in range(1, max(count, 0) + 1):
        if plan_at == "middle" and k == 2:
            lines.append(plan)
        ok = rng.choice(["ok", "ok", "not ok"])
        num = "" if rng.random() < 0.2 else f" {k if rng.random() < 0.9 else k + 5}"
        desc = rng.choice(["", " - adds", " - has \\# escaped", " handles # hash", " -   spaced  ", " é☺"])
        directive = rng.choice(["", "", "", " # SKIP not here", " # TODO later", " # skip", " #todo  why  ", " # SKIPPED nope"])
        lines.append(ok + num + desc + directive)
        if v13 and rng.random() < 0.2:
            lines += ["  ---", "  message: failed", "  got: 1", "  ..."]
        if rng.random() < 0.15:
            lines.append(rng.choice(["# diag line", "unknown junk", "", "  indented", "Bail out! db down", "    Bail out!"]))
    if plan_at in ("end", "both"):
        lines.append(plan)
    return "\n".join(lines) + "\n"


taps = [write(tap_text(i), "tap") for i in range(60)]
for path, want in zip(taps, run(["perl", os.path.join(LIB, "tap.pl")] + taps).splitlines()):
    def check(got, want=want):
        g = got.split("|")
        w = want.split("|")
        # TAP::Parser keeps a bail-out as a token, not a result; compare the
        # rest field by field.
        g[3] = w[3] = "-"
        return g == w, want[:200], got[:200]
    cases.append(("tap", path, check, "tap"))


# --- V8, Storable, Java serialization ------------------------------------------------------

def generated(cmd, op, family):
    for line in run(cmd).splitlines():
        f, want = line.split(" ", 1)

        def check(got, f=f, want=want):
            if got == "ERR":
                return False, want[:100], got
            dump, again = got.rsplit("|", 1)
            if dump != want:
                return False, want[:200], dump[:200]
            orig = open(f, "rb").read().hex()
            return again == orig, "written back as read", "differs at byte " + str(next((k for k in range(min(len(again), len(orig))) if again[k] != orig[k]), 0) // 2)
        cases.append((op, f, check, family))


for sub in ("v8", "st", "js"):
    os.makedirs(os.path.join(work, sub))
generated(["node", os.path.join(LIB, "v8.js"), str(SEED % 1000003), os.path.join(work, "v8"), "60"], "v8", "v8serial")
generated(["perl", os.path.join(LIB, "storable.pl"), str(SEED % 1000003), os.path.join(work, "st"), "60"], "storable", "perlstorable")
generated(JAVA + ["jgen", str(SEED), os.path.join(work, "js"), "60"], "jser", "javaserial")


# --- class files ------------------------------------------------------------------------------

JMODS = "/Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home/jmods"
if not os.path.isdir(JMODS):
    JMODS = os.path.join(subprocess.run(["/usr/libexec/java_home"], capture_output=True, text=True).stdout.strip(), "jmods")
NEWARRAY = {"boolean": 4, "char": 5, "float": 6, "double": 7, "byte": 8, "short": 9, "int": 10, "long": 11}
class_files = []
for mod in sorted(rng.sample(sorted(os.listdir(JMODS)), 6) + ["java.base.jmod"]):
    z = zipfile.ZipFile(io.BytesIO(open(os.path.join(JMODS, mod), "rb").read()[4:]))
    names = [n for n in z.namelist() if n.startswith("classes/") and n.endswith(".class")]
    for n in rng.sample(names, min(len(names), 12)):
        class_files.append(write(z.read(n), "class"))


def javap_unescape(t):
    if "\\" in t or any(ord(c) < 32 or ord(c) > 126 for c in t):
        return None
    return t


def parse_javap(text):
    """javap -v -c -p output as the harness's dump, None where javap does
    not print a value (floats and escaped strings)."""
    lines = text.split("\n")
    head = {}
    pool = {}
    members = []
    i = 0
    while i < len(lines):
        l = lines[i]
        if l == "Constant pool:":
            i += 1
            while i < len(lines):
                m = re.match(r"\s+#(\d+) = (\w+)\s*(.*)$", lines[i])
                if not m:
                    break
                kind, rest = m.group(2), m.group(3)
                if kind != "Utf8":
                    rest = rest.split("//")[0].strip()
                pool[int(m.group(1))] = (kind, rest)
                i += 1
            continue
        m = re.match(r"\s+(minor|major) version: (\d+)", l)
        if m:
            head[m.group(1)] = int(m.group(2))
        m = re.match(r"  flags: \(0x([0-9a-f]+)\)", l)
        if m and "access" not in head:
            head["access"] = int(m.group(1), 16)
        if l == "}":
            # The class body ends here. What follows are class attributes,
            # and a record's `Record:` attribute lists its components with
            # `descriptor:` lines of their own -- not members.
            break
        m = re.match(r"    descriptor: (.*)$", l)
        if m:
            fl = re.match(r"    flags: \(0x([0-9a-f]+)\)", lines[i + 1])
            members.append({"desc": m.group(1), "access": int(fl.group(1), 16) if fl else 0, "code": None, "ex": []})
        if l.strip() == "Code:" and members:
            st = re.match(r"\s+stack=(\d+), locals=(\d+)", lines[i + 1])
            code, i = parse_code(lines, i + 2)
            members[-1]["code"] = (int(st.group(1)), int(st.group(2)), code[0], code[1])
            continue
        i += 1
    return head, pool, members


def parse_code(lines, i):
    ins, ex = [], []
    while i < len(lines):
        l = lines[i]
        m = re.match(r"\s+(\d+): (\w+)\s*(.*?)\s*(?://.*)?$", l)
        if m and not l.strip().startswith(("from", "Exception", "LineNumber", "LocalVariable", "StackMap", "RuntimeVisible")):
            off, mn, rest = int(m.group(1)), m.group(2), m.group(3)
            if mn in ("tableswitch", "lookupswitch"):
                low_high = re.search(r"//\s*(-?\d+) to (-?\d+)", l)
                pairs, i = [], i + 1
                default = None
                while not lines[i].strip().startswith("}"):
                    k, t = lines[i].strip().split(":")
                    if k.strip() == "default":
                        default = int(t)
                    else:
                        pairs.append((int(k), int(t)))
                    i += 1
                if mn == "tableswitch":
                    ops = [default, int(low_high.group(1)), int(low_high.group(2))] + [t for _, t in pairs]
                else:
                    ops = [default, len(pairs)] + [x for p in pairs for x in p]
                ins.append((off, mn, ops))
                i += 1
                continue
            if mn == "newarray":
                ops = [NEWARRAY[rest.split()[0]]]
            else:
                ops = [int(x) for x in re.findall(r"-?\d+", rest.replace("#", " "))]
            ins.append((off, mn, ops))
            i += 1
            continue
        if l.strip().startswith("Exception table:"):
            i += 2
            while i < len(lines) and re.match(r"\s+\d+\s+\d+\s+\d+", lines[i]):
                f = lines[i].split()
                ex.append((int(f[0]), int(f[1]), int(f[2]), " ".join(f[3:])))
                i += 1
            continue
        if l.strip() and not l.startswith("        ") and not re.match(r"\s+\d+:", l):
            break
        i += 1
    return (ins, ex), i


def check_class(path, javap):
    head, pool, members = parse_javap(javap)

    def check(got):
        if got == "ERR":
            return False, "a class", got
        rows = bytes.fromhex(got).decode().split("\n")
        v, access = rows[0].split(":")[0], int(rows[0].split(":")[1])
        if v != f"{head['major']}.{head['minor']}" or access != head["access"]:
            return False, f"{head}", rows[0]
        mine_pool, mine_members = {}, []
        for r in rows[1:]:
            if r.startswith("#"):
                n, kind = r[1:].split("=", 1)
                mine_pool[int(n)] = kind
            else:
                mine_members.append(r)
        if sorted(mine_pool) != sorted(pool):
            return False, f"{len(pool)} constants", f"{len(mine_pool)}"
        for n, (kind, refs) in pool.items():
            mk = mine_pool[n]
            if not mk.startswith(kind):
                return False, f"#{n} {kind}", mk
            body = mk[len(kind) + 1:] if ":" in mk else ""
            if kind == "Utf8":
                # javap does not print a constant's leading or trailing spaces.
                t = javap_unescape(refs)
                if t is not None and bytes.fromhex(body).decode().strip() != t.strip():
                    return False, f"#{n} {refs}", bytes.fromhex(body).decode()
            elif kind in ("Integer", "Long"):
                if body != refs.rstrip("l"):
                    return False, f"#{n} {refs}", body
            elif kind not in ("Float", "Double"):
                if body.replace(":#", ":").replace("#", "") != refs.replace("#", "").replace(".", ".").replace(":", ":"):
                    if body.replace("#", "") != refs.replace("#", ""):
                        return False, f"#{n} {kind} {refs}", body
        if len(mine_members) != len(members):
            return False, f"{len(members)} members", f"{len(mine_members)}"
        for jm, mm in zip(members, mine_members):
            parts = mm.split("|")
            kind, name, desc, access = parts[0].split(":")
            if bytes.fromhex(desc).decode() != jm["desc"] or int(access) != jm["access"]:
                return False, f"{jm['desc']} {jm['access']}", f"{bytes.fromhex(desc).decode()} {access}"
            if jm["code"] is None:
                if len(parts) > 1:
                    return False, "no code", "code"
                continue
            stack, local, ins, ex = jm["code"]
            if parts[1] != f"C:{stack}:{local}":
                return False, f"stack {stack} locals {local}", parts[1]
            mine = []
            for x in parts[2].split(";"):
                off, mn, ops = x.split(":")
                ops = [int(o) for o in ops.split(",")] if ops else []
                if mn in ("invokeinterface", "invokedynamic"):
                    ops = ops[:2]
                mine.append((int(off), mn, ops))
            if mine != ins:
                k = next((k for k in range(min(len(mine), len(ins))) if mine[k] != ins[k]), 0)
                return False, f"{jm['desc']} #{k} {ins[k:k + 1]}", str(mine[k:k + 1])
            mex = [tuple(int(y) for y in e.split(":")) for e in parts[3].split(";") if e]
            if [(a, b, c) for a, b, c, _ in mex] != [(a, b, c) for a, b, c, _ in ex]:
                return False, str(ex), str(mex)
        return True, "", ""
    return check


jp = run(["javap", "-v", "-c", "-p"] + class_files)
chunks = re.split(r"(?m)^Classfile ", jp)[1:]
for path, chunk in zip(class_files, chunks):
    cases.append(("class", path, check_class(path, chunk), "classfile"))


# --- YAML ---------------------------------------------------------------------------------------

WORDS = ["alpha", "beta gamma", "x", "key", "k-2", "a.b", "über", "☺ ok", "path/to", "CamelCase"]
TYPED = ["~", "null", "Null", "NULL", "", "yes", "No", "TRUE", "off", "On", "0", "-17", "+3", "0x1F", "017", "0b101", "1_000",
         "190:20:30", "1.5", "-.5", "1.", "1.0e+3", "6.8523015e+5", ".inf", "-.Inf", ".NaN", "2001-12-14",
         "2001-12-14t21:59:43.10-05:00", "2001-12-14 21:59:43.10 -05:00", "<<", "1e3", "0o17", "08", "y", "n", "12e03", "0.1e3"]


def plain_ok(s):
    return bool(re.fullmatch(r"[A-Za-zü☺][A-Za-z0-9 _./☺ü-]*[A-Za-z0-9ü☺]|[A-Za-z]", s)) and ": " not in s and " #" not in s


def scalar_text(s, ctx_flow, indent):
    styles = ["plain", "single", "double", "literal", "folded"]
    style = rng.choice(styles)
    if style == "plain" and plain_ok(s) and not (ctx_flow and any(c in s for c in ",[]{}")):
        return s
    if style in ("literal", "folded") and not ctx_flow and "\n" in s and all(ord(c) >= 32 or c == "\n" for c in s) and not s.startswith(" "):
        ind = " " * (indent + 2)
        chomp = "" if s.endswith("\n") and not s.endswith("\n\n") else ("-" if not s.endswith("\n") else "+")
        body = s[:-1] if s.endswith("\n") else s
        if chomp == "+":
            body = s[:-1]
        mark = "|" if style == "literal" else ">"
        if style == "folded" and re.search(r"\n[^\n ]", body):
            return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'
        return mark + chomp + "".join("\n" + (ind + l if l else "") for l in body.split("\n"))
    if style == "single" and all(ord(c) >= 32 for c in s) and "\n" not in s:
        return "'" + s.replace("'", "''") + "'"
    out = []
    for c in s:
        o = ord(c)
        out.append({"\\": "\\\\", '"': '\\"', "\n": "\\n", "\t": "\\t"}.get(c) or (f"\\x{o:02x}" if o < 32 else c))
    return '"' + "".join(out) + '"'


def yaml_node(d, indent, flow, anchors):
    k = rng.randrange(10 if d < 4 else 6)
    prefix = ""
    if rng.random() < 0.15:
        name = f"a{len(anchors)}"
        anchors.append(name)
        prefix = f"&{name} "
    elif anchors and rng.random() < 0.08:
        return "*" + rng.choice(anchors)
    if rng.random() < 0.08:
        tag = rng.choice(["!!str ", "!local ", "!e!thing ", "!<tag:x.com,2000:t> "])
        prefix += tag if k < 5 or not tag.startswith("!!") else ""
    if k < 3:
        return prefix + scalar_text(rng.choice(WORDS + ["two\nlines", "ends\n", "it's", "a: b", "#hash", " lead", "tab\there", "x" * 90]), flow, indent)
    if k < 5:
        t = rng.choice(TYPED)
        return prefix + (t if t else '""')
    if k == 5 or flow:
        if rng.random() < 0.5:
            return prefix + "[" + ", ".join(yaml_node(d + 1, indent, True, anchors) for _ in range(rng.randrange(4))) + "]"
        return prefix + "{" + ", ".join(rng.choice(WORDS).replace(" ", "_") + ": " + yaml_node(d + 1, indent, True, anchors) for _ in range(rng.randrange(4))) + "}"
    pad = " " * (indent + 2)
    if k in (6, 7):
        items = [yaml_node(d + 1, indent + 2, False, anchors) for _ in range(rng.randrange(1, 4))]
        return (prefix.rstrip() + "\n" if prefix else "\n") + "\n".join(pad + "- " + it for it in items)
    keys = rng.sample([w.replace(" ", "_") for w in WORDS], rng.randrange(1, 4))
    if rng.random() < 0.1:
        keys = ["<<"] + keys
    lines = []
    for key in keys:
        v = yaml_node(d + 1, indent + 2, False, anchors)
        if key == "<<":
            v = rng.choice(["{merged: 1, key: 2}", "[{a1: 1}, {a2: 2}]"])
        lines.append(pad + key + ":" + ("" if v.startswith("\n") else " ") + v)
    if rng.random() < 0.1:
        lines.append(pad + "? complex key\n" + pad + ": value")
    if rng.random() < 0.1:
        lines.append(pad + "# a comment")
    return (prefix.rstrip() + "\n" if prefix else "\n") + "\n".join(lines)


def yaml_text(i):
    docs = []
    for n in range(rng.randrange(1, 4)):
        head = ""
        if rng.random() < 0.2:
            head += "%YAML 1.1\n"
        if rng.random() < 0.3:
            head += "%TAG !e! tag:example.com,2000:\n"
        body = yaml_node(0, -2, False, [])
        explicit = head or n > 0 or rng.random() < 0.5
        doc = (head + "---" if explicit else "") + ("" if body.startswith("\n") else " ") + body if explicit else body.lstrip("\n")
        if "!e!" in doc and "%TAG" not in head:
            doc = doc.replace("!e!", "!")
        if rng.random() < 0.2:
            doc += "\n..."
        docs.append(doc)
    text = "\n".join(docs) + "\n"
    if i % 6 == 5:
        # Damage it: the readers must agree on whether it is still YAML.
        k = rng.randrange(len(text))
        text = text[:k] + rng.choice(["", ":", "- ", "\t", "[", "'", "&", "  "]) + text[k + 1:]
    return text


CURATED = [
    "a: |2\n   leading\n  text\n", "a: |-\n  strip\n\n", "a: |+\n  keep\n\n\nb: 1\n", "a: >\n  folded\n  lines\n\n  para\n    more indented\n  back\n",
    "a: >-\n\n  starts blank\n", "- |\n  in seq\n- >+\n  kept\n\n", "plain: multi\n  line\n  scalar\n", "q: 'single\n  quoted\n\n  para'\n",
    "q: \"double \\\n  joined \\t tab\\u263A\\x41\"\n", "- - a\n  - b\n- - c\n", "? - a\n  - b\n: - c\n", "? |\n  block key\n: v\n",
    "[a, {b: c}, ? d : e, f: g]\n", "{a: [1, 2], b: {c: d}, ? e}\n", "a: &x\nb: *x\n", "&a [*a, &b {k: *b}]\n", "- !!null\n- !!str\n- ! x\n",
    "%TAG ! tag:example.com,2000:\n--- !foo bar\n", "%YAML 1.1\n%TAG !e! tag:e.com:\n--- !e!x [1]\n...\n--- second\n", "\ufeffa: bom\n",
    "a: 1\r\nb:\r\n  - 2\r\n", "# only a comment\n", "", "---\n...\n---\n", "a:\n  b:\n    c:\n      d: deep\n", "a: \"tab\tinside\"\n",
    "k: v # comment\n# full\nk2: v2\n", "- a\n-\n- c\n", "a:\n- b\n- c\nd: e\n", "? a\n? b\n: c\n", "a: [\n  1,\n  2,\n]\n",
    "a: {x: 1,\n  y: 2}\n", "'quoted key': 1\n\"dq key\": 2\n", "a: 'it''s'\n", "x: \"\\N\\_\\L\\P\\e\\0\"\n", "a: b: c\n", "- a\n b\n",
    "a:\n\t- b\n", "key: [a: b]\n", "a: |0\n  x\n", "- &a\n- *a\n", "--- >\n folded at root\n", "[a, b]: c\n", "? [a, b]\n: c\n",
]
yaml_files = [write(t, "yaml") for t in CURATED]
STRANGE = ["", " ", "a b", "\t", "x\ty", "line\nbreak", "trail\n", "two\n\n", "\n", "\nlead", " lead", "trail ", "#", "a #b", "a: b", "-",
           "- x", "?", "? x", ":x", "x:", "@at", "`tick", "%pct", "!bang", "&amp", "*star", "|pipe", ">gt", "'q", '"dq', "{", "}", "[", "]",
           ",", "yes", "No", "NULL", "~", "0", "-1", "0x10", "0o7", "1e3", "1.5", ".5", "1_000", "1:30", "2001-12-14", "y", "n", "on",
           "\u00e9", "\u263a", "\U0001F600", "\x00", "\x07", "\x1b", "\x7f", "\x85", "\u2028", "\ufeff", "\r", "a\rb", "\\", "--- doc",
           "... end", "<<", "=", "very " * 30]


def dq(t):
    out = []
    for c in t:
        o = ord(c)
        out.append({"\\": "\\\\", '"': '\\"', "\n": "\\n", "\t": "\\t", "\r": "\\r"}.get(c)
                   or (f"\\x{o:02x}" if o < 32 or o == 127 else f"\\u{o:04x}" if 128 <= o < 160 or o in (0x2028, 0x2029, 0xfeff) else c))
    return '"' + "".join(out) + '"'


for i in range(40):
    picks = rng.sample(STRANGE, 6)
    body = "- " + "\n- ".join(dq(t) for t in picks[:3]) + "\n- {" + ", ".join(dq(a) + ": " + dq(b) for a, b in zip(picks[3:5], picks[4:6])) + "}\n"
    yaml_files.append(write(body, "yaml"))
yaml_files += [write(yaml_text(i), "yaml") for i in range(160)]
ruby_ev = run(["ruby", os.path.join(LIB, "yaml.rb"), "ev"] + yaml_files).splitlines()
ruby_load = run(["ruby", os.path.join(LIB, "yaml.rb"), "load"] + yaml_files).splitlines()
# Where Psych's values are its own and not YAML 1.1's (Ruby symbols, an
# untagged reading of unknown tags, "!" ignored), values are not compared.
# An unknown tag in YAML's own namespace counts: a damaged `!!str` becomes
# `!!st`, which Psych reads as if untagged and YAML 1.1 as a string.
YAML11_TYPES = "str|int|float|bool|null|binary|timestamp|seq|map|set|omap|pairs|merge|value|yaml"
PSYCH_OWN = re.compile(r"(^|[\s\[{,:-])!(?!!)|:\s*:|!!(?!(?:" + YAML11_TYPES + r")(?![\w-]))")
for path, ev, ld in zip(yaml_files, ruby_ev, ruby_load):
    cases.append(("yev", path, (lambda got, ev=ev: (got == ev, bytes.fromhex(ev).decode(errors="replace")[:300] if ev != "ERR" else ev,
                                                      bytes.fromhex(got).decode(errors="replace")[:300] if got not in ("ERR", "NOFILE") else got)), "yaml-events"))
    text = open(path, encoding="utf-8").read()
    if ev != "ERR" and not PSYCH_OWN.search(text):
        cases.append(("yload", path, (lambda got, ld=ld: (got == ld, bytes.fromhex(ld).decode()[:300] if ld != "ERR" else ld,
                                                          bytes.fromhex(got).decode()[:300] if got not in ("ERR", "NOFILE") else got)), "yaml-values"))
        cases.append(("yemit", path, (lambda got, ld=ld: check_emit(got, ld)), "yaml-emit"))


def check_emit(got, ld):
    if got in ("ERR", "NOFILE"):
        return False, "YAML", got
    text = bytes.fromhex(got).decode()
    f = write(text, "yaml")
    back = run(["ruby", os.path.join(LIB, "yaml.rb"), "load", f]).strip()
    return back == ld, bytes.fromhex(ld).decode()[:200] if ld != "ERR" else ld, (bytes.fromhex(back).decode()[:200] if back != "ERR" else "ERR") + " from " + text[:200]


# --- run -------------------------------------------------------------------------------------------

def finish():
    answers = []
    for i in range(0, len(cases), 100):
        argv = []
        for op, path, _, _ in cases[i:i + 100]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            for op, path, _, _ in cases[i:i + 100]:
                one = subprocess.run([harness, op, path], capture_output=True, text=True)
                if one.returncode != 0:
                    print(f"harness crashed on {op} {path}")
                    break
            print(ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])
    families, differ = {}, 0
    only = os.environ.get("ONLY", "")
    for (op, path, check, fam), got in zip(cases, answers):
        try:
            ok, want, mine = check(got)
        except Exception as e:  # an answer the oracle cannot read is a disagreement
            ok, want, mine = False, "an answer the reference can read", f"{type(e).__name__}: {str(e)[:200]}"
        f = families.setdefault(fam, [0, 0])
        if ok:
            f[0] += 1
        else:
            f[1] += 1
            differ += 1
            if fam.startswith(only) and (differ <= 8 or (only and f[1] <= 8)):
                print(f"  {fam} {op[:40]} {path}")
                print(f"    want  {str(want)[:300]}")
                print(f"    grasm {str(mine)[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad:
        print("disagreeing:", " ".join(bad), f"(SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    thin = [k for k, (a, d) in families.items() if a + d < 10]
    if thin or len(families) < int(os.environ.get("FAMILIES", "13")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
