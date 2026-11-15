"""Manifests to fuzz the TOML readers with (scripts/tomldiff.sh).

    python3 scripts/lib/tomlfuzz.py OUTDIR [COUNT] [SEED]

Writes COUNT files, `OUTDIR/NNNNN-<family>.toml`, the same bytes for the same
seed. Each is a manifest the way this repository writes them -- `[package]`
with its keys, `[dependencies.<name>]` tables, now and then an array of
tables -- written with the freedoms TOML allows and then, in two families of
three, changed:

  plain    the manifest dialect the Rust reader takes (`grasm_pkg::toml`):
           basic strings with `\\n`, `\\t`, `\\"` and `\\\\`, decimal integers,
           booleans, one-line arrays of those, comments, blank lines, tabs,
           CR LF line ends, quoted keys, `#` and quotes inside strings
  toml     valid TOML outside that dialect: floats, inline tables, arrays
           across lines, literal and multi-line strings, `\\u` escapes,
           integers in hex or with `_`, dates, dotted keys, nested arrays
  broken   not TOML: a header not closed, a line with no `=`, an empty key,
           a string or array not closed, a key or table twice, a bare word,
           a leading zero, an unknown escape, a key with a space in it

The family is a generator's intent, not a verdict: what each reader makes of
a file is what the gate compares.
"""

import os
import random
import sys

NAMES = ["app", "lib", "json", "http_server", "x", "_priv", "Δ", "my-app", "match", "a1"]
WORDS = ["io", "net", "proc", "unsafe", "mem", "async", "Clock", "Log", "", "a b", "é", "日本"]
STR_PIECES = ["a", "b", " ", "#", "x y", "é", "日本語", "😀", "\\n", "\\t", '\\"', "\\\\",
              "'", "=", "[", "]", ",", "{", "}", "0", "src/main.gr", "../lib"]


def plain_string(r):
    body = "".join(r.choice(STR_PIECES) for _ in range(r.randint(0, 5)))
    return '"' + body + '"'


def toml_string(r):
    pick = r.randint(0, 5)
    if pick == 0:
        return "'" + "".join(r.choice(["a", " ", "#", '"', "\\n", "é"]) for _ in range(r.randint(0, 5))) + "'"
    if pick == 1:
        return '"' + r.choice(["\\u00e9", "\\U0001F600", "\\r", "\\b", "\\f", "\\u0041"]) + "x" + '"'
    if pick == 2:
        return '"""' + r.choice(["one", "a\"b", "x # y"]) + '"""'
    if pick == 3:
        return "'''" + r.choice(["lit", "a'b", "c # d"]) + "'''"
    if pick == 4:
        return '"' + r.choice(["a#b", "#", "x' y"]) + '"'
    return '"\\u' + r.choice(["00e9", "0022", "005c"]) + '"'


def plain_int(r):
    n = r.choice([0, 1, 7, 42, -3, 9223372036854775807, -9223372036854775808, 1000])
    s = str(n)
    if n >= 0 and r.random() < 0.2:
        s = "+" + s
    return s


def plain_scalar(r):
    pick = r.randint(0, 3)
    if pick <= 1:
        return plain_string(r)
    if pick == 2:
        return plain_int(r)
    return r.choice(["true", "false"])


def plain_array(r):
    items = [plain_scalar(r) if r.random() < 0.3 else plain_string(r) for _ in range(r.randint(0, 4))]
    sep = r.choice([", ", ",", " , ", ",\t"])
    tail = "," if items and r.random() < 0.2 else ""
    pad = r.choice(["", " "])
    return "[" + pad + sep.join(items) + tail + pad + "]"


def plain_value(r):
    if r.random() < 0.25:
        return plain_array(r)
    return plain_scalar(r)


def toml_value(r):
    pick = r.randint(0, 9)
    if pick == 0:
        return r.choice(["1.5", "-0.0", "3e10", "inf", "nan", "+1.0", "6.02E23"])
    if pick == 1:
        return "{ " + ", ".join(f"{r.choice(['path', 'version', 'a'])}{i} = {plain_scalar(r)}"
                                for i in range(r.randint(0, 3))) + " }"
    if pick == 2:
        return "[\n  " + ",\n  ".join(plain_string(r) for _ in range(r.randint(1, 3))) + ",\n]"
    if pick == 3:
        return toml_string(r)
    if pick == 4:
        return r.choice(["0x1f", "0o17", "0b101", "1_000", "-1_2_3"])
    if pick == 5:
        return r.choice(["2026-08-27", "07:32:00", "1979-05-27T07:32:00Z", "1979-05-27 07:32:00"])
    if pick == 6:
        return "[[1, 2], [\"a\"]]"
    if pick == 7:
        return "[" + plain_string(r) + ", " + r.choice(["1.5", "{ a = 1 }", "[]"]) + "]"
    if pick == 8:
        return "[ # a comment\n  " + plain_string(r) + "\n]"
    return toml_string(r)


def key(r):
    k = r.choice(["name", "version", "capabilities", "entry", "links", "runtime", "path", "x", "k_2", "a-b"])
    if r.random() < 0.1:
        return '"' + k + '"'
    return k


def assign(r, k, v):
    eq = r.choice([" = ", "=", " =", "= ", "\t=\t", "  =  "])
    lead = r.choice(["", "", "", "  ", "\t"])
    trail = r.choice(["", "", "", " ", "\t", " # a comment", "  # \"quoted\" # twice"])
    return lead + k + eq + v + trail


def manifest(r):
    """The lines of a manifest in the plain dialect."""
    lines = []
    if r.random() < 0.3:
        lines.append("# " + r.choice(["a manifest", "with \"quotes\"", "and = signs", "[not a header]"]))
    if r.random() < 0.2:
        lines.append("")
    lines.append(r.choice(["[package]", "[ package ]", "[package] # the package", "  [package]"]))
    lines.append(assign(r, "name", '"' + r.choice(NAMES) + '"'))
    lines.append(assign(r, "version", '"' + r.choice(["0.1.0", "1.2.3", "+01.002.3", "1.0", "1.0.x"]) + '"'))
    if r.random() < 0.6:
        caps = [f'"{r.choice(WORDS)}"' for _ in range(r.randint(0, 3))]
        lines.append(assign(r, "capabilities", "[" + ", ".join(caps) + "]"))
    if r.random() < 0.2:
        lines.append(assign(r, "entry", '"' + r.choice(["src/main.gr", "src/Main.GR", "../x.gr", "a.txt"]) + '"'))
    if r.random() < 0.15:
        lines.append(assign(r, "links", "[" + ", ".join(f'"{x}"' for x in r.sample(["m", "SDL2", "-lm", "z"], 2)) + "]"))
    if r.random() < 0.1:
        lines.append(assign(r, "runtime", r.choice(["true", "false", '"yes"'])))
    used = {"name", "version", "capabilities", "entry", "links", "runtime"}
    for _ in range(r.randint(0, 2)):
        k = key(r)
        # A key set twice is refused by every reader; one in ten is enough.
        while k.strip('"') in used and r.random() < 0.9:
            k = key(r)
        used.add(k.strip('"'))
        lines.append(assign(r, k, plain_value(r)))
    for d in r.sample(["json", "util", "x", "far"], r.randint(0, 3)):
        lines.append("")
        lines.append(f"[dependencies.{d}]")
        if r.random() < 0.7:
            lines.append(assign(r, "path", f'"../{d}"'))
        if r.random() < 0.5:
            lines.append(assign(r, "version", '"' + r.choice(["^1.0.0", "=2.0.0", "1.2.3", "^0.1.0"]) + '"'))
    if r.random() < 0.2:
        for i in range(r.randint(1, 3)):
            lines.append("")
            lines.append("[[bin]]")
            if r.random() < 0.8:
                lines.append(assign(r, "name", f'"b{i}"'))
            if r.random() < 0.5:
                lines.append(assign(r, "args", plain_array(r)))
    return lines


def edit_toml(r, lines):
    pick = r.randint(0, 5)
    i = r.randint(1, len(lines))
    if pick <= 2:
        lines.insert(i, assign(r, key(r), toml_value(r)))
    elif pick == 3:
        lines.insert(i, assign(r, r.choice(["a.b", "deps.x.path", '"a.b"', "'lit'", "physical.color"]),
                               plain_scalar(r)))
    elif pick == 4:
        lines.append("")
        lines.append(r.choice(["[a . b]", '[a."b.c"]', "[ dependencies . y ]", "['quoted']", "[x.y.z]"]))
        lines.append(assign(r, "k", plain_scalar(r)))
    else:
        lines.insert(i, assign(r, key(r), toml_value(r)))
        lines.insert(i, assign(r, "z" + str(i), toml_value(r)))


def edit_broken(r, lines):
    pick = r.randint(0, 13)
    i = r.randint(1, len(lines))
    bad = [
        "[package",
        "[[bin]",
        "[]",
        "name",
        '= "x"',
        'k = "open',
        'k = ["a", "b"',
        "k = word",
        "k = 007",
        'k = "bad \\q escape"',
        'k = "a" "b"',
        "a key = 1",
        "k = ",
        'k = "x"y"',
    ]
    if pick < len(bad):
        lines.insert(i, bad[pick])
    else:
        # A key or a table twice.
        lines.append("[package]")


def write(out_dir, count, seed):
    r = random.Random(seed)
    os.makedirs(out_dir, exist_ok=True)
    for n in range(count):
        family = ["plain", "toml", "broken"][n % 3]
        lines = manifest(r)
        if family == "toml":
            for _ in range(r.randint(1, 2)):
                edit_toml(r, lines)
        elif family == "broken":
            edit_broken(r, lines)
        nl = "\r\n" if r.random() < 0.1 else "\n"
        text = nl.join(lines) + (nl if r.random() < 0.9 else "")
        with open(os.path.join(out_dir, f"{n:05d}-{family}.toml"), "w", encoding="utf-8", newline="") as f:
            f.write(text)


def main():
    out_dir = sys.argv[1]
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 600
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    write(out_dir, count, seed)
    return 0


if __name__ == "__main__":
    sys.exit(main())
