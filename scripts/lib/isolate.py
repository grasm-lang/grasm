#!/usr/bin/env python3
"""Runs a program in the Linux machine with a /tmp of its own
(docs/self-hosting.md §6.18; `scripts/lib/linux.sh`, `LINUX_ISOLATE`).

    isolate.py <dir> <program> [args...]

Fifteen of the corpus's programs write under /tmp by name, and the machine's
/tmp is the owner's: run as they are, they would leave their files there,
and two runs of one program at once would share them. So the program runs in
a user and mount namespace of its own, in which `<dir>` is mounted over
/tmp. The user namespace maps the calling user to itself and nothing else,
so the program runs as that user with that user's permissions: the
capabilities the new namespace gives are used for the mount and given up
when the program is executed, as they are for any program a user other than
root runs.
"""
import ctypes
import os
import sys

MS_BIND = 4096


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    uid, gid = os.getuid(), os.getgid()
    os.unshare(os.CLONE_NEWUSER | os.CLONE_NEWNS)
    with open("/proc/self/setgroups", "w") as f:
        f.write("deny")
    with open("/proc/self/uid_map", "w") as f:
        f.write(f"{uid} {uid} 1")
    with open("/proc/self/gid_map", "w") as f:
        f.write(f"{gid} {gid} 1")
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.mount(sys.argv[1].encode(), b"/tmp", None, MS_BIND, None) != 0:
        print(f"isolate.py: cannot mount {sys.argv[1]} on /tmp: errno {ctypes.get_errno()}", file=sys.stderr)
        return 125
    os.execvp(sys.argv[2], sys.argv[2:])


if __name__ == "__main__":
    sys.exit(main())
