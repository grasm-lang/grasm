"""Markdown to the site's HTML, for the pages rendered from `docs/` and `spec/`.

The tutorial and the library reference were summarised on the site, and a
summary is a second copy: `library.html` listed 81 methods while
`spec/library.md` grew past them. Rendered from the file, the page *is* the
document.

Deliberately small. It knows the constructs those documents use -- headings,
paragraphs, fenced and indented code, tables, lists, block quotes, and inline
code, emphasis and links -- and **refuses** anything else rather than guessing,
because a renderer that silently mangles a construct produces a page that reads
fine and says something different from the source. A new construct in a
rendered document fails `scripts/site.py`, which is where it gets noticed.
"""

import html
import re

from site_highlight import code

# Documents this site renders, and the page each becomes. A link between two
# of them becomes a link between the pages; a link to any other document in the
# repository has no page to go to, so it is kept as its text.
PAGE_OF = {
    "tutorial.md": "tutorial.html",
    "library.md": "library.html",
}


class Unsupported(Exception):
    pass


def slug(text):
    s = re.sub(r"<[^>]+>", "", text)
    s = re.sub(r"[^a-z0-9]+", "-", html.unescape(s).lower()).strip("-")
    return s or "section"


def inline(text):
    """Inline markup, with everything that is not markup escaped.

    Code spans are taken out first -- nothing inside one is markup -- and put
    back after, as placeholders, so emphasis and links *around* code still
    pair up: `**`x` is it**` is one bold run, not two stray `**`."""
    spans = []

    def take(m):
        body = m.group(2)
        if body.strip():
            body = body.strip()
        spans.append(f"<code>{html.escape(body, quote=False)}</code>")
        return f"\x00{len(spans) - 1}\x00"

    held = re.sub(r"(`+)(.+?)\1", take, text)
    return re.sub(r"\x00(\d+)\x00", lambda m: spans[int(m.group(1))], _prose(held))


def _prose(text):
    t = html.escape(text, quote=False)

    def link(m):
        label, target = m.group(1), m.group(2)
        base = target.split("#")[0].split("/")[-1]
        if re.match(r"https?://", target):
            return f'<a href="{html.escape(target)}">{label}</a>'
        if base in PAGE_OF:
            frag = target.split("#", 1)[1] if "#" in target else ""
            href = PAGE_OF[base] + (f"#{frag}" if frag else "")
            return f'<a href="{href}">{label}</a>'
        return label

    t = re.sub(r"\[([^\]]+)\]\(([^)\s]+)\)", link, t)
    t = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", t)
    t = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"<em>\1</em>", t)
    t = re.sub(r"(?<![\w_])_(?!\s)(.+?)(?<!\s)_(?![\w_])", r"<em>\1</em>", t)
    # A clause tag becomes a place a link can point at.
    t = re.sub(
        r"<strong>\[([A-Z][A-Z0-9-]+)\]</strong>",
        r'<strong id="\1">[\1]</strong>',
        t,
    )
    return t


def _table(lines):
    def cells(line):
        body = line.strip()
        if not (body.startswith("|") and body.endswith("|")):
            raise Unsupported(f"table row without outer pipes: {line!r}")
        # A pipe inside a code span is content, not a column break.
        out, cur, in_code = [], "", False
        for ch in body[1:-1]:
            if ch == "`":
                in_code = not in_code
            if ch == "|" and not in_code:
                out.append(cur.strip())
                cur = ""
            else:
                cur += ch
        out.append(cur.strip())
        return out

    head = cells(lines[0])
    if not re.fullmatch(r"\|(\s*:?-+:?\s*\|)+", lines[1].strip()):
        raise Unsupported(f"table without a separator row: {lines[1]!r}")
    rows = [cells(l) for l in lines[2:]]
    th = "".join(f"<th>{inline(c)}</th>" for c in head)
    trs = "".join(
        "<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>" for r in rows
    )
    return f'<div class="scroll"><table><thead><tr>{th}</tr></thead><tbody>{trs}</tbody></table></div>'


def render(src, drop_title=True):
    """HTML for a whole document. Its `#` title is dropped by default, because
    the page supplies its own heading."""
    lines = src.split("\n")
    out = []
    para = []
    seen = set()

    def flush():
        if para:
            out.append("<p>" + inline(" ".join(l.strip() for l in para)) + "</p>")
            para.clear()

    i = 0
    while i < len(lines):
        line = lines[i]
        s = line.strip()

        if s.startswith("```"):
            flush()
            lang = s[3:].strip()
            j = i + 1
            while j < len(lines) and lines[j].strip() != "```":
                j += 1
            if j == len(lines):
                raise Unsupported(f"unclosed code fence at line {i + 1}")
            body = "\n".join(lines[i + 1 : j])
            if lang in ("", "grasm", "gr"):
                out.append(code(body))
            else:
                out.append(f"<pre><code>{html.escape(body, quote=False)}</code></pre>")
            i = j + 1
            continue

        if not s:
            flush()
            i += 1
            continue

        if re.fullmatch(r"-{3,}|\*{3,}", s) and not para:
            out.append("<hr>")
            i += 1
            continue

        m = re.match(r"(#{1,6})\s+(.*)", line)
        if m:
            flush()
            level = len(m.group(1))
            if level == 1 and drop_title:
                i += 1
                continue
            text = inline(m.group(2))
            anchor = slug(text)
            while anchor in seen:
                anchor += "-"
            seen.add(anchor)
            # The document's `##` is the page's `h2`: the page's own `h1`
            # stands in for the dropped title.
            out.append(f'<h{max(level, 2)} id="{anchor}">{text}</h{max(level, 2)}>')
            i += 1
            continue

        if s.startswith("|"):
            flush()
            j = i
            while j < len(lines) and lines[j].strip().startswith("|"):
                j += 1
            out.append(_table(lines[i:j]))
            i = j
            continue

        if line.startswith("    ") and not para:
            j = i
            block = []
            while j < len(lines) and (lines[j].startswith("    ") or not lines[j].strip()):
                block.append(lines[j][4:])
                j += 1
            while block and not block[-1].strip():
                block.pop()
            out.append(code("\n".join(block)))
            i = j
            continue

        if s.startswith("> "):
            flush()
            j = i
            quote = []
            while j < len(lines) and lines[j].strip().startswith(">"):
                quote.append(lines[j].strip()[1:].strip())
                j += 1
            out.append(f'<div class="note"><p>{inline(" ".join(quote))}</p></div>')
            i = j
            continue

        lm = re.match(r"(\s*)([-*]|\d+\.)\s+(.*)", line)
        if lm and not para:
            ordered = lm.group(2)[0].isdigit()
            items = []
            j = i
            while j < len(lines):
                mm = re.match(r"(\s*)([-*]|\d+\.)\s+(.*)", lines[j])
                if mm and not mm.group(1):
                    items.append([mm.group(3)])
                    j += 1
                elif items and lines[j].startswith("  ") and lines[j].strip():
                    if re.match(r"\s+([-*]|\d+\.)\s", lines[j]):
                        raise Unsupported(f"nested list at line {j + 1}")
                    items[-1].append(lines[j].strip())
                    j += 1
                else:
                    break
            tag = "ol" if ordered else "ul"
            lis = "".join(f"<li>{inline(' '.join(it))}</li>" for it in items)
            out.append(f"<{tag}>{lis}</{tag}>")
            i = j
            continue

        if s.startswith("<") and re.match(r"<[a-zA-Z/!]", s) and not para:
            raise Unsupported(f"raw HTML at line {i + 1}: {s[:40]!r}")

        para.append(line)
        i += 1

    flush()
    return "\n".join(out)
