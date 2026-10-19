"""Object files, signatures, debug lines, SQLite side files, x86 lengths
and Go tokens, each against the tool that reads them.

- Mach-O symbols against `nm -ap`; universal files against `lipo`
  (`-create`, byte for byte, `-detailed_info`, `-thin`, `-remove`).
- Code signatures against `codesign -d -vvv` and `codesign -v`, and the
  signature ld writes rebuilt byte for byte.
- Export tries and chained fixups against `dyld_info`, and tries rebuilt
  as ld writes them.
- DWARF line tables (versions 2, 4 and 5, objects and .dSYMs) against
  `dwarfdump --debug-line`.
- WAL files against the database SQLite checkpoints from them, logs
  written here read by SQLite, and hot rollback journals against the
  database SQLite rolls back to.
- x86-64 instruction lengths against `objdump -d`.
- Go tokens against go/scanner, on the standard library and on
  generated sources full of edge cases.
"""
import os
import random
import re
import shutil
import sqlite3
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b42")
harness = os.path.join(work, "b42test")
built = subprocess.run([GRASM, "build", "packages/b42test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
goscan = os.path.join(work, "goscan")
built = subprocess.run(["go", "build", "-o", goscan, os.path.join(ROOT, "scripts/lib/b42/goscan.go")], capture_output=True, text=True,
                       env=dict(os.environ, GOFLAGS="-mod=mod", GO111MODULE="off"))
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
cases = []
counter = [0]


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
        raise RuntimeError(" ".join(map(str, cmd[:10])) + "\n" + r.stderr.decode(errors="replace")[-600:])
    return r.stdout


def unhex(got):
    return bytes.fromhex(got).decode(errors="replace")


def text_check(want):
    def check(got):
        if got in ("ERR", "NOFILE"):
            return False, want[:200], got
        mine = unhex(got)
        return mine == want, want[:300], mine[:300]
    return check


def file_check(out, want):
    def check(got):
        if got != "ok":
            return False, "ok", got
        mine = open(out, "rb").read()
        if mine == want:
            return True, "", ""
        at = next((i for i in range(min(len(mine), len(want))) if mine[i] != want[i]), min(len(mine), len(want)))
        return False, f"{len(want)} bytes", f"{len(mine)} bytes, first difference at {at}"
    return check


# --- C programs ------------------------------------------------------------------------------------

WORDS = ["alpha", "beta", "gamma", "delta", "kappa", "omega", "zeta", "theta"]
names = [0]


def c_source(switches=True, tls=True, main=True):
    out = ["#include <stdio.h>", "#include <string.h>", "#include <stdlib.h>"]
    calls = []
    # Names never repeat across sources, so units link together.
    names[0] += 20
    for i in range(names[0], names[0] + rng.randrange(3, 14)):
        n = rng.randrange(1, 9999)
        w = rng.choice(WORDS)
        kind = rng.randrange(14 if switches else 13)
        if kind == 0:
            out.append(f"int g{i} = {n};")
        elif kind == 1:
            out.append(f"static int s{i} = {n}; int gs{i}(void) {{ return s{i}++; }}"); calls.append(f"gs{i}()")
        elif kind == 2:
            out.append(f"int c{i};")
        elif kind == 3:
            out.append(f"static long b{i}[{n % 50 + 1}]; long *pb{i} = &b{i}[{n % 50}];")
        elif kind == 4:
            out.append(f'const char *str{i} = "{w}{n}";')
        elif kind == 5 and tls:
            out.append(f"__thread int t{i} = {n}; int gt{i}(void) {{ return t{i}; }}"); calls.append(f"gt{i}()")
        elif kind == 6:
            out.append(f"__attribute__((weak)) int w{i}(int x) {{ return x + {n}; }}"); calls.append(f"w{i}(argc)")
        elif kind == 7:
            out.append(f'__attribute__((visibility("hidden"))) int h{i}(int x) {{ return x * {n}; }}'); calls.append(f"h{i}(argc)")
        elif kind == 8:
            out.append(f"static int sf{i}(int x) {{ return x - {n}; }} int (*fp{i})(int) = sf{i};"); calls.append(f"fp{i}(argc)")
        elif kind == 9:
            out.append(f"int (*pp{i})(const char *) = puts; size_t (*sl{i})(const char *) = strlen;")
        elif kind == 10:
            out.append(f"int lp{i}(int n) {{ int s = 0; for (int k = 0; k < n; k++) s += k * {n} ^ (s >> 3); return s; }}"); calls.append(f"lp{i}(argc)")
        elif kind == 11:
            out.append(f"double fl{i}(double a, double b) {{ return a * b + {n}.5; }}"); calls.append(f"(int)fl{i}(argc, 2.0)")
        elif kind == 12:
            out.append(f'int pr{i}(int x) {{ char buf[64]; memcpy(buf, "{w}", 5); buf[5] = 0; printf("%s %d\\n", buf, x); return (int)strlen(buf); }}')
            calls.append(f"pr{i}(argc)")
        else:
            out.append(f'int sw{i}(int x) {{ switch (x) {{ case 0: return {n}; case 1: return {n} + 7; case 2: return x * 3; case 3: return x + 11; '
                       f'case 4: return (int)strlen("{w}"); case 5: return 99; default: return -1; }} }}')
            calls.append(f"sw{i}(argc)")
    if main:
        out.append("int main(int argc, char **argv) { int s = 0; " + "".join(f"s += {c}; " for c in calls) + "return s & 1; }")
    return "\n".join(out) + "\n"


pool = []   # (arch, kind, path)


def build(arch, kind, opt="-O1", debug=False, extra=(), src=None, keep_objects=False):
    src = src or c_source(main=kind == "exe")
    s = write(src, "c")
    out = path_for({"obj": "o", "exe": "exe", "dylib": "dylib", "bundle": "bundle"}[kind])
    cmd = ["clang", "-arch", arch, opt, "-fcommon", "-w"] + (["-g"] if debug else []) + list(extra)
    cmd += {"obj": ["-c"], "exe": [], "dylib": ["-dynamiclib"], "bundle": ["-bundle"]}[kind]
    run(cmd + [s, "-o", out])
    return out


for arch in ["arm64", "x86_64", "arm64e"]:
    for kind in ["obj", "exe", "dylib", "bundle"]:
        for _ in range(2):
            pool.append((arch, kind, build(arch, kind, rng.choice(["-O0", "-O1", "-O2", "-Os"]), rng.random() < 0.5)))


def macho_text_section(path):
    d = open(path, "rb").read()
    ncmds = struct.unpack("<I", d[16:20])[0]
    p = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack("<II", d[p:p + 8])
        if cmd == 0x19:
            for k in range(struct.unpack("<I", d[p + 64:p + 68])[0]):
                s = p + 72 + 80 * k
                if d[s:s + 16].rstrip(b"\0") == b"__text" and d[s + 16:s + 32].rstrip(b"\0") == b"__TEXT":
                    size_, off = struct.unpack("<QI", d[s + 40:s + 52])
                    return d[off:off + size_]
        p += size
    return b""


# --- nm ------------------------------------------------------------------------------------------------

for arch, kind, p in rng.sample(pool, 16):
    cases.append(("nm", p, text_check(run(["nm", "-ap", p]).decode().rstrip("\n")), "symtab"))


# --- lipo ----------------------------------------------------------------------------------------------

def thin_set(kind):
    archs = rng.sample(["arm64", "x86_64", "arm64e", "x86_64h"], rng.randrange(2, 5))
    src = c_source(main=kind == "exe")
    paths = []
    for a in archs:
        extra = []
        if kind == "dylib" and rng.random() < 0.6:
            unit = 0x4000 if a.startswith("arm64") else 0x1000
            extra = ["-Wl,-image_base," + hex(unit * rng.choice([1, 2, 3, 5, 8, 16, 64, 1024]))]
        if kind == "lib":
            o = build(a, "obj", src=src)
            lib = path_for("a")
            run(["libtool", "-static", "-o", lib, o, build(a, "obj")])
            paths.append(lib)
        else:
            paths.append(build(a, kind, extra=extra, src=src))
    return paths


fats = []
for i in range(14):
    kind = rng.choice(["obj", "exe", "dylib", "lib"])
    paths = thin_set(kind)
    wide = rng.random() < 0.3
    want_path = path_for("fat")
    run(["lipo", "-create"] + (["-fat64"] if wide else []) + paths + ["-output", want_path])
    fats.append(want_path)
    spec = write(f"page 16384\nfat64 {int(wide)}\n" + "\n".join(paths) + "\n", "spec")
    cases.append(("fatcreate", spec, file_check(spec + ".fat", open(want_path, "rb").read()), "fat-create"))


def fat_slices(path):
    d = open(path, "rb").read()
    wide = d[3] == 0xbf
    n = struct.unpack(">I", d[4:8])[0]
    out = []
    for i in range(n):
        if wide:
            ct, cs, off, size, al, _ = struct.unpack(">IIQQII", d[8 + 32 * i:40 + 32 * i])
        else:
            ct, cs, off, size, al = struct.unpack(">IIIII", d[8 + 20 * i:28 + 20 * i])
        out.append((ct, cs, off, size, al))
    return out


for f in fats:
    info = run(["lipo", "-detailed_info", f]).decode()
    archs = re.findall(r"^architecture (\S+)", info, re.M)
    offs = re.findall(r"^\s+offset (\d+)", info, re.M)
    sizes = re.findall(r"^\s+size (\d+)", info, re.M)
    aligns = re.findall(r"^\s+align 2\^(\d+)", info, re.M)
    want = ",".join(f"{a}:{o}:{s}:{al}" for a, o, s, al in zip(archs, offs, sizes, aligns))
    cases.append(("fatinfo", f, (lambda got, want=want: (got == want, want, got)), "fat-read"))
    slices = fat_slices(f)
    ct, cs = rng.choice(slices)[:2]
    name = archs[[s[:2] for s in slices].index((ct, cs))]
    copy = path_for("fat")
    shutil.copy(f, copy)
    thin = path_for("thin")
    run(["lipo", f, "-thin", name, "-output", thin])
    cases.append((f"fatthin:{ct}:{cs}", copy, file_check(copy + ".thin", open(thin, "rb").read()), "fat-read"))
    if len(slices) > 2:
        copy2 = path_for("fat")
        shutil.copy(f, copy2)
        rm = path_for("rm")
        run(["lipo", f, "-remove", name, "-output", rm])
        cases.append((f"fatremove:{ct}:{cs}", copy2, file_check(copy2 + ".rm", open(rm, "rb").read()), "fat-read"))


# --- code signatures -----------------------------------------------------------------------------------

CS_LINES = ("Identifier=", "CodeDirectory v=", "Hash type=", "CDHash=", "TeamIdentifier=")


def codesign_lines(path):
    r = subprocess.run(["codesign", "-d", "-vvv", path], capture_output=True, text=True)
    return [l for l in r.stderr.splitlines() if l.startswith(CS_LINES)]


ENT = ('<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
       '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict><key>com.apple.security.get-task-allow</key><true/>'
       '<key>com.example.n</key><integer>{n}</integer></dict></plist>\n')
signed = []
linked = [p for a, k, p in pool if k != "obj"] + [b for b in (build(rng.choice(["arm64", "arm64e"]), rng.choice(["exe", "dylib"])) for _ in range(4))]
for p in linked:
    if any(l.startswith("CodeDirectory") and "linker-signed" in l for l in codesign_lines(p)):
        signed.append(p)
        cases.append(("csls", p, (lambda got: (got == "same", "same", got)), "codesig-linker"))
for i in range(14):
    p = path_for("signed")
    shutil.copy(rng.choice(linked), p)
    opts = ["-s", "-", "-f"]
    if rng.random() < 0.5:
        opts += ["--identifier", f"com.example.{rng.choice(WORDS)}{i}"]
    if rng.random() < 0.5:
        opts += ["-o", ",".join(rng.sample(["runtime", "kill", "hard", "library", "restrict"], rng.randrange(1, 4)))]
    if rng.random() < 0.4:
        opts += ["--entitlements", write(ENT.format(n=i), "plist")]
    if rng.random() < 0.4:
        opts += ["--digest-algorithm=" + rng.choice(["sha1,sha256", "sha256"])]
    run(["codesign"] + opts + [p])
    signed.append(p)
for p in rng.sample(signed, min(len(signed), 16)):
    want = "\n".join(codesign_lines(p))

    def check(got, want=want):
        if got == "ERR":
            return False, want[:200], got
        lines, problems = got.split("|")
        return unhex(lines) == want and unhex(problems) == "", want[:300], (unhex(lines) + " / " + unhex(problems))[:300]
    cases.append(("cs", p, check, "codesig-read"))


def code_limit_and_pages(d):
    ncmds, sizeofcmds = struct.unpack("<II", d[16:24])
    p = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack("<II", d[p:p + 8])
        if cmd == 0x1d:
            off, sz = struct.unpack("<II", d[p + 8:p + 16])
        p += size
    blob = d[off:off + sz]
    pages = []
    for i in range(struct.unpack(">I", blob[8:12])[0]):
        slot, o = struct.unpack(">II", blob[12 + 8 * i:20 + 8 * i])
        if struct.unpack(">I", blob[o:o + 4])[0] == 0xfade0c02:
            names = {1: "sha1", 2: "sha256", 3: "sha256-truncated", 4: "sha384"}
            pages.append((names[blob[o + 37]], 1 << blob[o + 39]))
    return 32 + sizeofcmds, off, pages


for i in range(14):
    src = rng.choice(signed)
    p = path_for("flip")
    d = bytearray(open(src, "rb").read())
    start, limit, dirs = code_limit_and_pages(bytes(d))
    flipped = None
    if i % 4 != 0:
        flipped = rng.randrange(start, limit)
        d[flipped] ^= 1 << rng.randrange(8)
    open(p, "wb").write(bytes(d))
    valid = subprocess.run(["codesign", "-v", p], capture_output=True).returncode == 0
    want = "" if flipped is None else "\n".join(f"{name} page {flipped // size}" for name, size in dirs)

    def check(got, want=want, valid=valid):
        if got == "ERR":
            return False, want, got
        mine = unhex(got)
        return mine == want and valid == (want == ""), f"{want!r} (codesign -v says {'valid' if valid else 'invalid'})", repr(mine)
    cases.append(("csproblems", p, check, "codesig-verify"))


# --- export tries and chained fixups ---------------------------------------------------------------------

def reexporting_dylib(arch):
    b = build(arch, "dylib", src="int other(void) { return 9; }\nint other2(void) { return 8; }\n")
    lst = write("_other\n", "txt")
    src = c_source(main=False) + 'extern int other(void);\nint use_other(void) { return other(); }\n__asm__(".globl _absval\\n_absval = 0x1234");\n'
    return build(arch, "dylib", src=src, extra=[b, "-Wl,-reexported_symbols_list," + lst, "-Wl,-exported_symbols_list," + write("_*\n", "txt")])


ADDENDS = """extern char ext_a[]; extern char ext_b[]; extern int weak_one __attribute__((weak_import));
char *p1 = ext_a + 200; char *p2 = ext_b + 70000; char *p3 = ext_a - 16; char *p4 = ext_b + 0x123456789; int *p5 = &weak_one;
"""
images = [p for a, k, p in pool if k != "obj"]
for arch in ["arm64", "x86_64", "arm64e"]:
    images.append(reexporting_dylib(arch))
    images.append(build(arch, "dylib", src=c_source(main=False) + ADDENDS, extra=["-Wl,-U,_ext_a", "-Wl,-U,_ext_b", "-Wl,-U,_weak_one"]))


def dyld_info(flag, path):
    lines = run(["dyld_info", flag, path]).decode().splitlines()
    return "\n".join(l[8:] for l in lines[3:])


for p in rng.sample(images, 14):
    cases.append(("exports", p, text_check(dyld_info("-exports", p)), "exports-read"))
for p in rng.sample(images, 10):
    cases.append(("exportenc", p, (lambda got: (got == "same", "same", got)), "exports-write"))


def arm64e_view(text):
    # dyld_info shows an arm64e pointer's 19-bit addend without its sign.
    def fix(m):
        v = int(m.group(1), 16)
        return f"+ 0x{v & 0x7FFFF:X}" if v >= 1 << 63 and (1 << 64) - v < 1 << 18 else m.group(0)
    return re.sub(r"\+ 0x([0-9A-F]{16})", fix, text)


for p in rng.sample(images, 16):
    want = dyld_info("-fixups", p)
    is_e = "[arm64e]" in run(["dyld_info", "-fixups", p]).decode().splitlines()[0]

    def check(got, want=want, is_e=is_e):
        if got == "ERR":
            return False, want[:200], got
        mine = unhex(got)
        mine = arm64e_view(mine) if is_e else mine
        return mine == want, want[:300], mine[:300]
    cases.append(("fixups", p, check, "fixups"))


# --- DWARF line tables -----------------------------------------------------------------------------

def dwarfdump_lines(path):
    out = run(["dwarfdump", "--debug-line", path]).decode().splitlines()
    res = []
    version = 0
    fileno = None
    for l in out:
        m = re.match(r"^\s+version: (\d+)", l)
        if m:
            version = int(m.group(1))
            res.append(f"table {version}")
            continue
        m = re.match(r"^file_names\[\s*(\d+)\]:", l)
        if m:
            fileno = int(m.group(1))
            continue
        m = re.match(r'^\s+name: "(.*)"$', l)
        if m and fileno is not None:
            name = m.group(1)
            continue
        m = re.match(r"^\s+dir_index: (\d+)", l)
        if m and fileno is not None:
            res.append(f"file {fileno} {m.group(1)} {name}")
            fileno = None
            continue
        if re.match(r"^0x[0-9a-f]{16} ", l):
            res.append(l)
    return res


def ours_view(text):
    # Before version 5 the list keeps an empty slot 0 dwarfdump does not print.
    return "\n".join(l for l in text.split("\n") if not re.match(r"^file 0 0 $", l))


def dwarf_check(want):
    def check(got):
        if got == "ERR":
            return False, want[:200], got
        mine = ours_view(unhex(got))
        return mine == want, want[:300], mine[:300]
    return check


for i in range(10):
    arch = rng.choice(["arm64", "x86_64"])
    ver = rng.choice(["-gdwarf-2", "-gdwarf-4", "-gdwarf-5", "-g"])
    o = build(arch, "obj", rng.choice(["-O0", "-O1", "-O2"]), True, extra=[ver])
    cases.append(("dwarf", o, dwarf_check("\n".join(dwarfdump_lines(o))), "dwarfline"))
for i in range(5):
    objs = []
    d = tempfile.mkdtemp(dir=work)
    for k in range(rng.randrange(2, 4)):
        src = os.path.join(d, f"unit{k}.c")
        open(src, "w").write(c_source(main=k == 0).replace("int main(", f"int main{'' if k == 0 else k}("))
        obj = os.path.join(d, f"unit{k}.o")
        run(["clang", "-g", rng.choice(["-O0", "-O2"]), "-w", "-c", src, "-o", obj])
        objs.append(obj)
    exe = os.path.join(d, "prog")
    run(["clang"] + objs + ["-o", exe])
    run(["dsymutil", exe, "-o", exe + ".dSYM"])
    dw = exe + ".dSYM/Contents/Resources/DWARF/prog"
    cases.append(("dwarf", dw, dwarf_check("\n".join(dwarfdump_lines(dw))), "dwarfline"))


# --- SQLite ----------------------------------------------------------------------------------------------

def fill(c, n):
    for _ in range(n):
        c.execute("INSERT INTO t(b) VALUES (?)", ("x" * rng.randrange(1, 400),))


for i in range(12):
    d = tempfile.mkdtemp(dir=work)
    db = os.path.join(d, "w.db")
    c = sqlite3.connect(db, isolation_level=None)
    c.execute("PRAGMA page_size=%d" % rng.choice([512, 1024, 4096, 8192]))
    c.execute("PRAGMA journal_mode=WAL"); c.execute("PRAGMA wal_autocheckpoint=0")
    c.execute("CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)")
    for _ in range(rng.randrange(1, 6)):
        c.execute("BEGIN"); fill(c, rng.randrange(1, 150))
        if rng.random() < 0.3:
            c.execute("DELETE FROM t WHERE a % 3 = 0")
        c.execute("COMMIT")
        if rng.random() < 0.2:
            c.execute("PRAGMA wal_checkpoint(PASSIVE)")
    c.execute("BEGIN"); fill(c, rng.randrange(0, 40))
    snap_db, snap_wal = path_for("db"), path_for("wal")
    shutil.copy(db, snap_db); shutil.copy(db + "-wal", snap_wal)
    wal = bytearray(open(snap_wal, "rb").read())
    damage = i % 4
    if damage == 1 and len(wal) > 100:
        wal[rng.randrange(32, len(wal))] ^= 1 << rng.randrange(8)
    elif damage == 2 and len(wal) > 100:
        wal = wal[:rng.randrange(32, len(wal))]
    elif damage == 3:
        wal += bytes(rng.randrange(256) for _ in range(rng.randrange(1, 3000)))
    open(snap_wal, "wb").write(bytes(wal))
    # The writer stays open, its last transaction uncommitted, as a crash leaves it.
    o = tempfile.mkdtemp(dir=work)
    shutil.copy(snap_db, os.path.join(o, "w.db")); shutil.copy(snap_wal, os.path.join(o, "w.db-wal"))
    oc = sqlite3.connect(os.path.join(o, "w.db"), isolation_level=None)
    oc.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchall(); oc.close()
    spec = write(f"{snap_db} {snap_wal}\n", "spec")
    cases.append(("walapply", spec, file_check(spec + ".out", open(os.path.join(o, "w.db"), "rb").read()), "wal-apply"))

for i in range(10):
    d = tempfile.mkdtemp(dir=work)
    db = os.path.join(d, "a.db")
    ps = rng.choice([512, 1024, 4096])
    c = sqlite3.connect(db, isolation_level=None)
    c.execute(f"PRAGMA page_size={ps}"); c.execute("PRAGMA journal_mode=WAL")
    c.execute("CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)")
    c.execute("BEGIN"); fill(c, rng.randrange(1, 100)); c.execute("COMMIT")
    c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    a_bytes = open(db, "rb").read()
    c.execute("BEGIN"); fill(c, rng.randrange(1, 150))
    if rng.random() < 0.5:
        c.execute("UPDATE t SET b = b || 'y' WHERE a % 2 = 0")
    c.execute("COMMIT"); c.execute("PRAGMA wal_checkpoint(TRUNCATE)"); c.close()
    b_bytes = open(db, "rb").read()
    pages = []
    for k in range(len(b_bytes) // ps):
        if b_bytes[k * ps:(k + 1) * ps] != a_bytes[k * ps:(k + 1) * ps]:
            pages.append((k + 1, write(b_bytes[k * ps:(k + 1) * ps], "page")))
    rng.shuffle(pages)
    salts = (rng.randrange(1 << 32), rng.randrange(1 << 32))
    spec = write(f"{ps} {rng.randrange(1 << 20)} {salts[0]} {salts[1]} {len(b_bytes) // ps}\n" + "".join(f"{n} {p}\n" for n, p in pages), "spec")

    def check(got, spec=spec, a_bytes=a_bytes, b_bytes=b_bytes):
        if got != "ok":
            return False, "ok", got
        o = tempfile.mkdtemp(dir=work)
        open(os.path.join(o, "x.db"), "wb").write(a_bytes)
        shutil.copy(spec + ".wal", os.path.join(o, "x.db-wal"))
        oc = sqlite3.connect(os.path.join(o, "x.db"), isolation_level=None)
        oc.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchall(); oc.close()
        mine = open(os.path.join(o, "x.db"), "rb").read()
        return mine == b_bytes, f"the later database, {len(b_bytes)} bytes", f"{len(mine)} bytes"
    cases.append(("walenc", spec, check, "wal-encode"))

CHILD = r'''
import sqlite3, os
c = sqlite3.connect("j.db", isolation_level=None)
c.execute("PRAGMA cache_size=%d")
c.execute("BEGIN")
c.execute("UPDATE t SET b = b || 'zz' WHERE a %% %d = 0")
c.execute("INSERT INTO t(b) SELECT b FROM t WHERE a < %d")
c.execute("DELETE FROM t WHERE a %% %d = 1")
os._exit(0)
'''
for i in range(12):
    d = tempfile.mkdtemp(dir=work)
    c = sqlite3.connect(os.path.join(d, "j.db"), isolation_level=None)
    c.execute("PRAGMA page_size=%d" % rng.choice([512, 1024, 4096]))
    c.execute("PRAGMA journal_mode=%s" % rng.choice(["DELETE", "PERSIST", "TRUNCATE"]))
    c.execute("CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)")
    c.execute("BEGIN"); fill(c, rng.randrange(50, 400)); c.execute("COMMIT"); c.close()
    run([sys.executable, "-c", CHILD % (rng.choice([1, 2, 5]), rng.randrange(2, 5), rng.randrange(10, 200), rng.randrange(3, 9))], cwd=d)
    snap_db, snap_j = path_for("db"), path_for("journal")
    shutil.copy(os.path.join(d, "j.db"), snap_db)
    jp = os.path.join(d, "j.db-journal")
    shutil.copy(jp, snap_j) if os.path.exists(jp) else open(snap_j, "wb").close()
    if i % 3 == 2:
        j = bytearray(open(snap_j, "rb").read())
        if len(j) > 2000:
            j[rng.randrange(600, len(j))] ^= 1 << rng.randrange(8)
            open(snap_j, "wb").write(bytes(j))
    o = tempfile.mkdtemp(dir=work)
    shutil.copy(snap_db, os.path.join(o, "j.db")); shutil.copy(snap_j, os.path.join(o, "j.db-journal"))
    oc = sqlite3.connect(os.path.join(o, "j.db")); oc.execute("SELECT count(*) FROM t").fetchall(); oc.close()
    spec = write(f"{snap_db} {snap_j}\n", "spec")
    cases.append(("journal", spec, file_check(spec + ".out", open(os.path.join(o, "j.db"), "rb").read()), "journal"))


# --- x86-64 lengths --------------------------------------------------------------------------------------

PREFIX_LINES = {"lock", "rep", "repne", "repe", "data16", "data32", "addr32", "rex64", "cs", "ds", "es", "ss", "fs", "gs", "xacquire", "xrelease",
                "notrack"}


def objdump_lengths(obj):
    out = run(["objdump", "-d", obj]).decode().splitlines()
    res, pending, inside = [], None, False
    for line in out:
        if line.startswith("Disassembly of section"):
            inside = line.strip().endswith("__TEXT,__text:")
            continue
        m = re.match(r"^\s*([0-9a-f]+):\s([0-9a-f ]+)\t(\S*)", line)
        if not m or not inside:
            continue
        off, n = int(m.group(1), 16), len(m.group(2).split())
        # objdump puts a lone prefix on a line of its own.
        if m.group(3) in PREFIX_LINES and n == 1:
            pending = (pending[0] if pending else off, (pending[1] if pending else 0) + 1)
            continue
        if pending:
            off, n, pending = pending[0], n + pending[1], None
        res.append(f"{off}:{n}")
    return ",".join(res)


corpus = [l for l in open(os.path.join(ROOT, "scripts/lib/b42/x86_corpus.txt")).read().splitlines() if l and not l.startswith("#")]
for i in range(14):
    if i < 8:
        flags = rng.choice([["-O0"], ["-O2"], ["-O3", "-mavx2", "-mfma", "-mbmi2"], ["-O2", "-mavx512f", "-mavx512bw", "-mavx512vl", "-mavx512dq"],
                            ["-Os", "-msse4.2", "-maes", "-mpclmul", "-msha"], ["-O1", "-mavx512fp16"]])
        obj = build("x86_64", "obj", flags[0], False, extra=flags[1:], src=c_source(switches=False, main=True))
    else:
        lines = corpus if i == 8 else rng.sample(corpus, rng.randrange(20, len(corpus)))
        lines = [l for l in lines if not re.match(r"^\d+:", l)] if i > 8 else lines
        obj = path_for("o")
        run(["clang", "-arch", "x86_64", "-c", write(".text\n" + "\n".join(lines) + "\n", "s"), "-o", obj])
    code = write(macho_text_section(obj), "text")
    want = objdump_lengths(obj)
    cases.append(("x86", code, (lambda got, want=want: (got == want, want[:200], got[:200])), "x86len"))


# --- Go tokens ---------------------------------------------------------------------------------------------

goroot = run(["go", "env", "GOROOT"]).decode().strip()
gofiles = sorted(os.path.join(dp, f) for dp, _, fs in os.walk(os.path.join(goroot, "src")) for f in fs if f.endswith(".go"))
TEXT_FRAGMENTS = ["package p\n", "func f() {\n", "}\n", "x := 0x1F_00\n", "y = 0b10_1 + 0o17 + 017 + 08 + 0x + 1e + 0x1.8p3 + 0x1.8 + 1__0 + 1_",
                  " + 0b12 + .5i + 1.e+5 + 0X_1p-2 + 0o8 + 0b.1 + 1p3 + 0_x\n", 's := "a\\x41\\u00e9\\U0001F600\\q\\400\\uD800"\n',
                  "r := 'a' + '\\n' + '' + 'ab' + '\\400' + '\\uD800' + '\\x4'\n", "/* c\r\n */", "// line c\r\n", "`raw\r\nx`", "\ufeff",
                  "é := 1\n", "ℵ0 := 2\n", "x٣ := 3\n", "return\n", "break // c\n", "a++ /* x\ny */ b\n", '"unterminated\n', "'\n", "`never",
                  "/* never", "…", "“q”", "a.b...c..\n", "<<= >>= &^= &^ <- && || != == := ~ %= ^= |=\n", "\r\n", "\t", " ", "x--\n", ")\n", "]\n",
                  "fallthrough\n", "continue/*\n*/\n", "a /* one line */\n", "//line foo.go:10\n", "/*line bar.go:3:4*/", "1.5e3i\n", "0x1p+2i\n",
                  "'\\''\n", '"\\""\n', "'\\U00110000'\n", "@", "#", "$", "?", "\\"]
# Bytes no UTF-8 decoder accepts, a NUL, and a BOM out of place.
RAW_FRAGMENTS = [b"\x00", b"\x80", b"\xe2\x82", b"\xed\xa0\x80", b"\xf4\x90\x80\x80", b"\xc0\xaf", b"\xef\xbb\xbf"]


def go_check(want):
    def check(got):
        return got == want, want[-60:], got[-60:]
    return check


go_inputs = rng.sample(gofiles, 16)
for i in range(30):
    parts = [rng.choice(TEXT_FRAGMENTS).encode() if rng.random() < 0.85 else rng.choice(RAW_FRAGMENTS) for _ in range(rng.randrange(3, 30))]
    if rng.random() < 0.2:
        parts.append(bytes(rng.randrange(256) for _ in range(rng.randrange(1, 20))))
    src = b"".join(parts)
    if i % 7 == 0:
        src = (b"\xff\xfe" if i % 14 == 0 else b"\xef\xbb\xbf") + src
    go_inputs.append(write(src, "go"))
go_answers = run([goscan] + go_inputs).decode().splitlines()
for k, (p, want) in enumerate(zip(go_inputs, go_answers)):
    cases.append(("go", p, go_check(want), "go-stdlib" if k < 16 else "go-fuzz"))


# --- run ----------------------------------------------------------------------------------------------------

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
    if thin or len(families) < int(os.environ.get("FAMILIES", "16")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
