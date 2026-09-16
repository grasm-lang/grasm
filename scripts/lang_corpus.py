"""Cases for langtest, with the answers of five language runtimes.

`build()` returns (argv, expected, labels). Every command is three words: a
name and two hex arguments, each prefixed with `:` so an empty one survives.

The answers come from `node`, `tclsh`, `lua`, `ruby` and `perl` on this
machine — each asked through its own interpreter, in bulk, because starting
one of them per case would take longer than the rest of the gate put
together. A double is passed as the bits of its IEEE-754 representation, in
both directions, so nothing is lost to the decimal notation of whichever side
wrote it down — which is the whole subject of `jsnumber` and would be a
strange thing to assume in the test for it.
"""
import json
import os
import struct
import subprocess
import sys
import tempfile

ENV = dict(os.environ, TZ="UTC", LC_ALL="C")


def h(text):
    return ":" + text.encode("utf-8").hex()


def hb(raw):
    return ":" + raw.hex()


def th(text):
    return text.encode("utf-8").hex()


def bits(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


class Cases:
    def __init__(self):
        self.argv, self.expected, self.labels = [], [], []

    def add(self, what, a, b, want, raw_a=None):
        self.argv += [what, hb(raw_a) if raw_a is not None else h(a), h(b)]
        self.expected.append(want)
        self.labels.append(
            f"{what} {(raw_a if raw_a is not None else a)!r:.100} {b!r:.100}")


def run(args, data=None):
    return subprocess.run(args, input=data, capture_output=True, env=ENV)


def have(tool):
    return subprocess.run(["which", tool], capture_output=True).returncode == 0


# --- the doubles every one of these languages has to print -------------------

def doubles(extra=0, seed=17):
    """The doubles every one of these languages has to print."""
    out = [0.0, -0.0, 1.0, -1.0, 0.5, 2.0, 3.0, 10.0, 100.0, 1e21, 1e-7,
           1234567890.123, 0.1, 0.2, 0.3, 1 / 3, 2 / 3, 1e300, 1e-300,
           5e-324, 1.7976931348623157e308, 123456789012345680000.0,
           4.35, 1.005, 8.7, 0.000001, 0.0000001, 1e20, 1e-6,
           -1234.5678, 255.0, 256.0, 65535.0, 2 ** 53, 2 ** 53 + 2,
           -(2 ** 31), 2 ** 31 - 1, 1e15, 1e16, 1e17]
    for e in range(-40, 41, 3):
        out.append(float("1e%d" % e))
        out.append(float("9.87e%d" % e))
    for n in range(1, 60):
        out.append(n / 7.0)
        out.append(float(n) * 1024.0)
    if extra:
        # Bit patterns rather than decimal literals: a random *double* is what
        # shortest-round-trip printing is hard for, and picking one by writing
        # a decimal and parsing it only ever produces the easy ones.
        import random
        rng = random.Random(seed)
        while len(out) < extra:
            raw = rng.getrandbits(64)
            x = struct.unpack("<d", struct.pack("<Q", raw))[0]
            if x == x and abs(x) != float("inf"):
                out.append(x)
    return out


# --- JavaScript --------------------------------------------------------------

JS = r"""
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const buf = new ArrayBuffer(8);
const view = new DataView(buf);
const fromBits = (s) => { view.setBigInt64(0, BigInt(s)); return view.getFloat64(0); };
const toBits = (x) => { view.setFloat64(0, x); return view.getBigInt64(0).toString(); };
const out = [];
for (const line of lines) {
  if (!line) continue;
  const [what, a, b] = JSON.parse(line);
  let answer;
  try {
    if (what === 'jsstr') answer = fromBits(a).toString();
    else if (what === 'jsfix') answer = fromBits(a).toFixed(Number(b));
    else if (what === 'jsexp') answer = b === '' ? fromBits(a).toExponential()
      : fromBits(a).toExponential(Number(b));
    else if (what === 'jsprec') answer = b === '' ? fromBits(a).toPrecision()
      : fromBits(a).toPrecision(Number(b));
    else if (what === 'jspf') answer = toBits(parseFloat(a));
    else answer = toBits(Number(a));
  } catch (e) { answer = '?'; }
  out.push(answer);
}
process.stdout.write(out.join('\n') + '\n');
"""


def javascript(c):
    if not have("node"):
        return 0
    asks = []
    for x in doubles(4000):
        asks.append(("jsstr", str(bits(x)), ""))
        if abs(x) < 1e21:
            for digits in (0, 1, 2, 3, 6, 10, 20):
                asks.append(("jsfix", str(bits(x)), str(digits)))
        for digits in (0, 1, 2, 3, 6, 10, 20):
            asks.append(("jsexp", str(bits(x)), str(digits)))
        for digits in (1, 2, 3, 5, 15, 21):
            asks.append(("jsprec", str(bits(x)), str(digits)))
    texts = ["1", "1.5", " 12 ", "0x10", "0b101", "0o17", "1e3", "1e-3",
             ".5", "5.", "+7", "-7", "Infinity", "-Infinity", "NaN", "",
             "   ", "12abc", "abc", "1_000", "0.0000001", "1e400", "1e-400",
             "  0x1f  ", "0xg", "1.2.3", "٣", "1e", "e5", "-0"]
    for t in texts:
        asks.append(("jspf", t, ""))
        asks.append(("jsnum", t, ""))
    stdin = "\n".join(json.dumps(list(ask)) for ask in asks) + "\n"
    done = run(["node", "-e", JS], stdin.encode())
    if done.returncode != 0:
        print(done.stderr.decode()[-500:], file=sys.stderr)
        return 0
    answers = done.stdout.decode().split("\n")
    for (what, a, b), want in zip(asks, answers):
        if what in ("jspf", "jsnum"):
            c.add(what, a, b, want)
        else:
            c.add(what, a, b, th(want))
    return len(asks)


# --- Tcl ---------------------------------------------------------------------

TCL_SCRIPT = r"""
# Tcl 8.5 spellings: `binary encode/decode hex` arrived in 8.6, and the gate
# has to run against whichever `tclsh` is on the machine.
proc to_hex {s} {
    binary scan [encoding convertto utf-8 $s] H* out
    return $out
}

proc from_hex {h} {
    return [encoding convertfrom utf-8 [binary format H* $h]]
}

# `llength` is the only thing that says whether a string is a well-formed
# list; anything it refuses is what this package has to refuse too.
proc split_list {text} {
    if {[catch {llength $text} n]} { return -code error "unmatched" }
    set out {}
    foreach item $text { lappend out $item }
    return $out
}

set unit "\u0001"
while {[gets stdin line] >= 0} {
    if {$line eq ""} continue
    set what [string range $line 0 4]
    set body [from_hex [string range $line 6 end]]
    if {$what eq "tlist"} {
        set elements [split $body $unit]
        puts [to_hex [eval list $elements]]
    } else {
        if {[catch {split_list $body} parts]} {
            # Only the fact of the refusal is compared: Tcl's wording is
            # Tcl's, and a package that copied it word for word would be
            # worse to read an error from, not better.
            puts [to_hex "?"]
        } else {
            puts [to_hex [join $parts $unit]]
        }
    }
}
"""


def tcl(c):
    if not have("tclsh"):
        return 0
    lists = [["a"], ["a", "b"], [], [""], ["", ""], ["a b"], ["a\tb"],
             ["a\nb"], ["{"], ["}"], ["{}"], ["[a]"], ["$x"], ["\\"],
             ["a\\b"], ['"'], ["'"], ["a{b"], ["a}b"], ["one", "two three"],
             ["with space", "and{brace"], [" lead"], ["trail "], ["#hash"],
             ["a;b"], ["\u00e9"], ["\u4e2d\u6587"], ["0"], ["00"],
             ["a", "", "b"], ["{a b}"], ["]["], ["a\rb"], ["\x0b"],
             ["nested {list} here"], ["\\{"], ["\\}"], ["a\\ b"]]
    texts = ["a b c", "  a  b  ", "{a b} c", "a {b c}", "{}", "{ }",
             "a\\ b", "{a\\ b}", '"a b"', '"a" b', "a{b", "a}b", "{a",
             "a}", "", "   ", "\ta\tb\t", "a\nb", "{a {b c} d}", "{{}}",
             "\\{a\\}", "a\\nb", "[list a b]", "$x", "#c", "{a}{b}",
             "a{b}c", "\u00e9 \u4e2d", "{\u00e9}", "one\\", "{a b",
             "a b}", '"unterminated', "{deep {deeper {deepest}}}"]
    asks = []
    for one in lists:
        asks.append(("tlist", "\u0001".join(one)))
    for t in texts:
        asks.append(("tsplt", t))
    stdin = "".join("%s %s\n" % (what, body.encode("utf-8").hex())
                    for what, body in asks)
    with tempfile.NamedTemporaryFile("w", suffix=".tcl", delete=False) as f:
        f.write(TCL_SCRIPT)
        path = f.name
    done = subprocess.run(["tclsh", path], input=stdin.encode(),
                          capture_output=True, env=ENV)
    os.unlink(path)
    if done.returncode != 0:
        print(done.stderr.decode()[-500:], file=sys.stderr)
        return 0
    answers = done.stdout.decode().strip().split("\n")
    for (what, body), want in zip(asks, answers):
        c.add(what, body, "", want.strip())
    return len(asks)



# --- Lua ---------------------------------------------------------------------

LUA_SCRIPT = r"""
local function from_hex(h)
  return (h:gsub("%x%x", function(c) return string.char(tonumber(c, 16)) end))
end

local function to_hex(s)
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end

local function bits_to_float(s)
  return (string.unpack("<d", string.pack("<i8", math.tointeger(s))))
end

for line in io.lines() do
  if #line > 0 then
    local what = line:sub(1, 5)
    local rest = line:sub(7)
    local space = rest:find(" ")
    local a = from_hex(rest:sub(1, space - 1))
    local b = from_hex(rest:sub(space + 1))
    local answer
    if what == "lq   " then
      answer = string.format("%q", a)
    elseif what == "lqi  " then
      answer = string.format("%q", math.tointeger(tonumber(a)))
    elseif what == "lqf  " then
      answer = string.format("%q", bits_to_float(a))
    elseif what == "lts  " then
      answer = tostring(bits_to_float(a))
    elseif what == "lread" then
      local f = load("return " .. a)
      if f then
        local ok, v = pcall(f)
        answer = ok and v or "?"
      else
        answer = "?"
      end
    else
      local n = tonumber(a)
      if n == nil then
        answer = "?"
      elseif math.type(n) == "integer" then
        answer = "i:" .. tostring(n)
      else
        answer = "f:" .. tostring(string.unpack("<i8", string.pack("<d", n)))
      end
    end
    io.write(to_hex(tostring(answer)), "\n")
  end
end
"""


def lua(c):
    if not have("lua"):
        return 0
    strings = ["", "a", "abc", "a\nb", "a\tb", "a\"b", "a\\b", "a\rb",
               "\x00", "\x01\x02", "line1\nline2", "tab\there", "'single'",
               '"double"', "\x7f", "\xff", "caf\u00e9", "\u4e2d\u6587",
               "a" * 40, "0", "12", "-1", " ", "\v\f", "\a\b"]
    integers = [0, 1, -1, 7, -7, 255, 256, 65535, 2 ** 31, -(2 ** 31),
                2 ** 53, 2 ** 62, -(2 ** 62), 9223372036854775807,
                -9223372036854775807 - 1]
    # Every byte value on its own, which is what `%q` has to decide about one
    # at a time: printable, escape by name, or escape by number — and the
    # number form changes when a digit follows it.
    for b in range(256):
        strings.append(chr(b) if b < 128 else "\\x%02x" % b)
        strings.append((chr(b) if b < 128 else "") + "7")
    asks = []
    for one in strings:
        asks.append(("lq   ", one, ""))
        asks.append(("lread", '"%s"' % one.replace("\\", "\\\\").replace('"', '\\"')
                     .replace("\n", "\\n").replace("\r", "\\r")
                     .replace("\x00", "\\0"), ""))
    for n in integers:
        asks.append(("lqi  ", str(n), ""))
    for x in doubles(1200):
        asks.append(("lqf  ", str(bits(x)), ""))
        asks.append(("lts  ", str(bits(x)), ""))
    numbers = ["1", "1.5", "0x10", "0xA.8p1", "1e3", ".5", "5.", "  7  ",
               "-3", "+3", "inf", "nan", "0x", "", "1e", "9223372036854775807",
               "9223372036854775808", "0x7fffffffffffffff", "1_0", "0b101",
               "1.5e-3", "  -0x1f  ", "3.", ".", "..", "1e+3", "1e-"]
    for t in numbers:
        asks.append(("ltonu", t, ""))
    stdin = "".join("%s %s %s\n" % (what, a.encode("utf-8", "surrogateescape").hex(),
                                    b.encode("utf-8").hex())
                    for what, a, b in asks)
    with tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False) as f:
        f.write(LUA_SCRIPT)
        path = f.name
    done = subprocess.run(["lua", path], input=stdin.encode(),
                          capture_output=True, env=ENV)
    os.unlink(path)
    if done.returncode != 0:
        print(done.stderr.decode()[-500:], file=sys.stderr)
        return 0
    answers = done.stdout.decode().strip().split("\n")
    kept = 0
    for (what, a, b), want in zip(asks, answers):
        raw = a.encode("utf-8", "surrogateescape")
        c.add(what.strip(), a, b, want.strip(), raw_a=raw)
        kept += 1
    return kept



# --- Ruby --------------------------------------------------------------------

# `describe` here writes exactly what `langtest` writes, so the comparison is
# of the values rather than of two notations for them.
RUBY_SCRIPT = r"""
def describe(v)
  case v
  when nil then "nil"
  when true then "true"
  when false then "false"
  when Integer then "i#{v}"
  when Float then "f#{[v].pack('d').unpack1('q')}"
  when Symbol then ":#{v}"
  when Struct
    "S:#{v.class.name}(" + v.members.map { |m|
      m.to_s + "=" + describe(v[m]) }.join(",") + ")"
  when String
    enc = v.encoding == Encoding::UTF_8 ? "UTF-8" : (v.encoding == Encoding::BINARY ? "" : v.encoding.name)
    "s(#{enc})#{v.dup.force_encoding('BINARY').unpack1('H*')}"
  when Array then "[" + v.map { |x| describe(x) }.join(",") + "]"
  when Hash
    out = "{"
    v.each { |k, val| out += describe(k) + "=>" + describe(val) + "," }
    out += "default=" + describe(v.default) unless v.default.nil?
    out + "}"
  when Regexp
    "r(#{v.source.encoding.name},#{v.options})" +
      v.source.dup.force_encoding('BINARY').unpack1('H*')
  when Class then "c:#{v.name}"
  when Module then "m:#{v.name}"
  else
    names = v.instance_variables.map { |n| n.to_s }
    "o:#{v.class.name}(" + names.map { |n|
      n + "=" + describe(v.instance_variable_get(n)) }.join(",") + ")"
  end
end

Point = Struct.new(:x, :y)

class Thing
  def initialize(a, b)
    @a = a
    @b = b
  end
end

VALUES = [
  nil, true, false, 0, 1, -1, 7, 255, 256, 65535, 1073741823, 1073741824,
  -1073741824, 2**62, -(2**62), 2**100, -(2**100), 0.0, -0.0, 1.0, 0.5,
  1.0 / 3, 1e300, -1.5e-7, Float::INFINITY, -Float::INFINITY,
  "", "a", "hello", "café", "中文", "a\nb", "\x00\x01\xff".b,
  ("x" * 300), :sym, :"with space", [], [1], [1, "two", :three],
  [[1, [2, [3]]]], {}, {1 => 2}, {"a" => [1, 2], :b => nil},
  /abc/, /abc/ix, /café/u,
  Thing.new(1, "two"),
  [1, 1, 1], ["same", "same"],
]
shared = "shared"
VALUES << [shared, shared]
sym = :repeated
VALUES << [sym, sym, sym]
VALUES << Point.new(3, 4) if defined?(Point)

VALUES.each do |v|
  begin
    raw = Marshal.dump(v)
    puts raw.unpack1("H*") + " " + describe(v).unpack1("H*")
  rescue => e
    puts "- -"
  end
end
"""


def ruby(c):
    if not have("ruby"):
        return 0
    with tempfile.NamedTemporaryFile("w", suffix=".rb", delete=False) as f:
        f.write(RUBY_SCRIPT)
        path = f.name
    done = subprocess.run(["ruby", path], capture_output=True, env=ENV)
    os.unlink(path)
    if done.returncode != 0:
        print(done.stderr.decode()[-800:], file=sys.stderr)
        return 0
    kept = 0
    for line in done.stdout.decode().strip().split("\n"):
        if not line or line.startswith("-"):
            continue
        dumped, described = line.split(" ")
        raw = bytes.fromhex(dumped)
        # What it is, and that writing it again gives the same bytes.
        c.add("rload", "", "", described, raw_a=raw)
        c.add("rdump", "", "", dumped, raw_a=raw)
        kept += 2
    return kept


# --- Perl --------------------------------------------------------------------

PERL_SCRIPT = r"""
use strict;
use warnings;
no warnings 'pack';
binmode(STDOUT);
my $unit = chr(1);
while (my $line = <STDIN>) {
    chomp $line;
    next unless length $line;
    my ($what, $a, $b) = split / /, $line, 3;
    $a = defined $a ? pack("H*", $a) : "";
    $b = defined $b ? pack("H*", $b) : "";
    my $answer;
    if ($what eq "ppack") {
        my @values = length($b) ? split /$unit/, $b, -1 : ();
        my $packed = eval { pack($a, @values) };
        # `pack "U"` makes a *character* string, and this package answers in
        # bytes because the language has no other kind. Encoding it is what a
        # byte-oriented consumer of Perl's answer would get anyway — writing
        # such a string to a handle emits the same UTF-8, with a warning.
        utf8::encode($packed) if defined $packed && utf8::is_utf8($packed);
        $answer = defined $packed ? unpack("H*", $packed) : "3f";
        print "$answer\n";
        next;
    }
    my @got = eval { unpack($b, $a) };
    if ($@) {
        print unpack("H*", "?"), "\n";
    } else {
        print unpack("H*", join($unit, map { defined $_ ? $_ : "" } @got)), "\n";
    }
}
"""


def perl(c):
    if not have("perl"):
        return 0
    packs = [
        ("C", ["0", "65", "255", "256"]), ("c", ["-1", "127", "-128"]),
        ("n", ["0", "1", "65535"]), ("v", ["0", "258", "65535"]),
        ("N", ["0", "1", "4294967295"]), ("V", ["0", "16909060"]),
        ("s", ["-1", "32767"]), ("S", ["65535"]), ("l", ["-1", "2147483647"]),
        ("L", ["4294967295"]), ("q", ["-1", "9223372036854775807"]),
        ("Q", ["18446744073709551615"]),
        ("A5", ["ab"]), ("a5", ["ab"]), ("A*", ["hello"]), ("a*", ["hi"]),
        ("Z5", ["ab"]), ("Z*", ["abc"]),
        ("H*", ["deadbeef"]), ("h*", ["deadbeef"]), ("H4", ["abcd"]),
        ("B*", ["10110001"]), ("b*", ["10110001"]),
        ("w", ["0", "127", "128", "16384"]),
        ("f", ["1.5"]), ("d", ["1.5"]), ("d", ["0.1"]),
        ("x", []), ("x3", []), ("C x C", ["1", "2"]),
        ("CCC", ["1", "2", "3"]), ("C3", ["1", "2", "3"]),
        ("C*", ["1", "2", "3", "4"]), ("nN", ["1", "2"]),
        ("A3 n", ["abc", "7"]), ("Z* C", ["hi", "9"]),
        ("N n C", ["70000", "300", "9"]),
        ("j", ["-5"]), ("J", ["5"]), ("s<", ["-2"]), ("s>", ["-2"]),
        ("l<", ["-2"]), ("l>", ["-2"]), ("q<", ["-2"]), ("q>", ["-2"]),
        ("n!", ["7"]), ("@4 C", ["3"]), ("C @6 C", ["1", "2"]),
        ("U", ["233"]), ("U*", ["233", "20013"]),
    ]
    unpacks = [
        ("C*", bytes([0, 1, 254, 255])), ("c*", bytes([0, 127, 128, 255])),
        ("n*", bytes([1, 2, 3, 4])), ("v*", bytes([1, 2, 3, 4])),
        ("N", bytes([1, 2, 3, 4])), ("V", bytes([1, 2, 3, 4])),
        ("A5", b"ab   "), ("a5", b"ab\x00\x00\x00"), ("Z*", b"abc\x00def"),
        ("H*", b"\xde\xad"), ("h*", b"\xde\xad"), ("B*", b"\xb1"),
        ("b*", b"\xb1"), ("w*", bytes([0x7f, 0x81, 0x00])),
        ("f", struct.pack("<f", 1.5)), ("d", struct.pack("<d", 0.1)),
        ("q", struct.pack("<q", -2)), ("Q", struct.pack("<Q", 2 ** 63)),
        ("x C", bytes([9, 8])), ("C x C", bytes([1, 0, 2])),
        ("A3 n", b"abc\x00\x07"), ("N n C", bytes(range(1, 8))),
        ("U*", "café".encode("utf-8")),
        ("C/A", b"\x03abcdef"), ("n/A*", b"\x00\x02hi"),
        ("@2 C", bytes([1, 2, 3])), ("s<", bytes([254, 255])),
        ("s>", bytes([255, 254])), ("l>", bytes([0, 0, 0, 5])),
        ("j", struct.pack("<q", -5)), ("J", struct.pack("<Q", 5)),
    ]
    asks = []
    for template, values in packs:
        asks.append(("ppack", template, "\x01".join(values), b""))
    for template, data in unpacks:
        asks.append(("punpk", "", template, data))
    lines = []
    for what, a, b, raw in asks:
        if what == "ppack":
            lines.append("%s %s %s" % (what, text_hex(a), text_hex(b)))
        else:
            lines.append("%s %s %s" % (what, raw.hex(), text_hex(b)))
    with tempfile.NamedTemporaryFile("w", suffix=".pl", delete=False) as f:
        f.write(PERL_SCRIPT)
        path = f.name
    done = subprocess.run(["perl", path], input=("\n".join(lines) + "\n").encode(),
                          capture_output=True, env=ENV)
    os.unlink(path)
    if done.returncode != 0:
        print(done.stderr.decode()[-800:], file=sys.stderr)
        return 0
    answers = done.stdout.decode().strip().split("\n")
    kept = 0
    for (what, a, b, raw), want in zip(asks, answers):
        if what == "ppack":
            c.add(what, a, b, want.strip())
        else:
            c.add(what, "", b, want.strip(), raw_a=raw)
        kept += 1
    return kept


def text_hex(text):
    return text.encode("utf-8").hex()


def build():
    c = Cases()
    javascript(c)
    tcl(c)
    lua(c)
    ruby(c)
    perl(c)
    return c.argv, c.expected, c.labels
