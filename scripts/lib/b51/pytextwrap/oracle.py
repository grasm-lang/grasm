"""The oracle for packages/pytextwrap: CPython 3.13's textwrap. Text is
hex UTF-8, '-' when empty; options are joined by '/'."""
import sys
import textwrap


def text(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


def hx(s):
    return s.encode().hex() or "-"


def opts(spec):
    f = spec.split("/")
    return dict(width=int(f[0]), initial_indent=text(f[1]), subsequent_indent=text(f[2]), expand_tabs=f[3] == "1", tabsize=int(f[4]),
                replace_whitespace=f[5] == "1", fix_sentence_endings=f[6] == "1", break_long_words=f[7] == "1", drop_whitespace=f[8] == "1",
                break_on_hyphens=f[9] == "1", max_lines=None if f[10] == "~" else int(f[10]), placeholder=text(f[11]))


def line(f):
    try:
        if f[0] == "wr":
            return ",".join(hx(l) for l in textwrap.TextWrapper(**opts(f[2])).wrap(text(f[1]))) or "~"
        if f[0] == "sh":
            o = opts(f[2])
            del o["max_lines"]
            width = o.pop("width")
            return hx(textwrap.shorten(text(f[1]), width, **o))
        if f[0] == "de":
            return hx(textwrap.dedent(text(f[1])))
        return hx(textwrap.indent(text(f[1]), text(f[2])))
    except ValueError as e:
        return "E" + hx(str(e))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
