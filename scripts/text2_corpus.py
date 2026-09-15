"""Z85, string.Template, expandvars and format strings, against Python.

Format strings are checked exhaustively: every string of up to five
characters drawn from the ones that matter to the parser, so the rules are
pinned by Python's own answers rather than by a reading of its C source.
"""
import base64
import itertools
import os
import posixpath
import random
import string


def hexed(text):
    return ":" + text.encode().hex()


def pairs(mapping):
    return "\x01".join(f"{k}={v}" for k, v in mapping.items())


def describe(pieces):
    out = []
    for literal, name, spec, conversion in pieces:
        if name is None:
            out.append(literal + "\x02-")
        else:
            out.append(literal + "\x02" + name + "\x02" + spec + "\x02"
                       + (conversion or ""))
    return "\x03".join(out)


def build():
    random.seed(23)
    argv, expected, labels = [], [], []

    def add(command, first, second, want, label):
        argv.extend([command, first, second])
        expected.append(want)
        labels.append(label)

    # Z85: every length up to a dozen, and text that is not Z85.
    for n in list(range(0, 13)) + [64, 100]:
        raw = bytes(random.randrange(256) for _ in range(n))
        encoded = base64.z85encode(raw).decode()
        add("z85enc", ":" + raw.hex(), ":", encoded.encode().hex(),
            f"z85 encode {n}")
        add("z85dec", hexed(encoded), ":", raw.hex(), f"z85 decode {n}")
    for bad in ("Hel~o", "#####", "a", "ab", "HelloWorld!", "héllo"):
        try:
            want = base64.z85decode(bad.encode()).hex()
        except Exception:
            want = "?refused?".encode().hex()
        add("z85dec", hexed(bad), ":", want, f"z85 edge {bad!r}")

    # string.Template.
    values = {"who": "tim", "what": "kung pao", "a_b": "x", "A": "big"}
    for text in ("$who likes $what", "${who}s", "$$5 costs $$", "$9x",
                 "${bad", "$ alone", "$who$what", "${a_b}_${A}", "$missing",
                 "", "no placeholders", "$A$a_b", "$_x", "${}", "$who.",
                 "trailing $"):
        template = string.Template(text)
        try:
            want = template.substitute(values)
        except Exception:
            want = "?refused?"
        add("tsub", hexed(text), hexed(pairs(values)), want.encode().hex(),
            f"substitute {text!r}")
        add("tsafe", hexed(text), hexed(pairs(values)),
            template.safe_substitute(values).encode().hex(),
            f"safe_substitute {text!r}")
        add("tids", hexed(text), ":",
            ",".join(template.get_identifiers()).encode().hex(),
            f"identifiers {text!r}")
        add("tvalid", hexed(text), ":",
            "yes" if template.is_valid() else "no", f"is_valid {text!r}")

    # expandvars and expanduser, with a controlled environment.
    env = {"HOME": "/home/u", "A": "1", "A_B": "2", "EMPTY": "", "1": "one"}
    saved = dict(os.environ)
    try:
        os.environ.clear()
        os.environ.update(env)
        for text in ("$A $A_B ${A} $Z ${Z} $ $1 ${} $A-B", "a${A_B}b",
                     "$$A", "${A", "$EMPTY|", "${a b}", "no dollars", "$",
                     "$A$A_B$A", "~/x $A"):
            add("evars", hexed(text), hexed(pairs(env)),
                posixpath.expandvars(text).encode().hex(),
                f"expandvars {text!r}")
        for home in ("/home/u", "/home/u/", "/", ""):
            os.environ["HOME"] = home
            for text in ("~", "~/x", "~/", "x/~", "~root/x", "~~"):
                if text.startswith("~root"):
                    continue
                add("euser", hexed(text), hexed(home),
                    posixpath.expanduser(text).encode().hex(),
                    f"expanduser {text!r} home {home!r}")
    finally:
        os.environ.clear()
        os.environ.update(saved)

    # Formatter.parse, exhaustively over short strings.
    formatter = string.Formatter()
    alphabet = "{}!:[]a0"
    for length in range(0, 6):
        for chars in itertools.product(alphabet, repeat=length):
            text = "".join(chars)
            try:
                want = describe(list(formatter.parse(text)))
            except Exception:
                want = "?refused?"
            add("fparse", hexed(text), ":", want.encode().hex(),
                f"parse {text!r}")

    # str.format with string values. Attribute access is left out: on a
    # string it reaches a bound method, whose text includes an address.
    for text, positional, named in (
            ("{}{}", ["a", "b"], {}), ("{0}{}", ["a", "b"], {}),
            ("{}{0}", ["a", "b"], {}), ("{0:{1}}", ["x", ">5"], {}),
            ("{n!r}", [], {"n": "it's"}), ("{n!r}", [], {"n": 'a"b\'c'}),
            ("{n!s:^7}", [], {"n": "ab"}), ("{0[1]}", ["abc"], {}),
            ("{name:>10}|{size}", [], {"name": "file", "size": "12"}),
            ("{missing}", [], {}), ("{0}", [], {}), ("{n!x}", [], {"n": "a"}),
            ("{{literal}} {0}", ["x"], {}),
            ("{:{}}", ["x", "4"], {}), ("{n!r}", [], {"n": "tab\there"})):
        try:
            want = text.format(*positional, **named)
        except Exception:
            want = "?refused?"
        add("fformat", hexed(text),
            hexed("\x01".join(positional) + "\x04" + pairs(named)),
            want.encode().hex(), f"format {text!r}")
    # ndiff, over line lists that exercise the similar-line pairing, the
    # window, whitespace kept under the markers, and blocks either side of
    # a synch pair.
    import difflib
    pairs_of_lines = [
        (["one\n", "two\n", "three\n"], ["ore\n", "tree\n", "emu\n"]),
        ([], []), (["a\n"], []), ([], ["a\n"]),
        (["same\n"], ["same\n"]),
        (["\tindented line here\n"], ["\tindented lime here\n"]),
        (["abcDefghiJkl\n"], ["abcdefGhijkl\n"]),
        (["completely different\n"], ["nothing alike at all\n"]),
        (["x = 1\n", "y = 2\n", "z = 3\n"], ["x = 1\n", "y = 20\n",
                                              "z = 3\n"]),
        (["  spaced   out\n"], ["spaced out\n"]),
        (["no newline"], ["no newlines"]),
        (["line %d\n" % i for i in range(30)],
         ["line %d\n" % (i if i % 7 else i + 100) for i in range(30)]),
    ]
    for _ in range(40):
        a = ["".join(random.choice("abcde \t") for _ in
                     range(random.randrange(0, 12))) + "\n"
             for _ in range(random.randrange(0, 8))]
        b = list(a)
        for _ in range(random.randrange(0, 4)):
            if not b:
                break
            where = random.randrange(len(b))
            line = list(b[where])
            if len(line) > 1:
                line[random.randrange(len(line) - 1)] = random.choice("xyz")
            b[where] = "".join(line)
        if random.random() < 0.3 and b:
            del b[random.randrange(len(b))]
        pairs_of_lines.append((a, b))
    for a, b in pairs_of_lines:
        delta = list(difflib.ndiff(a, b))
        add("ndiff", hexed("\x01".join(a)), hexed("\x01".join(b)),
            "\x01".join(delta).encode().hex(), f"ndiff {a[:2]!r}")
        for which in (1, 2):
            add("nrestore", hexed("\x01".join(delta)), hexed(str(which)),
                "\x01".join(difflib.restore(delta, which)).encode().hex(),
                f"restore {which} {a[:2]!r}")
    return argv, expected, labels
