#!/usr/bin/env python3
"""Write `editors/vscode/syntaxes/grasm.tmLanguage.json` from the compiler.

The grammar is a second list of every keyword, builtin and primitive type in
the language, and a second list is a list that drifts. This one had: four
keywords missing (`interface`, `impl`, `unsafe`, `ref`), two primitives
missing (`bytes`, `range`), 41 of 52 builtins missing, two *non*-builtins
present, and no handling of `\"\"\"` raw strings at all -- so a file with a raw
block highlighted as if the rest of it were a string.

So the grammar is not maintained. It is derived, from the three places the
compiler already keeps the facts:

* keywords      `crates/grasm_syntax/src/token.rs`   `keyword_from`
* builtins      `crates/grasm_sema/src/builtins.rs`  the table
* primitives    `crates/grasm_sema/src/check.rs`     the `prim` match

and `--check` fails the build when the committed file is not what this would
write. Same contract as `scripts/site.py`, for the same reason.

Adding a keyword to the language forces a decision here about how it reads,
because a keyword with no scope in SCOPES is an error rather than a default.
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "editors/vscode/syntaxes/grasm.tmLanguage.json"

# Every keyword, and how it reads. A keyword absent from here fails the build:
# the question "is this control flow or a modifier?" has an answer, and the
# person adding the keyword is the one who knows it.
SCOPES = {
    # Control flow.
    "if": "keyword.control",
    "else": "keyword.control",
    "match": "keyword.control",
    "for": "keyword.control",
    "while": "keyword.control",
    "in": "keyword.control",
    "return": "keyword.control",
    "break": "keyword.control",
    "continue": "keyword.control",
    "try": "keyword.control",
    "with": "keyword.control",
    "handled_by": "keyword.control",
    # Declarations.
    "fn": "keyword.declaration",
    "type": "keyword.declaration",
    "effect": "keyword.declaration",
    "interface": "keyword.declaration",
    "impl": "keyword.declaration",
    "import": "keyword.declaration",
    "test": "keyword.declaration",
    # Bindings and parameter modes: what a name may do, not what it is.
    "let": "storage.modifier",
    "var": "storage.modifier",
    "own": "storage.modifier",
    "inout": "storage.modifier",
    "ref": "storage.modifier",
    "pub": "storage.modifier",
    "unsafe": "storage.modifier",
    # Spelled-out operators.
    "and": "keyword.operator.word",
    "or": "keyword.operator.word",
    "not": "keyword.operator.word",
    # Literals, which read as constants rather than as keywords.
    "true": "constant.language",
    "false": "constant.language",
}

# Variants the library defines, which no table in the compiler enumerates as
# strings -- they are declarations in Grasm source, not Rust data. Highlighted
# because `Some`/`None` carry meaning the generic capitalised-identifier rule
# would flatten into "some type".
VARIANTS = ["Some", "None", "Ok", "Err"]


def read(rel):
    return (ROOT / rel).read_text()


def keywords():
    """The arms of `TokenKind::keyword_from`."""
    src = read("crates/grasm_syntax/src/token.rs")
    body = re.search(
        r"pub fn keyword_from\(s: &str\) -> Option<TokenKind> \{(.*?)\n    \}",
        src,
        re.S,
    )
    if not body:
        sys.exit("grammar.py: could not find `keyword_from` in token.rs")
    found = re.findall(r'"([a-z_]+)" => Kw', body.group(1))
    if len(found) < 20:
        sys.exit(f"grammar.py: only {len(found)} keywords parsed; the shape changed")
    return found


def builtins():
    """Every `name:` in the builtin table."""
    src = read("crates/grasm_sema/src/builtins.rs")
    found = re.findall(r'^\s+name: "([a-z_0-9]+)",', src, re.M)
    if len(found) < 40:
        sys.exit(f"grammar.py: only {len(found)} builtins parsed; the shape changed")
    return found


def primitives():
    """The `prim` match in the type checker."""
    src = read("crates/grasm_sema/src/check.rs")
    body = re.search(r"let prim = match name\.as_str\(\) \{(.*?)_ => None,", src, re.S)
    if not body:
        sys.exit("grammar.py: could not find the `prim` match in check.rs")
    found = re.findall(r'"([a-z_0-9]+)"', body.group(1))
    if "bytes" not in found or "i64" not in found:
        sys.exit("grammar.py: the `prim` match did not parse as expected")
    return found


def alt(names):
    """An alternation, longest first so `i64` cannot be eaten by `i6`."""
    return "|".join(sorted(set(names), key=lambda s: (-len(s), s)))


def grammar():
    kws = keywords()
    missing = [k for k in kws if k not in SCOPES]
    if missing:
        sys.exit(
            "grammar.py: no scope for keyword(s) "
            + ", ".join(repr(m) for m in missing)
            + "\n  Add each to SCOPES in this file: a new keyword has to be "
            "told how it reads."
        )
    stale = [k for k in SCOPES if k not in kws]
    if stale:
        sys.exit(
            "grammar.py: SCOPES names "
            + ", ".join(repr(s) for s in stale)
            + ", which the lexer no longer treats as a keyword."
        )

    by_scope = {}
    for kw in kws:
        by_scope.setdefault(SCOPES[kw], []).append(kw)

    keyword_patterns = [
        {"name": f"{scope}.grasm", "match": rf"\b({alt(words)})\b"}
        for scope, words in sorted(by_scope.items())
        if scope != "constant.language"
    ]
    constant_patterns = [
        {
            "name": "constant.language.grasm",
            "match": rf"\b({alt(by_scope.get('constant.language', []) + VARIANTS)})\b",
        }
    ]

    return {
        "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
        "name": "Grasm",
        "scopeName": "source.grasm",
        "_generated": "scripts/grammar.py — derived from the compiler; edit that, not this",
        "patterns": [
            {"include": "#comments"},
            {"include": "#strings"},
            {"include": "#keywords"},
            {"include": "#constants"},
            {"include": "#types"},
            {"include": "#functions"},
            {"include": "#numbers"},
        ],
        "repository": {
            "comments": {
                "patterns": [
                    {"name": "comment.line.double-slash.grasm", "match": "//.*$"},
                    {
                        "name": "comment.block.grasm",
                        "begin": r"/\*",
                        "end": r"\*/",
                        # Nested, as the lexer nests them: BlockComment(n).
                        "patterns": [{"include": "#comments"}],
                    },
                ]
            },
            "strings": {
                "patterns": [
                    # First, or the opening `"""` reads as an empty string
                    # followed by one that swallows the rest of the file.
                    # No escapes and no interpolation inside: verbatim bytes.
                    {
                        "name": "string.quoted.triple.grasm",
                        "begin": '"""',
                        "end": '"""',
                    },
                    {
                        "name": "string.quoted.double.grasm",
                        "begin": '"',
                        "end": '"',
                        "patterns": [
                            {
                                "name": "constant.character.escape.grasm",
                                "match": r"\\.",
                            },
                            {
                                "name": "meta.embedded.grasm",
                                "begin": r"(?<!\\)\{",
                                "end": r"\}",
                                "patterns": [{"include": "$self"}],
                            },
                        ],
                    },
                ]
            },
            "keywords": {"patterns": keyword_patterns},
            "constants": {"patterns": constant_patterns},
            "types": {
                "patterns": [
                    {
                        "name": "support.type.grasm",
                        "match": rf"\b({alt(primitives())})\b",
                    },
                    {
                        "name": "entity.name.type.grasm",
                        "match": r"\b[A-Z][A-Za-z0-9_]*\b",
                    },
                ]
            },
            "functions": {
                "patterns": [
                    {
                        "name": "entity.name.function.grasm",
                        "match": r"(?<=\bfn\s)[a-z_][A-Za-z0-9_]*",
                    },
                    {
                        "name": "support.function.grasm",
                        "match": rf"\b({alt(builtins())})\b(?=\s*\()",
                    },
                ]
            },
            "numbers": {
                "patterns": [
                    {
                        "name": "constant.numeric.grasm",
                        # Hex and binary only: the lexer has no octal.
                        "match": r"\b(0x[0-9a-fA-F_]+|0b[01_]+|[0-9][0-9_]*(\.[0-9_]+)?([eE][+-]?[0-9]+)?)\b",
                    }
                ]
            },
        },
    }


def main():
    text = json.dumps(grammar(), indent=2) + "\n"
    if "--check" in sys.argv:
        have = OUT.read_text() if OUT.exists() else ""
        if have != text:
            print(
                f"    {OUT.relative_to(ROOT)} is not what scripts/grammar.py writes.\n"
                "    The compiler's keywords, builtins or primitives moved.\n"
                "    Run: python3 scripts/grammar.py",
                file=sys.stderr,
            )
            return 1
        kws, bis, prims = keywords(), builtins(), primitives()
        print(
            f"    grammar: {len(kws)} keywords   {len(bis)} builtins   "
            f"{len(prims)} primitives   checked"
        )
        return 0
    OUT.write_text(text)
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
