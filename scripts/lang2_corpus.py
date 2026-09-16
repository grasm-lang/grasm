"""Cases for langtest2, with the answers of Python, Node and Perl.

`build()` returns (argv, expected, labels). Every command is three words: a
name and two hex arguments, each prefixed with `:` so an empty one survives.

`shlex`, `configparser` and `tokenize` are asked of Python itself; the query
string is asked of Node, whose `querystring` is the one being matched; and
`printf` is asked of Perl, because C's own `printf` is not reachable from here
and Perl's is the same conversions with the same rounding.
"""
import io
import json
import os
import struct
import subprocess
import sys
import tempfile

ENV = dict(os.environ, TZ="UTC", LC_ALL="C")


def h(text):
    return ":" + text.encode("utf-8", "surrogatepass").hex()


def th(text):
    return text.encode("utf-8", "surrogatepass").hex()


def bits(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


class Cases:
    def __init__(self):
        self.argv, self.expected, self.labels = [], [], []

    def add(self, what, a, b, want):
        self.argv += [what, h(a), h(b)]
        self.expected.append(want)
        self.labels.append(f"{what} {a!r:.90} {b!r:.60}")


def have(tool):
    return subprocess.run(["which", tool], capture_output=True).returncode == 0


UNIT = "\x01"


# --- shlex -------------------------------------------------------------------

def shlex_cases(c):
    import shlex as pyshlex
    pieces = ["a", " ", "'", '"', "\\", "#", "b", "\t", "="]
    generated = []
    for one in pieces:
        for two in pieces:
            for three in pieces:
                generated.append(one + two + three)
    words = generated + ["", " ", "a", "a b", "  a   b  ", "a\tb", "a\nb", "'a b'",
             '"a b"', "'a b'c", "a'b'c", '"a"b', "''", '""', "a\\ b", "a\\\\b",
             "a\\", "'unterminated", '"unterminated', "a#b", "#c", "a #c",
             "a \\#b", "x=1 y=2", "--flag=va lue", "'it'\\''s'", "\\'",
             '"a\\"b"', '"a\\nb"', "'a\\nb'", "$HOME", "`cmd`", "a|b", "a;b",
             "a&&b", "(a)", "{a}", "a>b", "*.py", "~/x", "\\n", "\\\\",
             "a''b", 'a""b', "'\"'", "\"'\"", "   \t  ", "a  \\  b",
             "\u00e9 \u4e2d", "'\u00e9'", "a\x0bb", "a\x0cb"]
    for w in words:
        for what, comments in (("ssp", False), ("ssc", True)):
            try:
                got = UNIT.join(pyshlex.split(w, comments=comments))
            except ValueError as e:
                got = "?" + str(e)
            c.add(what, w, "", th(got))
    # quote and join, whose job is that the answer survives being split again
    for w in words + ["it's", "a\"b", "!", "$x", "\n", "\t", "-", "--",
                      "a=b", "/usr/bin", "1.5", "x" * 40]:
        c.add("sq", w, "", th(pyshlex.quote(w)))
    # No list of empty strings: the harness passes a list as one string with
    # a separator, and `[""]` and `[]` are the same string.
    lists = [["a"], ["a", "b"], ["a b", "c"], ["it's", "x"],
             ["a\nb"], ["-l", "--flag=a b"], ["$x", "`y`"], ["a" * 30, "b"]]
    for one in lists:
        c.add("sj", UNIT.join(one), "", th(" ".join(pyshlex.quote(x)
                                                   for x in one)))


# --- configparser ------------------------------------------------------------

CONFIGS = [
    "[a]\nx = 1\n",
    "[a]\nx: 1\n",
    "[a]\nX = 1\n",
    "[a]\nx=1\ny=2\n[b]\nz=3\n",
    "[a]\nx = 1\n  continued\n  more\n",
    "[a]\nx =\n",
    "[a]\nx = a:b=c\n",
    "[DEFAULT]\nd = 9\n[a]\nx = 1\n",
    "[DEFAULT]\nd = 9\n[a]\nd = 1\n",
    "[a]\n# comment\n; other\nx = 1\n",
    "[a]\nx = 1\n\n[a]\ny = 2\n",
    "[a]\nx = 1\nx = 2\n",
    "[a with spaces]\nx = 1\n",
    "[a]\n\n\nx = 1\n",
    "[a]\nkey with spaces = 1\n",
    "[a]\nx = value # not a comment\n",
    "[a]\nx = %%literal\n",
    "[home]\ndir = /x\nbin = %(dir)s/bin\n",
    "[home]\ndir = /x\nbin = %(dir)s/bin\nboth = %(bin)s:%(dir)s\n",
    "[a]\nx = 1\n[b]\ny = ${a:x}/2\n",
    "[a]\nx = ${y}\ny = 2\n",
    "[a]\nx = \u00e9\n",
]


def configparser_cases(c):
    import configparser

    def answer(text, extended, what, arg):
        kind = (configparser.ExtendedInterpolation() if extended
                else configparser.BasicInterpolation())
        p = configparser.ConfigParser(allow_no_value=True, interpolation=kind,
                                      strict=False)
        try:
            p.read_string(text)
        except Exception as e:
            return "?" + type(e).__name__
        if what == "sections":
            return UNIT.join(p.sections())
        if what == "keys":
            if not p.has_section(arg):
                return ""
            return UNIT.join(p[arg].keys())
        section, key = arg.split(UNIT)
        try:
            value = p.get(section, key)
        except Exception as e:
            return "?" + type(e).__name__
        return "" if value is None else value

    for text in CONFIGS:
        for extended in (False, True):
            suffix = "x" if extended else ""
            c.add("csec" + suffix, text, "", th(answer(text, extended,
                                                       "sections", "")))
            for section in ("a", "b", "home", "DEFAULT", "missing"):
                c.add("ckeys" + suffix, text, section,
                      th(answer(text, extended, "keys", section)))
                for key in ("x", "y", "d", "bin", "both", "missing"):
                    arg = section + UNIT + key
                    c.add("cget" + suffix, text, arg,
                          th(answer(text, extended, "get", arg)))


# --- the query string, as Node writes it -------------------------------------

QUERIES = [one + two + three
           for one in ("a", "=", "&", "%", "2", "+")
           for two in ("a", "=", "&", "%", "2", "+")
           for three in ("a", "=", "&", "%", "2", "+")] + ["a=1&b=2", "a=1&a=2", "a", "a=", "=1", "a=1&&b=2", "a%20b=c%2Bd",
           "a=%E4%B8%AD", "a[]=1&a[]=2", "", "&", "a=1;b=2", "a=%zz", "+=+",
           "a=b=c", "%2F=%2F", "a=1&b&c=3", "a+b=c+d", "%=%", "a=%E4",
           "key=value&key=value", "a=%00", "\u00e9=\u00e9", "a=%ZZ%20",
           "a==b", "&&&", "a=1&", "&a=1", "a%3Db=c"]
TEXTS = ["", "a", "a b", "a+b", "a/b?c#d", "\u4e2d\u6587", "\u00e9",
         "!*'()", "-_.~", "%", "%20", "a%2Bb", "\x00\x01", "A" * 20, "1.5",
         "a&b=c", "\ud83d\ude00" if False else "\U0001f600"]

NODE_SCRIPT = r"""
const qs = require('querystring');
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const out = [];
const UNIT = '\u0001';
for (const line of lines) {
  if (!line) continue;
  const [what, a] = JSON.parse(line);
  let answer;
  if (what === 'qesc') answer = qs.escape(a);
  else if (what === 'qune') answer = qs.unescape(a);
  else {
    const parsed = qs.parse(a);
    if (what === 'qstr') answer = qs.stringify(parsed);
    else {
      const parts = [];
      for (const key of Object.keys(parsed)) {
        const v = parsed[key];
        parts.push(key + '=' + (Array.isArray(v) ? v.join(',') : v));
      }
      answer = parts.join(UNIT);
    }
  }
  out.push(Buffer.from(answer, 'utf8').toString('hex'));
}
process.stdout.write(out.join('\n') + '\n');
"""


def query_cases(c):
    if not have("node"):
        return
    asks = []
    for q in QUERIES:
        asks.append(("qpar", q))
        asks.append(("qstr", q))
    for t in TEXTS:
        asks.append(("qesc", t))
        asks.append(("qune", t))
    stdin = "\n".join(json.dumps([what, a]) for what, a in asks) + "\n"
    done = subprocess.run(["node", "-e", NODE_SCRIPT], input=stdin.encode(),
                          capture_output=True, env=ENV)
    if done.returncode != 0:
        print(done.stderr.decode()[-600:], file=sys.stderr)
        return
    for (what, a), want in zip(asks, done.stdout.decode().strip().split("\n")):
        c.add(what, a, "", want.strip())


# --- printf, as Perl writes it -----------------------------------------------

PERL_SCRIPT = r"""
use strict;
use warnings;
no warnings;
my $unit = chr(1);
while (my $line = <STDIN>) {
    chomp $line;
    next unless length $line;
    my ($fmt_hex, $args_hex) = split / /, $line, 2;
    my $fmt = pack("H*", $fmt_hex);
    my $raw = defined $args_hex && length $args_hex ? pack("H*", $args_hex) : "";
    my @args;
    if (length $raw) {
        for my $piece (split /$unit/, $raw, -1) {
            my $kind = substr($piece, 0, 2);
            my $rest = substr($piece, 2);
            if ($kind eq 'i:') { push @args, 0 + $rest; }
            elsif ($kind eq 'f:') { push @args, unpack("d", pack("q", 0 + $rest)); }
            else { push @args, $rest; }
        }
    }
    my $answer = eval { sprintf($fmt, @args) };
    $answer = "?" unless defined $answer;
    # `%c` of a code point above 127 makes a character string; these packages
    # answer in bytes, so Perl is asked for the bytes it would write out.
    utf8::encode($answer) if utf8::is_utf8($answer);
    print unpack("H*", $answer), "\n";
}
"""

FORMATS = [
    ("%d", ["i:42"]), ("%d", ["i:-42"]), ("%d", ["i:0"]),
    ("%5d", ["i:42"]), ("%-5d|", ["i:42"]), ("%05d", ["i:42"]),
    ("%+d", ["i:42"]), ("% d", ["i:42"]), ("%+d", ["i:-42"]),
    ("%.5d", ["i:42"]), ("%8.5d", ["i:42"]), ("%-8.5d|", ["i:42"]),
    ("%x", ["i:255"]), ("%X", ["i:255"]), ("%#x", ["i:255"]),
    ("%#X", ["i:255"]), ("%08x", ["i:255"]), ("%o", ["i:8"]),
    ("%#o", ["i:8"]), ("%b", ["i:5"]), ("%#b", ["i:5"]), ("%x", ["i:0"]),
    ("%#x", ["i:0"]),
    # `%c` of a code point above 127: Perl writes one byte, this writes the
    # character, because a `str` is UTF-8 here. Asked only where they agree.
    ("%c", ["i:65"]), ("%c", ["i:10"]), ("%5c|", ["i:65"]),
    ("%s", ["s:hi"]), ("%10s|", ["s:hi"]), ("%-10s|", ["s:hi"]),
    ("%.1s", ["s:hi"]), ("%.0s|", ["s:hi"]), ("%s", ["s:"]),
    ("%%"), ("a%%b"), ("%s%%%s", ["s:a", "s:b"]),
    ("%2$s-%1$s", ["s:a", "s:b"]), ("%1$s%1$s", ["s:a"]),
    ("%*d", ["i:6", "i:42"]), ("%-*d|", ["i:6", "i:42"]),
    ("%.*f", ["i:2", "f:" + str(bits(3.14159))]),
]


def printf_cases(c):
    if not have("perl"):
        return
    asks = []
    for entry in FORMATS:
        if isinstance(entry, str):
            asks.append((entry, []))
        else:
            asks.append(entry)
    floats = [0.0, -0.0, 0.5, 1.5, 2.5, -1.5, 1.0 / 3, 2.0 / 3, 1e-7, 1e21,
              123456.789, 0.0001, 0.00001, 1234567.0, 1e300, 5e-324,
              1.7976931348623157e308, 0.1, 0.2, 0.3, 99.995, 9.995, 1.005,
              1e15, 1e16, -1234.5678, 255.0, 1e-300, 3.0, 100.0]
    import random
    rng = random.Random(23)
    while len(floats) < 900:
        raw = rng.getrandbits(64)
        x = struct.unpack("<d", struct.pack("<Q", raw))[0]
        if x == x and abs(x) != float("inf"):
            floats.append(x)
    for x in floats:
        for fmt in ("%f", "%.0f", "%.1f", "%.3f", "%.10f", "%e", "%.0e",
                    "%.2e", "%g", "%.1g", "%.3g", "%.10g", "%#g", "%E",
                    "%G", "%12.4f", "%-12.4f|", "%+.2f", "%012.3f", "%.17g",
                    "% .4e", "%#.0f", "%-+15.6g|", "%.0g"):
            asks.append((fmt, ["f:" + str(bits(x))]))
    ints = [0, 1, -1, 7, 255, 256, 65535, 2 ** 31, -(2 ** 31), 2 ** 62,
            -(2 ** 62), 9223372036854775807, -9223372036854775807 - 1]
    rng2 = random.Random(29)
    while len(ints) < 300:
        ints.append(rng2.getrandbits(64) - 2 ** 63)
    for n in ints:
        for fmt in ("%d", "%10d", "%-10d|", "%010d", "%+d", "%x", "%X", "%o",
                    "%b", "%#x", "%.20d", "% d", "%#o", "%#b", "%5.3d",
                    "%-20.15d|", "%u"):
            asks.append((fmt, ["i:" + str(n)]))
    for text in ["", "a", "hello", "a b", "\u00e9", "\u4e2d\u6587", "x" * 30]:
        for fmt in ("%s", "%10s|", "%-10s|", "%.0s|", "%20.3s|"):
            # Not `%.2s`: two bytes lands inside a three-byte character, and
            # Perl answers with half of one. A `str` cannot hold that, so the
            # package cuts back to the last whole character instead — the one
            # place it deliberately says something else, documented where it
            # happens.
            asks.append((fmt, ["s:" + text]))
    lines = []
    for fmt, args in asks:
        lines.append("%s %s" % (th(fmt), th(UNIT.join(args))))
    with tempfile.NamedTemporaryFile("w", suffix=".pl", delete=False) as f:
        f.write(PERL_SCRIPT)
        path = f.name
    done = subprocess.run(["perl", path], input=("\n".join(lines) + "\n").encode(),
                          capture_output=True, env=ENV)
    os.unlink(path)
    if done.returncode != 0:
        print(done.stderr.decode()[-600:], file=sys.stderr)
        return
    answers = done.stdout.decode().strip().split("\n")
    for (fmt, args), want in zip(asks, answers):
        c.add("fmt", fmt, UNIT.join(args), want.strip())


# --- tokenize ----------------------------------------------------------------

SOURCES = [
    "x = 1\n",
    "x=1\ny=2\n",
    "if x:\n    y = 1\n",
    "if x:\n    if y:\n        z = 1\n    w = 2\nv = 3\n",
    "def f(a, b=1, *c, **d):\n    return a\n",
    "x = 'a'\n",
    'x = "a"\n',
    "x = '''a\nb'''\n",
    'x = """a\nb"""\n',
    "x = r'\\n'\n",
    "x = b'\\x00'\n",
    "x = rb'a'\n",
    "x = f'{y}'\n",
    "x = f'a{y}b'\n",
    "x = f'{y!r:>10}'\n",
    "x = f'{{literal}}'\n",
    "x = f'{a+b}'\n",
    "x = f''\n",
    "x = f'{a[0]}'\n",
    "x = f'{f(1)}'\n",
    "x = F'{x}'\n",
    "x = f'{x:{w}}'\n",
    'x = f"""a{b}c\nd{e}f\n"""\ny = 1\n',
    'x = f"""plain\nlines\n"""\n',
    'x = f"""{a}\n{b}"""\n',
    "x = f'a{1}b{2}c'\n",
    "x = 'it\\'s'\n",
    "x = 1_000\n",
    "x = 0x_ff\n",
    "x = 0o777\n",
    "x = 0b1010\n",
    "x = 1.5e-3\n",
    "x = .5\n",
    "x = 5.\n",
    "x = 1j\n",
    "x = 1.5j\n",
    "x = (1,\n     2)\n",
    "x = [1,\n 2]\n",
    "x = {1:\n2}\n",
    "x = 1 + \\\n    2\n",
    "# just a comment\n",
    "\n\n\nx = 1\n",
    "    # indented comment\nx = 1\n",
    "x = 1  # trailing\n",
    "x == y != z <= w >= v\n",
    "x **= 2\n",
    "x //= 2\n",
    "x := 2\n",
    "a @ b\n",
    "x -> y\n",
    "...\n",
    "x = a.b.c\n",
    "x = a[1:2]\n",
    "class A(B):\n    pass\n",
    "async def f():\n    await g()\n",
    "lambda x: x\n",
    "x = '\u00e9'\n",
    "\u00e9 = 1\n",
    "x = 1;y = 2\n",
    "\tx = 1\n",
    "if a:\n\tb = 1\n",
    "x = ()\n",
    "x = 1 if y else 2\n",
]


# Three cases whose own escaping is the point, built rather than written out:
# a raw string holding backslash-quote, a plain string holding two
# backslashes, and a string holding an escaped quote.
SOURCES += [
    'x = r"' + chr(92) + "'" + chr(92) + '""' + chr(10),
    "x = '" + chr(92) + chr(92) + "'" + chr(10),
    'x = "a' + chr(92) + '"b"' + chr(10),
]


def repo_sources():
    """This repository's own Python, which is the only input here that nobody
    wrote for the test."""
    import glob
    out = []
    for path in sorted(glob.glob("scripts/*.py")):
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except OSError:
            continue
        # A few thousand tokens each is plenty; the whole file keeps the
        # indentation stack and the bracket depth honest.
        out.append(text)
    return out


def tokenize_cases(c):
    import tokenize as pytok
    import token as toknames

    for src in SOURCES + repo_sources():
        parts = []
        try:
            for t in pytok.generate_tokens(io.StringIO(src).readline):
                name = toknames.tok_name[t.type]
                if name == "ENCODING":
                    continue
                parts.append("%d,%d-%d,%d:\t%s\t%s" % (
                    t.start[0], t.start[1], t.end[0], t.end[1], name,
                    t.string))
            got = "\n".join(parts)
        except Exception as e:
            got = "?" + type(e).__name__
        c.add("tok", src, "", th(got))


def build():
    c = Cases()
    shlex_cases(c)
    configparser_cases(c)
    query_cases(c)
    printf_cases(c)
    tokenize_cases(c)
    return c.argv, c.expected, c.labels
