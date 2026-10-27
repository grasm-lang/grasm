"""Pre-chained fixups, build versions, code requirements, archive
indexes, .lzma, PackBits, DWARF 5's side tables and compact unwind info,
each against the tool that reads them.

- Rebase and bind opcodes (images built for old systems, or with chained
  fixups turned off) against objdump's bind, lazy-bind and rebase tables,
  and against `dyld_info -fixups` where it reads them as objdump does: it
  mispairs symbols and addresses after a bind that steps backwards, and
  those images are counted and left out of that family. Weak-bind tables
  against `objdump --macho --weak-bind`, except where it misplaces every
  entry after a strong definition (its iterator re-applies the last
  step); those are counted and left out too.
- LC_BUILD_VERSION and LC_VERSION_MIN_* against `vtool -show-build`, on
  objects for every platform clang targets and on commands vtool writes.
- Code-signing requirements compiled to the bytes `csreq -b` writes and
  printed back as `csreq -t` prints them; requirement sets inside real
  signatures as `codesign -d -r-` lists them.
- `__.SYMDEF SORTED` indexes against `nm --print-armap`, and the index
  libtool writes rebuilt byte for byte.
- `.lzma` files against liblzma through Python (FORMAT_ALONE): sizes known
  and unknown, truncated, with flipped bits.
- PackBits rows encoded byte for byte as libtiff writes them (through
  Pillow), and decoded back.
- `.debug_addr`, `.debug_str_offsets`, `.debug_rnglists` and
  `.debug_names` in objects and .dSYMs against `dwarfdump`.
- `__unwind_info` in linked images, ours (two with more encodings than a
  compressed page can index, so ld writes regular pages) and the
  system's (fat ones too), against `objdump --unwind-info`.
"""
import glob
import io
import lzma
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tempfile

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b45")
harness = os.path.join(work, "b45test")
built = subprocess.run([GRASM, "build", "packages/b45test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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
        if mine == want:
            return True, "", ""
        a, b = mine.split("\n"), want.split("\n")
        k = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
        return False, f"line {k}: {(b[k] if k < len(b) else '<end>')[:200]}", (a[k] if k < len(a) else "<end>")[:200]
    return check


def file_check(out, want):
    def check(got):
        if want is None:
            return got == "ERR", "a refusal", got
        if got != "ok":
            return False, "ok", got
        mine = open(out, "rb").read()
        if mine == want:
            return True, "", ""
        at = next((i for i in range(min(len(mine), len(want))) if mine[i] != want[i]), min(len(mine), len(want)))
        return False, f"{len(want)} bytes", f"{len(mine)} bytes, first difference at {at}"
    return check


# --- programs ------------------------------------------------------------------------------------------

WORDS = ["alpha", "beta", "gamma", "delta", "kappa", "omega", "zeta", "theta"]
serial = [0]


def c_source(main=True, extern=False):
    out = ["#include <stdio.h>", "#include <string.h>", "#include <stdlib.h>"]
    if extern:
        out.append("extern char ext_a[]; extern char ext_b[]; extern int weak_one __attribute__((weak_import));")
        out.append("char *pa = ext_a + 200; char *pb = ext_b + 70000; char *pc = ext_a - 16; int *pw = &weak_one;")
    calls = []
    serial[0] += 40
    for i in range(serial[0], serial[0] + rng.randrange(4, 30)):
        n = rng.randrange(1, 9999)
        w = rng.choice(WORDS)
        kind = rng.randrange(12)
        if kind == 0:
            out.append(f"int g{i} = {n}; int *pg{i} = &g{i};")
        elif kind == 1:
            out.append(f"static int s{i} = {n}; int gs{i}(void) {{ return s{i}++; }}"); calls.append(f"gs{i}()")
        elif kind == 2:
            out.append(f"int c{i};")
        elif kind == 3:
            out.append(f"static long b{i}[{n % 50 + 1}]; long *pb{i} = &b{i}[{n % 50}];")
        elif kind == 4:
            out.append(f'const char *str{i} = "{w}{n}"; const char **pstr{i} = &str{i};')
        elif kind == 5:
            out.append(f"__attribute__((weak)) int w{i}(int x) {{ return x + {n}; }}"); calls.append(f"w{i}(argc)")
        elif kind == 6:
            out.append(f'__attribute__((visibility("hidden"))) int h{i}(int x) {{ return x * {n}; }}'); calls.append(f"h{i}(argc)")
        elif kind == 7:
            out.append(f"static int sf{i}(int x) {{ return x - {n}; }} int (*fp{i})(int) = sf{i};"); calls.append(f"fp{i}(argc)")
        elif kind == 8:
            out.append(f"int (*pp{i})(const char *) = puts; size_t (*sl{i})(const char *) = strlen; void *(*ma{i})(size_t) = malloc;")
        elif kind == 9:
            out.append(f"static inline int in{i}(int k) {{ return k * {n} ^ (k >> 3); }}\n"
                       f"int lp{i}(int n) {{ int s = 0; for (int k = 0; k < n; k++) {{ s += in{i}(k); if (s > {n}) s -= in{i}(s & 7); }} return s; }}")
            calls.append(f"lp{i}(argc)")
        elif kind == 10:
            out.append(f"double fl{i}(double a, double b) {{ return a * b + {n}.5; }}"); calls.append(f"(int)fl{i}(argc, 2.0)")
        else:
            out.append(f'int pr{i}(int x) {{ char buf[64]; memcpy(buf, "{w}", 5); buf[5] = 0; printf("%s %d\\n", buf, x); return (int)strlen(buf); }}')
            calls.append(f"pr{i}(argc)")
    if main:
        out.append("int main(int argc, char **argv) { int s = 0; " + "".join(f"s += {c}; " for c in calls) + "return s & 1; }")
    return "\n".join(out) + "\n"


def cpp_source(main=True):
    n = rng.randrange(3, 120)
    out = ["#include <cstdio>", "#include <cstdlib>", "#include <string>", "#include <vector>", "#include <map>", "#include <stdexcept>",
           "template <int N> struct K { static int v; static int f(int x) { return x + N + v; } };", "template <int N> int K<N>::v = N;",
           "inline int helper(int x) { return x * 2; }"]
    if rng.random() < 0.6:
        out.append("void *operator new(size_t n) { return malloc(n ? n : 1); }")
    if rng.random() < 0.5:
        out.append("void operator delete(void *p) noexcept { free(p); }")
    start = rng.randrange(1000)
    out.append("int (*tab[])(int) = {" + ",".join(f"K<{start + i}>::f" for i in range(n)) + "};")
    out.append("int *vt[] = {" + ",".join(f"&K<{start + i}>::v" for i in range(n)) + "};")
    out.append("int thrower(int c) { if (c > 5) throw std::length_error(\"x\"); return c; }")
    if main:
        out.append("int main(int c, char **v) { std::map<std::string, int> m; m[v[0]] = c; std::vector<std::string> xs(c, v[0]); int s = helper(c);"
                   f" for (int i = 0; i < {n}; i++) s += tab[i](c) + *vt[i];"
                   " try { s += thrower(c); } catch (std::exception &e) { return 1; } printf(\"%zu %d\\n\", xs.size() + m.size(), s); return 0; }")
    return "\n".join(out) + "\n"


def build(src, out_ext, cmd):
    s = write(src, "cpp" if "#include <cstdio>" in src else "c")
    out = path_for(out_ext)
    run(cmd + [s, "-o", out])
    return out


def has_dyld_info(p):
    return b"LC_DYLD_INFO" in run(["otool", "-l", p])


# --- rebase and bind opcodes --------------------------------------------------------------------------

OLD = [("x86_64", ["-mmacosx-version-min=10.9"]), ("x86_64", ["-mmacosx-version-min=10.13"]),
       ("arm64", ["-mmacosx-version-min=11.0", "-Wl,-no_fixup_chains"]), ("arm64", ["-mmacosx-version-min=13.0", "-Wl,-no_fixup_chains"]),
       ("x86_64", ["-mmacosx-version-min=12.0", "-Wl,-no_fixup_chains"])]
KINDS = {"exe": [], "dylib": ["-dynamiclib"], "bundle": ["-bundle"]}
UNDEF = ["-Wl,-U,_ext_a", "-Wl,-U,_ext_b", "-Wl,-U,_weak_one"]


def fixup_set(text):
    """Fixups printed as dyld_info prints them, reduced to what objdump also
    prints: kind, address, library, symbol and addend."""
    out = []
    for l in text.splitlines():
        m = re.match(r"^\S+\s+(?:\S+\s+)?0x([0-9A-F]+)\s+(bind|lazy-bind|rebase)  (.*)$", l)
        if not m:
            out.append("unreadable " + l)
            continue
        addr, kind, target = int(m.group(1), 16), m.group(2), m.group(3)
        if kind == "rebase":
            out.append(f"rebase {addr:x}")
            continue
        weak = target.endswith(" [weak-import]")
        target = target.removesuffix(" [weak-import]")
        m = re.match(r"^(.*?)/(\S+)(?: \+ 0x([0-9A-F]+))?$", target)
        if not m:
            out.append("unreadable " + l)
            continue
        addend = int(m.group(3) or "0", 16)
        addend = addend - (1 << 64) if addend >= 1 << 63 else addend
        lib = {"<weak-def-coalesce>": "weak"}.get(m.group(1), m.group(1).strip("<>"))
        out.append(f"{kind} {addr:x} {lib} {m.group(2)} {addend if kind == 'bind' else 0} {int(weak)}")
    return sorted(out)


def objdump_fixups(path):
    out = []
    for flag, kind in (("--bind", "bind"), ("--lazy-bind", "lazy-bind"), ("--rebase", "rebase")):
        lines = run(["objdump", "--macho", flag, path]).decode().splitlines()
        start = next(i for i, l in enumerate(lines) if l.startswith("segment ")) + 1
        for l in lines[start:]:
            f = l.split()
            if kind == "rebase":
                out.append(f"rebase {int(f[2], 16):x}")
            elif kind == "bind":
                out.append(f"bind {int(f[2], 16):x} {f[5]} {f[6]} {int(f[4])} {int(l.endswith('(weak_import)'))}")
            else:
                out.append(f"lazy-bind {int(f[2], 16):x} {f[3]} {f[4]} 0 {int(l.endswith('(weak_import)'))}")
    return sorted(out)


def set_check(want):
    def check(got):
        if got == "ERR":
            return False, "fixups", got
        mine = fixup_set(unhex(got))
        if mine == want:
            return True, "", ""
        extra = sorted(set(mine) - set(want))
        missing = sorted(set(want) - set(mine))
        return False, f"{len(want)} fixups, missing {missing[:3]}", f"{len(mine)}, extra {extra[:3]}"
    return check


misread = []
opcode_images, cpp_images = [], []
attempts = 0
while len(opcode_images) < 14:
    attempts += 1
    if attempts > 40:
        print(f"only {len(opcode_images)} of 40 images had opcodes dyld_info reads; see {work}"); sys.exit(1)
    arch, flags = OLD[attempts % len(OLD)]
    kind = rng.choice(list(KINDS))
    if attempts % 3 == 2:
        p = build(cpp_source(main=kind == "exe"), kind, ["clang++", "-std=c++17", "-w", rng.choice(["-O0", "-O1", "-O2"]), "-arch", arch] + flags + KINDS[kind])
        cpp_images.append(p)
    else:
        extern = rng.random() < 0.6
        p = build(c_source(main=kind == "exe", extern=extern), kind,
                  ["clang", "-w", "-fcommon", rng.choice(["-O0", "-O1", "-O2"]), "-arch", arch] + flags + KINDS[kind] + (UNDEF if extern else []))
    if not has_dyld_info(p):
        print(f"{p} came out with chained fixups; the opcode family would check nothing"); sys.exit(1)
    # Every image's binds, lazy binds and rebases against objdump's tables.
    tables = objdump_fixups(p)
    cases.append(("fixups", p, set_check(tables), "fixup-tables"))
    # dyld_info's own listing where it reads the opcodes as objdump does. It
    # mispairs symbols and addresses after a bind that steps backwards
    # (BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB with a negative step), and then
    # reads past the stream, so such images are left out of this family.
    r = subprocess.run(["dyld_info", "-fixups", p], capture_output=True)
    listing = "\n".join(l[8:] for l in r.stdout.decode(errors="replace").splitlines()[3:])
    if r.returncode != 0 or fixup_set(listing) != tables:
        misread.append(os.path.basename(p))
        continue
    opcode_images.append(p)
    cases.append(("fixups", p, text_check(listing), "dyld-fixups"))


def weak_table(path):
    lines = run(["objdump", "--macho", "--weak-bind", path]).decode().splitlines()
    return "\n".join(lines[next(i for i, l in enumerate(lines) if l.startswith("segment  section")):])


def strong_shift(table):
    """objdump's weak-bind iterator, having yielded a strong definition,
    re-applies the previous bind's step before the next entry, so every
    entry after one is misplaced by a pointer (the regular bind table and
    dyld_info put the same slots where this package does)."""
    seen = False
    for l in table.splitlines()[1:]:
        if l.lstrip().startswith("strong "):
            seen = True
        elif seen:
            return True
    return False


weak_cases, shifted = 0, []
pool = list(cpp_images)
attempts = 0
while weak_cases < 10:
    attempts += 1
    if attempts > 40:
        print(f"only {weak_cases} weak-bind tables objdump places soundly; see {work}"); sys.exit(1)
    if pool:
        p = pool.pop()
    else:
        arch, flags = OLD[rng.randrange(len(OLD))]
        kind = rng.choice(list(KINDS))
        p = build(cpp_source(main=kind == "exe"), kind, ["clang++", "-std=c++17", "-w", "-O1", "-arch", arch] + flags + KINDS[kind])
    want = weak_table(p)
    if len(want.splitlines()) < 2:
        print(f"{p} has an empty weak-bind table"); sys.exit(1)
    if strong_shift(want):
        shifted.append(os.path.basename(p))
        continue
    cases.append(("weak", p, text_check(want), "weak-binds"))
    weak_cases += 1


# --- build versions ------------------------------------------------------------------------------------

TRIPLES = ["arm64-apple-macosx14.4.1", "x86_64-apple-macosx10.10", "x86_64-apple-macosx10.15", "arm64-apple-ios11.0", "arm64-apple-ios17.0",
           "arm64-apple-ios17.0-simulator", "x86_64-apple-ios14.0-macabi", "arm64-apple-tvos11.0", "arm64-apple-tvos16.0", "arm64-apple-watchos9.0",
           "arm64_32-apple-watchos5.0", "arm64-apple-xros1.0", "arm64-apple-xros1.0-simulator", "arm64-apple-driverkit22.0",
           "x86_64-apple-tvos13.0-simulator", "arm64-apple-watchos10.0-simulator"]
PLATFORMS = ["macos", "ios", "tvos", "watchos", "bridgeos", "maccatalyst", "iossim", "8", "watchossim", "driverkit", "xros", "xrossim", "firmware", "sepos",
             "macos-exclavekit", "ios-exclavecore", "xros-exclavekit", "macos-kernelkit", "30", "31", "77"]
small = write("int f(int x) { return x + 1; }\n", "c")


def vtool_show(path):
    return "\n".join(run(["vtool", "-show-build", path]).decode().splitlines()[1:])


build_inputs = []
for t in rng.sample(TRIPLES, 8):
    o = path_for("o")
    run(["clang", "-target", t, "-c", small, "-o", o])
    build_inputs.append(o)
# Room in the header, so vtool can grow the commands it rewrites.
padded = [build(c_source(), "exe", ["clang", "-w", "-Wl,-headerpad,0x800"]),
          build(c_source(), "exe", ["clang", "-w", "-arch", "x86_64", "-mmacosx-version-min=10.9", "-Wl,-headerpad,0x800"])]
build_inputs += padded


def version():
    parts = [str(rng.randrange(0, 2000)), str(rng.randrange(0, 256)), str(rng.randrange(0, 256))]
    return ".".join(parts[:rng.randrange(1, 4)])


for i in range(6):
    o = path_for("o")
    cmd = ["vtool"]
    if i % 3 == 2:
        cmd += ["-set-version-min", rng.choice(["macos", "ios", "tvos", "watchos"]), version(), version()]
    else:
        cmd += ["-set-build-version", rng.choice(PLATFORMS), version(), version()]
        for t in rng.sample(["clang", "swift", "ld", str(rng.choice([4, 1024, 1031, rng.randrange(5, 2000)]))], rng.randrange(0, 4)):
            cmd += ["-tool", t, version()]
    run(cmd + ["-replace", "-output", o, rng.choice(padded)])
    build_inputs.append(o)
for p in build_inputs:
    cases.append(("build", p, text_check(vtool_show(p)), "build-version"))


# --- code-signing requirements -------------------------------------------------------------------------

VAL = ['"com.example.app"', 'a', '"a b"', 'com.ex.app', '"1.2"', '"B*"', '0x00ff', '"x\\"y"', 'X9', '"and"', 'abc.def', '""', 'v1.2.3']
KEY = ['CFBundleVersion', '"com.apple.security.app-sandbox"', 'subject.OU', 'subject.CN', 'A', '"a.b"']
FIELDS = ['subject.OU', 'subject.CN', 'field.1.2.840.113635.100.6.2.6', 'field.1.2.840.113635.100.6.1.13', 'policy.1.2.840.113635.100.5.1',
          'extension.2.5.29.19']


def hexs(n):
    return "".join(rng.choice("0123456789abcdef") for _ in range(n))


def req_primary(d):
    k = rng.randrange(16 if d < 3 else 12)
    if k == 0:
        return rng.choice(["always", "true", "never", "false", "notarized", "legacy"])
    if k == 1:
        return "identifier " + rng.choice(["", "= ", "== "]) + rng.choice(VAL[:5] + ['x', 'org.x'])
    if k == 2:
        return rng.choice(["anchor apple", "anchor apple generic", "anchor generic apple", "anchor apple foo", "anchor trusted", "certificate trusted"])
    if k == 3:
        return "cdhash " + rng.choice(["", "= "]) + f'H"{hexs(40)}"'
    if k == 4:
        return f"info[{rng.choice(KEY)}]" + rng.choice(["", " exists", " absent", " = " + rng.choice(VAL), " = *" + rng.choice(VAL[:3]),
                                                      " = " + rng.choice(VAL[:3]) + "*", " = *" + rng.choice(VAL[:3]) + "*", " ~ x", " < 2.0",
                                                      " >= \"1.2\"", " <= a", " > b"])
    if k == 5:
        return f"entitlement[{rng.choice(KEY)}]" + rng.choice(["", " exists", " = 1", " absent"])
    if k == 6:
        return f"certificate {rng.choice(['leaf', 'root', '1', '2', '-1', '0'])}[{rng.choice(FIELDS)}]" + rng.choice(["", " exists", " = X", ' = "ABCDE12345"'])
    if k == 7:
        return f"certificate {rng.choice(['leaf', 'root', '1'])} = H\"{hexs(40)}\""
    if k == 8:
        return f"cert {rng.choice(['leaf', '1'])} trusted"
    if k == 9:
        return f"platform = {rng.randrange(0, 20)}"
    if k == 10:
        return "(" + rng.choice(['foo', '"named code"']) + ")"
    if k == 11:
        return f'anchor H"{hexs(40)}"'
    if k in (12, 13):
        return "(" + req_expr(d + 1) + ")"
    return "! " + req_primary(d + 1)


def req_expr(d=0):
    terms = [" and ".join(req_primary(d) for _ in range(rng.randrange(1, 4 if d < 3 else 2))) for _ in range(rng.randrange(1, 3 if d < 3 else 2))]
    return " or ".join(terms)


def csreq_answer(text):
    out = path_for("req")
    r = subprocess.run(["csreq", "-r", "=" + text, "-b", out], capture_output=True)
    if r.returncode != 0:
        return "ERR"
    t = run(["csreq", "-r", out, "-t"]).decode().rstrip("\n")
    return open(out, "rb").read().hex() + "|" + t.encode().hex()


for i in range(10):
    texts = [req_expr() for _ in range(30)]
    want = ";".join(csreq_answer(t) for t in texts)
    cases.append(("req", write("\n".join(texts) + "\n", "txt"), (lambda got, want=want: (got == want, want[:200], got[:200])), "csreq-compile"))


def requirement_set(path):
    """The requirements blob in a thin image's signature, or None."""
    d = open(path, "rb").read()
    ncmds = struct.unpack("<I", d[16:20])[0]
    p = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack("<II", d[p:p + 8])
        if cmd == 0x1d:
            off, sz = struct.unpack("<II", d[p + 8:p + 16])
            blob = d[off:off + sz]
            for k in range(struct.unpack(">I", blob[8:12])[0]):
                slot, o = struct.unpack(">II", blob[12 + 8 * k:20 + 8 * k])
                if slot == 2:
                    n = struct.unpack(">I", blob[o + 4:o + 8])[0]
                    return blob[o:o + n]
        p += size
    return None


def thin(path):
    if open(path, "rb").read(4) not in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
        return path
    out = path_for("thin")
    run(["lipo", path, "-thin", "arm64e" if "arm64e" in run(["lipo", "-archs", path]).decode() else "arm64", "-output", out])
    return out


def codesign_reqs(path):
    # A "# designated => cdhash ..." line is codesign's own suggestion for a
    # set with no designated requirement; it is not in the blob.
    return "\n".join(l for l in run(["codesign", "-d", "-r-", path]).decode().splitlines() if not l.startswith("# "))


sets = 0
system = sorted(p for p in glob.glob("/bin/*") + glob.glob("/usr/bin/*") if os.path.isfile(p) and not os.path.islink(p))
rng.shuffle(system)
for p in system:
    if sets >= 6:
        break
    try:
        t = thin(p)
        blob = requirement_set(t)
    except Exception:
        continue
    if blob and len(blob) > 12:
        cases.append(("reqset", write(blob, "reqs"), text_check(codesign_reqs(t)), "csreq-set"))
        sets += 1
signable = [p for p in opcode_images if p.endswith((".exe", ".dylib"))] + padded
made = 0
while made < 8:
    kinds = rng.sample(["designated", "library", "host", "guest"], rng.randrange(1, 4))
    text = " ".join(f"{k} => {req_expr(1)}" for k in kinds)
    p = path_for("signed")
    shutil.copy(rng.choice(signable), p)
    if subprocess.run(["codesign", "-s", "-", "-f", "-r=" + text, p], capture_output=True).returncode != 0:
        continue
    blob = requirement_set(p)
    cases.append(("reqset", write(blob, "reqs"), text_check(codesign_reqs(p)), "csreq-set"))
    made += 1


# --- archive indexes ---------------------------------------------------------------------------------------

def armap(path):
    out = run(["nm", "--print-armap", path]).decode().split("\n\n")[0]
    return out.rstrip("\n")


libs = []
for i in range(12):
    arch = rng.choice(["arm64", "x86_64"])
    d = tempfile.mkdtemp(dir=work)
    objs = []
    for k in range(rng.randrange(1, 6)):
        name = rng.choice(["u", "unit_with_a_long_name_past_sixteen", "m", "obj with space"]) + f"{k}.o"
        o = os.path.join(d, name)
        run(["clang", "-arch", arch, "-w", "-fcommon", rng.choice(["-O0", "-O2"]), "-c", write(c_source(main=k == 0), "c"), "-o", o])
        objs.append(o)
    lib = os.path.join(d, "lib.a")
    if i % 4 == 3:
        run(["ar", "rcs", lib] + objs)
    else:
        run(["libtool", "-static", "-o", lib] + objs)
        libs.append(lib)
    cases.append(("armap", lib, text_check(armap(lib)), "armap"))
for lib in libs:
    cases.append(("ranlib", lib, (lambda got: (got == "same", "same", got)), "ranlib-write"))


# --- .lzma -----------------------------------------------------------------------------------------------

def sample_data():
    k = rng.randrange(5)
    if k == 0:
        return b""
    if k == 1:
        return bytes(rng.randrange(256) for _ in range(rng.randrange(1, 5000)))
    if k == 2:
        return (rng.choice(WORDS) + " ").encode() * rng.randrange(1, 20000)
    if k == 3:
        return open(rng.choice([p for p in glob.glob("/usr/share/dict/*") if os.path.isfile(p)] or [small]), "rb").read()[:rng.randrange(1000, 400000)]
    return b"".join(bytes([rng.randrange(256)]) * rng.randrange(1, 300) for _ in range(rng.randrange(1, 400)))


for i in range(36):
    data = sample_data()
    lc, lp = rng.choice([(3, 0), (0, 2), (1, 3), (4, 0), (0, 0), (2, 2)])
    filt = {"id": lzma.FILTER_LZMA1, "preset": rng.randrange(10), "lc": lc, "lp": lp, "pb": rng.randrange(5),
            "dict_size": rng.choice([4096, 1 << 16, 1 << 20, 1 << 23])}
    comp = lzma.compress(data, format=lzma.FORMAT_ALONE, filters=[filt])
    damage = i % 6
    if damage in (1, 4, 5):
        # The size in the header, which makes the end marker optional.
        comp = comp[:5] + struct.pack("<Q", len(data)) + comp[13:]
    if damage in (2, 4):
        comp = comp[:rng.randrange(13, len(comp))]
    elif damage in (3, 5):
        b = bytearray(comp)
        # A flipped bit; one time in three in the range coder's first
        # byte, which must be zero.
        b[13 if rng.random() < 0.34 else rng.randrange(13, len(b))] ^= 1 << rng.randrange(8)
        comp = bytes(b)
    # One stream, as liblzma reads it; what follows its end is not its business.
    dec = lzma.LZMADecompressor(format=lzma.FORMAT_ALONE)
    try:
        out = dec.decompress(comp)
        want = out if dec.eof else None
    except lzma.LZMAError:
        want = None
    p = write(comp, "lzma")
    cases.append(("lzma", p, file_check(p + ".out", want), "lzma-alone"))


# --- PackBits ---------------------------------------------------------------------------------------------

def row_bytes(n):
    out = bytearray()
    while len(out) < n:
        k = rng.randrange(4)
        if k == 0:
            out += bytes([rng.randrange(256)]) * rng.randrange(1, 300)
        elif k == 1:
            out += bytes(rng.randrange(256) for _ in range(rng.randrange(1, 200)))
        elif k == 2:
            a, b = rng.randrange(256), rng.randrange(256)
            out += bytes([a, a, b] * rng.randrange(1, 40))
        else:
            out += bytes(rng.choice([0, 255]) for _ in range(rng.randrange(1, 60)))
    return bytes(out[:n])


for i in range(10):
    mode = rng.choice(["L", "L", "RGB"])
    w, h = rng.randrange(1, 700), rng.randrange(1, 40)
    row = w * (3 if mode == "RGB" else 1)
    raw = b"".join(row_bytes(row) for _ in range(h))
    buf = io.BytesIO()
    Image.frombytes(mode, (w, h), raw).save(buf, format="TIFF", compression="packbits")
    buf.seek(0)
    im = Image.open(buf)
    blob = buf.getvalue()
    packed = b"".join(blob[o:o + n] for o, n in zip(im.tag_v2[273], im.tag_v2[279]))
    p = write(raw, "raw")
    cases.append((f"pbenc:{row}", p, file_check(p + ".out", packed), "packbits-encode"))
    q = write(packed, "pb")
    cases.append(("pbdec", q, file_check(q + ".out", raw), "packbits-decode"))


# --- DWARF 5 side tables ------------------------------------------------------------------------------------

def dwarfdump_sort(text):
    # dwarfdump lists a name index's abbreviations in a hash set's order.
    def fix(m):
        blocks = re.findall(r"    Abbreviation 0x[0-9a-f]+ \{\n(?:      .*\n)*?    \}\n", m.group(1))
        return "  Abbreviations [\n" + "".join(sorted(blocks)) + "  ]\n"
    return re.sub(r"  Abbreviations \[\n((?:    .*\n)*?)  \]\n", fix, text + "\n").rstrip("\n")


def dwarfdump(flags, path):
    text = "".join(run(["dwarfdump", f, path]).decode() for f in flags)
    return "\n".join(l for l in text.splitlines() if l and "file format" not in l and not l.endswith("contents:"))


def names_check(want):
    inner = text_check(want)

    def check(got):
        if got == "ERR":
            return False, want[:200], got
        return inner(dwarfdump_sort(unhex(got)).encode().hex())
    return check


def loops():
    # Vectorised loops with a counter scoped to them: lexical blocks whose
    # code is split, described by range lists.
    serial[0] += 40
    return "".join(f"int v{i}(int x) {{ int s = 0; for (int k = 0; k < x; k++) s += (k * {rng.randrange(2, 99)}) ^ {rng.randrange(999)}; return s + {i}; }}\n"
                   for i in range(serial[0], serial[0] + rng.randrange(4, 14)))


range_cases = [0]


def add_dwarf(p):
    cases.append(("addr", p, text_check(dwarfdump(["--debug-addr", "--debug-str-offsets"], p)), "debug-addr"))
    want = dwarfdump(["--debug-rnglists"], p)
    if want:
        cases.append(("rng", p, text_check(want), "rnglists"))
        range_cases[0] += 1
    cases.append(("names", p, names_check(dwarfdump_sort(dwarfdump(["--debug-names"], p))), "debug-names"))


for i in range(4):
    d = tempfile.mkdtemp(dir=work)
    objs = []
    for k in range(rng.randrange(2, 5)):
        src = os.path.join(d, f"unit{k}.c")
        open(src, "w").write(c_source(main=k == 0) + loops())
        obj = os.path.join(d, f"unit{k}.o")
        run(["clang", "-gdwarf-5", rng.choice(["-O2", "-O3"]), "-w", "-fcommon", "-c", src, "-o", obj])
        objs.append(obj)
    exe = os.path.join(d, "prog")
    run(["clang"] + objs + ["-o", exe])
    run(["dsymutil", exe, "-o", exe + ".dSYM"])
    add_dwarf(exe + ".dSYM/Contents/Resources/DWARF/prog")
objects = 0
while objects < 6 or range_cases[0] < 10:
    objects += 1
    if objects > 30:
        print(f"only {range_cases[0]} of the DWARF files had range lists"); sys.exit(1)
    arch = rng.choice(["arm64", "x86_64"])
    add_dwarf(build(c_source() + loops(), "o", ["clang", "-arch", arch, "-gdwarf-5", "-w", "-fcommon", rng.choice(["-O2", "-O3"]), "-c"]))


# --- compact unwind ---------------------------------------------------------------------------------------

def objdump_unwind(path):
    # `--arch=all`: without it, objdump shows a fat image's every slice only
    # when none is the host's own architecture, and just the arm64 one when
    # there is. Most of the system's fat binaries are x86_64 and arm64e, so
    # they agreed; /usr/bin/trace and a few others carry arm64 too, and one
    # drawn at random failed the gate on 2026-10-28 with every slice right.
    lines = run(["objdump", "--unwind-info", "--arch=all", path]).decode().splitlines()
    k = next((i for i, l in enumerate(lines) if l == "Contents of __unwind_info section:"), len(lines))
    return "\n".join(lines[k:])


unwind_images = []
for i in range(8):
    arch = rng.choice(["arm64", "x86_64"])
    if i % 2:
        unwind_images.append(build(cpp_source(), "exe", ["clang++", "-std=c++17", "-w", "-O1", "-arch", arch]))
    else:
        src = "\n".join(c_source(main=False) for _ in range(rng.randrange(1, 40))) + "int main(void) { return 0; }\n"
        flags = rng.choice([[], ["-fomit-frame-pointer"], ["-fno-omit-frame-pointer"], ["-Os"]])
        unwind_images.append(build(src, "exe", ["clang", "-w", "-fcommon", "-O2", "-arch", arch] + flags))
def many_encodings():
    # Frameless x86-64 functions with stack sizes and saved registers that
    # all differ: more encodings than a compressed page can index, so ld
    # writes regular pages.
    out = ["__attribute__((noinline)) int g(int x) { return x * 3; }"]
    n = rng.randrange(900, 1400)
    for i in range(n):
        regs = rng.randrange(0, 6)
        vals = " ".join(f"int a{k} = g(x + {k});" for k in range(regs))
        use = " + ".join([f"g(a{k})" for k in range(regs)] + ["b[x & 3]"])
        out.append(f"__attribute__((noinline)) int f{i}(int x) {{ volatile char b[{rng.randrange(1, 250) * 8}]; b[x & 3] = (char)x; {vals} return {use}; }}")
    out.append("int main(int c, char **v) { int s = 0; " + "".join(f"s += f{i}(c); " for i in range(n)) + "return s; }")
    return "\n".join(out) + "\n"


for i in range(2):
    p = build(many_encodings(), "exe", ["clang", "-w", "-arch", "x86_64", "-O2", "-fomit-frame-pointer"])
    if not re.search(r"\]: function offset=0x[0-9a-f]+, encoding=0x", objdump_unwind(p)):
        print(f"{p} came out without a regular second-level page"); sys.exit(1)
    unwind_images.append(p)
# And a few of the system's own, fat ones among them.
others = []
candidates = sorted(p for p in glob.glob("/usr/libexec/*") + glob.glob("/usr/bin/*") if os.path.isfile(p) and not os.path.islink(p))
rng.shuffle(candidates)
for p in candidates[:200]:
    if len(others) >= 3:
        break
    try:
        if objdump_unwind(p):
            others.append(p)
    except Exception:
        continue
for p in unwind_images + others:
    cases.append(("unwind", p, text_check(objdump_unwind(p)), "unwind-info"))


# --- run ----------------------------------------------------------------------------------------------------

def finish():
    answers = []
    for i in range(0, len(cases), 60):
        argv = []
        for op, path, _, _ in cases[i:i + 60]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            for op, path, _, _ in cases[i:i + 60]:
                one = subprocess.run([harness, op, path], capture_output=True, text=True)
                if one.returncode != 0:
                    print(f"harness crashed on {op} {path}")
                    break
            print(ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])
    if len(answers) != len(cases):
        print(f"{len(answers)} answers for {len(cases)} cases"); sys.exit(1)
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
    if misread:
        print(f"left out of dyld-fixups, dyld_info misreads a backward bind step: {len(misread)} images")
    if shifted:
        print(f"left out of weak-binds, objdump misplaces entries after a strong one: {len(shifted)} images")
    thin_fams = [k for k, (a, d) in families.items() if a + d < 8]
    if thin_fams or len(families) < int(os.environ.get("FAMILIES", "15")):
        print(f"too few cases to mean anything: {thin_fams} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
