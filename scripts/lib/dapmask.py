#!/usr/bin/env python3
"""Whether two debug-adapter sessions differ only in the variables pane.

    dapmask.py <rust answers> <grasm answers>

Exit 0 when the two framed answer streams are the same once every shown
variable's `value` and `type`, and every `evaluate` result's, are set aside
-- the Rust adapter shows a `u64` as an `i64` because it runs without the
checker's facts (docs/known-issues.md, S2), and the coordinator made such a
session a counted, printed exception (docs/self-hosting.md §6.22). Prints
the first pane entry that differs; exit 1, saying where, otherwise.
"""
import json
import re
import sys


def messages(path):
    d = open(path, "rb").read()
    i, out = 0, []
    while i < len(d):
        m = re.match(rb"Content-Length: (\d+)\r\n\r\n", d[i:])
        if not m:
            out.append(("unframed", d[i:i + 80]))
            break
        n = int(m.group(1))
        s = i + m.end()
        out.append(("ok", d[s:s + n]))
        i = s + n
    return out


def masked(body):
    j = json.loads(body)
    b = j.get("body")
    if isinstance(b, dict) and isinstance(b.get("variables"), list):
        for v in b["variables"]:
            v["value"] = v["type"] = "?"
    if j.get("command") == "evaluate" and isinstance(b, dict):
        b["result"] = b["type"] = "?"
    return j


a, b = messages(sys.argv[1]), messages(sys.argv[2])
if len(a) != len(b):
    print("%d and %d messages" % (len(a), len(b)))
    sys.exit(1)
first = None
for k, (x, y) in enumerate(zip(a, b)):
    if x == y:
        continue
    if x[0] != "ok" or y[0] != "ok" or masked(x[1]) != masked(y[1]):
        print("message %d differs outside the pane" % (k + 1))
        sys.exit(1)
    if first is None:
        xs = json.loads(x[1])["body"]
        ys = json.loads(y[1])["body"]
        for p, q in zip(xs.get("variables", [xs]), ys.get("variables", [ys])):
            if p != q:
                first = "message %d: Rust %s, Grasm %s" % (k + 1, json.dumps(p, ensure_ascii=False), json.dumps(q, ensure_ascii=False))
                break
print(first or "the same")
sys.exit(0)
