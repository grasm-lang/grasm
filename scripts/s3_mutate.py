#!/usr/bin/env python3
"""Plants one fault in a copy of the backend's source, for `scripts/s3controls.sh`.

    s3_mutate.py <file> <old> <new>

Replaces the text `old` with `new` in `file`. The text must occur exactly
once: a mutation that matched nothing would build the unmutated backend, and
the control would then "fail to be caught" for a reason that has nothing to
do with the gates -- or, worse, a gate that passes would be read as one that
caught nothing. Written in Python, not `sed`, so that no character in a
pattern means anything but itself.
"""
import sys


def main():
    if len(sys.argv) != 4:
        print(__doc__)
        return 2
    path, old, new = sys.argv[1:]
    text = open(path, encoding="utf-8").read()
    n = text.count(old)
    if n != 1:
        print(f"{path}: the mutation's text occurs {n} times, not once: {old!r}")
        return 1
    open(path, "w", encoding="utf-8").write(text.replace(old, new))
    return 0


if __name__ == "__main__":
    sys.exit(main())
