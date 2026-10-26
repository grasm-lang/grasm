"""The Grasm backend as a third engine for the generators.

The generators (`scripts/fuzz_*.py`) build each program they invent with
`grasm run` and the Cranelift build and hold the two to each other. Stage S3
of docs/self-hosting.md asks the same of the backend written in Grasm
(`compiler/s3`): every generated program the Cranelift build accepts is also
built this way -- `grasm tir`, the driver, `cc` with the runtime, as
`scripts/lib/s3.sh` links the corpus -- and run beside the Cranelift binary
with the same arguments, environment and input. Its stdout, exit status and
stderr must be the Cranelift build's byte for byte (both are native builds,
so neither prints a panic trace), and when that build returns from `main`
the Grasm one must also have nothing live at exit.

There is one copy of this, here, and every generator calls it, because F47
is what three copies of a rule did to the trace filter.

What each program comes to, counted:

  * agreed    -- built, linked, and identical;
  * declined  -- the driver refused it by name (exit 3, "outside the subset",
                 each reason printed). Counted and its reasons shown, never
                 counted as agreeing;
  * failed    -- anything else: no TIR, the driver crashed or its allocation
                 checker objected, the object did not link, or the binary
                 answered differently or held allocations at exit.

A generator fails on any failure, and also when nothing was compared at all:
an engine that silently skipped every program would otherwise read as one
that agreed with all of them.

`S3=<binary>` uses that driver instead of building one, and `S3_EXTRA` adds
arguments to each compilation, as the S3 gates take them (the controls in
`scripts/s3controls.sh` point them at a broken one). `S3_RT=` names the
runtime to link.
"""

import os
import re
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def _run(cmd, timeout, env=None, stdin=None, cwd=None):
    """(exit, stdout, stderr) as bytes; 124 and b"timeout" when it overran."""
    try:
        p = subprocess.run(
            cmd,
            capture_output=True,
            timeout=timeout,
            env=env,
            input=stdin,
            stdin=None if stdin is not None else subprocess.DEVNULL,
            cwd=cwd,
        )
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, b"", b"timeout"


def _clean(err):
    return b"leak check: nothing live at exit" in err.splitlines()


def _short(b, n=300):
    if isinstance(b, bytes):
        b = b.decode("utf-8", "replace")
    b = b.strip()
    return b if len(b) <= n else b[: n // 2] + " ... " + b[-n // 2 :]


class Engine:
    """The Grasm backend, built once per generator run."""

    def __init__(self, grasm, work, root=ROOT):
        self.grasm = grasm
        self.work = work
        self.rt = os.environ.get("S3_RT") or os.path.join(
            root, "target", "release", "libgrasm_rt_abi.a")
        self.extra = (os.environ.get("S3_EXTRA") or "").split()
        self.offered = 0
        self.agreed = 0
        self.declined = 0
        self.failed = 0
        self.leak_checked = 0
        self.reasons = {}
        self.failures = []
        self.exe = os.path.join(work, "s3.bin")
        self.tir = os.path.join(work, "s3.tir")
        self.obj = os.path.join(work, "s3.o")
        self.error = None
        if not os.path.isfile(self.rt):
            self.error = f"the runtime {self.rt} is missing"
        self.driver = os.environ.get("S3")
        self.own_driver = not self.driver
        if self.driver:
            if not os.access(self.driver, os.X_OK):
                self.error = f"S3={self.driver} is not an executable"
        else:
            self.driver = os.path.join(work, "s3")
            p = subprocess.run(
                [grasm, "build", os.path.join(root, "compiler", "s3", "src", "main.gr"),
                 "--native", "-o", self.driver],
                capture_output=True, text=True, cwd=root)
            if p.returncode != 0 or not os.path.exists(self.driver):
                self.error = "compiler/s3 does not build:\n" + (p.stdout + p.stderr)[:1200]

    # ----- one program --------------------------------------------------------

    def _fail(self, label, why):
        self.failed += 1
        self.failures.append((label, why))
        return "failed"

    def _link(self):
        """Links the object into `self.exe`, keeping its inode (`lib/inplace.py`)."""
        stage = self.exe + ".stage"
        p = subprocess.run(["cc", self.obj, self.rt, "-o", stage, "-Wl,-dead_strip"],
                           capture_output=True, text=True)
        if p.returncode != 0 or not os.path.exists(stage):
            if os.path.exists(stage):
                os.remove(stage)
            return p.stdout + p.stderr
        with open(stage, "rb") as f:
            data = f.read()
        if os.path.exists(self.exe):
            with open(self.exe, "r+b") as f:
                f.truncate(0)
                f.write(data)
            os.remove(stage)
        else:
            os.replace(stage, self.exe)
        return None

    def check(self, src, ref, label, args=(), env=None, stdin=None, cwd=None,
              timeout=60, interp=None, leak=True):
        """Builds `src` with the Grasm backend and runs it beside `ref`.

        `ref` is the Cranelift binary the generator built from the same
        source; both run with `args`, `env` (the whole environment, as
        `subprocess` takes it), `stdin` (bytes or text) and `cwd`. `interp`,
        if given, is what `grasm run` printed, (exit, stdout), and is shown
        beside a disagreement so it says which engine moved. Answers
        "agreed", "declined" or "failed", and counts it.
        """
        self.offered += 1
        if self.error:
            return self._fail(label, self.error)
        if isinstance(stdin, str):
            stdin = stdin.encode()
        for path in (self.tir, self.obj):
            if os.path.exists(path):
                os.remove(path)
        t = _run([self.grasm, "tir", src], timeout=120, cwd=cwd)
        if t[0] != 0:
            return self._fail(label, f"`grasm tir` failed ({t[0]}): {_short(t[2])}")
        with open(self.tir, "wb") as f:
            f.write(t[1])
        c = _run([self.driver, self.tir, "-o", self.obj] + self.extra, timeout=120)
        if c[0] == 3:
            self.declined += 1
            for line in c[2].decode("utf-8", "replace").splitlines():
                # Counted by what was refused, not where: a span per program
                # would make every reason its own line.
                line = re.sub(r" @[0-9]+:[0-9]+", "", line.strip())
                if line:
                    self.reasons[line] = self.reasons.get(line, 0) + 1
            return "declined"
        if c[0] != 0 or not os.path.exists(self.obj):
            return self._fail(label, f"the Grasm backend answered {c[0]}: {_short(c[2] + c[1])}")
        why = self._link()
        if why is not None:
            return self._fail(label, f"the object does not link: {_short(why)}")
        cmd_tail = list(args)
        want = _run([ref] + cmd_tail, timeout, env, stdin, cwd)
        got = _run([self.exe] + cmd_tail, timeout, env, stdin, cwd)
        # Two runs stopped at the limit print the same nothing; that is not
        # agreement.
        if want == (124, b"", b"timeout") or got == (124, b"", b"timeout"):
            return self._fail(label, f"ran past {timeout}s: Cranelift exit {want[0]}, "
                                     f"Grasm backend exit {got[0]}")
        if want != got:
            parts = [f"Cranelift exit {want[0]}, Grasm backend exit {got[0]}"]
            if interp is not None:
                irc, iout = interp
                if isinstance(iout, str):
                    iout = iout.encode()
                side = []
                if (irc, iout) == (want[0], want[1]):
                    side.append("the interpreter agrees with Cranelift")
                if (irc, iout) == (got[0], got[1]):
                    side.append("the interpreter agrees with the Grasm backend")
                parts.append("; ".join(side) or f"the interpreter agrees with neither (exit {irc})")
            parts.append(f"  Cranelift stdout: {_short(want[1])!r}  stderr: {_short(want[2], 200)!r}")
            parts.append(f"  Grasm     stdout: {_short(got[1])!r}  stderr: {_short(got[2], 200)!r}")
            return self._fail(label, "\n".join(parts))
        if leak and want[0] == 0:
            lenv = dict(env if env is not None else os.environ)
            lenv["GRASM_LEAK_CHECK"] = "1"
            lk = _run([self.exe] + cmd_tail, timeout, lenv, stdin, cwd)
            if not (lk[0] == 0 and _clean(lk[2])):
                # Held to the Cranelift build's standard, not past it: when
                # that build holds something at exit too, this is not where
                # the difference is.
                rk = _run([ref] + cmd_tail, timeout, lenv, stdin, cwd)
                if rk[0] == 0 and _clean(rk[2]):
                    return self._fail(label, f"the Grasm build holds allocations at exit "
                                             f"(exit {lk[0]}): {_short(lk[2], 200)}")
            else:
                self.leak_checked += 1
        self.agreed += 1
        return "agreed"

    # ----- the verdict --------------------------------------------------------

    def summary(self):
        return (f"s3 engine: offered: {self.offered}   agreed: {self.agreed}   "
                f"declined: {self.declined}   failed: {self.failed}   "
                f"leak-checked: {self.leak_checked}")

    def report(self, show=5):
        """Prints the failures, the reasons for refusals, and the summary.
        Answers True when this engine passes."""
        for label, why in self.failures[:show]:
            print(f"S3 DISAGREES {label}: {why}")
        if len(self.failures) > show:
            print(f"  ... and {len(self.failures) - show} more")
        if self.reasons:
            print("the Grasm backend declined, by reason (counted, not agreement):")
            top = sorted(self.reasons.items(), key=lambda kv: (-kv[1], kv[0]))
            for line, n in top[:8]:
                print(f"  {n:5d}  {_short(line, 200)}")
        print(self.summary())
        if self.failed:
            return False
        if self.agreed == 0:
            print("the Grasm backend compared nothing: an engine that skipped every "
                  "program would read the same")
            return False
        return True

    def cleanup(self):
        """Removes what this engine wrote, for a generator that keeps its
        directory."""
        mine = [self.tir, self.obj, self.exe, self.exe + ".stage"]
        if self.own_driver:
            mine.append(self.driver)
        for path in mine:
            if os.path.exists(path):
                os.remove(path)
