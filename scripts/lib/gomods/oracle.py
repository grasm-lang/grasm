"""Build scripts/lib/gomods, the Go oracle over modules in the module cache.

Offline: GOFLAGS=-mod=mod, GOPROXY=off, GOSUMDB=off, so only what the module
cache already holds is used. golang.org/x/net needs Go 1.26 or later; when
the `go` on PATH is older, the newest go1.26+ toolchain in the module cache
is run directly (switching with GOTOOLCHAIN would need the checksum
database).
"""
import glob
import os
import re
import subprocess


def _version(go):
    out = subprocess.run([go, "version"], capture_output=True, text=True).stdout
    m = re.search(r"go(\d+)\.(\d+)", out)
    return (int(m.group(1)), int(m.group(2))) if m else (0, 0)


def go_binary():
    if _version("go") >= (1, 26):
        return "go"
    cache = subprocess.run(["go", "env", "GOMODCACHE"], capture_output=True, text=True).stdout.strip()
    found = []
    for path in glob.glob(os.path.join(cache, "golang.org", "toolchain@v0.0.1-go1.*", "bin", "go")):
        m = re.search(r"go(\d+)\.(\d+)(?:\.(\d+))?", path.split("toolchain@")[1])
        if m and (int(m.group(1)), int(m.group(2))) >= (1, 26):
            found.append(((int(m.group(1)), int(m.group(2)), int(m.group(3) or 0)), path))
    if not found:
        raise SystemExit("no Go 1.26 or later on PATH or in the module cache")
    return max(found)[1]


def build(out):
    here = os.path.dirname(os.path.abspath(__file__))
    env = dict(os.environ, GOFLAGS="-mod=mod", GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local")
    built = subprocess.run([go_binary(), "build", "-o", out, "."], cwd=here, capture_output=True, text=True, env=env)
    if built.returncode != 0:
        raise SystemExit(built.stderr[-1500:])
    return out
