"""Syntax highlighting for Grasm, with the keyword list taken from the compiler.

A highlighter with its own copy of the keywords drifts the moment one is added,
and the drift is invisible: the page still renders, one word is just the wrong
colour. So the list is read out of `crates/grasm_syntax/src/token.rs` when the
site is written, and `scripts/site.py --check` fails if this file cannot find
it. That is the same rule every count on the site follows.

`tests/highlight` in `site.py` cross-checks the classification against
`grasm tokens` — the real lexer — so a rule that disagrees with the compiler is
a failure rather than a colour nobody notices.
"""

import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def keywords():
    """Every keyword, from the compiler's own `keyword_from`."""
    path = os.path.join(ROOT, "crates", "grasm_syntax", "src", "token.rs")
    src = open(path).read()
    body = re.search(r"pub fn keyword_from.*?\n    \}", src, re.S)
    if not body:
        raise SystemExit("token.rs no longer has a keyword_from to read")
    found = re.findall(r'"([a-z_]+)" =>', body.group(0))
    if len(found) < 20:
        raise SystemExit(f"only {len(found)} keywords found in token.rs — the shape changed")
    return set(found)


KW = keywords()

# Types and constructors are capitalised; effects are the lowercase names in a
# row. Neither is a keyword, and both want their own colour.
BUILTIN_TYPES = {
    "i64", "f64", "bool", "str", "bytes", "Option", "Map", "Some", "None",
    "Ok", "Err",
}

TOKEN = re.compile(
    r"""
    (?P<comment>//[^\n]*)
  | (?P<string>"(?:\\.|\{[^}]*\}|[^"\\])*")
  | (?P<number>\b\d[\d_]*(?:\.\d+)?\b)
  | (?P<word>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<op>->|=>|\|>|\?\?|\?\.|\.\.=|\.\.|==|!=|<=|>=|\+=|-=|\*=|/=|::|[-+*/%^=<>!|&?:.,;(){}\[\]@])
    """,
    re.X,
)

ESCAPE = str.maketrans({"&": "&amp;", "<": "&lt;", ">": "&gt;"})


def classify(word, after):
    if word in KW:
        return "kw"
    if word in BUILTIN_TYPES:
        return "ty"
    if word[0].isupper():
        return "ty"
    # A name immediately followed by `(` is being called.
    if after == "(":
        return "fn"
    return None


def highlight(src):
    """Grasm source as HTML, with every token in a span of its own class."""
    src = src.strip("\n")
    out = []
    i = 0
    for m in TOKEN.finditer(src):
        if m.start() > i:
            out.append(src[i:m.start()].translate(ESCAPE))
        text = m.group(0)
        kind = m.lastgroup
        cls = None
        if kind == "comment":
            cls = "cm"
        elif kind == "string":
            cls = "st"
        elif kind == "number":
            cls = "nu"
        elif kind == "op":
            cls = "op"
        elif kind == "word":
            nxt = src[m.end():m.end() + 1]
            cls = classify(text, nxt)
        safe = text.translate(ESCAPE)
        out.append(f'<span class="{cls}">{safe}</span>' if cls else safe)
        i = m.end()
    out.append(src[i:].translate(ESCAPE))
    return "".join(out)


def code(src, caption=None):
    """A highlighted code block, optionally captioned."""
    body = f"<pre><code>{highlight(src)}</code></pre>"
    if caption:
        return f'<figure class="code">{body}<figcaption>{caption}</figcaption></figure>'
    return body
