"""Archives and git's inner formats, each against the tool that writes it.

- ZipCrypto: archives `zip -P` writes opened here, and archives written
  here opened by Python's zipfile and `unzip -P`.
- shar: archives macOS `shar` writes read here as `sh` unpacks them, and
  archives written here unpacked by `sh`.
- .deb: packages assembled with `ar` and Python's tarfile (gzip, xz,
  bzip2, zstd and plain), read here; packages written here taken apart by
  `ar` and `tar`; relationship fields against a reference reading.
- xar: archives `xar` writes listed and extracted here; archives written
  here listed and extracted by `xar`.
- BOM: files `mkbom` writes against `lsbom`.
- reftable, bitmaps, notes, ignore rules and status: random repositories
  against git 2.48 (`for-each-ref`, `reflog`, `rev-list --objects`,
  `cat-file`, `notes list`, `notes merge`, `ls-files -o -i`, `status
  --porcelain=v2`).
"""
import gzip as gz
import bz2
import hashlib
import io
import lzma
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b41")
harness = os.path.join(work, "b41test")
built = subprocess.run([GRASM, "build", "packages/b41test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
cases = []
counter = [0]
GIT = ["git", "-c", "user.name=Tester", "-c", "user.email=t@example.com", "-c", "init.defaultBranch=main", "-c", "core.autocrlf=false",
       "-c", "gc.auto=0", "-c", "core.excludesFile=/dev/null"]


def path_for(ext):
    counter[0] += 1
    return os.path.join(work, f"f{counter[0]}.{ext}")


def write(data, ext="bin"):
    p = path_for(ext)
    with open(p, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode())
    return p


def run(cmd, data=None, cwd=None, ok=(0,)):
    r = subprocess.run(cmd, input=data, capture_output=True, cwd=cwd)
    if r.returncode not in ok:
        raise RuntimeError(" ".join(map(str, cmd[:8])) + "\n" + r.stderr.decode(errors="replace")[-600:])
    return r.stdout


def equal(want):
    return lambda got: (got == want, str(want)[:200], str(got)[:200])


def blob(n=None):
    n = rng.choice([0, 1, 5, 40, 300, 5000]) if n is None else n
    kind = rng.randrange(3)
    if kind == 0:
        return bytes(rng.randrange(256) for _ in range(n))
    if kind == 1:
        return ("".join(rng.choice("abc \n\tXé'\"\\$") for _ in range(n))).encode()
    return (b"line %d\n" * (n // 8 + 1)) % tuple(range(n // 8 + 1))


def name():
    return rng.choice(["a", "b.txt", "dir1", "x y", "q'uote", "é", "deep", "README", "data.bin", "-dash", "Xlead"])


def tree(depth=0):
    """A random file tree: {name: bytes | dict}."""
    t = {}
    for _ in range(rng.randrange(1, 5)):
        n = name()
        if depth < 2 and rng.random() < 0.3:
            t[n] = tree(depth + 1)
        else:
            t[n] = blob()
    return t


def materialise(t, root):
    os.makedirs(root, exist_ok=True)
    for n, v in t.items():
        p = os.path.join(root, n)
        if isinstance(v, dict):
            materialise(v, p)
        else:
            with open(p, "wb") as f:
                f.write(v)


def snapshot(root):
    """{relative path: bytes | None for a directory} of everything under root."""
    out = {}
    for d, dirs, files in os.walk(root):
        for x in dirs:
            out[os.path.relpath(os.path.join(d, x), root)] = None
        for x in files:
            p = os.path.join(d, x)
            out[os.path.relpath(p, root)] = os.readlink(p).encode() if os.path.islink(p) else open(p, "rb").read()
    return out


# --- ZipCrypto -------------------------------------------------------------------------------

for i in range(20):
    src = os.path.join(work, f"z{i}")
    files = {f"f{k}{rng.choice(['.txt', '.bin', ''])}": blob() for k in range(rng.randrange(1, 5))}
    materialise(files, src)
    pw = rng.choice(["secret", "p@ss wörd", "x", "0123456789abcdef"])
    archive = os.path.join(work, f"z{i}.zip")
    run(["zip", "-q", "-P", pw, "-" + rng.choice(["0", "6", "9"]), archive] + sorted(files), cwd=src)
    for n, data in files.items():
        good = rng.random() < 0.8
        tried = pw if good else pw + "!"
        z = zipfile.ZipFile(archive)
        try:
            want = z.read(n, pwd=tried.encode()).hex()
        except RuntimeError:
            want = "WRONG"
        except zipfile.BadZipFile:
            want = "ERR"
        cases.append((f"zcopen:{tried.encode().hex()}:{n.encode().hex()}", archive, equal(want), "zipcrypto-read"))

for i in range(15):
    files = [(f"n{k}" + rng.choice(["", ".txt", "/sub.bin"]), blob()) for k in range(rng.randrange(1, 5))]
    pw = rng.choice(["pw", "longer password", "ünï"]).encode()
    salt = bytes(rng.randrange(256) for _ in range(rng.randrange(1, 40)))
    spec = write("\n".join(n.encode().hex() + " " + d.hex() for n, d in files) + "\n", "spec")

    def check(got, files=files, pw=pw):
        if got == "ERR":
            return False, "an archive", got
        z = write(bytes.fromhex(got), "zip")
        zf = zipfile.ZipFile(z)
        for n, d in files:
            if zf.read(n, pwd=pw) != d:
                return False, f"{n} read by zipfile", "differs"
            if run(["unzip", "-p", "-P", pw.decode(), z, n]) != d:
                return False, f"{n} read by unzip", "differs"
        return True, "", ""
    cases.append((f"zcenc:{pw.hex()}:{salt.hex()}", spec, check, "zipcrypto-write"))


# --- shar ------------------------------------------------------------------------------------

def shar_check(want):
    def check(got):
        if got == "ERR":
            return want == "ERR", str(want)[:100], got
        mine = {}
        for item in got.split(";") if got else []:
            kind, rest = item.split(":", 1)
            if kind == "D":
                mine[bytes.fromhex(rest).decode()] = None
            else:
                p, d = rest.split(":")
                mine[bytes.fromhex(p).decode()] = bytes.fromhex(d)
        return mine == want, str(sorted(want.items()))[:200], str(sorted(mine.items()))[:200]
    return check


def sh_unpack(archive_bytes):
    out = os.path.join(work, f"sh{counter[0]}")
    counter[0] += 1
    os.makedirs(out)
    a = write(archive_bytes, "shar")
    subprocess.run(["sh", a], cwd=out, capture_output=True)
    return snapshot(out)


for i in range(20):
    src = os.path.join(work, f"s{i}")
    # macOS shar is a shell script: it takes neither names that start with
    # "-" nor bytes outside ASCII, and writes names unquoted.
    def ascii_tree(t):
        out = {}
        for k, v in t.items():
            k = re.sub(r"[^A-Za-z0-9._]", "_", k).lstrip("-") or "n"
            out[k] = ascii_tree(v) if isinstance(v, dict) else bytes(c for c in v if 32 <= c < 127 or c == 10)
        return out
    t = ascii_tree(tree())
    materialise(t, src)
    entries = sorted(snapshot(src))
    archive = run(["shar"] + entries, cwd=src)
    cases.append(("shar", write(archive, "shar"), shar_check(sh_unpack(archive)), "shar-read"))

for i in range(15):
    t = tree()
    items = []

    def flat(t, prefix):
        for k, v in t.items():
            p = prefix + k
            if isinstance(v, dict):
                items.append(("D", p))
                flat(v, p + "/")
            else:
                items.append(("F", p, v))
    flat(t, "")
    spec = write("\n".join(" ".join([it[0], it[1].encode().hex()] + ([it[2].hex()] if it[0] == "F" else [])) for it in items) + "\n", "spec")
    want = {it[1]: (it[2] if it[0] == "F" else None) for it in items}

    def check(got, want=want):
        if got == "ERR":
            return False, "an archive", got
        back = sh_unpack(bytes.fromhex(got))
        if back != want:
            return False, str(sorted(want))[:200], str(sorted(back))[:200]
        # The same archive (printf lines, octal escapes, names macOS shar
        # will not take) read here as sh unpacked it.
        mine = subprocess.run([harness, "shar", write(bytes.fromhex(got), "shar")], capture_output=True, text=True).stdout.strip()
        return shar_check(back)(mine)
    cases.append(("sharw", spec, check, "shar-write"))


# --- .deb --------------------------------------------------------------------------------------

def tar_bytes(members, compression):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as tf:
        for n, d in members:
            info = tarfile.TarInfo(n)
            info.size = len(d)
            info.mtime = 1700000000
            tf.addfile(info, io.BytesIO(d))
    raw = buf.getvalue()
    if compression == "gz":
        return gz.compress(raw, mtime=0)
    if compression == "xz":
        return lzma.compress(raw)
    if compression == "bz2":
        return bz2.compress(raw)
    if compression == "zst":
        return run(["zstd", "-q", "-c"], data=raw)
    return raw


def control_text(fields):
    lines = []
    for k, v in fields:
        parts = v.split("\n")
        lines.append(f"{k}: {parts[0]}")
        lines += [" " + (p if p else ".") for p in parts[1:]]
    return "\n".join(lines) + "\n"


def deb_fields():
    f = [("Package", rng.choice(["hello", "libfoo2", "x-y.z"])), ("Version", rng.choice(["1.0-1", "2:3.4~rc1-0ubuntu1", "0.9+git20240101"])),
         ("Architecture", rng.choice(["amd64", "all", "arm64"])), ("Maintainer", "Ana Ö <ana@example.com>")]
    if rng.random() < 0.7:
        f.append(("Depends", rng.choice(["libc6 (>= 2.34)", "a | b (<< 2), c:any", "python3 (>= 3.9~) [amd64 arm64], foo"])))
    f.append(("Description", rng.choice(["short", "summary\nlong line one\n\nafter a blank", "x\n  indented more"])))
    return f


for i in range(20):
    fields = deb_fields()
    files = [(f"usr/share/doc/p/f{k}", blob()) for k in range(rng.randrange(0, 4))]
    md5s = "".join(f"{hashlib.md5(d).hexdigest()}  {n}\n" for n, d in files)
    if i % 7 == 3 and files:
        md5s = md5s.replace(md5s[:4], "0000", 1)
    comp_c, comp_d = rng.choice(["gz", "xz", "zst"]), rng.choice(["gz", "xz", "bz2", "zst", ""])
    d = os.path.join(work, f"deb{i}")
    os.makedirs(d)
    names = ["debian-binary", f"control.tar.{comp_c}", "data.tar" + (f".{comp_d}" if comp_d else "")]
    open(os.path.join(d, names[0]), "w").write("2.0\n")
    open(os.path.join(d, names[1]), "wb").write(tar_bytes([("./control", control_text(fields).encode()), ("./md5sums", md5s.encode())], comp_c))
    open(os.path.join(d, names[2]), "wb").write(tar_bytes([("./" + n, b) for n, b in files], comp_d))
    out = os.path.join(work, f"p{i}.deb")
    run(["ar", "rc", out] + names, cwd=d)
    bad = [n for n, b in files if f"{hashlib.md5(b).hexdigest()}  {n}" not in md5s]
    want = "2.0|" + ",".join(k.encode().hex() + "=" + v.encode().hex() for k, v in fields) + "|" + ",".join(
        x.encode().hex() for x in ["./control", "./md5sums"]) + "|" + ",".join(("./" + n).encode().hex() + ":" + hashlib.md5(b).hexdigest() for n, b in files) + "|" + \
        ",".join(n.encode().hex() for n in bad)
    cases.append(("deb", out, equal(want), "deb-read"))

for i in range(12):
    fields = deb_fields()
    files = [(f"opt/x/f{k}", blob()) for k in range(rng.randrange(0, 4))]
    spec = write("\n".join(["F " + k.encode().hex() + " " + v.encode().hex() for k, v in fields] +
                           ["P " + n.encode().hex() + " " + b.hex() for n, b in files]) + "\n", "spec")

    def check(got, fields=fields, files=files):
        if got == "ERR":
            return False, "a package", got
        deb = write(bytes.fromhex(got), "deb")
        members = run(["ar", "t", deb]).decode().split()
        if members != ["debian-binary", "control.tar.gz", "data.tar.gz"]:
            return False, "debian-binary control.tar.gz data.tar.gz", " ".join(members)
        control = run(["tar", "-xzOf", "-", "./control"], data=run(["ar", "p", deb, "control.tar.gz"])).decode()
        if control != control_text(fields):
            return False, control_text(fields)[:120], control[:120]
        data = run(["ar", "p", deb, "data.tar.gz"])
        for n, b in files:
            if run(["tar", "-xzOf", "-", "./" + n], data=data) != b:
                return False, n, "differs"
        return True, "", ""
    cases.append(("debbuild", spec, check, "deb-build"))


def ref_depends(value):
    """A reference reading of a relationship field."""
    groups = []
    for g in value.replace("\n", " ").split(","):
        if not g.strip():
            continue
        alts = []
        for a in g.split("|"):
            m = re.fullmatch(r"\s*([^\s:(\[]+)(?::(\S+?))?\s*(?:\(\s*(>>|<<|>=|<=|=(?!=)|>(?![>=])|<(?![<=]))\s*([^)\s=<>][^)\s]*)\s*\))?\s*(?:\[([^\]]*)\])?\s*(?:<[^>]*>\s*)*", a)
            if not m:
                return "ERR"
            alts.append("/".join([m.group(1), m.group(2) or "", m.group(3) or "", m.group(4) or "", "+".join((m.group(5) or "").split())]))
        groups.append("|".join(alts))
    return ",".join(groups)


DEPS = ["libc6 (>= 2.34)", "a | b", "foo:any (= 1.0), bar [amd64 !i386]", "x (<< 2:1.0~rc1)", "pkg (>=1)", "p (>> 1) | q (<= 2) [arm64]",
        "", "a,,b", "broken (>= )", "bad (~ 1)", "unclosed (>= 1", "py3 <!nocheck>", "lib\n  , other"]
for i in range(30):
    v = rng.choice(DEPS) if i < 20 else ", ".join(rng.sample(DEPS[:6], 2))
    cases.append(("debdeps", write(v, "txt"), equal(ref_depends(v)), "deb-depends"))


# --- xar ----------------------------------------------------------------------------------------

def xar_rows(archive):
    """Each entry as the harness prints it, from `xar -tv` and `xar -x`."""
    out = os.path.join(work, f"xx{counter[0]}")
    counter[0] += 1
    os.makedirs(out)
    run(["xar", "-xf", archive, "-C", out])
    snap = snapshot(out)
    listing = run(["xar", "-tvf", archive]).decode().splitlines()
    rows = []
    for line in listing:
        # perm, owner, size, date, time, path (which may hold spaces)
        f = line.split(None, 5)
        perm, path = f[0], f[5]
        kind = {"d": "directory", "l": "symlink", "-": "file"}[perm[0]]
        mode = int(oct_from_perm(perm[1:]), 8)
        content = b"" if kind != "file" else snap.get(path, b"")
        rows.append((path, kind, mode, len(content), hashlib.md5(content).hexdigest()))
    return sorted(rows)


def oct_from_perm(p):
    v = 0
    for k, c in enumerate(p[:9]):
        if c != "-":
            v |= 1 << (8 - k)
    return oct(v)[2:]


def xar_mine(got):
    ok, rows = got.split("|", 1)
    out = []
    for r in rows.split(";") if rows else []:
        f = r.split(":")
        out.append((bytes.fromhex(f[0]).decode(), f[1], int(f[2]), int(f[6]), f[7]))
    return ok, sorted(out)


for i in range(15):
    src = os.path.join(work, f"x{i}")
    t = {k.replace(" ", "_").replace("'", "").lstrip("-") or "n": v for k, v in tree().items()}
    materialise(t, src)
    if rng.random() < 0.4:
        os.symlink(next(iter(t)), os.path.join(src, "link"))
    archive = os.path.join(work, f"x{i}.xar")
    run(["xar", "-c", "-f", archive] + sorted(os.listdir(src)), cwd=src)
    want = xar_rows(archive)

    def check(got, want=want):
        if got == "ERR":
            return False, "entries", got
        ok, mine = xar_mine(got)
        # xar -tv shows a link's own size; compare links by kind and mode.
        mine = [(p, k, m, s if k == "file" else 0, h if k == "file" else hashlib.md5(b"").hexdigest()) for p, k, m, s, h in mine]
        w = [(p, k, m, s if k == "file" else 0, h) for p, k, m, s, h in want]
        return ok == "true" and mine == w, str(w)[:200], ok + str(mine)[:200]
    cases.append(("xar", archive, check, "xar-read"))

for i in range(10):
    t = {k.replace(" ", "_") or "n": v for k, v in tree().items()}
    items = []

    def flat(t, prefix):
        for k, v in t.items():
            p = prefix + k
            if isinstance(v, dict):
                items.append(f"D {p.encode().hex()} 755")
                flat(v, p + "/")
            else:
                items.append(f"F {p.encode().hex()} {rng.choice(['644', '755', '600'])} {v.hex()}")
    flat(t, "")
    spec = write("\n".join(items) + "\n", "spec")
    want = {}
    for it in items:
        f = it.split(" ")
        want[bytes.fromhex(f[1]).decode()] = None if f[0] == "D" else bytes.fromhex(f[3]) if len(f) > 3 else b""

    def check(got, want=want):
        if got == "ERR":
            return False, "an archive", got
        a = write(bytes.fromhex(got), "xar")
        out = os.path.join(work, f"xw{counter[0]}")
        counter[0] += 1
        os.makedirs(out)
        run(["xar", "-xf", a, "-C", out])
        back = snapshot(out)
        return back == want, str(sorted(want))[:200], str(sorted(back))[:200]
    cases.append(("xarw", spec, check, "xar-write"))


# --- BOM -----------------------------------------------------------------------------------------

for i in range(15):
    src = os.path.join(work, f"b{i}")
    materialise({k.replace(" ", "_") or "n": v for k, v in tree().items()}, src)
    if rng.random() < 0.5:
        os.symlink("target-" + str(i), os.path.join(src, "lnk"))
    files_only = sorted(p for p, v in snapshot(src).items() if v is not None and not os.path.islink(os.path.join(src, p)))
    for p in rng.sample(files_only, min(2, len(files_only))):
        os.chmod(os.path.join(src, p), rng.choice([0o755, 0o644, 0o700]))
    bom = os.path.join(work, f"b{i}.bom")
    run(["mkbom", src, bom])
    want = sorted(run(["lsbom", bom]).decode().splitlines())
    cases.append(("bom", bom, (lambda got, want=want: (got != "ERR" and sorted(bytes.fromhex(got).decode().splitlines()) == want, str(want)[:200], got[:200])), "bom"))


# --- reftable ------------------------------------------------------------------------------------

def git(repo, *args, data=None, ok=(0,)):
    return run(GIT + list(args), data=data, cwd=repo, ok=ok).decode()


def commit(repo, msg):
    with open(os.path.join(repo, "f"), "w") as f:
        f.write(msg + str(rng.random()))
    git(repo, "add", "f")
    git(repo, "commit", "-q", "-m", msg)
    return git(repo, "rev-parse", "HEAD").strip()


for i in range(12):
    repo = os.path.join(work, f"rt{i}")
    run(GIT + ["init", "-q", "--ref-format=reftable", repo])
    commits = [commit(repo, f"c{k}") for k in range(rng.randrange(2, 5))]
    for k in range(rng.choice([3, 10, 200 if i % 4 == 0 else 20])):
        kind = rng.randrange(6)
        c = rng.choice(commits)
        if kind == 0:
            git(repo, "branch", "-f", f"b{k % 7}", c)
        elif kind == 1:
            git(repo, "tag", "-f", f"t{k % 5}", c)
        elif kind == 2:
            git(repo, "tag", "-f", "-a", "-m", f"ann {k}", f"a{k % 4}", c)
        elif kind == 3:
            git(repo, "update-ref", "-m", f"move é {k}", f"refs/x/r{k}", c)
        elif kind == 4 and k > 3:
            git(repo, "branch", "-D", f"b{(k - 1) % 7}", ok=(0, 1))
        else:
            git(repo, "symbolic-ref", f"refs/heads/alias{k % 3}", f"refs/heads/b{k % 7}")
    if rng.random() < 0.5:
        git(repo, "pack-refs", "--all")
    refs = {}
    for line in git(repo, "for-each-ref", "--format=%(refname) %(objectname) %(*objectname) %(symref)").splitlines():
        name, oid, peeled, symref = (line.split(" ") + ["", "", ""])[:4]
        refs[name] = (oid, peeled, symref)
    head = git(repo, "symbolic-ref", "HEAD").strip()
    logs = {}
    for name in ["HEAD"] + sorted(refs):
        out = git(repo, "reflog", "show", "--format=%H%x00%gn%x00%ge%x00%gs", name, "--", ok=(0, 128))
        logs[name] = [tuple(l.split("\0")) for l in out.splitlines() if l]
    listfile = write(os.path.join(repo, ".git/reftable"), "path")

    def check(got, refs=refs, head=head, logs=logs):
        if got == "ERR":
            return False, "refs", got
        r, l = got.split("|")
        mine = {}
        symrefs = {}
        for x in r.split(";") if r else []:
            name, kind, value, peeled, target = (x.split(" ") + [""] * 5)[:5]
            if kind == "3":
                symrefs[name] = target
            else:
                mine[name] = (value, peeled)
        if symrefs.get("HEAD") != head:
            return False, "HEAD -> " + head, str(symrefs.get("HEAD"))
        # for-each-ref shows a symbolic ref with its target's value.
        resolved = {}
        for name in mine:
            resolved[name] = (mine[name][0], mine[name][1], "")
        for name, target in symrefs.items():
            if name != "HEAD" and target in mine:
                resolved[name] = (mine[target][0], mine[target][1], target)
        if resolved != refs:
            k = sorted(set(resolved) ^ set(refs)) or [n for n in refs if refs[n] != resolved.get(n)]
            return False, str({n: refs.get(n) for n in k[:3]}), str({n: resolved.get(n) for n in k[:3]})
        mylogs = {}
        for x in l.split(";") if l else []:
            name, old, new, who, email, t, tz, msg = x.split(" ")
            mylogs.setdefault(name, []).append((new, bytes.fromhex(who).decode(), bytes.fromhex(email).decode(), bytes.fromhex(msg).decode().split("\n")[0]))
        for name, entries in logs.items():
            # `git reflog show` on a symbolic ref shows its own log, or, when
            # it has none, the log of the ref it points at.
            m = mylogs.get(name) or (mylogs.get(symrefs[name], []) if name != "HEAD" and name in symrefs else [])
            if [e[:3] + (e[3],) for e in m] != [tuple(e) for e in entries]:
                return False, f"{name}: {entries[:2]}", str(m[:2])
        return True, "", ""
    cases.append(("reftable", listfile, check, "reftable"))


# --- bitmaps ------------------------------------------------------------------------------------

for i in range(10):
    repo = os.path.join(work, f"bm{i}")
    run(GIT + ["init", "-q", repo])
    for k in range(rng.randrange(3, 40)):
        for _ in range(rng.randrange(1, 4)):
            p = os.path.join(repo, f"d{rng.randrange(4)}", f"f{rng.randrange(20)}")
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "wb").write(blob(rng.randrange(1, 200)))
        git(repo, "add", "-A")
        git(repo, "commit", "-q", "--allow-empty", "-m", f"c{k}")
        if rng.random() < 0.2:
            git(repo, "tag", "-a", "-m", "t", f"t{k}")
    git(repo, "repack", "-adbq")
    pack = [f for f in os.listdir(os.path.join(repo, ".git/objects/pack")) if f.endswith(".bitmap")][0]
    bmp = os.path.join(repo, ".git/objects/pack", pack)
    idx = bmp.replace(".bitmap", ".idx")
    objs = [l.split()[:2] for l in run(["git", "show-index"], data=open(idx, "rb").read()).decode().splitlines()]
    by_offset = sorted(objs, key=lambda x: int(x[0]))
    pack_pos = {oid: k for k, (_, oid) in enumerate(by_offset)}
    types = run(["git", "cat-file", "--batch-check=%(objectname) %(objecttype)"], data="\n".join(o for _, o in objs).encode(), cwd=repo).decode()
    by_type = {"commit": [], "tree": [], "blob": [], "tag": []}
    for line in types.splitlines():
        o, t = line.split()
        by_type[t].append(pack_pos[o])

    def check(got, objs=objs, pack_pos=pack_pos, by_type=by_type, repo=repo):
        if got == "ERR":
            return False, "a bitmap", got
        parts = got.split("|")
        for k, t in enumerate(["commit", "tree", "blob", "tag"]):
            mine = [int(x) for x in parts[k].split(",") if x]
            if mine != sorted(by_type[t]):
                return False, f"{t}s {sorted(by_type[t])[:8]}", str(mine[:8])
        for e in parts[4].split(";"):
            pos, bits = e.split(":")
            oid = objs[int(pos)][1]
            reach = run(["git", "rev-list", "--objects", "--no-object-names", oid], cwd=repo).decode().split()
            if sorted(pack_pos[o] for o in reach) != [int(x) for x in bits.split(",") if x]:
                return False, f"reachable from {oid[:8]}", "differs"
        if parts[5] != str(len(objs)):
            return False, f"{len(objs)} name hashes", parts[5]
        same, differ = parts[6].split(":")
        return differ == "0", "every EWAH written back as git wrote it", parts[6]
    cases.append((f"bitmap:{len(objs)}", bmp, check, "ewah"))


# --- notes ---------------------------------------------------------------------------------------

def notes_repo(i, many):
    repo = os.path.join(work, f"nt{i}")
    run(GIT + ["init", "-q", repo])
    commits = [commit(repo, f"c{k}") for k in range(3)]
    # Blobs to annotate: many objects so the notes tree fans out.
    blobs = run(GIT + ["hash-object", "-w", "--stdin-paths"], data="\n".join(write(f"obj {i} {k}", "o") for k in range(many)).encode(),
                cwd=repo).decode().split()
    return repo, commits + blobs


for i in range(8):
    repo, targets = notes_repo(i, rng.choice([5, 300, 700 if i == 0 else 60]))
    for k, t in enumerate(targets):
        git(repo, "notes", "add", "-f", "-m", f"note {k}", t)
        if rng.random() < 0.05:
            git(repo, "notes", "append", "-m", "more", t)
    want = sorted(" ".join(reversed(l.split())) for l in git(repo, "notes", "list").splitlines())
    tree = git(repo, "rev-parse", "refs/notes/commits^{tree}").strip()

    def check(got, want=want):
        if got == "ERR":
            return False, "notes", got
        notes, other = got.split("|")
        mine = sorted(notes.split(";")) if notes else []
        return mine == want and other == "0", f"{len(want)} notes {want[:2]}", f"{len(mine)} notes {mine[:2]} other {other}"
    cases.append((f"notes:{tree}", write(os.path.join(repo, ".git"), "path"), check, "notes-read"))

for i in range(8):
    repo, targets = notes_repo(100 + i, rng.choice([3, 280, 600 if i == 0 else 40]))
    blob_ids = run(GIT + ["hash-object", "-w", "--stdin-paths"], data="\n".join(write(f"text {k}", "o") for k in range(5)).encode(), cwd=repo).decode().split()
    notes = sorted((t, rng.choice(blob_ids)) for t in targets)
    spec = write("\n".join(f"{t} {b}" for t, b in notes) + "\n", "spec")

    def check(got, repo=repo, notes=notes):
        if got == "ERR":
            return False, "trees", got
        root, trees = got.split("|")
        for t in trees.split(";"):
            tid, content = t.split(":")
            made = run(GIT + ["hash-object", "-w", "-t", "tree", "--literally", "--stdin"], data=bytes.fromhex(content), cwd=repo).decode().strip()
            if made != tid:
                return False, f"tree {tid}", made
        c = git(repo, "commit-tree", "-m", "notes", root).strip()
        git(repo, "update-ref", "refs/notes/mine", c)
        listed = sorted(tuple(reversed(l.split())) for l in git(repo, "notes", "--ref", "mine", "list").splitlines())
        return listed == notes, f"{len(notes)} notes", f"{len(listed)} listed"
    cases.append(("notesw", spec, check, "notes-write"))

# The layout itself: `git notes copy --stdin` adds every note in one
# process to a tree holding one, so it writes git's fanout for the whole
# set; the tree written here must be the same tree, id for id.
for i, count in enumerate([3, 40, 260, 300, 700, 1500, 90, 520]):
    repo, targets = notes_repo(200 + i, count)
    first = targets[0]
    git(repo, "notes", "add", "-m", "shared note", first)
    git(repo, "notes", "copy", "--stdin", data="\n".join(f"{first} {t}" for t in targets[1:]).encode() + b"\n")
    want_tree = git(repo, "rev-parse", "refs/notes/commits^{tree}").strip()
    blob_id_of_note = git(repo, "notes", "list", first).strip()
    spec = write("\n".join(f"{t} {blob_id_of_note}" for t in sorted(targets)) + "\n", "spec")
    cases.append(("notesw", spec, (lambda got, want_tree=want_tree: (got != "ERR" and got.split("|")[0] == want_tree, want_tree, got.split("|")[0])),
                  "notes-fanout"))

for i in range(20):
    repo = os.path.join(work, f"nm{i}")
    run(GIT + ["init", "-q", repo])
    c = commit(repo, "c")
    lines = ["apple", "banana", "cherry", "", "apple", "zeta", "Banana"]
    # An empty note is a removal to git: every note has a line.
    a = "\n".join(rng.sample(lines, rng.randrange(1, 5)) + ["fig"]) + rng.choice(["", "\n", "\n\n"])
    b = "\n".join(["grape"] + rng.sample(lines, rng.randrange(1, 5))) + rng.choice(["", "\n"])
    strategy = rng.choice(["union", "cat_sort_uniq"])
    git(repo, "notes", "--ref", "other", "add", "-f", "-F", write(b, "n"), c)
    git(repo, "notes", "add", "-f", "-F", write(a, "n"), c)
    git(repo, "notes", "merge", "-q", "-s", strategy, "other", ok=(0, 1))
    want = git(repo, "notes", "show", c)
    # git stores a note as written by `add -F` after stripspace; read them back.
    na = run(GIT + ["notes", "--ref", "other", "show", c], cwd=repo).decode()
    log = git(repo, "log", "-g", "--format=%H", "refs/notes/commits").split()
    nb_cur = git(repo, "show", f"{log[-1]}:{c}") if len(log) > 1 else a
    cases.append((f"notescombine:{strategy}", write(nb_cur.encode().hex() + "\n" + na.encode().hex(), "txt"),
                  (lambda got, want=want: (got != "ERR" and bytes.fromhex(got).decode() == want, want, bytes.fromhex(got).decode() if got != "ERR" else got)),
                  "notes-combine"))


# --- ignore rules and status -----------------------------------------------------------------------

PATTERNS = ["*.log", "!keep.log", "build/", "/top.txt", "d1/*.tmp", "**/cache", "x?y", "[ab]*.c", "!d1/", "*.o", "d2/**", "!d2/keep",
            "deep/", "\\#hash", "trailing\\ ", "/d1/sub/", "*", "!*.txt", "**/z/*"]
NAMES = ["a.log", "keep.log", "build", "top.txt", "x.tmp", "cache", "xay", "a1.c", "b.c", "c.c", "main.o", "keep", "deep", "#hash",
         "notes.txt", "z", "w"]


# Pairs of opposite rules: a conflict family alternates them by depth, so
# which .gitignore git consults last decides every matching path.
CONFLICTS = [("*.log", "!*.log"), ("*.o", "!*.o"), ("keep.log", "!keep.log"), ("*.tmp", "!*.tmp"), ("x?y", "!x?y"), ("[ab]*.c", "!a1.c")]


def ignore_tree(repo, k, conflict=False):
    files = []
    for _ in range(rng.randrange(8, 30)):
        depth = rng.randrange(1, 4)
        parts = [rng.choice(["d1", "d2", "build", "sub", "z", "deep"]) for _ in range(depth - 1)] + [rng.choice(NAMES)]
        files.append("/".join(parts))
    made = []
    for f in sorted(set(files)):
        p = os.path.join(repo, f)
        if any(os.path.isfile(os.path.join(repo, "/".join(f.split("/")[:j]))) for j in range(1, f.count("/") + 1)):
            continue
        if os.path.isdir(p):
            continue
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").write("x")
        made.append(f)
    gitignores = []
    dirs = sorted({os.path.dirname(f) for f in made})
    chosen = rng.sample(CONFLICTS, 3)
    flip = rng.randrange(2)
    for d in [""] + [x for x in dirs if x]:
        if conflict and rng.random() < 0.9:
            level = 0 if not d else d.count("/") + 1
            text = "\n".join(pair[(level + flip) % 2] for pair in chosen) + "\n"
            p = os.path.join(d, ".gitignore") if d else ".gitignore"
            open(os.path.join(repo, p), "w").write(text)
            gitignores.append((p, text))
        elif not conflict and rng.random() < 0.6:
            text = "\n".join(rng.sample(PATTERNS, rng.randrange(1, 5))) + "\n"
            p = os.path.join(d, ".gitignore") if d else ".gitignore"
            open(os.path.join(repo, p), "w").write(text)
            gitignores.append((p, text))
    if conflict:
        info = "\n".join(pair[(flip + 1) % 2] for pair in chosen) + "\n"
    else:
        info = "\n".join(rng.sample(PATTERNS, rng.randrange(0, 3))) + "\n"
    open(os.path.join(repo, ".git/info/exclude"), "w").write(info)
    return made, gitignores, info


for i in range(30):
    repo = os.path.join(work, f"ig{i}")
    run(GIT + ["init", "-q", repo])
    made, gitignores, info = ignore_tree(repo, i, conflict=i >= 20)
    lines = [f"P {f.encode().hex()} 0" for f in made + [g for g, _ in gitignores]]
    lines += [f"G {p.encode().hex()} {t.encode().hex()}" for p, t in gitignores] + [f"X {info.encode().hex()}"]
    spec = write("\n".join(lines) + "\n", "spec")
    want = sorted(git(repo, "ls-files", "-o", "-i", "--exclude-standard", "-z").split("\0")[:-1])

    def check(got, want=want):
        if got == "ERR":
            return False, "paths", got
        mine = sorted(bytes.fromhex(h).decode() for h in got.split(",") if h)
        return mine == want, str(want)[:200], str(mine)[:200]
    cases.append(("ignore", spec, check, "ignorewalk" if i < 20 else "ignore-precedence"))

# Always one tree where `d/**` ignores everything inside d but does not
# exclude d itself, so `!d/keep` brings that file back and git still reads
# d's own .gitignore; an ignored directory above it (`x/`) keeps its files out.
repo = os.path.join(work, "igfixed")
run(GIT + ["init", "-q", repo])
fixed_files = ["d/keep", "d/drop", "d/sub/keep", "d/.gitignore", "x/keep", "top"]
fixed_ignores = [(".gitignore", "d/**\n!d/keep\n!d/.gitignore\nx/\n!x/keep\n"), ("d/.gitignore", "!sub/keep\n")]
for f in fixed_files:
    os.makedirs(os.path.join(repo, os.path.dirname(f) or "."), exist_ok=True)
    open(os.path.join(repo, f), "w").write("u")
for p, t in fixed_ignores:
    open(os.path.join(repo, p), "w").write(t)
lines = [f"P {f.encode().hex()} 0" for f in fixed_files + [".gitignore"]]
lines += [f"G {p.encode().hex()} {t.encode().hex()}" for p, t in fixed_ignores] + [f"X {''.encode().hex()}"]
spec = write("\n".join(lines) + "\n", "spec")
want = sorted(git(repo, "ls-files", "-o", "-i", "--exclude-standard", "-z").split("\0")[:-1])
cases.append(("ignore", spec, lambda got, want=want: (got != "ERR" and sorted(bytes.fromhex(h).decode() for h in got.split(",") if h) == want,
                                                      str(want), got), "ignorewalk"))


def blob_id(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def add_status_case(repo, gitignores, info):
    head = git(repo, "ls-tree", "-r", "-z", "HEAD").split("\0")[:-1]
    lines = []
    for e in head:
        meta, path = e.split("\t", 1)
        mode, _t, oid = meta.split()
        lines.append(f"H {path.encode().hex()} {mode} {oid}")
    for d, dirs, files in os.walk(repo):
        if ".git" in d.split(os.sep):
            continue
        dirs[:] = [x for x in dirs if x != ".git"]
        for f in files:
            p = os.path.join(d, f)
            rel = os.path.relpath(p, repo)
            if os.path.islink(p):
                mode, data = "120000", os.readlink(p).encode()
            else:
                mode = "100755" if os.stat(p).st_mode & 0o100 else "100644"
                data = open(p, "rb").read()
            lines.append(f"W {rel.encode().hex()} {mode} {blob_id(data)}")
    # Edits after the commit may have changed, removed or replaced a .gitignore by a
    # link; git reads what the work tree holds now, so give the current text
    # of a regular file, and the old text otherwise for gitstatus to discard.
    def now(p, t):
        q = os.path.join(repo, p)
        return open(q).read() if os.path.isfile(q) and not os.path.islink(q) else t
    lines += [f"G {p.encode().hex()} {now(p, t).encode().hex()}" for p, t in gitignores] + [f"X {info.encode().hex()}"]
    lines.append("I " + os.path.join(repo, ".git/index").encode().hex())
    spec = write("\n".join(lines) + "\n", "spec")
    want = git(repo, "-c", "status.renames=false", "status", "--porcelain=v2", "--untracked-files=all").rstrip("\n")
    cases.append(("status", spec, (lambda got, want=want: (got != "ERR" and bytes.fromhex(got).decode() == want, want[:300],
                                                           bytes.fromhex(got).decode()[:300] if got != "ERR" else got)), "gitstatus"))


for i in range(15):
    repo = os.path.join(work, f"st{i}")
    run(GIT + ["init", "-q", repo])
    made, gitignores, info = ignore_tree(repo, i)
    git(repo, "add", "-A", "-f")
    git(repo, "commit", "-q", "-m", "base", "--allow-empty")
    tracked = git(repo, "ls-files", "-z").split("\0")[:-1]
    for f in rng.sample(tracked, min(len(tracked), rng.randrange(0, 6))):
        act = rng.randrange(6)
        p = os.path.join(repo, f)
        if act == 0:
            open(p, "a").write("more")
        elif act == 1:
            os.remove(p)
        elif act == 2:
            os.chmod(p, 0o755)
        elif act == 3:
            open(p, "a").write("staged")
            git(repo, "add", "-f", f)
        elif act == 4:
            git(repo, "rm", "-q", "--cached", f)
        else:
            os.remove(p)
            os.symlink("target", p)
    for k in range(rng.randrange(0, 3)):
        open(os.path.join(repo, f"new{k}.txt"), "w").write("n")
        if rng.random() < 0.5:
            git(repo, "add", "-f", f"new{k}.txt")
    add_status_case(repo, gitignores, info)


# Always one repository where the ignore files themselves changed after the
# commit: git reads the work tree's .gitignore, so a link it will not follow
# and a deleted file add no rules, and an edited one gives its new rules.
repo = os.path.join(work, "stfixed")
run(GIT + ["init", "-q", repo])
fixed_ignores = [("ln/.gitignore", "a\n"), ("gone/.gitignore", "b\n"), ("edit/.gitignore", "c\n")]
for p, t in fixed_ignores:
    os.makedirs(os.path.join(repo, os.path.dirname(p)), exist_ok=True)
    open(os.path.join(repo, p), "w").write(t)
git(repo, "add", "-A")
git(repo, "commit", "-q", "-m", "base")
os.remove(os.path.join(repo, "ln/.gitignore"))
os.symlink("target", os.path.join(repo, "ln/.gitignore"))
os.remove(os.path.join(repo, "gone/.gitignore"))
open(os.path.join(repo, "edit/.gitignore"), "a").write("d\n")
for p in ["ln/a", "gone/b", "edit/c", "edit/d", "edit/e"]:
    open(os.path.join(repo, p), "w").write("u")
add_status_case(repo, fixed_ignores, "")


# --- run --------------------------------------------------------------------------------------------

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
        except Exception as e:
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
    thin = [k for k, (a, d) in families.items() if a + d < 8]
    if thin or len(families) < int(os.environ.get("FAMILIES", "19")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
