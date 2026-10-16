import json, subprocess, sys
SRV = sys.argv[1]
# Lines a well-behaved client never sends. Each must come back as one
# JSON-RPC error line, or produce no line at all for a notification --
# never a crash, and never a reply to a notification.
CASES = [
    ("not JSON at all",                 "{not json",                         "error"),
    ("empty object",                    "{}",                                "error"),
    ("a bare array",                    "[1,2,3]",                           "error"),
    ("a bare string",                   '"hello"',                           "error"),
    ("a number",                        "42",                                "error"),
    ("null",                            "null",                              "error"),
    ("wrong jsonrpc version",           '{"jsonrpc":"1.0","id":1,"method":"ping"}', "error"),
    ("no jsonrpc field",                '{"id":1,"method":"ping"}',          "error"),
    ("no method",                       '{"jsonrpc":"2.0","id":1}',          "error"),
    ("unknown method",                  '{"jsonrpc":"2.0","id":1,"method":"nope"}', "error"),
    ("notification: no id",             '{"jsonrpc":"2.0","method":"notifications/initialized"}', "silent"),
    ("notification: unknown",           '{"jsonrpc":"2.0","method":"notifications/whatever"}', "silent"),
    ("string id",                       '{"jsonrpc":"2.0","id":"abc","method":"ping"}', "result"),
    ("tools/call no name",              '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{}}', "error"),
    ("tools/call no arguments",         '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add"}}', "result"),
    ("tools/call args wrong type",      '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"add","arguments":{"a":"x","b":2}}}', "result"),
    ("tools/call arguments is a list",  '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"add","arguments":[1,2]}}', "result"),
    ("params is a string",              '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":"nope"}', "error"),
    ("resources/read missing uri",      '{"jsonrpc":"2.0","id":7,"method":"resources/read","params":{}}', "error"),
    ("deeply nested arguments",         '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"shout","arguments":{"text":"' + "a"*2000 + '"}}}', "result"),
    ("unicode in arguments",            json.dumps({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"shout","arguments":{"text":"日本 ünïcode"}}}), "result"),
    ("id is an object",                 '{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}', "result"),
]
inp = "\n".join(c[1] for c in CASES) + "\n"
p = subprocess.run([SRV], input=inp, capture_output=True, text=True, timeout=60)
if p.returncode != 0:
    print(f"CRASH: exit {p.returncode}\n{p.stderr[:400]}"); sys.exit(1)
lines = [l for l in p.stdout.split("\n") if l.strip()]
expected = [c for c in CASES if c[2] != "silent"]
print(f"sent {len(CASES)} lines, {len([c for c in CASES if c[2]=='silent'])} notifications, got {len(lines)} replies (want {len(expected)})")
bad = 0
if len(lines) != len(expected):
    print("  REPLY COUNT WRONG -- a notification was answered, or a request was not"); bad += 1
for (label, _src, want), line in zip(expected, lines):
    try: o = json.loads(line)
    except Exception as e:
        print(f"  {label}: reply is not JSON: {e}"); bad += 1; continue
    if o.get("jsonrpc") != "2.0":
        print(f"  {label}: reply missing jsonrpc 2.0"); bad += 1
    got = "error" if "error" in o else ("result" if "result" in o else "neither")
    if got != want:
        print(f"  {label}: wanted {want}, got {got} -- {line[:110]}"); bad += 1
    if got == "error" and not isinstance(o["error"].get("code"), int):
        print(f"  {label}: error code is not an integer"); bad += 1
print("RAW OK" if bad == 0 else f"RAW FAILURES: {bad}")
sys.exit(1 if bad else 0)
