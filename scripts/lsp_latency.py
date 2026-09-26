#!/usr/bin/env python3
"""G3's second half: how long from an edit to its diagnostic, in the editor.

MASTERPLAN §2.3 sets incremental edit-to-diagnostic at <= 50 ms. It was
recorded as "not built -- there is no query engine", which is a statement about
an implementation, not a measurement of the goal. The goal is a latency, and
`grasm lsp` has one: it re-checks the whole program, imports included, on every
change. This takes that latency the way an editor would see it.

For each file: start `grasm lsp`, open the file, then alternately insert a type
error and take it out again, each as a full-document `didChange`, and time each
from the moment the change is written to the moment its `publishDiagnostics`
arrives. An edit that introduces an error has to report it and one that
removes it has to report none -- otherwise the timing is of the wrong thing
and this fails.

    scripts/lsp_latency.py [grasm] [file.gr ...]

With no files it measures the largest program in the repository by line count
and the largest one that imports -- examples, ProjectGold, packages and apps --
which are the worst cases for a checker that re-reads the whole program.
"""

import glob
import json
import os
import statistics
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EDITS = 40


def send(p, msg):
    body = json.dumps(msg).encode()
    p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    p.stdin.flush()


def recv(p):
    length = None
    while True:
        line = p.stdout.readline()
        if not line:
            raise SystemExit("error: the language server exited")
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    return json.loads(p.stdout.read(length))


def wait_diagnostics(p, uri):
    while True:
        m = recv(p)
        if m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri:
            return m["params"]["diagnostics"]


def measure(grasm, path):
    path = os.path.abspath(path)
    uri = "file://" + path
    text = open(path).read()
    # A statement the checker must reject, appended at the end so no span in
    # the rest of the file moves.
    broken = text.rstrip("\n") + '\n\nfn __lsp_probe() -> i64 {\n    "not a number"\n}\n'

    p = subprocess.Popen([grasm, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL)
    send(p, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"processId": None, "rootUri": None, "capabilities": {}}})
    while recv(p).get("id") != 1:
        pass
    send(p, {"jsonrpc": "2.0", "method": "initialized", "params": {}})
    send(p, {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
        "textDocument": {"uri": uri, "languageId": "grasm", "version": 1, "text": text}}})
    base = wait_diagnostics(p, uri)
    if base:
        raise SystemExit(f"error: {path} does not check clean, so an edit cannot be timed against it")

    times = []
    for i in range(EDITS):
        want_error = i % 2 == 0
        send(p, {"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": {"uri": uri, "version": i + 2},
            "contentChanges": [{"text": broken if want_error else text}]}})
        t = time.perf_counter()
        got = wait_diagnostics(p, uri)
        times.append((time.perf_counter() - t) * 1000)
        if bool(got) != want_error:
            raise SystemExit(f"error: edit {i} reported {len(got)} diagnostics on {path}")
    send(p, {"jsonrpc": "2.0", "id": 2, "method": "shutdown"})
    send(p, {"jsonrpc": "2.0", "method": "exit"})
    p.wait(timeout=10)
    times.sort()
    return times


def default_files():
    """The largest program in the repository, and the largest that imports --
    the worst cases for a checker that re-reads the whole program."""
    corpus = []
    for pat in ("examples/*.gr", "ProjectGold/*/*.gr", "packages/*/src/main.gr",
                "apps/*/src/main.gr"):
        corpus += glob.glob(os.path.join(ROOT, pat))
    lines = lambda f: sum(1 for _ in open(f))
    imports = lambda f: any(l.startswith("import ") for l in open(f))
    single = max(corpus, key=lines)
    multi = max((f for f in corpus if imports(f)), key=lines, default=None)
    return [f for f in dict.fromkeys((single, multi)) if f]


def main():
    grasm = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "release", "grasm")
    files = sys.argv[2:] or default_files()
    worst = 0.0
    worst_p50 = 0.0
    for f in files:
        t = measure(grasm, f)
        n = sum(1 for _ in open(f))
        p50 = statistics.median(t)
        p95 = t[int(len(t) * 0.95) - 1]
        worst = max(worst, p95)
        worst_p50 = max(worst_p50, p50)
        print(f"{os.path.relpath(f, ROOT)}: {n} lines, {EDITS} edits  "
              f"p50 {p50:.1f} ms  p95 {p95:.1f} ms  max {t[-1]:.1f} ms")
    print(f"worst file: p50 {worst_p50:.1f} ms  p95 {worst:.1f} ms")


if __name__ == "__main__":
    main()
