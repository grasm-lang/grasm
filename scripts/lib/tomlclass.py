"""Why the toolchain's TOML reader and `tomllib` disagree on a file
(scripts/tomldiff.sh).

    python3 scripts/lib/tomlclass.py LIST

LIST has one line per file the two disagree on, `<class>\\t<path>`, the class
being how they disagree:

  refuses-valid    the reader refuses a file `tomllib` reads
  accepts-invalid  the reader reads a file `tomllib` refuses
  reads-otherwise  both read it, to different values

The reader is the Rust toolchain's (`grasm_pkg::toml`), reproduced while it
is the oracle, so each disagreement has to be one docs/known-issues.md
records, or the dialect it documents. For each file this prints the class,
the first recorded cause found on one of its lines, and the path; a file with
none is `UNEXPLAINED` -- a disagreement nobody has recorded. Then one line
of counts, by class and by cause.
"""

import re
import sys

# Each cause: a name (docs/known-issues.md, or the documented dialect) and
# a pattern over one line of the file.
CAUSES = {
    "refuses-valid": [
        # The dialect: what the Rust reader documents it does not read.
        ("float", r"(?<![\w.\"'-])[+-]?(\d+\.\d+([eE][+-]?\d+)?|\d+[eE][+-]?\d+|inf|nan)(?![\w\"'])"),
        ("inline table", r"(=|,|\[)\s*\{"),
        ("array across lines", r"=\s*\[[^\]]*$"),
        ("literal string", r"=\s*'|\[\s*'|,\s*'"),
        ("multi-line string", r'"""|\'\'\''),
        ("integer form", r"=\s*[+-]?(0[xob]|\d+_\d)"),
        ("date or time", r"\d{4}-\d\d-\d\d|\d\d:\d\d:\d\d"),
        ("nested array", r"=\s*\[\s*\["),
        # docs/known-issues.md: an escaped quote ends the string for the
        # comment cut and the array split.
        ("escaped quote", r'\\"'),
    ],
    "accepts-invalid": [
        ("text after a string", r"=\s*\"([^\"\\]|\\.)*\"\s*[^\s#,\]]"),
        ("key with a space", r"^\s*[A-Za-z0-9_-]+\s+[A-Za-z0-9_-]+\s*="),
        ("leading zero", r"=\s*[+-]?0\d"),
        ("unknown escape", r"\\[^ntr\"\\buUf]"),
        ("empty table name part", r"^\s*\[\[?[^\]]*(\.\s*\]|\[\s*\.|\.\s*\.)"),
        ("escaped quote", r'\\"'),
    ],
    "reads-otherwise": [
        ("escape not undone", r"\\[uUbfr]"),
        ("escaped quote", r'\\"'),
        ("quoted table name", r"^\s*\[\[?[^\]]*[\"']"),
        ("spaced table name", r"^\s*\[\[?[^\]]*(\s\.|\.\s)"),
        ("quoted key", r"^\s*('[^']*'|\"[^\"]*\.[^\"]*\")\s*="),
        ("multi-line string", r'"""'),
    ],
}


def explain(cls, path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        lines = f.read().replace("\r\n", "\n").split("\n")
    for name, pat in CAUSES[cls]:
        rx = re.compile(pat)
        for line in lines:
            if rx.search(line):
                return name
    return "UNEXPLAINED"


def main():
    counts = {}
    for row in open(sys.argv[1], encoding="utf-8"):
        row = row.rstrip("\n")
        if not row:
            continue
        cls, path = row.split("\t", 1)
        why = explain(cls, path)
        print(f"{cls}\t{why}\t{path}")
        counts[(cls, why)] = counts.get((cls, why), 0) + 1
    shown = ", ".join(f"{c} / {w}: {n}" for (c, w), n in sorted(counts.items()))
    print(f"causes: {shown}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
