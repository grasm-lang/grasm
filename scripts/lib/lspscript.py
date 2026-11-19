#!/usr/bin/env python3
"""Scripted language-server sessions, for `scripts/clidiff.sh`'s `lsp` section.

    lspscript.py file <path.gr> <out>
    lspscript.py fixed <name> <out>
    lspscript.py list-fixed

`file` writes, for one source file, the bytes a client would send `grasm
lsp` (docs/self-hosting.md §6.22): `initialize`; `didOpen` with the file's
text under `file://<path>`; `documentSymbol`; `hover` and `definition` at
every `fn`/`type`/`effect` name and at 40 identifiers spread over the file;
`completion`; a `didChange` adding a function with a type error, and two
hovers on it; a `didChange` with the file's last `}` dropped; `didSave`;
`shutdown`; `exit`. `fixed` writes one of the sessions the files do not
reach: a URI that is not a file, a file that does not exist, a request
before `didOpen`, positions past the end, non-ASCII text, a method nobody
handles, every kind of `id`, malformed frames and JSON.
"""
import json
import re
import sys


def frame(b):
    return b"Content-Length: %d\r\n\r\n" % len(b) + b


class Script:
    def __init__(self):
        self.out = []
        self.nid = 0

    def send(self, m):
        self.out.append(frame(json.dumps(m, separators=(",", ":"), ensure_ascii=False).encode()))

    def req(self, method, params, rid=None):
        if rid is None:
            self.nid += 1
            rid = self.nid
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})

    def note(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def bytes(self):
        return b"".join(self.out)


def tdp(uri, line, ch):
    return {"textDocument": {"uri": uri}, "position": {"line": line, "character": ch}}


def file_session(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    uri = "file://" + path
    s = Script()
    s.req("initialize", {"processId": None, "rootUri": None, "capabilities": {}})
    s.note("initialized", {})
    s.note("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "grasm", "version": 1, "text": text}})
    s.req("textDocument/documentSymbol", {"textDocument": {"uri": uri}})
    lines = text.split("\n")
    decl = [(i, m.start(1)) for i, l in enumerate(lines)
            for m in re.finditer(r"\b(?:fn|type|effect)\s+([A-Za-z_][A-Za-z0-9_]*)", l)]
    pos = [(i, m.start()) for i, l in enumerate(lines) for m in re.finditer(r"[A-Za-z_][A-Za-z0-9_]*", l)]
    step = max(1, len(pos) // 40)
    for (l, c) in decl + pos[::step]:
        s.req("textDocument/hover", tdp(uri, l, c))
        s.req("textDocument/definition", tdp(uri, l, c))
    s.req("textDocument/completion", tdp(uri, 0, 0))
    probe = text + "\nfn zz_lsp_probe() -> i64 {\n    \"x\"\n}\n"
    s.note("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": probe}]})
    for (l, c) in [(len(lines), 3), (len(lines) + 1, 4)]:
        s.req("textDocument/hover", tdp(uri, l, c))
    broken = text[::-1].replace("}", "", 1)[::-1]
    s.note("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": broken}]})
    s.note("textDocument/didSave", {"textDocument": {"uri": uri}})
    s.req("shutdown", None)
    s.note("exit", None)
    return s.bytes()


PROG = ("/// Twice `x`.\nfn dbl(x: i64) -> i64 {\n    x * 2\n}\n\n"
        "type Shape = | Circle(r: f64) | Dot\n\ntype P = { x: i64, y: [str] }\n\n"
        "effect Log {\n    fn log(m: str)\n}\n\n"
        "fn main() {\n    let é = \"ü\"\n    print(dbl(2))\n}\n")


def fixed(name):
    s = Script()
    s.req("initialize", {})
    if name == "untitled":
        # Not a file: the buffer alone, as one module, imports unresolved.
        for text in [PROG, "fn main() {\n  let x: i64 = \"s\"\n}\n", "import os\nfn main() {\n  os.x()\n}\n", "fn main( {\n"]:
            s.note("textDocument/didOpen", {"textDocument": {"uri": "untitled:Untitled-1", "text": text}})
            s.req("textDocument/hover", tdp("untitled:Untitled-1", 1, 4))
    elif name == "nofile":
        for uri in ["file://no/such/dir/x.gr", "file:///nonexistent.gr", "file://", "file:///", "x.gr"]:
            s.note("textDocument/didOpen", {"textDocument": {"uri": uri, "text": PROG}})
            s.req("textDocument/documentSymbol", {"textDocument": {"uri": uri}})
    elif name == "positions":
        uri = "untitled:p"
        s.req("textDocument/hover", tdp(uri, 0, 0))
        s.req("textDocument/completion", tdp(uri, 0, 0))
        s.req("textDocument/documentSymbol", {"textDocument": {"uri": uri}})
        s.note("textDocument/didOpen", {"textDocument": {"uri": uri, "text": PROG}})
        for (l, c) in [(0, 0), (1, 3), (1, 999), (2, 4), (5, 15), (5, 16), (5, 30), (7, 5), (9, 7),
                       (13, 9), (14, 8), (15, 10), (99, 0), (-1, 0), (1, -1), (4294967297, 3), (1.5, 2.7)]:
            for m in ["hover", "definition", "completion"]:
                s.req("textDocument/" + m, tdp(uri, l, c))
        s.req("textDocument/hover", {"textDocument": {"uri": uri}})
        s.req("textDocument/hover", {"textDocument": {"uri": uri}, "position": {"line": "1", "character": 1}})
        s.note("textDocument/didOpen", {"textDocument": {"uri": "untitled:e", "text": ""}})
        s.req("textDocument/hover", tdp("untitled:e", 0, 0))
        s.req("textDocument/documentSymbol", {"textDocument": {"uri": "untitled:e"}})
        s.note("textDocument/didChange", {"textDocument": {"uri": uri}, "contentChanges": []})
        s.note("textDocument/didChange", {"textDocument": {"uri": uri}, "contentChanges": [{"text": "a"}, {"text": PROG + "\r\n"}]})
        s.req("textDocument/hover", tdp(uri, 1, 3))
        s.note("textDocument/didSave", {"textDocument": {"uri": "untitled:never-opened"}})
    elif name == "ids":
        for rid in ["s", 0, -1, 1.5, 1e20, -0.0, None, True, [1, "a"], {"b": 1, "a": 2}, 2 ** 70]:
            s.req("shutdown", None, rid)
        s.req("textDocument/didOpen", {"textDocument": {"uri": "untitled:i", "text": PROG}}, 99)
        s.req("no/such/method", {}, 100)
        s.note("no/such/method", {})
        s.note("exit", None)
        s.req("textDocument/hover", tdp("untitled:i", 1, 4), 101)
    elif name == "exit-first":
        s.note("exit", None)
        s.req("textDocument/hover", tdp("untitled:x", 0, 0))
    elif name.startswith("frame-"):
        s.out.append(BAD[name[6:]])
        s.req("shutdown", None)
    else:
        raise SystemExit("unknown fixed session " + name)
    return s.bytes()


BAD = {
    "nolength": b"X-Other: 1\r\n\r\n",
    "badlength": b"Content-Length: 5\r\nContent-Length: x\r\n\r\n{\"id\":1}",
    "plus": b"Content-Length: +2\r\n\r\n{}",
    "short": b"Content-Length: 99\r\n\r\n{\"id\":1}",
    "utf8": frame(b'{"jsonrpc":"2.0","id":7,"method":"shutdown","x":"\xff"}'),
    "header-utf8": b"Content-Length: 2\xfe\r\n\r\n{}",
    "notjson": frame(b'{"id":1,'),
    "escapes": frame('{"jsonrpc":"2.0","id":"\\u0041\\ud83d\\ude00\\/\\b\\f\\n\\t\\"\\\\\\u001fé€","method":"shutdown"}'.encode()),
    "numbers": frame(b'{"jsonrpc":"2.0","id":-12.5e-3,"method":"shutdown"}'),
    "bignum": frame(b'{"jsonrpc":"2.0","id":1e400,"method":"shutdown"}'),
    "dupkey": frame(b'{"jsonrpc":"2.0","id":1,"id":2,"method":"x","method":"shutdown"}'),
    "ws": frame(b'\n\t {"jsonrpc" : "2.0" , "id" : 3 , "method" : "shutdown" }  junk'),
}


FIXED = ["untitled", "nofile", "positions", "ids", "exit-first"] + ["frame-" + k for k in BAD]


def main():
    a = sys.argv[1:]
    if a[0] == "list-fixed":
        print("\n".join(FIXED))
        return
    data = file_session(a[1]) if a[0] == "file" else fixed(a[1])
    with open(a[2], "wb") as f:
        f.write(data)


main()
