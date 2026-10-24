"""B61's packages against the references they reproduce.

- Perl 5.34's File::Spec::Unix: canonpath (the XS one), catdir, catfile,
  splitdir, splitpath, catpath, file_name_is_absolute, no_upwards, and
  rel2abs and abs2rel with the current directory given.
- Perl 5.34's Time::Local: timegm, timegm_nocheck, timegm_modern and
  timegm_posix with the rolling century relative to a given year.
- Rust 1.99's Duration: {:?} with its spec, try_from_secs_f64, mul_f64,
  div_f64, as_secs_f64, div_duration_f64 and the checked and saturating
  arithmetic, through generated programs compiled by rustc.
- Rust 1.99's std::net addresses: FromStr of IpAddr, Ipv4Addr, Ipv6Addr
  and the socket addresses with their errors, Display with width and
  precision, and the stable classification methods.
- jq 1.7.1's path builtins: getpath, setpath, delpaths, paths, paths(f)
  for the selectors, pick, to_entries, from_entries, tostream, fromstream
  and truncate_stream, with their error messages.
- Perl 5.34's JSON::PP: encode with its layout and escaping options and
  decode with its acceptance options and error messages.
- SBCL 2.6.8's PRIN1 and PRINC of integers, ratios, floats, strings and
  characters under *print-base*, *print-radix* and
  *read-default-float-format*.
- Perl 5.34's Term::ANSIColor: color, colored (both forms, with
  $EACHLINE), uncolor, colorstrip and colorvalid.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B61_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B61_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B61_LIB") or os.path.join(ROOT, "scripts/lib/b61")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b61")
harness = os.path.join(work, "b61test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b61test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stdout[-800:] + built.stderr[-1500:]); sys.exit(1)

cases = []   # (family, op, path, want lines)
hx = lambda t: t.encode("utf-8").hex() or "-"


def case_file(text):
    p = os.path.join(work, f"c{len(cases)}_{rng.randrange(1 << 30)}.txt")
    open(p, "w", encoding="utf-8").write(text)
    return p


def dbits(v):
    return struct.unpack("<Q", struct.pack("<d", v))[0]


JAVA = os.path.join(work, "java")
os.makedirs(JAVA)
if os.path.isdir(os.path.join(LIB, "java")):
    for f in os.listdir(os.path.join(LIB, "java")):
        if f.endswith(".java"):
            shutil.copy(os.path.join(LIB, "java", f), JAVA)
if any(f.endswith(".java") for f in os.listdir(JAVA)):
    subprocess.run(["javac", "-d", JAVA] + [os.path.join(JAVA, f) for f in os.listdir(JAVA) if f.endswith(".java")], check=True)


def rust_tool(name, src):
    out = os.path.join(work, name + ".rbin")
    subprocess.run(["rustc", "-O", src, "-o", out], check=True)
    return out


def go_tool(name, src):
    mod = os.path.join(work, name)
    os.makedirs(mod)
    shutil.copy(src, os.path.join(mod, "main.go"))
    open(os.path.join(mod, "go.mod"), "w").write(f"module {name}\n\ngo 1.25\n")
    out = os.path.join(work, name + ".bin")
    subprocess.run(["go", "build", "-o", out, "."], cwd=mod, check=True)
    return out


def family(name, op, lines, oracle_argv, chunk=500):
    if not lines:
        return
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.split("\n")[:-1]
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))


# --- Perl's File::Spec::Unix, against Perl 5.34 -------------------------------------------------------------

FS_PIECES = ["a", "b", "c", "usr", "lib", ".", ".", "..", "..", "...", ".a", "a.", "é", "☺", "x y", "-", "\n", "\x00", "a\\b", "~", "*"]


def fs_path():
    r = rng.random()
    if r < 0.08:
        return rng.choice(["", "/", "//", ".", "..", "./", "/.", "/..", "/../", "./.", "../..", "///", "/./", "a/", "./a", "/..x", "/.x"])
    parts = [rng.choice(FS_PIECES) for _ in range(rng.randrange(0, 6))]
    seps = [rng.choice(["/", "/", "/", "//", "/./", "///"]) for _ in parts]
    out = rng.choice(["", "", "/", "./", "//", "/../", "././", "../"])
    for p, sep in zip(parts, seps):
        out += p + sep
    if parts and rng.random() < 0.6:
        out = out[: -len(seps[-1])]
    return out + rng.choice(["", "", "", "/", "/.", "/..", "/./"])


def fs_cwd():
    return rng.choice(["/", "/home/u", "/a/b/c", "/a", "//x/", "/usr/lib/", "/é/☺", "/a/../b", "/./a"])


def perlfilespec_line():
    h = lambda t: hx(t)
    hl = lambda xs: ",".join(hx(x) for x in xs) if xs else "~"
    r = rng.random()
    if r < 0.25:
        return f"cp {h(fs_path())}"
    if r < 0.35:
        return f"cd {hl([fs_path() for _ in range(rng.randrange(0, 4))])}"
    if r < 0.45:
        return f"cf {hl([fs_path() for _ in range(rng.randrange(0, 4))])}"
    if r < 0.5:
        return f"sd {h(fs_path())}"
    if r < 0.57:
        return f"sp {h(fs_path())} {rng.choice('001')}"
    if r < 0.62:
        return f"ct {h(rng.choice(['', 'C:']))} {h(fs_path())} {h(fs_path())}"
    if r < 0.65:
        return f"ab {h(fs_path())}"
    if r < 0.68:
        return f"nu {hl([rng.choice(['.', '..', '...', 'a', '', '.a', '..\n']) for _ in range(rng.randrange(0, 5))])}"
    if r < 0.8:
        return f"ra {h(fs_path())} {h(fs_path() if rng.random() < 0.7 else '')} {h(fs_cwd())}"
    return f"ar {h(fs_path())} {h(fs_path() if rng.random() < 0.7 else '')} {h(fs_cwd())}"


family("perlfilespec", "perlfilespec", [perlfilespec_line() for _ in range(int(os.environ.get("PERLFILESPEC_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlfilespec/oracle.pl")])


# --- Perl's Time::Local, against Perl 5.34 ------------------------------------------------------------------

TL_VARIANTS = ["timegm", "timegm", "timegm", "timegm_nocheck", "timegm_modern", "timegm_posix"]
TL_NOW = [2026, 2026, 1980, 1999, 2000, 2049, 2050, 2051, 2075, 2100, 1970, 2401]


def tl_field(lo, hi, frac_ok=True):
    r = rng.random()
    if r < 0.75:
        return str(rng.randrange(lo, hi + 1))
    if r < 0.85:
        return str(rng.choice([lo - 1, hi + 1, lo, hi, -1, 0]))
    if frac_ok and r < 0.97:
        return rng.choice(["%.1f", "%.2f", "%.3f"]) % rng.uniform(lo - 0.5, hi + 0.9)
    return rng.choice(["0.0", "-0.0", "60.0", "59.999", "1.5"]) if frac_ok else str(rng.randrange(-30, 30))


def tl_year():
    r = rng.random()
    if r < 0.35:
        return rng.randrange(0, 100)
    if r < 0.45:
        return rng.randrange(100, 1000)
    if r < 0.75:
        return rng.randrange(1000, 3000)
    if r < 0.85:
        return rng.choice([1900, 2000, 2100, 2400, 1600, 1752, 1970, 1969, 2038, 4000, 0, -1, -1900, -4713])
    return rng.randrange(-10 ** rng.randrange(2, 9), 10 ** rng.randrange(2, 9))


def perltimelocal_line():
    if rng.random() < 0.05:
        year = rng.choice([1900, 2000, 2100, 2400, 1600, 1700, 2004, 1999, 0, 100, 400, 96, 104, 2200, 1800])
        return " ".join([rng.choice(["timegm", "timegm_modern", "timegm_posix"]), str(rng.choice(TL_NOW)), "0", "0", "0", rng.choice(["29", "28", "30"]),
                         "1", str(year)])
    v = rng.choice(TL_VARIANTS)
    month = tl_field(0, 11, False) if v != "timegm_nocheck" or rng.random() < 0.6 else str(rng.randrange(-30, 30))
    year = tl_year()
    if rng.random() < 0.02:
        year = rng.choice([2147485547, 2147485548, -2147481748, 2147485546, 99999999, -99999999])
    mday = tl_field(1, 31) if rng.random() < 0.9 else str(rng.choice([29, 30, 31, 28]))
    return " ".join([v, str(rng.choice(TL_NOW)), tl_field(0, 59), tl_field(0, 59), tl_field(0, 23), mday, month, str(year)])


family("perltimelocal", "perltimelocal", [perltimelocal_line() for _ in range(int(os.environ.get("PERLTIMELOCAL_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perltimelocal/oracle.pl")])


# --- Rust's Duration, against rustc 1.99 ---------------------------------------------------------------------

RD_SECS = [0, 0, 0, 1, 1, 2, 59, 60, 3600, 86400, 10 ** 9, 2 ** 32, 2 ** 53, 2 ** 63, 2 ** 64 - 1, 2 ** 64 - 2, 18446744073709551615 // 2]
RD_NANOS = [0, 0, 1, 2, 5, 499, 500, 501, 999, 1000, 1001, 1500, 2500, 999999, 1000000, 1500000, 2500000, 100000000, 500000000, 999999999,
            999999500, 999950000, 1234567, 123456789, 5, 50, 15]


def rd_dur():
    s = rng.choice(RD_SECS) if rng.random() < 0.6 else rng.randrange(0, 10 ** rng.randrange(1, 20))
    n = rng.choice(RD_NANOS) if rng.random() < 0.6 else rng.randrange(0, 10 ** 9)
    return f"{min(s, 2 ** 64 - 1)} {n}"


def rd_float():
    r = rng.random()
    if r < 0.4:
        x = rng.choice([0.0, -0.0, 1.5, 0.5, 1e-9, 0.999e-9, 0.5e-9, 1.5e-9, 2.5e-9, 1e-10, 4.656612873077393e-10, 1.0 - 1e-16, 0.9999999995,
                        0.99999999949, 1.9999999995, 1e9, 1.8446744073709552e19, 1.8446744073709550e19, 9.2e18, float("inf"), float("nan"),
                        -1.0, -1e-300, 5e-324, 123.456, 1e15 + 0.5, 4503599627370495.5, 2.0 ** 52, 2.0 ** 63, 1e-7, 3.7, 6e-10, 7e-10,
                        8e-10, 9e-10, 5.000001e-10, 4.9e-10])
    elif r < 0.7:
        x = rng.uniform(0, 1) * 10.0 ** rng.randrange(-12, 20)
    else:
        x = struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    return "%016x" % dbits(x)


def rd_spec():
    spec = ""
    if rng.random() < 0.3:
        spec += rng.choice(["", "", "*", "-", "0", "é"]) + rng.choice("<^>")
    if rng.random() < 0.2:
        spec += rng.choice(["+", "-"])
    if rng.random() < 0.1:
        spec += "#"
    if rng.random() < 0.1:
        spec += "0"
    if rng.random() < 0.35:
        spec += str(rng.randrange(0, 25))
    if rng.random() < 0.4:
        spec += "." + str(rng.choice([0, 0, 1, 2, 3, 5, 8, 9, 10, 12, rng.randrange(0, 15)]))
    return spec


def rustduration_line():
    r = rng.random()
    if r < 0.4:
        return f"dbg {rd_dur()} {hx(rd_spec())}"
    if r < 0.6:
        return f"tf {rd_float()}"
    if r < 0.65:
        return f"f64 {rd_dur()}"
    if r < 0.8:
        return f"{rng.choice(['add', 'sub', 'sadd', 'ssub', 'absdiff', 'ddf'])} {rd_dur()} {rd_dur()}"
    if r < 0.9:
        return f"{rng.choice(['mul', 'div', 'smul'])} {rd_dur()} {rng.choice([0, 1, 2, 3, 7, 1000, 10 ** 9, 2 ** 32 - 1, rng.randrange(0, 2 ** 32)])}"
    return f"{rng.choice(['mulf', 'divf'])} {rd_dur()} {rd_float()}"


family("rustduration", "rustduration", [rustduration_line() for _ in range(int(os.environ.get("RUSTDURATION_LINES", "10000")))],
       ["python3", os.path.join(LIB, "rustduration/oracle.py")])


# --- Rust's std::net addresses, against rustc 1.99 ----------------------------------------------------------


def ip_octet():
    r = rng.random()
    if r < 0.9:
        return str(rng.choice([0, 1, 2, 10, 127, 168, 172, 192, 254, 255, rng.randrange(256)]))
    if r < 0.95:
        return rng.choice(["00", "01", "010", "000", "0255"])
    if r < 0.98:
        return str(rng.choice([256, 300, 999, 1000, 2550]))
    return rng.choice(["", "a", "-1", "+1", " 1", "1 ", "0x1"])


def ip_v4_text():
    n = 4 if rng.random() < 0.85 else rng.choice([3, 5, 1])
    return ".".join(ip_octet() for _ in range(n))


def ip_group():
    r = rng.random()
    if r < 0.9:
        return rng.choice(["0", "1", "ffff", "FFFF", "fe80", "db8", "2001", "0000", "00ab", "Ab", "a", "fc00", "ff02"]) if rng.random() < 0.5 else "%x" % rng.randrange(1 << rng.choice([4, 8, 16]))
    if r < 0.95:
        return "%X" % rng.randrange(65536)
    return rng.choice(["10000", "fffff", "00000", "g", "", "-1", "0x1"])


def ip_v6_text():
    n = rng.choice([8, 8, 8, 7, 6, 5, 3, 1, 9])
    groups = [ip_group() for _ in range(n)]
    if rng.random() < 0.25:
        groups[-1] = ".".join(str(rng.randrange(256)) for _ in range(4))
        if rng.random() < 0.5:
            groups[-1:] = groups[-1:]
    if rng.random() < 0.6:
        k = rng.randrange(0, len(groups) + 1)
        m = rng.randrange(0, 4)
        groups[k:k + m] = [""] if 0 < k < len(groups) else ["", ""] if len(groups) > 0 else ["", "", ""]
    if rng.random() < 0.15:
        groups = ["0", "0", "0", "0", "0", rng.choice(["ffff", "FFFF", "0", "fffe"]), ip_v4_text()] if rng.random() < 0.5 else ["", "", rng.choice(["ffff", "FFFF", "0"]), ip_v4_text()]
    t = ":".join(groups)
    if rng.random() < 0.05:
        t = rng.choice(["::", ":::", "::1", "1::", ":1", "1:", "1::2::3", "::ffff:", "::1.2.3.4", "::ffff:1.2.3.4", "::1:2:3:4:5:6:7", "1:2:3:4:5:6:7::",
                        "1:2:3:4:5:6::1.2.3.4", "::2:3:4:5:6:1.2.3.4", "1:2:3:4:5::1.2.3.4", "1.2.3.4::", "1:2:3:4:5:6:1.2.3.4"])
    return t


def ip_port():
    r = rng.random()
    if r < 0.8:
        return str(rng.choice([0, 1, 80, 443, 8080, 65535, rng.randrange(65536)]))
    return rng.choice(["", "65536", "080", "00000000080", "99999", "-1", "+1", "a", "4294967296"])


def ip_mutate(t):
    if t and rng.random() < 0.1:
        k = rng.randrange(len(t))
        op = rng.randrange(3)
        if op == 0:
            return t[:k] + t[k + 1:]
        c = rng.choice(list(":.[]%0123456789abcdefABCDEFg x/é"))
        return t[:k] + c + t[k + (1 if op == 1 else 0):]
    return t


def ip_parse_text(kind):
    if kind == "v4" or kind == "ip" and rng.random() < 0.5:
        return ip_v4_text()
    if kind in ("v6", "ip"):
        return ip_v6_text()
    v4 = rng.random() < 0.5 if kind == "sock" else kind == "sock4"
    if rng.random() < 0.1:
        v4 = not v4
    if v4:
        return ip_v4_text() + (":" + ip_port() if rng.random() < 0.9 else "")
    scope = ""
    if rng.random() < 0.3:
        scope = "%" + rng.choice(["0", "1", "7", "0001", "4294967295", "4294967296", "", "eth0", str(rng.randrange(1 << 32))])
    inner = ip_v6_text() + scope
    if rng.random() < 0.05:
        inner = ip_v4_text()
    return ("[" + inner + "]" if rng.random() < 0.92 else inner) + (":" + ip_port() if rng.random() < 0.9 else "")


def ip_spec():
    spec = ""
    if rng.random() < 0.3:
        spec += rng.choice(["", "", "*", "-", "0", "é", " "]) + rng.choice("<^>")
    if rng.random() < 0.1:
        spec += rng.choice(["+", "-"])
    if rng.random() < 0.1:
        spec += "#"
    if rng.random() < 0.1:
        spec += "0"
    if rng.random() < 0.5:
        spec += str(rng.randrange(0, 50))
    if rng.random() < 0.3:
        spec += "." + str(rng.randrange(0, 45))
    return spec


def ip_v6_bits():
    r = rng.random()
    if r < 0.3:
        segs = [rng.choice([0, 0, 0, 1, 0xffff, 0xfe80, 0xfc00, 0xfd12, 0xff02, 0xffff, 0xdb8, 0x2001, rng.randrange(65536)]) for _ in range(8)]
    elif r < 0.5:
        segs = [0] * 8
        for _ in range(rng.randrange(0, 5)):
            segs[rng.randrange(8)] = rng.choice([1, 0xabcd, 0x10, rng.randrange(65536)])
    elif r < 0.7:
        segs = [0, 0, 0, 0, 0, rng.choice([0, 0xffff, 0xffff, 1, 0xfffe]), rng.randrange(65536), rng.randrange(65536)]
        if rng.random() < 0.2:
            segs[rng.randrange(5)] = rng.choice([1, 0xffff])
    elif r < 0.75:
        segs = [0] * 7 + [rng.choice([0, 1, 1, 2])]
        if rng.random() < 0.3:
            segs[rng.randrange(7)] = 1
    elif r < 0.85:
        segs = [rng.choice([0xfe80, 0xfebf, 0xfec0, 0xfe7f, 0xfc00, 0xfdff, 0xfe00, 0xfbff, 0xff00, 0xfeff, 0xff0e]), 0, 0, 0, 0, 0, 0, rng.choice([0, 1, 2])]
    else:
        segs = [rng.randrange(65536) for _ in range(8)]
    return "".join("%04x" % g for g in segs)


IP_V4_PREFIXES = [[10], [172, 16], [172, 31], [172, 15], [172, 32], [192, 168], [192, 169], [169, 254], [169, 253], [192, 0, 2], [198, 51, 100],
                  [203, 0, 113], [192, 0, 3], [198, 51, 101], [127], [0, 0, 0, 0], [255, 255, 255, 255], [255, 255, 255, 254], [224], [239], [223], [240],
                  [0, 0, 0, 1]]


def ip_v4_bits():
    if rng.random() < 0.5:
        o = rng.choice(IP_V4_PREFIXES)[:]
        while len(o) < 4:
            o.append(rng.choice([0, 1, 255, rng.randrange(256)]))
        return str((o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3])
    if rng.random() < 0.6:
        o = [rng.choice([0, 10, 127, 169, 172, 192, 198, 203, 223, 224, 239, 240, 255, rng.randrange(256)]),
             rng.choice([0, 15, 16, 31, 32, 51, 168, 254, 255, rng.randrange(256)]), rng.choice([0, 2, 100, 113, 255, rng.randrange(256)]),
             rng.choice([0, 1, 255, rng.randrange(256)])]
        return str((o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3])
    return str(rng.choice([0, 0xffffffff, rng.randrange(1 << 32)]))


def rustipaddr_line():
    r = rng.random()
    if r < 0.5:
        kind = rng.choice(["ip", "v4", "v6", "v6", "sock", "sock4", "sock6"])
        return f"p {kind} {hx(ip_mutate(ip_parse_text(kind)))}"
    if r < 0.6:
        return f"f4 {ip_v4_bits()} {hx(ip_spec())}"
    if r < 0.75:
        return f"f6 {ip_v6_bits()} {hx(ip_spec())}"
    if r < 0.8:
        return f"fs4 {ip_v4_bits()} {rng.choice([0, 80, 65535, rng.randrange(65536)])} {hx(ip_spec())}"
    if r < 0.85:
        return f"fs6 {ip_v6_bits()} {rng.choice([0, 80, 65535, rng.randrange(65536)])} {rng.choice([0, 0, 1, 4294967295, rng.randrange(1 << 32)])} {hx(ip_spec())}"
    if r < 0.92:
        return f"c4 {ip_v4_bits()}"
    return f"c6 {ip_v6_bits()}"


family("rustipaddr", "rustipaddr", [rustipaddr_line() for _ in range(int(os.environ.get("RUSTIPADDR_LINES", "10000")))],
       ["python3", os.path.join(LIB, "rustipaddr/oracle.py")])


# --- jq's path builtins, against jq 1.7.1 ------------------------------------------------------------------

JQ_KEYS = ["a", "b", "c", "key", "value", "name", "Key", "Value", "Name", "k", "v", "", "é", "a b", "\"q\"", "x\u0000y", "0", "long key of thirty two bytes!!!"]


def jq_number():
    r = rng.random()
    if r < 0.6:
        return str(rng.choice([0, 1, 2, 3, -1, -2, 5, 10, 100, rng.randrange(-50, 50)]))
    return rng.choice(["1.5", "-0", "0.0", "1.0", "2.000", "1e2", "1E+2", "-1.5e-3", "0.1", "1e-7", "0.0000001", "123456789012345678901234567890",
                       "3.14159", "-2.5", "1.0e1", "0e5", "2.5", "-0.5", "0.5", "1.9999", "-1.0"])


def jq_string():
    r = rng.random()
    if r < 0.7:
        return rng.choice(JQ_KEYS)
    return "".join(rng.choice(["a", "b", "é", "😀", "\n", "\t", "\\", "\"", "\u007f", "\u0001", "/", " ", " "]) for _ in range(rng.randrange(0, 20)))


def jq_tree(depth):
    r = rng.random()
    if depth <= 0 or r < 0.35:
        return rng.choice([("num", jq_number()), ("str", jq_string()), None, True, False, ("num", jq_number()), ("str", jq_string())])
    if r < 0.65:
        return [jq_tree(depth - 1) for _ in range(rng.randrange(0, 5))]
    keys = rng.sample(JQ_KEYS[:12], rng.randrange(0, 5))
    pairs = [(k, jq_tree(depth - 1)) for k in keys]
    if pairs and rng.random() < 0.05:
        pairs.append((pairs[0][0], jq_tree(depth - 1)))
    return {"pairs": pairs}


def jq_json_string(t):
    out = ['"']
    for ch in t:
        o = ord(ch)
        if ch == '"' or ch == '\\':
            out.append('\\' + ch)
        elif o < 32 or (rng.random() < 0.1 and o < 0x10000):
            out.append('\\u%04x' % o)
        elif o >= 0x10000 and rng.random() < 0.3:
            v = o - 0x10000
            out.append('\\u%04x\\u%04X' % (0xD800 + (v >> 10), 0xDC00 + (v & 0x3FF)))
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def jq_text(t):
    if t is None:
        return "null"
    if t is True:
        return "true"
    if t is False:
        return "false"
    if isinstance(t, tuple):
        return t[1] if t[0] == "num" else jq_json_string(t[1])
    if isinstance(t, list):
        return "[" + ",".join(jq_text(x) for x in t) + "]"
    return "{" + ",".join(jq_json_string(k) + ":" + jq_text(v) for k, v in t["pairs"]) + "}"


def jq_existing_paths(t, prefix, out):
    out.append(prefix)
    if isinstance(t, list):
        for i, x in enumerate(t):
            jq_existing_paths(x, prefix + [("num", str(i))], out)
    elif isinstance(t, dict):
        for k, v in t["pairs"]:
            jq_existing_paths(v, prefix + [("str", k)], out)


def jq_slice():
    pairs = []
    if rng.random() < 0.9:
        pairs.append(("start", rng.choice([None, ("num", str(rng.randrange(-4, 6))), ("num", rng.choice(["1.5", "-1.5", "0.5", "2.7", "1e1"])),
                                           ("str", "1")]) if rng.random() < 0.95 else True))
    if rng.random() < 0.9:
        pairs.append(("end", rng.choice([None, ("num", str(rng.randrange(-4, 7))), ("num", rng.choice(["1.5", "-0.5", "2.2", "100", "0.5", "-1.5", "3.01"]))])))
    if rng.random() < 0.1:
        pairs.append(("x", ("num", "1")))
    rng.shuffle(pairs)
    return {"pairs": pairs}


def jq_key():
    r = rng.random()
    if r < 0.45:
        return ("str", rng.choice(JQ_KEYS))
    if r < 0.8:
        return ("num", rng.choice([str(rng.randrange(-6, 8)), "1.5", "-1.5", "0.9", "-0", "1e1", "2.0"]))
    if r < 0.92:
        return jq_slice()
    return rng.choice([None, True, False, [("num", "1")], [], [("num", "1"), ("num", "2")]])


def jq_path(t):
    out = []
    jq_existing_paths(t, [], out)
    p = list(rng.choice(out))
    r = rng.random()
    if r < 0.3:
        p = p + [jq_key()]
    elif r < 0.45 and p:
        p[rng.randrange(len(p))] = jq_key()
    elif r < 0.55:
        p = [jq_key() for _ in range(rng.randrange(0, 4))]
    if len(p) > 0 and rng.random() < 0.1:
        p = p[:-1]
    return p


def jq_tame(t):
    """A tree used as a path: no index that would make jq allocate a huge array."""
    if isinstance(t, tuple) and t[0] == "num" and abs(float(t[1])) > 50:
        return ("num", "7")
    if isinstance(t, list):
        return [jq_tame(x) for x in t]
    if isinstance(t, dict):
        return {"pairs": [(k, jq_tame(v)) for k, v in t["pairs"]]}
    return t


def jq_sibling_key():
    r = rng.random()
    if r < 0.6:
        return ("num", str(rng.randrange(-5, 6)))
    if r < 0.75:
        return ("num", rng.choice(["1.5", "-1.5", "-0.5", "2.9"]))
    if r < 0.9:
        return jq_slice()
    return rng.choice([None, False, True, ("str", "a"), [("num", "0")]])


def jq_paths_value(t):
    if rng.random() < 0.4:
        out = []
        jq_existing_paths(t, [], out)
        base = list(rng.choice(out))
        return [base + [jq_sibling_key()] for _ in range(rng.randrange(1, 5))]
    return [jq_path(t) for _ in range(rng.randrange(0, 5))]


def jq_stream(t, prefix, out):
    kids = []
    if isinstance(t, list):
        kids = [(("num", str(i)), x) for i, x in enumerate(t)]
    elif isinstance(t, dict):
        kids = [(("str", k), v) for k, v in t["pairs"]]
    for k, v in kids:
        jq_stream(v, prefix + [k], out)
    if kids:
        out.append([prefix + [kids[-1][0]]])
    else:
        out.append([prefix, t])


def jq_events():
    out = []
    for _ in range(rng.randrange(1, 3)):
        jq_stream(jq_tree(rng.randrange(0, 4)), [], out)
    r = rng.random()
    if r < 0.3 and out:
        del out[rng.randrange(len(out))]
    elif r < 0.45 and out:
        k = rng.randrange(len(out))
        out[k] = rng.choice([[], [jq_path(jq_tree(2))], [jq_path(jq_tree(2)), jq_tree(1), ("num", "1")], [jq_tame(jq_tree(1)), jq_tree(1)], [None, ("num", "1")], [("num", "2"), ("num", "1")],
                             [("str", "ab"), ("num", "1")], [True], [{"pairs": [("a", ("num", "1"))]}, ("num", "3")]])
    elif r < 0.55 and len(out) > 1:
        i, j = rng.randrange(len(out)), rng.randrange(len(out))
        out[i], out[j] = out[j], out[i]
    return out


def jq_entries():
    r = rng.random()
    if r < 0.15:
        return rng.choice([None, ("num", "5"), ("str", "abc"), True, [], {"pairs": []}, jq_tree(2)])
    items = []
    for _ in range(rng.randrange(0, 5)):
        if rng.random() < 0.1:
            items.append(rng.choice([None, ("num", "1"), ("str", "s"), [("num", "1")], True, False]))
            continue
        names = rng.sample(["key", "Key", "name", "Name", "k", "value", "Value", "v"], rng.randrange(0, 5))
        pairs = [(n, rng.choice([("str", rng.choice(JQ_KEYS[:6])), ("str", rng.choice(JQ_KEYS[:6])), None, False, True, ("num", "0"),
                                 [("num", "1")], ("str", "a long key with many characters")])) for n in names]
        items.append({"pairs": pairs})
    if rng.random() < 0.15:
        return {"pairs": [(rng.choice(JQ_KEYS[:5]), x) for x in items]}
    return items


def jqpath_line():
    r = rng.random()
    h = lambda t: hx(jq_text(t))
    if r < 0.15:
        t = jq_tree(3)
        return f"get {h(t)} {h(jq_path(t) if rng.random() < 0.95 else jq_tame(jq_tree(1)))}"
    if r < 0.3:
        t = jq_tree(3) if rng.random() < 0.9 else None
        return f"set {h(t)} {h(jq_path(t) if rng.random() < 0.95 else jq_tame(jq_tree(1)))} {h(jq_tree(1))}"
    if r < 0.45:
        t = jq_tree(3)
        ps = jq_paths_value(t) if rng.random() < 0.95 else jq_tame(rng.choice([jq_tree(1), [jq_tree(1), [("str", "a")]]]))
        return f"del {h(t)} {h(ps)}"
    if r < 0.5:
        return f"paths {h(jq_tree(3))}"
    if r < 0.58:
        sel = rng.choice(["values", "nulls", "booleans", "numbers", "strings", "arrays", "objects", "iterables", "scalars"])
        return f"pathsf {h(jq_tree(3))} {h(('str', sel))}"
    if r < 0.65:
        return f"te {h(jq_tree(3) if rng.random() < 0.8 else rng.choice([None, ('num', '1'), ('str', 'ab'), True]))}"
    if r < 0.75:
        return f"fe {h(jq_entries())}"
    if r < 0.83:
        t = jq_tree(3)
        return f"pick {h(t)} {h(jq_paths_value(t))}"
    if r < 0.88:
        return f"ts {h(jq_tree(3))}"
    if r < 0.95:
        return f"fs {h(jq_events())}"
    return f"tr {h(jq_events())} {h(rng.choice([('num', '0'), ('num', '1'), ('num', '2'), ('num', '3'), ('num', '-1'), ('num', '1.5'), None, ('str', 'a')]))}"


family("jqpath", "jqpath", [jqpath_line() for _ in range(int(os.environ.get("JQPATH_LINES", "10000")))],
       ["python3", os.path.join(LIB, "jqpath/oracle.py")])


# --- Perl's JSON::PP, against Perl 5.34 ---------------------------------------------------------------------

PJ_CHARS = ["a", "b", "Z", "0", "9", " ", "\"", "\\", "/", "\n", "\r", "\t", "\f", "\b", "\x00", "\x01", "\x0b", "\x1f", "\x7f", "\x80", "\xe9",
            "\xff", "\u0100", "\u263a", "\u2028", "\U0001f600", "\U00010000", "'", "{", "}", "[", "]", ",", ":", "#"]


def pj_text(n=None):
    n = rng.randrange(0, 8) if n is None else n
    if rng.random() < 0.15:
        return rng.choice(["10", "1.5", "-3", "1e5", "", "true", "null", "0", " 1"])
    return "".join(rng.choice(PJ_CHARS) for _ in range(n))


def pj_double():
    r = rng.random()
    if r < 0.5:
        return rng.choice([0.5, 1.5, -2.25, 3.0, -0.0, 0.0, 1e15, 1e16, 1e17, 2.0 ** 53, 2.0 ** 53 - 1, 2.0 ** 62, -2.0 ** 62, 2.0 ** 62 - 1024,
                           4e18, 5e18, -1e17, 1e300, 1e-5, 1e-300, 5e-324, 0.1, 1 / 3, 1e21, 123456789012345.0, 1234567890123456.0,
                           float("inf"), float("-inf"), float("nan"), 2.0 ** 64, 1.8e19, -9.3e18])
    if r < 0.8:
        return rng.uniform(-1, 1) * 10.0 ** rng.randrange(-10, 25)
    return float(rng.randrange(-10 ** rng.randrange(1, 19), 10 ** rng.randrange(1, 19)))


def pj_scalar():
    r = rng.random()
    if r < 0.1:
        return ["u"]
    if r < 0.17:
        return [rng.choice(["t", "f"])]
    if r < 0.4:
        v = rng.choice([0, 1, -1, 42, 2 ** 31, 2 ** 53 + 1, 2 ** 63 - 1, -2 ** 63, 2 ** 64 - 1, 2 ** 63, rng.randrange(-10 ** 6, 10 ** 6)])
        return [f"i:{v}"]
    if r < 0.65:
        return ["x:%016x" % dbits(pj_double())]
    return ["s:" + hx(pj_text())]


def pj_value(depth, keys_seen):
    r = rng.random()
    if depth <= 0 or r < 0.4:
        return pj_scalar()
    if r < 0.7:
        out = ["["]
        for _ in range(rng.randrange(0, 4)):
            out += pj_value(depth - 1, keys_seen)
        return out + ["]"]
    out = ["{"]
    n = rng.randrange(0, 4)
    keys = [pj_text(rng.randrange(0, 4)) for _ in range(n)]
    if n and rng.random() < 0.1:
        keys.append(keys[0])
    keys_seen.append(len(set(keys)))
    for k in keys:
        out += ["k:" + hx(k)] + pj_value(depth - 1, keys_seen)
    return out + ["}"]


PJ_ENC_OPTS = ["ascii", "latin1", "utf8", "indent", "pretty", "space_before", "space_after", "escape_slash"]


def pj_encode_line():
    seen = []
    toks = pj_value(rng.randrange(0, 4), seen)
    opts = [o for o in PJ_ENC_OPTS if rng.random() < 0.18]
    if any(n > 1 for n in seen) or rng.random() < 0.5:
        opts.append("canonical")
    if rng.random() < 0.1:
        opts.append("nonref=0")
    if rng.random() < 0.15:
        opts.append(f"indent_length={rng.randrange(0, 16)}")
    if rng.random() < 0.1:
        opts.append(f"max_depth={rng.randrange(0, 4)}")
    return f"enc {','.join(opts) or '-'} {' '.join(toks)}"


def pj_json_string(t, quote='"'):
    out = [quote]
    for ch in t:
        o = ord(ch)
        if ch == quote or ch == "\\":
            out.append("\\" + ch)
        elif o < 32:
            out.append(rng.choice(["\\u%04x" % o, "\\u%04X" % o]) if ch not in "\n\t" or rng.random() < 0.7 else {"\n": "\\n", "\t": "\\t"}[ch])
        elif o > 127 and rng.random() < 0.3:
            if o > 0xffff:
                v = o - 0x10000
                out.append("\\u%04x\\u%04x" % (0xD800 + (v >> 10), 0xDC00 + (v & 0x3FF)))
            else:
                out.append("\\u%04x" % o)
        else:
            out.append(ch)
    out.append(quote)
    return "".join(out)


def pj_number_text():
    r = rng.random()
    if r < 0.5:
        return rng.choice(["0", "-0", "1", "-1", "10", "1.5", "-2.50", "1e2", "1E2", "1e+2", "1e-2", "1.5e3", "0.1", "1e15", "1e16", "1e19", "1e20",
                           "-1e19", "1e400", "-1e400", "1e-400", "0e0", "-0.0", "2.0", "18446744073709551615", "18446744073709551616",
                           "-9223372036854775808", "-9223372036854775809", "99999999999999999999", "123456789012345678901",
                           "-12345678901234567890", "12345678901234567890", "9007199254740993", "9007199254740993e0", "1.0000000000000002",
                           "123456789.123456789", "4.35", "0.000001", "0.0000001", "1e21", "12e20", "15e14", "9e18", "9.3e18e"])
    if r < 0.8:
        return str(rng.randrange(-10 ** rng.randrange(1, 22), 10 ** rng.randrange(1, 22)))
    m = "%d.%d" % (rng.randrange(0, 10 ** rng.randrange(1, 10)), rng.randrange(0, 10 ** rng.randrange(1, 10)))
    if rng.random() < 0.5:
        m += rng.choice(["e", "E"]) + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 30))
    return rng.choice(["", "-"]) + m


def pj_json(depth, relaxed):
    r = rng.random()
    ws = lambda: rng.choice(["", "", "", " ", "\n", "\t", "\r\n  "]) + (rng.choice(["/*c*/", "// x\n", "#c\n", "/**/ "]) if relaxed and rng.random() < 0.1 else "")
    if depth <= 0 or r < 0.4:
        k = rng.random()
        if k < 0.3:
            return pj_number_text()
        if k < 0.7:
            return pj_json_string(pj_text())
        return rng.choice(["true", "false", "null"])
    if r < 0.7:
        items = [ws() + pj_json(depth - 1, relaxed) + ws() for _ in range(rng.randrange(0, 4))]
        tail = "," if relaxed and items and rng.random() < 0.3 else ""
        return "[" + ",".join(items) + tail + ws() + "]"
    items = []
    for _ in range(rng.randrange(0, 4)):
        items.append(ws() + pj_json_string(pj_text(rng.randrange(0, 4))) + ws() + ":" + ws() + pj_json(depth - 1, relaxed) + ws())
    tail = "," if relaxed and items and rng.random() < 0.3 else ""
    return "{" + ",".join(items) + tail + ws() + "}"


PJ_SNIPPETS = ["", " ", "[", "]", "{", "}", ",", ":", "\"", "'", "\\", "-", "0", "1", "01", "0a", "0e", "1.", "1.e5", "1e", "1e+", ".5", "tru", "nul",
               "fals", "falsey", "nulls", "true1", "x", "/*", "*/", "//", "#", "\\u", "\\ud83d", "\\udc00", "\\ud83d\\u0041", "\\x", "\t", "\x01",
               "(", "é", "\u263a", "\ufeff", "'a'", "{a:1}", "{$b_1:2}", "[1,]", "{\"a\":1,}", "[1 2]", "{\"a\" 1}", "{\"a\":1 \"b\":2}"]


def pj_mutate(t):
    for _ in range(rng.randrange(1, 3)):
        r = rng.random()
        k = rng.randrange(len(t) + 1)
        if r < 0.3 and t:
            t = t[:k] + t[k + 1:]
        elif r < 0.7:
            t = t[:k] + rng.choice(PJ_SNIPPETS) + t[k:]
        else:
            t = t[:k]
    return t


PJ_DEC_OPTS = ["relaxed", "loose", "allow_singlequote", "allow_barekey", "utf8"]


def pj_decode_line():
    opts = [o for o in PJ_DEC_OPTS if rng.random() < 0.2]
    relaxed = "relaxed" in opts
    r = rng.random()
    if r < 0.45:
        text = pj_json(rng.randrange(0, 4), relaxed)
    elif r < 0.85:
        text = pj_mutate(pj_json(rng.randrange(0, 4), relaxed))
    else:
        text = rng.choice(["", " ", "{a:1, b_c:[1], $d:'x'}", "{'a':'b\\'c'}", "[1,2,]", "{\"a\":1,}", "# c\n[1]", "[1] // x", "/* a */ [1]",
                           "[\"a\tb\"]", "[\"\\x41\"]", "[\"a\x01\"]", "['a\"b']", "[\"\\ud83d\\ude00\"]", "[\"\\ud83d\"]", "[\"\\ude00\"]",
                           "[\"\\ud83dx\"]", "[\"\\u12\"]", "{\"a\":1,\"a\":2}", "[[[[[1]]]]]", "[1]x", "\"a\"", "1", "null", "true", " [ ] ",
                           "{é:1}", "[\"é\\é\"]", "[/*x", "[1,/*x*/2]", "{\"a\"/**/:1}", "[1e5,2.0,-0,1E400]"]) + pj_mutate("") * 0
    if rng.random() < 0.1:
        opts.append("nonref=0")
    if rng.random() < 0.1:
        opts.append(f"max_depth={rng.randrange(0, 4)}")
    if "utf8" in opts:
        b = text.encode("utf-8")
        if rng.random() < 0.15 and b:
            k = rng.randrange(len(b) + 1)
            b = b[:k] + bytes([rng.choice([0x80, 0xc3, 0xe2, 0xed, 0xf0, 0xf5, 0xff, 0xc0, 0x00])]) + b[k:]
        if len(b) >= 4 and (b[0] == 0 or b[1] == 0):
            b = b"[[" + b
        h = b.hex() or "-"
    else:
        h = hx(text)
    return f"dec {','.join(opts) or '-'} {h}"


def perljsonpp_line():
    return pj_encode_line() if rng.random() < 0.45 else pj_decode_line()


# Pinned: decimals that round to an integral double past 2^53. Perl reads a
# decimal point without an exponent as "not an integer" and keeps a double;
# through an exponent the same value becomes an integer. A random seed found
# the first shape once in many runs, so the boundary is pinned on both sides.
PJ_PINNED = ["99999999999999999.5999", "99999999999999999.0", "[99999999999999999.5999]", "-99999999999999999.5",
             "1.0e17", "99999999999999999.5999e0", "9007199254740994.0e0", "9007199254740992.0e0", "9007199254740994.0",
             "1.0e16", "1.0e15", "123.0", "123.0e0", "1e17"]

family("perljsonpp", "perljsonpp", [f"dec - {hx(t)}" for t in PJ_PINNED] +
       [perljsonpp_line() for _ in range(int(os.environ.get("PERLJSONPP_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perljsonpp/oracle.pl")])


# --- SBCL's PRIN1 and PRINC, against SBCL 2.6.8 ------------------------------------------------------------


def lp_single_bits():
    r = rng.random()
    if r < 0.35:
        x = rng.choice([0.0, -0.0, 1.5, 0.1, 1e7, 9999999.0, 1e-3, 9.999e-4, 123456.7, 1e38, 1e-38, 1e-45, 3.4028235e38, -2.5, 1.0, 100.0,
                        float("inf"), float("-inf")])
        return "%08x" % struct.unpack("<I", struct.pack("<f", x))[0]
    if r < 0.4:
        return rng.choice(["7fc00000", "ffc00000", "7f800001", "7fa00000"])
    if r < 0.75:
        x = rng.uniform(-1, 1) * 10.0 ** rng.randrange(-12, 12)
        return "%08x" % struct.unpack("<I", struct.pack("<f", x))[0]
    return "%08x" % rng.randrange(1 << 32)


def lp_double_bits():
    r = rng.random()
    if r < 0.35:
        return "%016x" % dbits(rng.choice([0.0, -0.0, 1.5, 0.1, 1e7, 9999999.999, 1e-3, 9.999e-4, 123456.7, 1e100, 1e-300, 5e-324, 1.7976931348623157e308,
                                           -2.5, 1.0, 1 / 3, float("inf"), float("-inf"), 1e16, 12345678.9]))
    if r < 0.4:
        return rng.choice(["7ff8000000000000", "fff8000000000000", "7ff0000000000001"])
    if r < 0.75:
        return "%016x" % dbits(rng.uniform(-1, 1) * 10.0 ** rng.randrange(-20, 25))
    return "%016x" % rng.randrange(1 << 64)


def lp_int():
    r = rng.random()
    if r < 0.5:
        return rng.choice([0, 1, -1, 10, -255, 35, 36, 2 ** 31, 2 ** 62, 2 ** 64, -2 ** 64, 10 ** 30, rng.randrange(-1000, 1000)])
    return rng.randrange(-10 ** rng.randrange(1, 40), 10 ** rng.randrange(1, 40))


import fractions


def lispprint_line():
    fn = rng.choice(["prin1", "prin1", "princ"])
    base = rng.choice([10, 10, 10, 2, 8, 16, 3, 36, rng.randrange(2, 37)])
    radix = rng.choice(["0", "1"])
    dff = rng.choice(["0", "0", "1"])
    r = rng.random()
    if r < 0.25:
        n = lp_int()
        if n == -2 ** 62 and base == 2:
            n = -2 ** 62 - 1
        rest = f"int {n}"
    elif r < 0.4:
        den = rng.choice([1, 2, 3, -4, 7, 10, 16, 1000, lp_int() or 3])
        num = lp_int()
        if base == 2 and fractions.Fraction(num, den) == -2 ** 62:
            num = num + 1
        rest = f"ratio {num} {den}"
    elif r < 0.6:
        rest = f"single {lp_single_bits()}"
    elif r < 0.8:
        rest = f"double {lp_double_bits()}"
    elif r < 0.9:
        rest = "string " + hx("".join(rng.choice(["a", "\"", "\\", " ", "é", "\n", "|", "#", "😀", "\x00", "\x7f"]) for _ in range(rng.randrange(0, 8))))
    else:
        code = rng.choice([rng.randrange(0, 160), rng.randrange(0, 160), rng.randrange(160, 1000)])
        rest = f"char {code}"
        if code >= 160:
            fn = "princ"
    return f"{fn} {base} {radix} {dff} {rest}"


family("lispprint", "lispprint", [lispprint_line() for _ in range(int(os.environ.get("LISPPRINT_LINES", "10000")))],
       ["python3", os.path.join(LIB, "lispprint/oracle.py")])


# --- Perl's Term::ANSIColor, against Perl 5.34 --------------------------------------------------------------

AC_NAMES = ["clear", "reset", "bold", "dark", "faint", "italic", "underline", "underscore", "blink", "reverse", "concealed", "black", "red", "green",
            "yellow", "blue", "magenta", "cyan", "white", "bright_red", "bright_white", "on_black", "on_red", "on_bright_blue", "on_bright_cyan",
            "ansi0", "ansi15", "ansi16", "ansi255", "ansi256", "ansi007", "on_ansi42", "grey0", "grey23", "grey24", "on_grey12", "rgb000", "rgb555",
            "rgb505", "rgb560", "on_rgb123", "r0g0b0", "r255g255b255", "r256g0b0", "on_r1g2b3", "r001g002b003", "R10G20B30", "r1g2", "rgb", "nope",
            "BOLD", "Red", "ON_BLUE", "bright_", "on_", "", "r1b2g3", "r1g2b3x", "ansi", "grey", "on_ansi300"]


def ac_codes():
    out = []
    for _ in range(rng.randrange(0, 4)):
        if rng.random() < 0.2:
            out.append(rng.choice([" ", "\t", "  "]).join(rng.choice(AC_NAMES) for _ in range(rng.randrange(1, 4))))
        else:
            out.append(rng.choice(AC_NAMES))
    return out


def ac_list(xs):
    return ",".join(hx(x) for x in xs) if xs else "~"


def ac_text():
    return "".join(rng.choice(["a", "b", "\n", "\n\n", " ", "é", "\x1b[1m", "\x1b[0m", "x", "\x1b[", "m", ";", "1", "--", "\x1b[38;5;1m", "\x1b[m"])
                   for _ in range(rng.randrange(0, 8)))


def ac_escape():
    nums = []
    for _ in range(rng.randrange(0, 4)):
        r = rng.random()
        if r < 0.5:
            nums.append(rng.choice(["0", "1", "2", "4", "5", "7", "8", "3", "6", "9", "31", "47", "97", "107", "39", "108", "00", "001", "030", "22"]))
        elif r < 0.7:
            nums.append(rng.choice(["38", "48", "038", "18", "0038"]) + ";" + rng.choice(["5", "05", "2", "6"]) + ";" +
                        str(rng.choice([0, 15, 16, 196, 231, 232, 255, 256, 999, 7])))
        else:
            nums.append(rng.choice(["38", "48", "038"]) + ";" + rng.choice(["2", "02", "5"]) + ";" +
                        ";".join(str(rng.choice([0, 1, 255, 256, 12, 300])) for _ in range(rng.randrange(1, 4))))
    body = ";".join(nums)
    if rng.random() < 0.1:
        body += ";"
    if rng.random() < 0.08:
        body = rng.choice(["1;;2", ";", "a", "1m", "x;1", "\x1b[1", " 1"])
    return rng.choice(["\x1b[", "\x1b[", ""]) + body + rng.choice(["m", "m", ""])


def perlansicolor_line():
    r = rng.random()
    if r < 0.25:
        return f"color {ac_list(ac_codes())}"
    if r < 0.45:
        each = rng.choice(["~", "~", hx("\n"), hx("--"), hx(" "), "-"])
        if rng.random() < 0.5:
            return f"colored {each} {hx(ac_text())} {ac_list(ac_codes())}"
        return f"coloredl {each} {ac_list(ac_codes())} {ac_list([ac_text() for _ in range(rng.randrange(0, 3))])}"
    if r < 0.7:
        return f"uncolor {ac_list([ac_escape() for _ in range(rng.randrange(0, 3))])}"
    if r < 0.85:
        return f"strip {ac_list([ac_text() for _ in range(rng.randrange(0, 3))])}"
    return f"valid {ac_list(ac_codes())}"


family("perlansicolor", "perlansicolor", [perlansicolor_line() for _ in range(int(os.environ.get("PERLANSICOLOR_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlansicolor/oracle.pl")])


# --- run ------------------------------------------------------------------------------------------------

def finish():
    got = []
    for i in range(0, len(cases), 50):
        argv = []
        for fam, op, path, w in cases[i:i + 50]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            print("harness failed:", ran.stderr[-2000:]); sys.exit(1)
        got.extend(ran.stdout.split("\n")[:-1])
    families, differ, shown, at = {}, 0, 0, 0
    for fam, op, path, w in cases:
        ins = open(path, encoding="utf-8").read().split("\n")[:-1]
        f = families.setdefault(fam, [0, 0])
        for k, want_line in enumerate(w):
            have = got[at] if at < len(got) else "<missing>"
            at += 1
            if have == want_line:
                f[0] += 1
                continue
            f[1] += 1
            differ += 1
            if shown < int(os.environ.get("SHOW", "6")):
                shown += 1
                print(f"  {fam} {ins[k][:200]}\n    want {want_line[:300]}\n    got  {have[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    total = sum(a + d for a, d in families.values())
    if bad:
        print("disagreeing: " + " ".join(bad) + f" (SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    print(f"lines: {total}   agreeing: {total - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)


finish()
