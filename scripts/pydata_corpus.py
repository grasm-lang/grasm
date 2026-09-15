"""Cases for pydatatest, with Python's answers.

`build()` returns (argv, expected, labels). Every command is three words:
a name and two hex arguments, each prefixed with `:` so an empty one
survives. Text answers are hex.

An expected answer that starts with `LOADS:` is checked by the driver rather
than compared: the harness's bytes are handed to `marshal.loads`, and what
comes back must print as the rest of the answer. That is how `dumps` is
checked at versions 3 to 5, where Python's own bytes depend on reference
counts inside the interpreter.
"""
import ast
import functools
import getopt
import itertools
import marshal
import math
import os
import pprint
import random
import re
import struct
import warnings

warnings.simplefilter("ignore")


def h(text):
    return ":" + text.encode("utf-8").hex()


def hb(raw):
    return ":" + raw.hex()


def th(text):
    return text.encode("utf-8").hex()


class Cases:
    def __init__(self):
        self.argv, self.expected, self.labels = [], [], []

    def add(self, what, a, b, want, raw_a=None):
        self.argv += [what, hb(raw_a) if raw_a is not None else h(a), h(b)]
        self.expected.append(want)
        self.labels.append(f"{what} {a!r} {b!r}")


# ---------------------------------------------------------------------------
# getopt

def getopts(c, rng):
    shorts = ["ab:c::", "-ab:", "+ab:", "a:", "", "x::y", ":a", "ab:c::d"]
    longs = [[], ["alpha", "beta=", "gamma=?"], ["verbose", "version", "verb="],
             ["out=", "output"], ["a", "ab", "abc="]]
    words = ["-a", "-b", "-bval", "-c", "-cval", "-abfoo", "-ac", "-x", "-",
             "--", "--alpha", "--beta", "--beta=1", "--gamma", "--gamma=2",
             "--alp", "--verb", "--vers", "--ver", "--out", "--output=x",
             "--out=", "--a", "--ab", "--abc", "--abc=v", "--=x", "file",
             "x", "-:", "--zzz", "-z", "val"]
    seen = set()
    while len(seen) < 4000:
        argv = tuple(rng.choice(words) for _ in range(rng.randint(0, 6)))
        spec = (rng.choice(["std", "gnu", "gnu"]), rng.choice(shorts),
                tuple(rng.choice(longs)), rng.choice([False, True]))
        seen.add((argv, spec))
    for argv, (mode, short, long, posix) in sorted(seen):
        if posix:
            os.environ["POSIXLY_CORRECT"] = "1"
        else:
            os.environ.pop("POSIXLY_CORRECT", None)
        try:
            if mode == "std":
                opts, rest = getopt.getopt(list(argv), short, list(long))
            else:
                opts, rest = getopt.gnu_getopt(list(argv), short, list(long))
            parts = []
            for o, v in opts:
                if o is None:
                    parts.append("@" + "\x04".join(v))
                else:
                    parts.append(o + "=" + v)
            want = "\x01".join(parts) + "\x03" + "\x01".join(rest)
        except getopt.GetoptError as e:
            want = "?" + e.opt + "|" + e.msg
        spec = "\x02".join([mode, short, "\x01".join(long), "1" if posix else "0"])
        c.add("gopt", "\x01".join(argv), spec, th(want))
    os.environ.pop("POSIXLY_CORRECT", None)


# ---------------------------------------------------------------------------
# floatmath

def bits(x):
    return str(struct.unpack("<q", struct.pack("<d", x))[0])


def interesting_floats(rng):
    fixed = [0.0, -0.0, 1.0, -1.0, 0.5, 2.0, 3.0, 0.1, 1e-310, -1e-310,
             5e-324, -5e-324, 2.2250738585072014e-308, 2.225073858507201e-308,
             1.7976931348623157e308, -1.7976931348623157e308, math.inf,
             -math.inf, math.nan, 1e16, 1e300, 123.456, 2.0 ** 52, 2.0 ** 53 + 2,
             0.9999999999999999, 4.9406564584124654e-324 * 3]
    out = list(fixed)
    for _ in range(400):
        out.append(struct.unpack("<d", struct.pack("<Q", rng.getrandbits(64)))[0])
    for _ in range(300):
        out.append(rng.uniform(-1e6, 1e6))
    for _ in range(150):
        out.append(struct.unpack("<d", struct.pack("<Q", rng.getrandbits(52)))[0])
    return out


def floats(c, rng):
    values = interesting_floats(rng)
    for x in values:
        c.add("fulp", bits(x), "", bits(math.ulp(x)))
        m, e = math.frexp(x)
        c.add("ffrexp", bits(x), "", bits(m) + "," + str(e))
        f, w = math.modf(x)
        c.add("fmodf", bits(x), "", bits(f) + "," + bits(w))
        c.add("fhex", bits(x), "", th(x.hex()))
        for i in [0, 1, -1, 1023, -1074, -1080, 2000, -2000, rng.randint(-1100, 1100)]:
            try:
                want = bits(math.ldexp(x, i))
            except OverflowError:
                want = "?"
            c.add("fldexp", bits(x), str(i), want)
    for _ in range(3000):
        x, y = rng.choice(values), rng.choice(values)
        c.add("fnext", bits(x), bits(y), bits(math.nextafter(x, y)))
        steps = rng.choice([0, 1, 2, 10, 2 ** 52, 2 ** 62, 2 ** 63 - 1, -1])
        try:
            want = bits(math.nextafter(x, y, steps=steps))
        except ValueError:
            want = "?"
        c.add("fsteps", bits(x), bits(y) + "," + str(steps), want)
        for name, fn in [("ffmod", math.fmod), ("frem", math.remainder)]:
            try:
                want = bits(fn(x, y))
            except ValueError:
                want = "?"
            c.add(name, bits(x), bits(y), want)
        rel = rng.choice([1e-9, 0.0, 0.5, -1.0])
        tol = rng.choice([0.0, 1e-3, 1.0, -1.0])
        try:
            want = "yes" if math.isclose(x, y, rel_tol=rel, abs_tol=tol) else "no"
        except ValueError:
            want = "?"
        c.add("fclose", bits(x) + "," + bits(y), bits(rel) + "," + bits(tol), want)
    for _ in range(2500):
        n = rng.randint(0, 12)
        kind = rng.random()
        if kind < 0.3:
            xs = [rng.choice(values) for _ in range(n)]
        elif kind < 0.7:
            xs = [rng.choice([1e16, 1.0, -1e16, 1e-16, 0.1, -0.1, 1e308, -1e308,
                              3.0, 2.0 ** -1074]) for _ in range(n)]
        else:
            xs = [rng.uniform(-1, 1) * 10 ** rng.randint(-20, 20) for _ in range(n)]
        try:
            want = bits(math.fsum(xs))
        except (OverflowError, ValueError):
            want = "?"
        c.add("ffsum", "\x01".join(bits(x) for x in xs), "", want)
    texts = ["0x1.8p1", "0x1p-1074", "0x1p-1075", "0x3p-1076", "0x1.fffffffffffff8p1023",
             "0x1.fffffffffffffp1023", "inf", "-Infinity", "nan", "  0x1p0  ", "1",
             "0x", "0x.", ".8", "0x.8p1", "1p1", "0X1P+2", "0x1p", "0x1p+",
             "-0x0p0", "0x0.0000000000001p-1022", "0x1.00000000000008p0",
             "0x1.00000000000018p0", "0x1.000000000000080000001p0", "infinit",
             "0x1p99999999999999999999", "0x1p-99999999999999999999", "+0x1p1", "\t0x10\n",
             "0x1_0", "0xg", "nan0", "0x1.p1", "0x.p1"]
    alphabet = "0123456789abcdefx.p+- "
    seen = set(texts)
    while len(seen) < 4000:
        seen.add("0x" + "".join(rng.choice("0123456789abcdef") for _ in range(rng.randint(1, 20)))
                 + rng.choice(["", "p", "p-"]) + str(rng.randint(-1100, 1100)))
        seen.add("".join(rng.choice(alphabet) for _ in range(rng.randint(0, 10))))
    for t in sorted(seen):
        try:
            want = bits(float.fromhex(t))
        except (ValueError, OverflowError):
            want = "?"
        c.add("ffromhex", t, "", want)


# ---------------------------------------------------------------------------
# Python values

def safe_lt(a, b):
    try:
        return a < b
    except TypeError:
        ta, tb = str(type(a)), str(type(b))
        if ta != tb:
            return ta < tb
        return canon(a) < canon(b)


def ordered(items):
    """pyliteral's order: by type name and repr first, so the start does not
    depend on hash-table order, then a stable merge sort by safe_lt."""
    return merged(sorted(items, key=lambda x: (str(type(x)), canon(x))))


def merged(items):
    items = list(items)
    if len(items) <= 1:
        return items
    mid = len(items) // 2
    left, right = merged(items[:mid]), merged(items[mid:])
    out, i, j = [], 0, 0
    while i < len(left) and j < len(right):
        if safe_lt(right[j], left[i]):
            out.append(right[j]); j += 1
        else:
            out.append(left[i]); i += 1
    return out + left[i:] + right[j:]


def canon(v):
    t = type(v)
    if t is tuple:
        if len(v) == 1:
            return "(" + canon(v[0]) + ",)"
        return "(" + ", ".join(map(canon, v)) + ")"
    if t is list:
        return "[" + ", ".join(map(canon, v)) + "]"
    if t is dict:
        return "{" + ", ".join(canon(k) + ": " + canon(x) for k, x in v.items()) + "}"
    if t is set:
        return "{" + ", ".join(map(canon, ordered(v))) + "}" if v else "set()"
    if t is frozenset:
        return "frozenset({" + ", ".join(map(canon, ordered(v))) + "})" if v else "frozenset()"
    return repr(v)


def literal_answer(text):
    try:
        return th(canon(ast.literal_eval(text)))
    except Exception:
        return th("?")


def random_value(rng, depth=0, hashable=False):
    kinds = ["int", "bigint", "float", "str", "bytes", "none", "bool", "complex"]
    if not hashable:
        kinds += ["ellipsis"]
    if depth < 3:
        kinds += ["tuple"] * 2
        if not hashable:
            kinds += ["list", "dict", "set"]
    k = rng.choice(kinds)
    if k == "int":
        return rng.choice([0, 1, -1, 7, 255, 2 ** 31 - 1, -2 ** 31, 2 ** 31, rng.randint(-10 ** 6, 10 ** 6)])
    if k == "bigint":
        return rng.choice([1, -1]) * rng.randint(2 ** 40, 2 ** 200)
    if k == "float":
        return rng.choice([0.0, -0.0, 0.1, 1e16, 1e-5, 1.5, math.inf, -math.inf, 1e300,
                           rng.uniform(-1e3, 1e3), 5e-324])
    if k == "complex":
        return complex(rng.choice([0.0, -0.0, 1.0, -2.5, math.inf]),
                       rng.choice([0.0, -0.0, 1.0, -3.0, math.inf]))
    if k == "str":
        pool = "abc xyz'\"\\\n\t\r\x00\x7f\x85\xa0é中😀​ \U000e0001"
        return "".join(rng.choice(pool) for _ in range(rng.randint(0, 8)))
    if k == "bytes":
        return bytes(rng.choice([0, 9, 10, 34, 39, 65, 92, 127, 128, 255]) for _ in range(rng.randint(0, 6)))
    if k == "none":
        return None
    if k == "bool":
        return rng.choice([True, False])
    if k == "ellipsis":
        return ...
    n = rng.randint(0, 4)
    if k == "tuple":
        return tuple(random_value(rng, depth + 1, hashable) for _ in range(n))
    if k == "list":
        return [random_value(rng, depth + 1) for _ in range(n)]
    if k == "set":
        return {random_value(rng, depth + 1, True) for _ in range(n)}
    return {random_value(rng, depth + 1, True): random_value(rng, depth + 1) for _ in range(n)}


SURROGATE = re.compile(r"\\(u[dD][89abAB]|U0000[dD][89a-fA-F])")


def accepted_source(text):
    # A Grasm str cannot hold a lone surrogate, and `\N{...}` needs the
    # Unicode name list; pyliteral refuses both, so neither is compared.
    return "\\N" not in text and not SURROGATE.search(text) and \
        not any(0xD800 <= ord(ch) <= 0xDFFF for ch in text)


def literals(c, rng):
    fixed = ["\n1", "1\n", "\n 1", " 1", "\t1", "\f1", "1 # c", "[1,\n 2]",
             "[1,\r\n2]", "[1,\r2]", "1\r\n", "(1\\\n)", "1\\\n", "\\\n1", "1,",
             "1, 2", "()", "(1)", "-0j", "1-0j", "-1-0j", "1+-0j", "0.0-0j",
             "-0.0+0j", "1e999j", "-1e999-1e999j", "{1: 'a', 1.0: 'b', True: 'c'}",
             "{1, 1.0, True}", "{0: 1, -0.0: 2, False: 3, 0j: 4}", "'a' 'b'",
             "b'a' 'b'", "'a' f'b'", "set()", "set( )", "u'x'", "r'\\n'",
             "b'\\777'", "'\\777'", "'\\8'", "08", "0_0", "00", "1__0", "0x_f",
             "1.e5", ".5j", "1_0.5_0e1_0", "1e", "True+1j", "1+2j+3j", "(1)+2j",
             "(-1)+2j", "- 1", "-(1)", "+1", "++1", "1 if 1 else 2", "[1, 2,]",
             "{}", "{1:2,}", "{**{}}", "...", "Ellipsis", "None", "1\x00",
             "'\\x4'", "x", "\xa01", "1\xa0", "[" * 200 + "]" * 200,
             "[" * 201 + "]" * 201, "'''a\r\nb'''", "'a\\\r\nb'", "r'a\\\nb'",
             "rb'\\x'", "BR'\\''", "t'x'", "Rb\"\"", "ub''", "'\\u00e9'",
             "'\\U0001F600'", "'\\U00110000'", "'\\u12'", "b'\\u1234'", "b'é'",
             "10**2", "1 2", "[1 2]", "{1: 2, 3}", "{1, 2: 3}", "(1, *x)",
             "set([])", "frozenset()", "{[]: 1}", "{(1, [2]): 3}", "{{}}",
             "0b101", "0o777", "0B_1", "0b2", "0o8", "0x1j", "1jj", "0e0",
             "01.5", "09j", "0_9e1", "0_9", "1if 1 else 2", "(1)(2)", "[1][0]",
             "None()", "'a'[0]", "1.real", "1 .real", "-True", "-'a'",
             "--1", "\n\n1", "# c\n1", "1\n# c", "1\n\n", "[\n# c\n1\n]",
             "(\n)", "1;", "(1,)+2j", "1e5_0", "1_e5", "1e_5", "1.__class__",
             "'\\N{BULLET}'", "0x" + "f" * 100, "9" * 300, "-" + "9" * 300 + "+1j"]
    seen = set(fixed)
    for _ in range(9000):
        v = random_value(rng)
        text = repr(v)
        seen.add(text)
        # The same value in other spellings Python also reads.
        seen.add(text.replace(", ", ",").replace(": ", ":"))
        seen.add(" " + text.replace("[", "[ \\\n").replace(",", " ,\n "))
    tokens = ["1", "0x1", "1.5", "2j", "'a'", "b'x'", "r'\\'", "None", "True", "set",
              "(", ")", "[", "]", "{", "}", ",", ":", "+", "-", " ", "\n", "...",
              "#c\n", "\\\n", "'''x'''", "u", "f'z'", "e5", "_", "."]
    while len(seen) < 30000:
        seen.add("".join(rng.choice(tokens) for _ in range(rng.randint(1, 9))))
    for text in sorted(seen):
        if accepted_source(text):
            c.add("lit", text, "", literal_answer(text))


def marshals(c, rng):
    values = [None, True, False, ..., 0, -1, 2 ** 31 - 1, -2 ** 31, 2 ** 31, -2 ** 31 - 1,
              2 ** 100, -(2 ** 100), 2 ** 15, 2 ** 30, 0.1, -0.0, 1e300, 1e-300, math.inf,
              -math.inf, 5e-324, 1.5 + 2j, -0.0j, "", "abc", "a" * 300, "é", "中" * 100,
              b"", b"\x00\xff", (), (1,), [], {}, set(), frozenset(), frozenset({1, 2}),
              {"k": [1, (2, 3)], 4: {5: 6}}, [[1, 2], [3, [4, [5]]]], (1.0, 2, "x")]
    for _ in range(1500):
        values.append(random_value(rng))
    for v in values:
        for version in range(6):
            data = marshal.dumps(v, version)
            c.add("mload", data.hex(), "", th(canon(v)), raw_a=data)
            text = repr(v)
            if not accepted_source(text):
                continue
            try:
                if canon(ast.literal_eval(text)) != canon(v):
                    continue
            except Exception:
                continue
            has_set = "{" in canon(v) and ("set" in type(v).__name__ or _has_set(v))
            if version <= 2 and not _has_set(v):
                c.add("mdump", text, str(version), data.hex())
            else:
                c.add("mdump", text, str(version), "LOADS:" + canon(v))
    broken = [b"", b"i\x01", b"l\x01\x00\x00\x00\x00\x00", b"l\x01\x00\x00\x00\x00\x80",
              b"f\x04 1.5", b"f\x031_0", b"f\x03INF", b"f\x08infinity", b"f\x020x",
              b"z\x01\xe9", b"a\x02\x00\x00\x00\xff\x80", b"r\x00\x00\x00\x00",
              b"\xdb\x01\x00\x00\x00r\x00\x00\x00\x00", b"c", b"[\x01\x00\x00\x00",
              b"{i\x01\x00\x00\x00", b"<\x01\x00\x00\x00[\x00\x00\x00\x00", b"q",
              b"(\xff\xff\xff\xff", b"u\x02\x00\x00\x00\xc3\x28", b"N trailing",
              b"\xe9\x05\x00\x00\x00[\x02\x00\x00\x00r\x00\x00\x00\x00r\x00\x00\x00\x00",
              b"\xa9\x02\xfa\x01xr\x00\x00\x00\x00"]
    for data in broken:
        try:
            want = th(canon(marshal.loads(data)))
        except Exception:
            want = th("?")
        c.add("mload", data.hex(), "", want, raw_a=data)
    for _ in range(3000):
        data = bytearray(marshal.dumps(random_value(rng), rng.randint(0, 5)))
        if data:
            for _ in range(rng.randint(1, 3)):
                data[rng.randrange(len(data))] = rng.randrange(256)
        data = bytes(data)
        try:
            v = marshal.loads(data)
            if _cyclic(v) or _refused(v):
                continue
            text = canon(v)
            if not accepted_source(repr(text)):
                continue
            want = th(text)
        except RecursionError:
            continue
        except Exception:
            want = th("?")
        c.add("mload", data.hex(), "", want, raw_a=data)


def _refused(v):
    """What pymarshal refuses by design: StopIteration, code objects and
    slices, which are not data, and a str holding a lone surrogate."""
    if isinstance(v, str):
        return any(0xD800 <= ord(ch) <= 0xDFFF for ch in v)
    if isinstance(v, (list, tuple, set, frozenset)):
        return any(_refused(x) for x in v)
    if isinstance(v, dict):
        return any(_refused(k) or _refused(x) for k, x in v.items())
    return not isinstance(v, (int, float, complex, bytes, type(None), type(...)))


def _has_set(v):
    if isinstance(v, (set, frozenset)):
        return True
    if isinstance(v, (list, tuple)):
        return any(_has_set(x) for x in v)
    if isinstance(v, dict):
        return any(_has_set(k) or _has_set(x) for k, x in v.items())
    return False


def _cyclic(v, seen=()):
    if isinstance(v, (list, tuple, dict, set, frozenset)):
        if any(v is s for s in seen):
            return True
        items = list(v.items()) if isinstance(v, dict) else list(v)
        flat = []
        for item in items:
            flat += list(item) if isinstance(v, dict) else [item]
        return any(_cyclic(x, seen + (v,)) for x in flat)
    return False


def consistent(members):
    """Whether safe_lt is a strict order on these, so any stable sort agrees."""
    for a in members:
        for b in members:
            if a is not b and safe_lt(a, b) == safe_lt(b, a):
                return False
            for x in members:
                if safe_lt(a, b) and safe_lt(b, x) and not safe_lt(a, x):
                    return False
    return True


def pprint_safe(v):
    """Whether pprint's output for v is deterministic and in the order
    prettyrepr uses: every set's flat repr is already sorted, and nothing
    that pprint sorts needs its address to break a tie."""
    if isinstance(v, (set, frozenset)):
        if repr(v) != canon(v) or not consistent(list(v)):
            return False
        members = list(v)
        for a in members:
            for b in members:
                if a is not b and type(a) is type(b):
                    try:
                        a < b
                    except TypeError:
                        return False
        return all(pprint_safe(x) for x in members)
    if isinstance(v, dict):
        keys = list(v)
        if not consistent(keys):
            return False
        for a in keys:
            for b in keys:
                if a is not b and type(a) is type(b):
                    try:
                        a < b
                    except TypeError:
                        return False
        return all(pprint_safe(k) and pprint_safe(x) for k, x in v.items())
    if isinstance(v, (list, tuple)):
        return all(pprint_safe(x) for x in v)
    return True


def pprints(c, rng):
    texts = ["'" + "word " * 40 + "'", "b'" + "\\x00" * 60 + "'",
             "{'a': 1, 'b': [" + ", ".join(str(i) for i in range(40)) + "]}",
             "[" + ", ".join("'x' * %d" % i for i in range(0)) + "]",
             "('" + "long line\\n" * 12 + "',)", "{" + ", ".join("%d: 'v%d'" % (i, i) for i in range(30)) + "}",
             "12345678901234567890123", "{3, 1, 2}", "frozenset({1, 2})"]
    values = []
    for t in texts:
        try:
            values.append(ast.literal_eval(t))
        except Exception:
            pass
    for _ in range(2500):
        v = random_value(rng)
        if rng.random() < 0.5:
            v = [v, "some text that is long enough to wrap " * rng.randint(0, 3),
                 list(range(rng.randint(0, 30)))]
        values.append(v)
    for v in values:
        text = repr(v)
        if not accepted_source(text) or not pprint_safe(v):
            continue
        try:
            if canon(ast.literal_eval(text)) != canon(v):
                continue
        except Exception:
            continue
        for _ in range(3):
            indent = rng.choice([1, 1, 0, 2, 4])
            width = rng.choice([80, 80, 40, 20, 10, 1, 120])
            depth = rng.choice([None, None, 1, 2, 3])
            compact = rng.choice([False, True])
            sort_dicts = rng.choice([True, True, False])
            underscore = rng.choice([False, True])
            want = pprint.pformat(v, indent=indent, width=width, depth=depth,
                                  compact=compact, sort_dicts=sort_dicts,
                                  underscore_numbers=underscore)
            options = ",".join(map(str, [indent, width, depth or 0, int(compact),
                                         int(sort_dicts), int(underscore)]))
            c.add("pp", text, options, th(want))


def build():
    rng = random.Random(2031)
    c = Cases()
    getopts(c, rng)
    floats(c, rng)
    literals(c, rng)
    marshals(c, rng)
    pprints(c, rng)
    return c.argv, c.expected, c.labels
