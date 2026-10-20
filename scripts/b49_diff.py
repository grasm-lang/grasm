"""B49's packages against the references they reproduce.

- Go's strconv for numbers against Go 1.25: FormatFloat in every format
  and both sizes, ParseFloat (decimal, hex, special values, underscores,
  out of range), ParseInt, ParseUint, Atoi, ParseBool, FormatInt and
  FormatUint, error messages included.
- net/netip against Go 1.25: ParseAddr, ParsePrefix and ParseAddrPort
  with every error message, printing, the predicates, Next and Prev,
  Compare, Contains, Overlaps and Prefix.
- net/url against Go 1.25: Parse and ParseRequestURI with their errors,
  every field and printed form, references resolved, JoinPath, the
  escape and unescape functions, and ParseQuery with Encode.
- mime against Go 1.25: ParseMediaType and FormatMediaType, RFC 2231
  continuations included, and the WordEncoder and WordDecoder with
  DecodeHeader.
- urllib.parse against Python 3.13: urlsplit, urlparse, urlunsplit,
  urlunparse, urljoin, urldefrag, the result properties, the quote and
  unquote family, parse_qsl, parse_qs and urlencode, ValueError texts
  included. Python's printable table is regenerated and compared first.
- math against Python 3.13: fsum, sumprod, prod, fma, gamma, lgamma, ulp,
  hypot, dist, remainder, nextafter with steps, isqrt, comb, perm,
  factorial, gcd and lcm, errors included.
- statistics against Python 3.13 over floats: the means, medians, mode,
  quantiles, the four spreads, covariance, correlation, linear
  regression and NormalDist's from_samples, pdf, zscore, inv_cdf and
  quantiles, errors included.
- random against Python 3.13: seeding from ints, text and bytes, every
  method and distribution, and gauss's kept value, call after call on one
  generator.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B49_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B49_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B49_LIB") or os.path.join(ROOT, "scripts/lib/b49")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b49")
harness = os.path.join(work, "b49test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b49test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


def go_tool(name, src):
    mod = os.path.join(work, name)
    os.makedirs(mod)
    shutil.copy(src, os.path.join(mod, "main.go"))
    open(os.path.join(mod, "go.mod"), "w").write(f"module {name}\n\ngo 1.25\n")
    out = os.path.join(work, name + ".bin")
    subprocess.run(["go", "build", "-o", out, "."], cwd=mod, check=True)
    return out


def family(name, op, lines, oracle_argv, chunk=500):
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.splitlines()
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))


# --- Go's strconv, against Go 1.25 ----------------------------------------------------------------------------

def random_double():
    r = rng.random()
    if r < 0.3:
        return rng.choice([0.0, -0.0, 1.0, 0.1, 0.5, 1.5, 2.5, 100.0, 1e21, 1e20, 123456789.0, 1e-5, 1e-7, 5e-324, 2.2250738585072014e-308,
                           1.7976931348623157e308, float("inf"), -float("inf"), float("nan"), 3.4028234663852886e38, 1.401298464324817e-45,
                           0.3, 1 / 3, 2 / 3, 1e23, 9007199254740993.0, 0.000123, 1.0000001])
    if r < 0.6:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    return rng.uniform(-1, 1) * 10 ** rng.randrange(-30, 30)


def float_text():
    r = rng.random()
    if r < 0.2:
        return rng.choice(["inf", "+Inf", "-infinity", "INFINITY", "infin", "nan", "NaN", "-nan", "+nan", "nanx", "1e", "1e+", ".", "-", "+.5", "5.",
                           "0x1p-2", "0x1.8p+3", "0X1P1", "0x.8p1", "0x1", "0x1.p1", "0x_1p1", "0x1_0p1", "1_000.5", "_1", "1__0", "1_", "0x1p1_0",
                           "1e1_0", "1e10000000", "-1e-10000000", "0x1p-1075", "0x1p-1074", "0x1.fffffffffffff8p1023", "1e309", "-1e309",
                           "3.4028235e38", "3.4028236e38", "1.17549435e-38", "7e-46", "0.000000000000000000000000000000000000000000001", "0.",
                           "00012", "1.5e-45", "١", "1é", "", " 1", "1 ", "0x", "0b101", "+-1"])
    if r < 0.5:
        return repr(random_double()).replace("inf", "Inf")
    digits = "".join(rng.choice("0123456789") for _ in range(rng.choice([1, 3, 17, 18, 19, 20, 25, 40, 800])))
    k = rng.randrange(0, len(digits) + 1)
    t = digits[:k] + ("." if rng.random() < 0.7 else "") + digits[k:]
    if rng.random() < 0.5:
        t += rng.choice("eE") + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 400))
    return rng.choice(["", "-", "+"]) + t


def int_text():
    r = rng.random()
    if r < 0.3:
        return rng.choice(["0", "-0", "+0", "1", "-1", "9223372036854775807", "9223372036854775808", "-9223372036854775808", "-9223372036854775809",
                           "18446744073709551615", "18446744073709551616", "0x10", "0X1f", "0b101", "0o17", "017", "08", "0x", "0b", "0_1", "1_000",
                           "_1", "1_", "1__0", "0x_1f", "0b_1", "z", "Z", "zz", "-zz", "", "+", "-", "12a", " 1", "1 ", "٣", "0x8000000000000000",
                           "127", "128", "-128", "-129", "255", "256", "4294967295", "4294967296", "2147483648", "-2147483649"])
    digits = "".join(rng.choice("0123456789abcdefxz_") for _ in range(rng.randrange(1, 22)))
    return rng.choice(["", "-", "+"]) + digits


def valid_int(base):
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"
    b = base if 2 <= base <= 36 else 10
    prefix = ""
    if base == 0:
        prefix, b = rng.choice([("", 10), ("", 10), ("0x", 16), ("0X", 16), ("0b", 2), ("0o", 8), ("0", 8)])
    digits = "".join(rng.choice(alphabet[:b]) for _ in range(rng.choice([1, 2, 5, 10, 19, 20, 64])))
    if rng.random() < 0.3:
        digits = digits.upper()
    if base == 0 and rng.random() < 0.2 and len(digits) > 2:
        digits = digits[:1] + "_" + digits[1:]
    return rng.choice(["", "", "-", "+"]) + prefix + digits


def strconv_line():
    r = rng.random()
    if r < 0.4:
        fmt = rng.choice("eEfgGbxX" + "fgge")
        prec = rng.choice([-1, -1, -1, 0, 1, 2, 3, 5, 6, 10, 15, 16, 17, 20, 30])
        return f"ff {dbits(random_double()):x} {fmt} {prec} {rng.choice([64, 64, 32])}"
    if r < 0.7:
        return f"pf {hx(float_text())} {rng.choice([64, 64, 32])}"
    if r < 0.85:
        base = rng.choice([0, 0, 10, 10, 16, 2, 8, 36, 1, 37, -1])
        size = rng.choice([0, 64, 64, 32, 8, 16, 1, 65, -1])
        return f"{rng.choice(['pi', 'pu'])} {hx(valid_int(base) if rng.random() < 0.7 else int_text())} {base} {size}"
    if r < 0.9:
        return f"ai {hx(valid_int(10) if rng.random() < 0.6 else int_text())}"
    if r < 0.93:
        return f"pb {hx(rng.choice(['1', 't', 'T', 'TRUE', 'true', 'True', '0', 'f', 'F', 'FALSE', 'false', 'False', 'tRUE', 'yes', '', '2']))}"
    v = rng.choice([0, 1, -1, 42, -9223372036854775808, 9223372036854775807, rng.randrange(-2 ** 63, 2 ** 63)])
    if rng.random() < 0.5:
        return f"fi {v} {rng.randrange(2, 37)}"
    return f"fu {v % 2 ** 64} {rng.randrange(2, 37)}"


strconv_bin = go_tool("strconvoracle", os.path.join(LIB, "strconv/oracle.go"))
family("strconv", "strconv", [strconv_line() for _ in range(int(os.environ.get("STRCONV_LINES", "8000")))], [strconv_bin])


# --- net/netip, against Go 1.25 ------------------------------------------------------------------------------

V4 = ["1.2.3.4", "0.0.0.0", "255.255.255.255", "127.0.0.1", "10.1.2.3", "172.16.5.4", "172.32.0.1", "192.168.1.1", "169.254.1.1", "224.0.0.1",
      "224.0.1.1", "239.1.1.1", "8.8.8.8", "100.64.0.1", "255.255.255.254", "0.0.0.1"]
V6 = ["::", "::1", "1::", "2001:db8::1", "2001:db8:0:0:1:0:0:1", "2001:0db8:0000:0000:0000:ff00:0042:8329", "fe80::1", "fe80::1%eth0", "ff02::1",
      "ff01::1", "ff05::2", "fc00::1", "fd12:3456::1", "::ffff:1.2.3.4", "::ffff:127.0.0.1", "::ffff:192.168.0.1", "::ffff:0:0", "1:2:3:4:5:6:7:8",
      "1:0:0:2:0:0:0:3", "0:0:1:0:0:1:0:0", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", "::%eth0", "::1%lo", "64:ff9b::1.2.3.4", "1::2:3:4:5:6:7",
      "::ffff:ffff:ffff", "2001:DB8::ABCD", "::ffff:1.2.3.4%x"]
BAD_ADDR = ["", "1.2.3", "1.2.3.4.5", "01.2.3.4", "1.2.3.256", "1..2.3", ".1.2.3", "1.2.3.", "1.2.3.x", "a", "%eth0", "::%", "1:2:3:4:5:6:7:8:9",
            "1:2:3:4:5:6:7", "12345::", "::g", "1:::2", "1::2::3", ":1", "1:", "::1.2.3", "1.2.3.4::", "1:2:3:4:5:6:1.2.3.4", "1::2:3:4:5:6:7:8",
            "::1.2.3.4.5", "::01.2.3.4", "fe80::1%", "1:2:3:4:5:6:7:1.2.3.4", "1:2:3:4:5:1.2.3.4", "::1 ", " ::1", "1.2.3.4%x", "[::1]", "::fffff",
            "1:2:3:4:5:6:7:8%", "x:y", "1.2.3.-4"]


def addr_text():
    r = rng.random()
    if r < 0.4:
        return rng.choice(V4 + V6)
    if r < 0.6:
        return rng.choice(BAD_ADDR)
    if r < 0.8:
        groups = [format(rng.choice([0, 0, 0, 1, 0xffff, rng.randrange(1 << 16)]), "x") for _ in range(8)]
        k = rng.randrange(0, 8)
        n = rng.randrange(0, 4)
        text = ":".join(groups)
        if n and rng.random() < 0.5:
            text = ":".join(groups[:k]) + "::" + ":".join(groups[k + n:])
        if rng.random() < 0.15:
            text += "%" + rng.choice(["eth0", "1", "x%y"])
        return text
    return ".".join(str(rng.choice([0, 1, 10, 127, 255, rng.randrange(256)])) for _ in range(4))


def netip_line():
    r = rng.random()
    a = addr_text()
    if r < 0.4:
        return "pa " + hx(a)
    if r < 0.6:
        bits = rng.choice(["0", "8", "16", "24", "32", "33", "64", "96", "128", "129", "-1", "+8", "08", "", "x", "1x", "99999999999999999999",
                           "1000000000000000000"])
        return "pp " + hx(a + "/" + bits if rng.random() < 0.95 else a)
    if r < 0.72:
        host = a if ("." in a or rng.random() < 0.2) else "[" + a + "]"
        return "pap " + hx(host + ":" + rng.choice(["80", "0", "65535", "65536", "", "x", "+1", "0080", "00000000080", "999999999999999999999"]))
    if r < 0.82:
        return f"cmp {hx(a)} {hx(rng.choice([a, addr_text(), addr_text()]))}"
    if r < 0.9:
        p = rng.choice(["10.0.0.0/8", "192.168.0.0/16", "0.0.0.0/0", "::/0", "2001:db8::/32", "fe80::/10", "1.2.3.4/32", "::ffff:0:0/96", "1.2.3.0/24"])
        return f"con {hx(p)} {hx(a)}"
    if r < 0.95:
        ps = ["10.0.0.0/8", "10.1.0.0/16", "11.0.0.0/8", "0.0.0.0/0", "::/0", "2001:db8::/32", "2001:db8:1::/48", "1.2.3.4/32", "1.2.3.5/32", "::ffff:0:0/96"]
        return f"ov {hx(rng.choice(ps))} {hx(rng.choice(ps))}"
    return f"pfx {hx(a)} {rng.choice([0, 1, 8, 24, 32, 33, 64, 100, 128, 129, -1])}"


netip_bin = go_tool("netiporacle", os.path.join(LIB, "netip/oracle.go"))
family("netip", "netip", [netip_line() for _ in range(int(os.environ.get("NETIP_LINES", "6000")))], [netip_bin])


# --- net/url, against Go 1.25 --------------------------------------------------------------------------------

URL_ODD = [" ", "%", "%2", "%zz", "%41", "%2F", "%2f", "%25", "%20", "%E2%82%AC", "%FF", "%00", "+", "&", ";", "=", "?", "#", "@", ":", "[", "]",
           "!", "$", "'", "(", ")", "*", ",", "~", "\u00e9", "\u20ac", "\x7f", "\t", "/", "//", ".", "..", "^", "`", "|", "{", "}", "\\", "\""]
URL_HOSTS = ["example.com", "EXAMPLE.com", "h", "h:80", "h:", "h:x", "h:8a", "1.2.3.4", "1.2.3.4:5", "[::1]", "[::1]:80", "[::1]x", "[::1", "::1",
             "[fe80::1%25en0]", "[fe80::1%en0]", "[fe80::1%25%41]", "[1.2.3.4]", "[x]", "a%20b", "a%41b", "a%E2%82%ACb", "a b", "h%2", "", "u@h",
             "u:p@h", "u:p@h:1", "a@b@h", "%75@h", "u%40x:p%3A@h", "\u00e9@h", "h\u00e9", "]x[", "h:65536", "h:0080"]


def url_odd():
    return rng.choice(URL_ODD)


def url_path():
    segs = [rng.choice(["a", "b", ".", "..", "", "x y", "a%20b", "a%2Fb", "%7E", "c;d", "e:f", "\u00e9"]) + (url_odd() if rng.random() < 0.15 else "")
            for _ in range(rng.randrange(0, 5))]
    p = "/".join(segs)
    return ("/" + p) if rng.random() < 0.7 else p


def random_go_url():
    r = rng.random()
    if r < 0.08:
        return "".join(url_odd() if rng.random() < 0.6 else rng.choice("ab:/?#") for _ in range(rng.randrange(0, 7)))
    if r < 0.12:
        return rng.choice(["*", "", "mailto:a@b", "http:x", "a:b/c", "./a:b", "//h/p", "///p", "/", "?", "#", "?#", "x?y?z", "http:", "http://", "HTTP://H/",
                           "file:///etc/x", "1http://h", "+x:y", ":x", "a+b.c-d://h"])
    u = ""
    if rng.random() < 0.75:
        u += rng.choice(["http", "https", "HTTP", "ftp", "mailto", "x+y", "file", "s3"]) + ":"
    if rng.random() < 0.7:
        u += "//" + rng.choice(URL_HOSTS)
    u += url_path()
    if rng.random() < 0.4:
        u += "?" + rng.choice(["", "a=1", "a=1&b=2", "a=1&a=2", "b=2&a=1", "x", "a=%20", "a+b=c+d", "q=%zz", "a=1;b=2", "&&a", "=x", "a==b", "%FF=1"])
    if rng.random() < 0.3:
        u += "#" + rng.choice(["", "f", "a b", "%41", "%2F", "x#y", "!$&()*", "%zz", "\u00e9"])
    return u


def url_line():
    r = rng.random()
    if r < 0.4:
        return "p " + hx(random_go_url())
    if r < 0.48:
        return "r " + hx(random_go_url())
    if r < 0.68:
        base = rng.choice(["http://a/b/c/d;p?q", "http://a/b/c/d;p?q#f", "http://u:p@a/b/", "mailto:x@y", "http:/a/b", "/a/b", "a/b", "http://a",
                           "http://a?x", random_go_url()])
        ref = rng.choice(["g:h", "g", "./g", "g/", "/g", "//g", "?y", "g?y", "#s", "g#s", "", ".", "./", "..", "../", "../g", "../..", "../../g",
                          "../../../g", "/./g", "/../g", "g.", ".g", "g..", "..g", "./../g", "./g/.", "g/./h", "g/../h", "g;x=1/../y", "a%2Fb/..", "?", "#",
                          "mailto:z", "http:g", random_go_url()])
        return f"ref {hx(base)} {hx(ref)}"
    if r < 0.76:
        elems = [rng.choice(["a", "b/", "../c", "/d", "", ".", "..", "x%20y", "e/", "%zz", "f?g", "\u00e9"]) for _ in range(rng.randrange(0, 4))]
        return "jp " + hx(rng.choice(["http://h/a/", "http://h/a", "http://h", "a", "/", "mailto:x", "http://h/a%2Fb/", "%zz", ":x"])) + "".join(" " + hx(e) for e in elems)
    if r < 0.86:
        t = "".join(url_odd() if rng.random() < 0.5 else rng.choice(["a", "Z", "0", " "]) for _ in range(rng.randrange(0, 6)))
        return rng.choice(["qe ", "pe ", "qu ", "pu "]) + hx(t)
    return "pq " + hx(rng.choice(["", "a=1&b=2", "b=2&a=1&a=0", "a", "a=", "=1", "&", "a=1;b=2", "a=%zz&b=1", "%zz=1&c=3", "a+b=c%20d", "\u00e9=\u00e9",
                                  "a=1&&b=2", "z=1&a=2&m=3", "%FF=%FE"]))


url_bin = go_tool("urloracle", os.path.join(LIB, "url/oracle.go"))
family("url", "url", [url_line() for _ in range(int(os.environ.get("URL_LINES", "6000")))], [url_bin])


# --- mime, against Go 1.25 -----------------------------------------------------------------------------------

def media_text():
    if rng.random() < 0.15:
        return rng.choice(["", " ", "text", "text/", "/plain", "text/plain/x", "text plain", "TEXT/HTML", " text/html ", "\u0130mage/png",
                           "\u212aext/plain", "t\u00e9xt/plain", "text/plain;", "text/plain; ", "text/plain;;", "text/plain; ;", "a/b;c", "a/b; c=",
                           "a/b; =d", "a/b; c=\"unterminated", "a/b; c=\"x\\\"y\"", "a/b; c=\"\r\"", "form-data; name=x; name=y", "form-data; name=x; name=x"])
    t = rng.choice(["text/plain", "TEXT/Html", "application/json", "multipart/form-data", "form-data", "attachment", "inline", "image/svg+xml", "a/b"])
    for _ in range(rng.randrange(0, 4)):
        key = rng.choice(["charset", "CHARSET", "name", "filename", "boundary", "title", "q", "x"])
        r = rng.random()
        if r < 0.2:
            key += rng.choice(["*", "*0", "*1", "*0*", "*1*", "*2", "*x"])
        val = rng.choice(["utf-8", "UTF-8", "a b", "\"a b\"", "\"a\\\"b\"", "\"\"", "us-ascii''%41%42", "utf-8'en'%E2%82%AC", "iso-8859-1''x",
                          "utf-8''%zz", "''x", "x'y", "%41", "token", "\"\u00e9\"", "a;b", "(c)"])
        sep = rng.choice(["; ", ";", " ; ", ";\t"])
        t += sep + key + rng.choice(["=", " = ", "="]) + val
    if rng.random() < 0.1:
        t += rng.choice([";", "; ", ";;", " junk", "; x"])
    return t


def mime_line():
    r = rng.random()
    if r < 0.4:
        return "pm " + hx(media_text())
    if r < 0.55:
        params = []
        for _ in range(rng.randrange(0, 3)):
            params += [hx(rng.choice(["charset", "Name", "filename", "a b", "q", "x", "Title"]) + str(rng.randrange(3))),
                       hx(rng.choice(["utf-8", "a b", "a\"b", "a\\b", "", "\u00e9t\u00e9", "x*y", "50%", "tab\there", "token", "(c)"]))]
        return "fm " + hx(rng.choice(["text/plain", "TEXT/HTML", "form-data", "a b/c", "a/", "/b", "x", "a/b/c", "application/x-y"])) + "".join(" " + x for x in params)
    if r < 0.75:
        text = rng.choice(["hello", "", "caf\u00e9", "\u00a1Hola, se\u00f1or!", "x" * 30 + "\u00e9" * 30, "\u20ac" * 40, "\U0001F600 " * 12,
                           "a=b?c_d", "tab\tok", "line\nbreak", "\u00e9" + "a" * 80, "plain ascii only", "\u00e9 \u00e9 \u00e9 " * 9])
        return f"we {rng.choice('bq')} {hx(rng.choice(['UTF-8', 'utf-8', 'iso-8859-1', 'Utf-8', 'x-unknown']))} {hx(text)}"
    words = ["=?utf-8?q?caf=C3=A9?=", "=?UTF-8?B?Y2Fmw6k=?=", "=?iso-8859-1?q?caf=E9?=", "=?us-ascii?q?a=FFb?=", "=?utf-8?b?Y2Fmw6k?=",
             "=?utf-8?b?Y2Fm\nw6k=?=", "=?utf-8?q?a_b?=", "=?x-unknown?q?a?=", "=?utf-8?x?a?=", "=?utf-8?q?=zz?=", "=?utf-8?q?=4?=", "=??q?a?=",
             "=?utf-8?q?\u00e9?=", "=?utf-8?b?====?=", "=?utf-8?b?Y2E=Zg==?=", "=?utf-8?b?Y2E?=", "=?utf-8?qq?a?=", "=?utf-8?q?a?b?=",
             "=?u\u017f-ascii?q?a?=", "=?utf-8?Q?=FF?=", "=?utf-8?q?a=\t1?="]
    if r < 0.85:
        return "wd " + hx(rng.choice(words))
    pieces = [rng.choice(words + ["plain", " ", "  ", "\t", "x", "=?", "?=", "=?broken"]) for _ in range(rng.randrange(1, 5))]
    return "dh " + hx(rng.choice(["", " ", "  "]).join(pieces))


mime_bin = go_tool("mimeoracle", os.path.join(LIB, "mime/oracle.go"))
family("mime", "mime", [mime_line() for _ in range(int(os.environ.get("MIME_LINES", "6000")))], [mime_bin])


# --- urllib.parse, against Python 3.13 ------------------------------------------------------------------------

want_table = subprocess.run(["python3.13", os.path.join(LIB, "pyurl/gen_printable.py")], capture_output=True, text=True).stdout.strip()
if want_table not in open(os.path.join(PKG, "pyurlparse/src/main.gr")).read():
    print("pyurlparse's printable table is not what Python 3.13 gives; regenerate it with gen_printable.py"); sys.exit(1)

PY_HOSTS = ["example.com", "EXAMPLE.Com", "h:80", "h:", "h:x", "h:0080", "h:99999", "h:\u0661", "u:p@h", "u@h:1", "a@b@h", "@h", ":@h", "[::1]", "[::1]:8",
            "[::1", "::1]", "[1.2.3.4]", "[::ffff:1.2.3.4]", "[fe80::1%eth0]", "[fe80::1%]", "[v1.x]", "[v.x]", "[vg.x]", "[v1.]", "[x]", "[1::2::3]",
            "[12345::]", "[::]:1", "h\u00e9", "H\u00c9", "\u2100h", "a\u2100b", "h\uff03x", "\uff20h", "h\u00a0", "h\x7f", "h\u0130", "",
            "[::1%25eth0]", "[1:2:3:4:5:6:7:8]", "[1:2:3:4:5:6:7:8:9]", "[::1.2.3.4]", "[1:2:3:4:5:6:1.2.3.4]", "[01.2.3.4]"]


def py_url():
    r = rng.random()
    if r < 0.1:
        return "".join(rng.choice([" ", "\t", "\n", "\x00", "\x1f", ":", "/", "?", "#", ";", "a", "%", "[", "]"]) for _ in range(rng.randrange(0, 8)))
    u = rng.choice(["", "", " ", "\x01 ", "\t"])
    if rng.random() < 0.7:
        u += rng.choice(["http", "HTTPS", "ftp", "mailto", "file", "sip", "tel", "svn+ssh", "x-y", "1a", "a_b", "itms-services", "data", "git"]) + ":"
    if rng.random() < 0.7:
        u += "//" + rng.choice(PY_HOSTS)
    u += rng.choice(["", "/", "/a/b", "/a/b/", "a/b", "/a;p", "/a/b;p;q", "/a;p/b", ";x", "/x y", "/%41%zz", "/\u00e9", "/a\tb", "/a\nb", "//x", "/./a/../b"])
    if rng.random() < 0.3:
        u += "?" + rng.choice(["", "a=1", "a=1&b=2", "q=%20+x", "x#y", "a;b"])
    if rng.random() < 0.3:
        u += "#" + rng.choice(["", "f", "a#b", "%41", "\u00e9"])
    if rng.random() < 0.1:
        u += rng.choice([" ", "  ", "\x00"])
    return u


PY_REFS = ["g:h", "g", "./g", "g/", "/g", "//g", "?y", "g?y", "#s", "g#s", "g?y#s", ";x", "g;x", "g;x?y#s", "", ".", "./", "..", "../", "../g", "../..",
           "../../", "../../g", "../../../g", "../../../../g", "/./g", "/../g", "g.", ".g", "g..", "..g", "./../g", "./g/.", "g/./h", "g/../h",
           "g;x=1/./y", "g;x=1/../y", "g?y/./x", "g?y/../x", "g#s/./x", "g#s/../x", "http:g", "http:", "mailto:x", "//h:x"]


def pyurl_line():
    r = rng.random()
    if r < 0.3:
        return f"{rng.choice(['us', 'up'])} {hx(py_url())} {hx(rng.choice(['', '', 'http', ' https ', 'x']))} {rng.choice('110')}"
    if r < 0.5:
        base = rng.choice(["http://a/b/c/d;p?q", "http://a/b/c/d;p?q#f", "https://a/b/", "ftp://a/b", "mailto:x@y", "http:/a/b", "a/b", "/a/b", "", "svn+ssh://h/x/y",
                           "file:///a/b", "x://a/b", py_url()])
        return f"uj {hx(base)} {hx(rng.choice(PY_REFS + [py_url()]))} {rng.choice('110')}"
    if r < 0.55:
        return "ud " + hx(py_url())
    if r < 0.75:
        text = "".join(rng.choice(["a", " ", "/", "+", "%", "%41", "%zz", "%e9", "%C3%A9", "%C3", "%E2%82", "\u00e9", "~", "_", "?", "=", "&", "\u20ac", "%ED%A0%80",
                                   "%F0%9F%98%80", "%f0%9f", "%", "%4"]) for _ in range(rng.randrange(0, 7)))
        op = rng.choice(["q", "qp", "uq", "uqp", "uqb", "qb"])
        if op == "qb":
            return f"qb {''.join(format(rng.randrange(256), '02x') for _ in range(rng.randrange(0, 6))) or '-'} {hx(rng.choice(['', '/', '+', '\u00e9/']))}"
        return f"{op} {hx(text)} {hx(rng.choice(['', '/', '+&=', '\u00e9', '%']))}"
    if r < 0.92:
        qs = rng.choice(["", "a=1&b=2", "a=1&a=2", "a", "a=", "=1", "&", "&&a=1", "a=1;b=2", "a+b=c%20d", "%zz=1", "x=%C3", "\u00e9=\u00e9", "a=1&b", "b=2&a=1&b=3"])
        sep = rng.choice(["&", "&", ";", "", "ab"])
        return f"qsl {hx(qs)} {rng.choice('01')} {rng.choice('001')} {rng.choice([-1, -1, 0, 1, 2, 5])} {hx(sep)}" if rng.random() < 0.7 else f"qs {hx(qs)} {rng.choice('01')}"
    pairs = []
    for _ in range(rng.randrange(0, 4)):
        k = rng.choice(["a", "b c", "\u00e9", "&", "", "x/y"])
        vs = [rng.choice(["1", "a b", "", "\u20ac", "+", "%"]) for _ in range(rng.randrange(1, 3))]
        pairs.append(hx(k) + "=" + "+".join(hx(v) for v in vs))
    return "ue" + "".join(" " + x for x in pairs)


family("pyurl", "pyurl", [pyurl_line() for _ in range(int(os.environ.get("PYURL_LINES", "6000")))], ["python3.13", os.path.join(LIB, "pyurl/oracle.py")])


# --- random, against Python 3.13 ------------------------------------------------------------------------------

def fx(v):
    return "%x" % dbits(v)


def random_call():
    r = rng.random()
    if r < 0.12:
        return "random"
    if r < 0.18:
        return f"bits,{rng.choice([0, 1, 5, 31, 32, 33, 53, 63])}"
    if r < 0.28:
        a = rng.randrange(-50, 50)
        return f"range,{a},{a + rng.choice([-3, 0, 1, 2, 10, 1000, 2 ** 40])},{rng.choice([1, 1, 2, 3, -1, -2, 0, 7])}"
    if r < 0.31:
        return f"range1,{rng.choice([0, -1, 1, 2, 10, 2 ** 31, 2 ** 62])}"
    if r < 0.35:
        a = rng.randrange(-10, 10)
        return f"int,{a},{a + rng.choice([-1, 0, 1, 5, 100])}"
    if r < 0.38:
        return f"choice,{rng.choice([0, 1, 2, 7, 100])}"
    if r < 0.42:
        return f"shuffle,{rng.choice([0, 1, 2, 5, 20])}"
    if r < 0.48:
        n = rng.choice([0, 1, 5, 20, 21, 22, 30, 100, 1000])
        return f"sample,{n},{rng.choice([0, 1, 3, 5, 6, 10, n, n + 1, -1])}"
    if r < 0.52:
        return f"choices,{rng.choice([1, 3, 10])},{rng.choice([0, 1, 5])}"
    if r < 0.56:
        w = [rng.choice([0.0, 1.0, 2.5, 0.1, 1e-300, 3.0]) for _ in range(rng.randrange(0, 5))]
        return f"{rng.choice(['choicesw', 'choicescw'])},{'/'.join(fx(x) for x in w)},{rng.choice([1, 4])}"
    if r < 0.59:
        return f"bytes,{rng.choice([0, 1, 3, 4, 5, 9])}"
    if r < 0.64:
        return f"binomial,{rng.choice([-1, 0, 1, 5, 20, 100, 1000, 100000])},{fx(rng.choice([0.0, 1.0, 0.5, 0.1, 0.01, 0.9, 0.3, 1.5, -0.1]))}"
    if r < 0.7:
        return f"triangular,{fx(0.0)},{fx(rng.choice([1.0, 10.0, 0.0]))},{rng.choice(['~', fx(0.3), fx(5.0)])}"
    name = rng.choice(["uniform", "normal", "gauss", "gauss", "lognormal", "expo", "vonmises", "gamma", "beta", "pareto", "weibull"])
    if name == "expo":
        return f"expo,{fx(rng.choice([1.0, 0.5, 3.0]))}"
    if name == "pareto":
        return f"pareto,{fx(rng.choice([1.0, 2.5, 0.5]))}"
    if name in ("gamma", "beta"):
        return f"{name},{fx(rng.choice([0.5, 1.0, 2.0, 7.5, 0.0, -1.0]))},{fx(rng.choice([1.0, 0.5, 3.0]))}"
    if name == "vonmises":
        return f"vonmises,{fx(rng.choice([0.0, 1.0, -2.0, 7.0]))},{fx(rng.choice([0.0, 1e-7, 0.5, 4.0, 50.0]))}"
    return f"{name},{fx(rng.choice([0.0, 1.0, -3.0]))},{fx(rng.choice([1.0, 0.5, 2.0]))}"


def random_line():
    k = rng.random()
    if k < 0.6:
        seed = "i:" + str(rng.choice([0, 1, 42, -42, 2 ** 32, 2 ** 63 - 1, -2 ** 63, rng.randrange(-2 ** 63, 2 ** 63)]))
    elif k < 0.85:
        seed = "s:" + hx(rng.choice(["", "a", "seed", "\u00e9", "x" * 100]))
    else:
        seed = "b:" + ("".join(format(rng.randrange(256), "02x") for _ in range(rng.randrange(0, 9))) or "-")
    return seed + "".join(" " + random_call() for _ in range(rng.randrange(1, 7)))


family("pyrandom", "pyrandom", [random_line() for _ in range(int(os.environ.get("PYRANDOM_LINES", "3000")))],
       ["python3.13", os.path.join(LIB, "pyrandom/oracle.py")], chunk=300)


# --- math, against Python 3.13 --------------------------------------------------------------------------------

def any_float():
    r = rng.random()
    if r < 0.25:
        return rng.choice([0.0, -0.0, 1.0, -1.0, 0.5, 2.0, 1e-300, 5e-324, 2.2250738585072014e-308, 1.7976931348623157e308, -1.7976931348623157e308,
                           float("inf"), -float("inf"), float("nan"), 1e16, 1.0000000000000002, 0.1, 3.0, -3.0, 1e308])
    if r < 0.5:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    return rng.uniform(-1, 1) * 10 ** rng.randrange(-20, 20)


def flt_list(n=None, finite_scale=None):
    n = rng.randrange(0, 8) if n is None else n
    out = []
    for _ in range(n):
        if finite_scale is not None:
            out.append(rng.uniform(-1, 1) * 10 ** rng.randrange(-finite_scale, finite_scale))
        else:
            out.append(any_float() if rng.random() < 0.3 else rng.choice([1e16, 1.0, -1e16, 1e-16, 0.1, 0.2, 0.3, rng.uniform(-100, 100)]))
    return "/".join(fx(x) for x in out)


def math_line():
    op = rng.choice(["fsum"] * 3 + ["sumprod"] * 3 + ["prod", "fma", "fma", "lgamma", "gamma", "gamma", "ulp", "hypot", "hypot", "dist", "remainder",
                     "nextafter", "isqrt", "comb", "perm", "factorial", "gcd", "lcm"])
    if op in ("fsum", "prod", "hypot"):
        return f"{op} {flt_list(finite_scale=rng.choice([None, 5, 300]))}"
    if op in ("sumprod", "dist"):
        n = rng.randrange(0, 8)
        return f"{op} {flt_list(n, rng.choice([None, 5, 150]))} {flt_list(n if rng.random() < 0.95 else n + 1, rng.choice([None, 5, 150]))}"
    if op == "fma":
        return f"fma {fx(any_float())} {fx(any_float())} {fx(any_float())}"
    if op in ("lgamma", "gamma"):
        # Integers (binomialvariate's use) and reals either side of the x < 5
        # split in CPython's Lanczos sum, where its fused multiply-adds bite.
        v = rng.choice([float(rng.randrange(3, 100000)), rng.uniform(0.01, 5.0), rng.uniform(5.0, 400.0), any_float(), rng.uniform(-200, 200), rng.uniform(0, 6), float(rng.randrange(-5, 30)), rng.uniform(140, 172), rng.uniform(-180, -0.5), 1e-21])
        return f"{op} {fx(v)}"
    if op == "ulp":
        return f"ulp {fx(any_float())}"
    if op == "remainder":
        return f"remainder {fx(any_float())} {fx(rng.choice([any_float(), 2.0, 0.5, 3.0, 0.0, float('inf')]))}"
    if op == "nextafter":
        return f"nextafter {fx(any_float())} {fx(any_float())} {rng.choice(['~', '~', 0, 1, 5, 2 ** 52, 2 ** 62, -1])}"
    if op == "isqrt":
        return f"isqrt {rng.choice([0, 1, 2, 3, 4, 15, 16, 17, 2 ** 62, 2 ** 63 - 1, 999999999999, -1, rng.randrange(1 << 62)])}"
    if op in ("comb", "perm"):
        n = rng.choice([0, 1, 5, 10, 52, 100, 300, -1])
        return f"{op} {n} {rng.choice([0, 1, 2, 5, 26, 50, 400, -1] + (['~'] if op == 'perm' else []))}"
    if op == "factorial":
        return f"factorial {rng.choice([0, 1, 5, 20, 21, 100, -1])}"
    ints = [rng.choice([0, 1, -1, 12, 18, 2 ** 40, -(2 ** 33), rng.randrange(-10 ** 9, 10 ** 9)]) for _ in range(rng.randrange(0, 5))]
    return f"{op} {'/'.join(map(str, ints))}"


family("pymath", "pymath", [math_line() for _ in range(int(os.environ.get("PYMATH_LINES", "6000")))], ["python3.13", os.path.join(LIB, "pymath/oracle.py")])


# --- statistics, against Python 3.13 --------------------------------------------------------------------------

def stat_data(n=None, specials=True):
    n = rng.choice([0, 1, 2, 3, 5, 8, 20]) if n is None else n
    kind = rng.random()
    out = []
    for _ in range(n):
        if kind < 0.3:
            v = float(rng.randrange(-5, 6))
        elif kind < 0.5:
            v = rng.choice([1e16, 1.0, -1e16, 0.1, 0.2, 0.3, 1e-16, 2.5])
        elif kind < 0.8:
            v = rng.uniform(-100, 100)
        else:
            v = rng.uniform(-1, 1) * 10 ** rng.randrange(-30, 30)
        if specials and rng.random() < 0.02:
            v = rng.choice([float("inf"), -float("inf"), float("nan"), 0.0, -0.0])
        out.append(v)
    return out


def flist_text(xs):
    return "/".join(fx(x) for x in xs)


def stat_line():
    op = rng.choice(["mean", "mean", "fmean", "geometric_mean", "median", "median_low", "median_high", "mode", "multimode", "fmeanw", "harmonic", "grouped",
                     "quantiles", "variance", "variance", "pvariance", "stdev", "stdev", "pstdev", "covariance", "correlation", "correlation",
                     "regression", "samples", "pdf", "zscore", "invcdf", "invcdf", "nquantiles"])
    # Orderings of data holding a NaN are timsort's own, so the ordered
    # statistics (and ranked correlation) are given none.
    order = op in ("median", "median_low", "median_high", "grouped", "quantiles", "mode", "multimode", "correlation")
    d = stat_data(specials=not order)
    if op in ("mean", "fmean", "median", "median_low", "median_high", "mode", "multimode", "samples"):
        return f"{op} {flist_text(d)}"
    if op == "geometric_mean":
        return f"{op} {flist_text([abs(x) if rng.random() < 0.9 else x for x in d])}"
    if op == "fmeanw":
        return f"fmeanw {flist_text(d)} {flist_text(stat_data(len(d) if rng.random() < 0.9 else len(d) + 1, False))}"
    if op == "harmonic":
        w = "~" if rng.random() < 0.5 else flist_text([abs(x) for x in stat_data(len(d), False)])
        return f"harmonic {flist_text([abs(x) if rng.random() < 0.95 else x for x in d])} {w}"
    if op == "grouped":
        return f"grouped {flist_text(d)} {fx(rng.choice([1.0, 0.5, 10.0, 3.0]))}"
    if op == "quantiles":
        return f"quantiles {flist_text(d)} {rng.choice([1, 2, 4, 10, 0, -1])} {rng.choice(['exclusive', 'exclusive', 'inclusive', 'bogus'])}"
    if op in ("variance", "pvariance", "stdev", "pstdev"):
        c = "~" if rng.random() < 0.7 else fx(rng.choice([0.0, 1.5, sum(d) / len(d) if d else 0.0]))
        return f"{op} {flist_text(d)} {c}"
    if op in ("covariance", "correlation", "regression"):
        n = len(d) if rng.random() < 0.95 else len(d) + 1
        y = stat_data(n, False) if rng.random() < 0.7 else [2.0 * x + 1.0 for x in d][:n] + ([0.0] if n > len(d) else [])
        last = {"covariance": "", "correlation": " " + rng.choice(["linear", "linear", "ranked", "bogus"]), "regression": " " + rng.choice("01")}[op]
        return f"{op} {flist_text(d)} {flist_text(y)}{last}"
    mu, sigma = rng.choice([0.0, 1.5, -3.0, 100.0]), rng.choice([1.0, 0.5, 2.0, 0.0, -1.0, 1e-3])
    if op == "nquantiles":
        return f"nquantiles {fx(mu)} {fx(sigma)} {rng.choice([1, 2, 4, 10, 100])}"
    p = rng.choice([rng.random(), 0.5, 0.025, 0.975, 1e-10, 1 - 1e-10, 0.0, 1.0, 0.075, 0.925, 1e-300, rng.uniform(0.07, 0.93)])
    return f"{op} {fx(mu)} {fx(sigma)} {fx(p if op == 'invcdf' else rng.uniform(-10, 10))}"


family("pystat", "pystat", [stat_line() for _ in range(int(os.environ.get("PYSTAT_LINES", "4000")))], ["python3.13", os.path.join(LIB, "pystatistics/oracle.py")])


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
        ins = open(path, encoding="utf-8").read().splitlines()
        f = families.setdefault(fam, [0, 0])
        for k, want_line in enumerate(w):
            have = got[at] if at < len(got) else "<missing>"
            at += 1
            if have == want_line:
                f[0] += 1
                continue
            f[1] += 1
            differ += 1
            if shown < 6:
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
