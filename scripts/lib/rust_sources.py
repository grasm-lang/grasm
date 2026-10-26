"""The Grasm programs written as string literals in the Rust test suites.

    python3 scripts/lib/rust_sources.py OUTDIR file.rs...

`check_tests.rs`, `parser_tests.rs` and the rest hold their programs inline,
as Rust string literals. This reads each file's literals the way rustc does
-- escapes, `\\` line continuations, raw strings with any number of `#` --
keeps the ones that are Grasm programs (a newline and a declaration
keyword), and writes each distinct one to OUTDIR as `NNNNN.gr`. It prints
how many it wrote.
"""

import os
import re
import sys

DECL = re.compile(r"(^|\n)\s*(pub\s+)?(fn|type|let|import|test|effect|interface|impl|extern|actor|bench)\b")


def decode(body):
    out = []
    i = 0
    n = len(body)
    while i < n:
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        nxt = body[i + 1] if i + 1 < n else ""
        if nxt == "\n":
            # A line continuation: the newline and the next line's leading
            # whitespace go.
            i += 2
            while i < n and body[i] in " \t\n\r":
                i += 1
            continue
        simple = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "0": "\0", "'": "'", '"': '"'}
        if nxt in simple:
            out.append(simple[nxt])
            i += 2
        elif nxt == "x":
            out.append(chr(int(body[i + 2:i + 4], 16)))
            i += 4
        elif nxt == "u":
            close = body.index("}", i)
            out.append(chr(int(body[i + 3:close], 16)))
            i = close + 1
        else:
            out.append(nxt)
            i += 2
    return "".join(out)


def literals(text):
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                    if depth == 0:
                        break
                else:
                    i += 1
            continue
        m = re.match(r'b?r(#*)"', text[i:])
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = m.group(1)
            start = i + m.end()
            end = text.find('"' + hashes, start)
            if end < 0:
                return
            yield text[start:end]
            i = end + 1 + len(hashes)
            continue
        if c == "'":
            m = re.match(r"'(\\.|\\u\{[0-9a-fA-F]+\}|\\x[0-9a-fA-F]{2}|[^'\\])'", text[i:])
            if m:
                i += m.end()
                continue
            i += 1
            continue
        if c == '"' or (c == "b" and text.startswith('b"', i)):
            j = i + (2 if c == "b" else 1)
            buf_start = j
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            yield decode(text[buf_start:j])
            i = j + 1
            continue
        i += 1


def main():
    outdir = sys.argv[1]
    os.makedirs(outdir, exist_ok=True)
    seen = set()
    count = 0
    for path in sys.argv[2:]:
        text = open(path, encoding="utf-8").read()
        for lit in literals(text):
            if "\n" not in lit or not DECL.search(lit) or lit in seen:
                continue
            seen.add(lit)
            with open(os.path.join(outdir, "%05d.gr" % count), "w", encoding="utf-8") as f:
                f.write(lit)
            count += 1
    print(count)


if __name__ == "__main__":
    main()
