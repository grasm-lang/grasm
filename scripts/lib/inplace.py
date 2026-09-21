"""Build a test binary without paying macOS's first-execution tax.

Every gate that builds native programs spends most of its time not compiling.
On this machine a freshly created executable costs ~450ms the first time it is
run and ~3ms every time after, and a byte-identical copy at a new path pays the
~450ms again -- so the cost belongs to the *file*, not to the program. It is
also flat: a 12MB binary scans no slower than a 0.5MB one. For a fuzzer that
builds one binary per generated program and runs it once, that fixed cost was
72% of the whole run.

Writing the new binary into the existing file, without unlinking it, keeps the
inode and the verdict that was already reached about it, and the same program
runs in ~3ms. The bytes executed are genuinely the new ones -- what is reused
is the policy decision, not the pages -- which is exactly the part that is
worth nothing here: these binaries were produced seconds earlier by our own
compiler from source this harness generated, and never left the machine.

This is a test-harness technique and belongs nowhere near what `grasm` emits
for a user. It leans on macOS not re-deciding about a file it has already
admitted; if a future release decides otherwise, the gates get slower and stay
correct, which is the right way round for that risk to fall.
"""

import os
import subprocess


def build(grasm, src, out, *extra, timeout=120, env=None):
    """Build `src` to `out`, reusing `out`'s inode. Returns the build process.

    On a failed build `out` is removed, so a caller that tests for the file
    cannot mistake the previous program's binary for this one's -- which is the
    one way a reused path could quietly lie.
    """
    stage = out + ".stage"
    p = subprocess.run(
        [grasm, "build", src, *extra, "-o", stage],
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
    )
    if not os.path.exists(stage):
        if os.path.exists(out):
            os.remove(out)
        return p
    with open(stage, "rb") as f:
        data = f.read()
    if os.path.exists(out):
        with open(out, "r+b") as f:
            f.truncate(0)
            f.write(data)
        os.remove(stage)
    else:
        os.replace(stage, out)
    return p
