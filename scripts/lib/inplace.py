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

It does decide otherwise, some of the time. Measured 2026-11-02: two small
programs swapped into one file this way, each run once after each swap, were
killed by SIGKILL on 0 to 39 of 40 runs depending on the file -- the kernel
keeping the old program's code signature for the inode -- and never printed
anything wrong; the same swaps through a new inode each time were killed on 0
of 130. In the fuzzers that was one program in a few hundred, read for weeks
as "killed under load". So a run of a file written over in place (here, or by
`s3engine`, through `reused`) that ends by SIGKILL is run once more from a
fresh inode: the ~250ms tax for that one program, and the program's own
answer -- one that really is killed is killed again.
"""

import os
import signal
import subprocess
import sys


def _state():
    """The files written over in place, and `subprocess.run` as it was.

    Kept on `subprocess` itself, not here, because each gate loads this file
    as a fresh module (and `s3engine` loads it again): one process, one set,
    one wrapper.
    """
    st = getattr(subprocess, "_grasm_inplace", None)
    if st is None:
        st = {"reused": set(), "run": subprocess.run, "reruns": 0}
        subprocess._grasm_inplace = st
        subprocess.run = _rerun_if_killed
    return st


def reused(path):
    """Records that `path` was just written over in place."""
    _state()["reused"].add(os.path.realpath(path))


def _rerun_if_killed(*args, **kwargs):
    st = _state()
    p = st["run"](*args, **kwargs)
    if p.returncode != -signal.SIGKILL:
        return p
    cmd = args[0] if args else kwargs.get("args")
    exe = cmd[0] if isinstance(cmd, (list, tuple)) and cmd else None
    if not isinstance(exe, str):
        return p
    path = os.path.realpath(os.path.join(kwargs.get("cwd") or ".", exe))
    if path not in st["reused"]:
        return p
    st["reused"].discard(path)
    stage = path + ".fresh"
    with open(path, "rb") as f:
        data = f.read()
    with open(stage, "wb") as f:
        f.write(data)
    os.chmod(stage, os.stat(path).st_mode & 0o777)
    os.replace(stage, path)
    st["reruns"] += 1
    print(f"inplace: {exe} was killed by SIGKILL; run again from a fresh inode", file=sys.stderr)
    return st["run"](*args, **kwargs)


_state()


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
        reused(out)
    else:
        os.replace(stage, out)
    return p
