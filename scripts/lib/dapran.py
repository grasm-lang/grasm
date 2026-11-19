#!/usr/bin/env python3
"""What a debug-adapter session says the program printed and how it ended.

    dapran.py < answers

Reads `grasm dap`'s framed answers and prints the program's stdout, as its
`output` events carry it, then `exit N` from `exited` -- to set beside what
`grasm run` printed (scripts/clidiff.sh, the `dap` section).
"""
import json
import re
import sys

d = sys.stdin.buffer.read()
i = 0
out = []
code = "none"
while i < len(d):
    m = re.match(rb"Content-Length: (\d+)\r\n\r\n", d[i:])
    if not m:
        break
    n = int(m.group(1))
    s = i + m.end()
    j = json.loads(d[s:s + n])
    i = s + n
    if j.get("event") == "output" and j["body"].get("category") == "stdout":
        out.append(j["body"]["output"])
    if j.get("event") == "exited":
        code = str(j["body"]["exitCode"])
sys.stdout.write("".join(out))
sys.stdout.write("exit %s\n" % code)
