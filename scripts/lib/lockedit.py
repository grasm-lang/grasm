"""A lockfile changed one way, for `deps` (scripts/clidiff.sh, regcases.py).

    python3 scripts/lib/lockedit.py HOW PACKAGE_DIR

HOW is one of the changes `grasm_pkg::diff_lock` names, or one it must take
in its stride:

  checksum      the first package's checksum, its first digit changed
  version       the first package's version, its patch made 9
  caps-none     the first ceiling the lockfile records, removed
  caps-empty    the first ceiling made `[]`
  caps-order    a ceiling of two or more, reversed
  removed       the last `[[package]]` dropped
  added         a `[[package]]` for a package not in the graph
  duplicate     the first `[[package]]` again, with another checksum
  key-twice     the first package's checksum set twice, the second time to
                another value (the reader keeps the later one)
  unreadable    a line no TOML reader takes
  missing-name  the first package's `name` line dropped
  not-utf8      a byte that is not UTF-8
  reordered     the `[[package]]` tables in the opposite order
  source        not the lockfile: a comment appended to the package's entry
"""

import os
import re
import sys


def blocks(text):
    parts = text.split("\n[[package]]\n")
    return parts[0], ["[[package]]\n" + p for p in parts[1:]]


def join(head, bs):
    return head + "".join("\n" + b for b in bs)


def main():
    how, pkg = sys.argv[1], sys.argv[2]
    path = os.path.join(pkg, "grasm.lock")
    if how == "source":
        with open(os.path.join(pkg, "src", "main.gr"), "a", encoding="utf-8") as f:
            f.write("\n// edited\n")
        return 0
    if how == "not-utf8":
        with open(path, "ab") as f:
            f.write(b"# \xff\n")
        return 0
    text = open(path, encoding="utf-8").read()
    head, bs = blocks(text)
    if how == "checksum":
        bs[0] = re.sub(r'checksum = "(.)', lambda m: 'checksum = "' + ("0" if m.group(1) != "0" else "1"), bs[0], count=1)
    elif how == "version":
        bs[0] = re.sub(r'version = "(\d+)\.(\d+)\.\d+"', r'version = "\1.\2.9"', bs[0], count=1)
    elif how == "caps-none":
        for i, b in enumerate(bs):
            if "capabilities = " in b:
                bs[i] = re.sub(r"capabilities = .*\n", "", b, count=1)
                break
    elif how == "caps-empty":
        for i, b in enumerate(bs):
            if "capabilities = " in b:
                bs[i] = re.sub(r"capabilities = .*\n", "capabilities = []\n", b, count=1)
                break
    elif how == "caps-order":
        for i, b in enumerate(bs):
            m = re.search(r"capabilities = \[(.*)\]\n", b)
            if m and "," in m.group(1):
                items = [x.strip() for x in m.group(1).split(",")]
                bs[i] = b.replace(m.group(0), "capabilities = [" + ", ".join(reversed(items)) + "]\n")
                break
    elif how == "removed":
        bs = bs[:-1]
    elif how == "added":
        bs.append('[[package]]\nname = "ghost"\nversion = "1.0.0"\nchecksum = "' + "0" * 64 + '"\n')
    elif how == "duplicate":
        bs.insert(1, re.sub(r'checksum = ".*"', 'checksum = "' + "f" * 64 + '"', bs[0], count=1))
    elif how == "key-twice":
        bs[0] = re.sub(r'(checksum = ".*"\n)', lambda m: m.group(1) + 'checksum = "' + "e" * 64 + '"\n', bs[0], count=1)
    elif how == "unreadable":
        bs.append("not = [toml\n")
    elif how == "missing-name":
        bs[0] = re.sub(r"name = .*\n", "", bs[0], count=1)
    elif how == "reordered":
        bs = list(reversed(bs))
    else:
        print(f"unknown edit {how}", file=sys.stderr)
        return 2
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(join(head, bs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
