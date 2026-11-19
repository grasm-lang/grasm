#!/usr/bin/env python3
"""Scripted debug-adapter sessions, for `scripts/clidiff.sh`'s `dap` section.

    dapscript.py step|next|bp <program> <out>
    dapscript.py fixed <name> <program> <out> [journal]

Writes the bytes a client would send `grasm dap` on stdin -- framed
requests, `seq` counting from 1 -- for one session (docs/self-hosting.md
§6.22). The session's input then ends: the adapter stops stopping, and the
program runs on to its end with its output still reported.

  step   stop on entry, then 200 times: threads, stackTrace, scopes,
         variables, stepIn
  next   stop on entry, then 100 times: stackTrace, variables, and a cycle
         of next, stepIn, next, stepOut, stepIn, stepIn, next
  bp     a breakpoint on every third line of the program's file, then 50
         times: stackTrace, variables, continue

and the fixed sessions, each one thing the corpus sessions do not reach:
evaluate, scopes of an outer frame, variables of reference 0, breakpoints
replaced while stopped, disconnect while stopped, configurationDone before
launch, a journal given to launch, and framing and JSON errors in each
phase.
"""
import json
import sys


class Script:
    def __init__(self):
        self.seq = 0
        self.out = []

    def raw(self, body):
        self.out.append(b"Content-Length: %d\r\n\r\n" % len(body) + body)

    def req(self, cmd, args=None):
        self.seq += 1
        m = {"seq": self.seq, "type": "request", "command": cmd}
        if args is not None:
            m["arguments"] = args
        self.raw(json.dumps(m, separators=(",", ":"), ensure_ascii=False).encode())

    def bytes(self):
        return b"".join(self.out)


def lines_of(prog):
    with open(prog, "rb") as f:
        return f.read().count(b"\n") + 1


def corpus(kind, prog):
    s = Script()
    s.req("initialize", {"adapterID": "grasm"})
    if kind == "step":
        s.req("launch", {"program": prog, "stopOnEntry": True})
        s.req("configurationDone")
        for _ in range(200):
            s.req("threads")
            s.req("stackTrace", {"threadId": 1})
            s.req("scopes", {"frameId": 0})
            s.req("variables", {"variablesReference": 1})
            s.req("stepIn", {"threadId": 1})
    elif kind == "next":
        s.req("launch", {"program": prog, "stopOnEntry": True})
        s.req("configurationDone")
        cycle = ["next", "stepIn", "next", "stepOut", "stepIn", "stepIn", "next"]
        for i in range(100):
            s.req("stackTrace", {"threadId": 1})
            s.req("variables", {"variablesReference": 1})
            s.req(cycle[i % len(cycle)], {"threadId": 1})
    elif kind == "bp":
        bps = [{"line": n} for n in range(1, lines_of(prog) + 1, 3)]
        s.req("setBreakpoints", {"source": {"path": prog}, "breakpoints": bps})
        s.req("launch", {"program": prog})
        s.req("configurationDone")
        for _ in range(50):
            s.req("stackTrace", {"threadId": 1})
            s.req("variables", {"variablesReference": 1})
            s.req("continue", {"threadId": 1})
    else:
        raise SystemExit("unknown session kind " + kind)
    return s.bytes()


def framed(body):
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


BAD = {
    "nolength": b"X-Other: 1\r\n\r\n",
    "badlength": b"Content-Length: 7x\r\n\r\n{}",
    "plus": b"Content-Length: +2\r\n\r\n{}",
    "short": b"Content-Length: 99\r\n\r\n{\"seq\":1}",
    "utf8": framed(b'{"seq":1,"command":"x\xff"}'),
    "utf8-short": framed(b'{"seq":1,"command":"x\xe2\x82"}'),
    "header-utf8": b"Content-Length: 2\xfe\r\n\r\n{}",
    "json-eof": framed(b'{"seq":1,"command":"x"'),
    "json-num": framed(b'{"seq":1e}'),
    "json-at": framed(b"@"),
    "json-escape": framed(b'{"command":"\\q"}'),
    "json-u": framed(b'{"command":"\\u12g4"}'),
    "json-u-short": framed(b'{"command":"\\u12'),
    "json-key": framed(b"{1:2}"),
    "json-arr": framed(b"[1 2]"),
    "json-lit": framed(b"nul"),
    "json-ok": framed(b'  {"seq":5,"command":"x\\u0041\\ud83d\\ude00\\/\\b","n":-0.5e1,"a":[true,false,null,{}],"command":"y"} trailing'),
}


def fixed(name, prog, journal):
    s = Script()
    if name == "evaluate":
        # Each name the first stops bind, one that is not, and spaces.
        s.req("initialize")
        s.req("launch", {"program": prog, "stopOnEntry": True})
        s.req("configurationDone")
        for _ in range(12):
            for expr in ["x", " total ", "name", "nosuch", "", "xs"]:
                s.req("evaluate", {"expression": expr, "frameId": 0})
            s.req("scopes", {"frameId": 1})
            s.req("variables", {"variablesReference": 0})
            s.req("variables", {})
            s.req("bogus")
            s.req("stepIn")
    elif name == "rebreak":
        # Breakpoints set before launch, replaced while stopped, then a
        # file's list emptied; an absolute path that never stops.
        s.req("initialize")
        s.req("setBreakpoints", {"source": {"path": prog}, "breakpoints": [{"line": 3}, {"line": 4}, {"line": 0}, {"line": -2}]})
        s.req("setBreakpoints", {"source": {"path": "/no/such/" + prog.split("/")[-1]}, "breakpoints": [{"line": 5}]})
        s.req("setBreakpoints", {"source": {"path": "nosuch.gr"}, "breakpoints": [{"line": 2}]})
        s.req("launch", {"program": prog})
        s.req("configurationDone")
        for i in range(6):
            s.req("stackTrace")
            s.req("setBreakpoints", {"source": {"path": prog}, "breakpoints": [{"line": 5 + i}, {"line": 9 + i}]})
            s.req("continue")
        s.req("setBreakpoints", {"source": {"path": prog}})
        s.req("continue")
    elif name == "disconnect":
        s.req("initialize")
        s.req("launch", {"program": prog, "stopOnEntry": True})
        s.req("configurationDone")
        s.req("stepIn")
        s.req("stackTrace")
        s.req("disconnect")
        s.req("stackTrace")
    elif name == "terminate-early":
        s.req("initialize")
        s.req("threads")
        s.req("whatever")
        s.req("terminate")
        s.req("launch", {"program": prog})
    elif name == "early-done":
        s.req("initialize")
        s.req("configurationDone")
    elif name == "nolaunch-eof":
        s.req("initialize")
        s.req("threads")
    elif name == "missing":
        s.req("initialize")
        s.req("launch", {"program": prog + ".missing.gr"})
        s.req("configurationDone")
    elif name == "launch":
        # A program that must not compile, or any other, launched plainly.
        s.req("initialize")
        s.req("launch", {"program": prog})
        s.req("configurationDone")
    elif name == "journal":
        s.req("initialize")
        s.req("launch", {"program": prog, "stopOnEntry": True, "journal": journal})
        s.req("configurationDone")
        for _ in range(60):
            s.req("variables", {"variablesReference": 1})
            s.req("next")
    elif name.startswith("frame-"):
        # A framing or JSON error, before launch (`frame-1-...`: the adapter
        # stops with `dap error`) or while stopped (`frame-2-...`: it stops
        # stopping and the program runs on).
        phase, key = name[6], name[8:]
        bad = BAD[key]
        s.req("initialize")
        if phase == "1":
            s.out.append(bad)
            s.req("threads")
        else:
            s.req("launch", {"program": prog, "stopOnEntry": True})
            s.req("configurationDone")
            s.req("stackTrace")
            s.out.append(bad)
            s.req("stackTrace")
    else:
        raise SystemExit("unknown fixed session " + name)
    return s.bytes()


FIXED = ["evaluate", "rebreak", "disconnect", "terminate-early", "early-done", "nolaunch-eof", "missing"] + \
    ["frame-%s-%s" % (p, k) for k in BAD for p in "12"]


def main():
    a = sys.argv[1:]
    if a and a[0] == "list-fixed":
        print("\n".join(FIXED))
        return
    if a and a[0] == "fixed":
        data = fixed(a[1], a[2], a[4] if len(a) > 4 else "")
        out = a[3]
    else:
        data = corpus(a[0], a[1])
        out = a[2]
    with open(out, "wb") as f:
        f.write(data)


main()
