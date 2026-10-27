"""A TOML file as `front toml` prints one, read by Python's `tomllib`.

    python3 scripts/lib/tomldump.py FILE

Every key as a dotted path, in byte order, a tab, then its value's kind and
the value: `str "..."` (JSON-quoted as TIR quotes), `int N`, `float X`,
`bool true`, `array [ ... ]`. A table's members are listed under their own
paths and the table itself is not. The entries of an array of tables follow,
the arrays by name, each entry numbered (`bin[1].name`) and in the file's
order, the keys sorted within it. A file tomllib refuses prints `refused`.
"""

import sys
import tomllib


def quote(s):
    out = ['"']
    for c in s:
        o = ord(c)
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif o < 0x20 or o == 0x7F:
            out.append("\\u%04x" % o)
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def value(v):
    if isinstance(v, bool):
        return "bool true" if v else "bool false"
    if isinstance(v, int):
        return f"int {v}"
    if isinstance(v, float):
        return f"float {v!r}"
    if isinstance(v, str):
        return "str " + quote(v)
    if isinstance(v, list):
        return "array [" + ", ".join(value(x) for x in v) + "]"
    return "other " + quote(str(v))


def is_tables(v):
    return isinstance(v, list) and len(v) > 0 and all(isinstance(x, dict) for x in v)


def flatten(prefix, table, out, arrays):
    """Plain keys into `out`; each array of tables into `arrays` as
    (name, [entry lines, ...]). Within an entry everything is one list."""
    for k, v in table.items():
        path = f"{prefix}.{k}" if prefix else k
        if isinstance(v, dict):
            flatten(path, v, out, arrays)
        elif is_tables(v) and arrays is not None:
            entries = []
            for i, x in enumerate(v):
                lines = []
                flatten(f"{path}[{i}]", x, lines, None)
                entries.append(lines)
            arrays.append((path, entries))
        elif is_tables(v):
            for i, x in enumerate(v):
                flatten(f"{path}[{i}]", x, out, None)
        else:
            out.append(path + "\t" + value(v))


def by_key(line):
    return line.split("\t", 1)[0].encode()


def main():
    with open(sys.argv[1], "rb") as f:
        data = f.read()
    try:
        doc = tomllib.loads(data.decode("utf-8"))
    except Exception as e:  # noqa: BLE001 -- any refusal is a refusal
        print(f"refused {type(e).__name__}")
        return 0
    out = []
    arrays = []
    flatten("", doc, out, arrays)
    out.sort(key=by_key)
    for line in out:
        print(line)
    for _, entries in sorted(arrays, key=lambda a: a[0].encode()):
        for lines in entries:
            for line in sorted(lines, key=by_key):
                print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
