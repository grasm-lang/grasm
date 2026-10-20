"""Runs a program and says how it ended: `exit N`, `signal N` or `timeout`.

A shell cannot tell those apart. `$?` is 139 both for a segfault and for a
program that called `exit(139)`, and `spec/conformance/L-LIB-EXIT.gr` ends
with `exit(-1)` -- 255 -- on purpose. Python's subprocess reports a death by
signal as a negative return code, so the difference survives here.

Usage: exitkind.py SECONDS PROGRAM [ARGS...]
The program runs with stdin closed and stdout discarded; its stderr is passed
through to this script's stderr, and the one line on stdout is the verdict.
"""

import subprocess
import sys

limit = float(sys.argv[1])
try:
    p = subprocess.run(sys.argv[2:], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                       stderr=subprocess.PIPE, timeout=limit if limit > 0 else None)
except subprocess.TimeoutExpired as e:
    sys.stderr.write((e.stderr or b"").decode(errors="replace"))
    print("timeout")
    sys.exit(0)
sys.stderr.write(p.stderr.decode(errors="replace"))
print(f"signal {-p.returncode}" if p.returncode < 0 else f"exit {p.returncode}")
