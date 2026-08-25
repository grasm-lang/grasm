"""A generator must produce the compiler it tests.

The shell gates have `lib/fresh.sh` because of F36: `wasmtest.sh` read a
prebuilt module it never built and reported 182 identical against a
week-old one for two full runs. The Python generators had the same shape --
they resolve `target/release/grasm` and, at most, complain when it is
absent. Running one straight after a build that failed measures the binary
from before the change and reports a clean run, which is exactly what
happened while mutation-testing a string group: the mutation did not
compile, the fuzzer ran the old compiler, and the mutation "passed".

Cargo is a no-op when nothing changed, so closing the class costs a tenth
of a second per run.

Skipped when the caller named its own binary through `GRASM=`: then the
artefact under test is theirs. `GRASM_NO_BUILD=1` skips it outright.
"""

import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DEFAULT = os.path.join(ROOT, "target", "release", "grasm")


def fresh_or_stop(grasm):
    """Builds the workspace unless the caller supplied its own binary."""
    if os.environ.get("GRASM_NO_BUILD") == "1":
        return
    if os.environ.get("GRASM"):
        return
    if grasm != DEFAULT:
        return
    r = subprocess.run(
        ["cargo", "build", "--release", "--workspace", "-q"], cwd=ROOT
    )
    if r.returncode != 0:
        print(
            "error: the workspace does not build — every result below would be\n"
            "       measured against a stale binary, so there are none."
        )
        sys.exit(1)
