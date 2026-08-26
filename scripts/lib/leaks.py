"""One definition of "did this binary hold anything at exit".

`GRASM_LEAK_CHECK=1` makes the runtime report allocations still live when
`main` returns and exit 7. `scripts/leakcheck.sh` asks that of the hand-written
corpus; the generators invent far more programs than the corpus holds and, for
most of their history, asked it of none of them.

That is the gap F57 came out of: a `with` block discarded its tail value
without releasing it, so every such block kept one allocation for the life of
the process -- while printing exactly the right answer. Comparing engines
cannot see that, because both engines print the right answer. Only this can.

The cost is one extra run of a binary that has already been built.

Two copies of this rule existed before it was one, in `fuzz_places.py` and
`fuzz_captures.py`, and F47 is what three copies of a rule did to the trace
filter. So it is a function, in a file, that everything imports.
"""

import os
import subprocess


def leaked(exe, timeout=180):
    """The runtime's complaint if the binary held allocations, else None."""
    p = subprocess.run(
        [exe],
        capture_output=True,
        text=True,
        timeout=timeout,
        env={**os.environ, "GRASM_LEAK_CHECK": "1"},
    )
    if p.returncode == 0:
        return None
    lines = p.stderr.strip().splitlines()
    return lines[0] if lines else f"exit {p.returncode} with nothing on stderr"
