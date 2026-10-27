#!/usr/bin/env python3
"""The Grasm frontend against the Rust one, on generated and mutated source.

`compiler/front` is the compiler's frontend written in Grasm
(docs/self-hosting.md, S2). `tokentest` and `asttest` hold it to the Rust
frontend on every file in the repository, and those files almost all parse.
What a frontend gets wrong is mostly what nobody writes on purpose: a token
deleted mid-expression, two swapped, a file cut off, nesting at and past the
depth cap, a literal at the edge of its range, a character no lexer rule
mentions. This makes those, and feeds each program to both frontends:

    grasm tokens / front tokens   the token streams, stdout and stderr
    grasm ast    / front ast      the AST and every diagnostic, in order

and they must agree byte for byte, exit statuses included. The AST dump ends
with the parser's diagnostics, so the first diagnostic -- message, span and
help -- is compared with everything after it.

Five kinds of program:

1. mutated -- a window of lines from a file in the repository, with one to
   four token edits: a deletion, an insertion from the language's
   vocabulary, two neighbours swapped, a duplicate, a cut;
2. deep -- every construct that nests, at depths around the 2048-level cap
   and far past it: brackets, braces, blocks, unary and binary chains, `^`
   and `??` chains, pipelines, postfix chains, `else if` chains, types,
   patterns, interpolations inside interpolations, closures;
3. literals -- integers in each base at and over 64 bits, floats at the
   edges of `f64`, underscores, strings with every escape and bad ones,
   `\\u{...}` at and past the last code point and on surrogates, raw blocks;
4. unicode -- characters from every plane in strings, comments and where no
   token may start, byte order marks, zero-width and control characters;
5. soup -- tokens of the language in random order.

Counted, never assumed: a program the Grasm side refuses by name (`front:
declined: ...`, exit 3) is declined, not agreeing; a program on which the
Rust frontend panics is a Rust bug, counted apart, and it fails the run
unless it is one docs/known-issues.md records (by its panic's location).
The counts must add up to the programs made, and a run that compared too
few, or too few with diagnostics, fails: a fuzzer that skips is not a pass.

    scripts/fuzz_front.py [--programs N] [--seed S] [--keep DIR]
    scripts/fuzz_front.py --emit DIR [--programs N] [--seed S]

`--emit` writes the programs and runs nothing, for `scripts/parsediag.sh`.
`GRASM=` must have `grasm ast`; `FRONT=` names a built frontend.
"""

import argparse
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# Rust frontend panics this gate knows: the place the panic names, and the
# docs/known-issues.md entry that records it. Any other panic fails the run.
# Empty since the lexer took the whole character after a backslash
# (lexer.rs:706, an unknown escape before a multi-byte character, fixed).
KNOWN_RUST_PANICS = {}

TIMEOUT = 60

KEYWORDS = ("fn pub let var own inout type interface impl effect with handled_by match if else for "
            "while unsafe ref in return try true false break continue and or not import test").split()
PUNCT = ("( ) [ ] { } , . .. ..= : :: -> => | |> && || + - * / % ^ = == != < <= > >= ! ? ?. ?? "
         "+= -= *= /= _ @").split()
WORDS = ["x", "y", "f", "g", "xs", "Some", "None", "Ok", "Err", "self", "Self", "parallel", "spawn",
         "region", "bench", "actor", "extern", "export", "linear", "thread_local", "process_local", "requires",
         "ensures", "concurrent", "go", "i64", "str", "Point", "print", "main"]
LITERALS = ["0", "1", "42", "0x1F", "0b101", "1.5", "2e10", "1_000", '""', '"s"', '"a{x}b"',
            '"\\n"', '"""raw"""', "18446744073709551615", "9223372036854775808"]
VOCAB = KEYWORDS + PUNCT + WORDS + LITERALS + ["\n", "\n", "\n"]

TOKEN_RE = re.compile(r'''
    """(?:.|\n)*?"""           |
    "(?:\\.|[^"\\\n])*"?       |
    //[^\n]*                   |
    /\*(?:.|\n)*?\*/           |
    0x[0-9A-Za-z_]* | 0b[0-9A-Za-z_]* |
    [0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:[eE][+-]?[0-9]+)? |
    [A-Za-z_][A-Za-z0-9_]*     |
    \.\.= | \.\. | \?\. | \?\? | \|> | -> | => | == | != | <= | >= | \+= | -= | \*= | /= | :: | && | \|\| |
    \n                         |
    [ \t\r]+                   |
    .
''', re.VERBOSE)


def seed_files():
    out = subprocess.run(["git", "ls-files", "*.gr"], cwd=ROOT, capture_output=True, text=True).stdout.split()
    return [os.path.join(ROOT, p) for p in out]


def mutated(rng, files):
    path = rng.choice(files)
    try:
        lines = open(path, encoding="utf-8").read().split("\n")
    except (UnicodeDecodeError, OSError):
        lines = ["fn main() {", "}"]
    n = rng.randint(3, 60)
    start = rng.randrange(max(1, len(lines) - n + 1))
    text = "\n".join(lines[start:start + n]) + "\n"
    toks = TOKEN_RE.findall(text)
    real = [i for i, t in enumerate(toks) if not t.isspace() or t == "\n"]
    for _ in range(rng.randint(1, 4)):
        if not real:
            break
        i = rng.choice(real)
        op = rng.randrange(6)
        if op == 0:
            toks[i] = ""
        elif op == 1:
            toks[i] = toks[i] + " " + rng.choice(VOCAB) + " "
        elif op == 2:
            j = rng.choice(real)
            toks[i], toks[j] = toks[j], toks[i]
        elif op == 3:
            toks[i] = toks[i] + " " + toks[i]
        elif op == 4:
            toks = toks[:i]
            real = [r for r in real if r < i]
        else:
            toks[i] = rng.choice(VOCAB)
    return "".join(toks)


DEPTHS = [3, 50, 400, 680, 681, 682, 1000, 1022, 1023, 1024, 1025, 2040, 2046, 2047, 2048, 2049, 2050,
          3000, 6000]


def deep(rng):
    d = rng.choice(DEPTHS)
    kind = rng.randrange(17)
    body = None
    if kind == 0:
        body = "let x = " + "(" * d + "1" + ")" * d
    elif kind == 1:
        body = "let x = " + "[" * d + "1" + "]" * d
    elif kind == 2:
        body = "{" * d + "1" + "}" * d
    elif kind == 3:
        body = "let x = " + rng.choice(["-", "not ", "try "]) * d + "1"
    elif kind == 4:
        body = "let x = 1" + (" " + rng.choice(["+", "*", "-", "==", "and", "or", "<"]) + " 1") * d
    elif kind == 5:
        body = "let x = 2" + " ^ 2" * d
    elif kind == 6:
        body = "let x = a" + " ?? a" * d
    elif kind == 7:
        body = "let x = a" + " |> f" * d
    elif kind == 8:
        body = "let x = a" + rng.choice([".f()", "[0]", ".x", "(1)"]) * d
    elif kind == 9:
        body = "let x = if a { 1 }" + " else if b { 2 }" * d + " else { 3 }"
    elif kind == 10:
        return "fn f(x: " + "[" * d + "i64" + "]" * d + ") {\n}\n"
    elif kind == 11:
        body = "match x {\n    " + "A(" * d + "_" + ")" * d + " -> 1\n}"
    elif kind == 12:
        inner = "x"
        for _ in range(min(d, 400)):
            inner = '"{' + inner + '}"'
        body = "let s = " + inner
    elif kind == 13:
        body = "let f = " + "|x| " * d + "x"
    elif kind == 14:
        body = "let x = " + "if a { " * d + "1" + " }" * d
    elif kind == 15:
        body = "let x = " + "{ 1: " * d + "2" + " }" * d
    else:
        body = "let x = " + "f(" * d + "1" + ")" * d
    return "fn main() {\n    " + body + "\n}\n"


def rand_int_lit(rng):
    kind = rng.randrange(6)
    if kind == 0:
        return str(rng.choice([0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615,
                               18446744073709551616, 10 ** 25, rng.randrange(10 ** rng.randint(1, 22))]))
    if kind == 1:
        digits = "".join(rng.choice("0123456789abcdefABCDEFg_") for _ in range(rng.randint(0, 20)))
        return "0x" + digits
    if kind == 2:
        digits = "".join(rng.choice("01_2") for _ in range(rng.randint(0, 70)))
        return "0b" + digits
    if kind == 3:
        return "_".join(str(rng.randrange(1000)) for _ in range(rng.randint(1, 4)))
    if kind == 4:
        return "0" * rng.randint(1, 25) + str(rng.randrange(100))
    return rng.choice(["0X1", "0x", "0b", "0x_", "1__", "1_"])


def rand_float_lit(rng):
    kind = rng.randrange(5)
    if kind == 0:
        return repr(rng.uniform(0, 1e6)).replace("e+", "e")
    if kind == 1:
        return "%de%d" % (rng.randrange(1, 100), rng.randint(-400, 400))
    if kind == 2:
        return "%d.%se%s%d" % (rng.randrange(100), "0" * rng.randint(1, 30) + "1", rng.choice(["", "+", "-"]),
                               rng.randint(0, 330))
    if kind == 3:
        return rng.choice(["1e16", "1e-4", "0.0001", "0.00009999", "9999999999999999.0", "1e15", "5e-324",
                           "1.7976931348623157e308", "1.7976931348623159e308", "1e", "1e+", "1.e5", "1._5",
                           "2.5E-3", "1_000.000_1e1_0", "t.1.0", "x.0.1"])
    return "%d.%d" % (rng.randrange(10 ** 8), rng.randrange(10 ** 8))


ESCAPES = ["\\n", "\\t", "\\r", "\\\\", '\\"', "\\0", "\\{", "\\}", "\\q", "\\'", "\\x", "\\u{1F600}",
           "\\u{0}", "\\u{7f}", "\\u{80}", "\\u{110000}", "\\u{10FFFF}", "\\u{d800}", "\\u{dfff}", "\\u{}",
           "\\u{1234567}", "\\u12", "\\u{12", "\\u{301}", "\\u{200d}", "\\u{feff}", "\\u{a0}", "\\u{e0001}",
           "{x}", "{ }", "{}", "{a + b}", "{\"{x}\"}", "{x", "\\", "{x}}", "{{x}}",
           # A backslash before a character of more than one byte: the Rust
           # lexer panicked on it until it took the whole character.
           "\\é", "\\✓", "\\🙂"]


def rand_char(rng):
    plane = rng.randrange(8)
    if plane == 0:
        cp = rng.randrange(0x20, 0x7F)
    elif plane == 1:
        cp = rng.randrange(0x80, 0x800)
    elif plane == 2:
        cp = rng.choice([0x00, 0x01, 0x07, 0x0B, 0x0C, 0x1B, 0x7F, 0x85, 0xA0, 0xAD, 0x200B, 0x200D,
                         0x200F, 0x2028, 0x2060, 0x3000, 0xFEFF, 0xFFFD, 0x301, 0xE0001])
    elif plane == 3:
        cp = rng.randrange(0x800, 0xD800)
    elif plane == 4:
        cp = rng.randrange(0xE000, 0x10000)
    elif plane == 5:
        cp = rng.randrange(0x10000, 0x110000)
    elif plane == 6:
        cp = rng.randrange(0x4E00, 0x9FFF)
    else:
        cp = rng.randrange(0x1F300, 0x1FAFF)
    if 0xD800 <= cp <= 0xDFFF:
        cp = 0xFFFD
    return chr(cp)


def rand_string(rng):
    parts = []
    for _ in range(rng.randint(0, 6)):
        r = rng.randrange(3)
        if r == 0:
            parts.append(rng.choice(ESCAPES))
        elif r == 1:
            parts.append(rand_char(rng))
        else:
            parts.append(rng.choice(["abc", " ", "x", "é", "ok"]))
    s = "".join(parts)
    if rng.randrange(8) == 0:
        return '"""' + s.replace('"""', "") + rng.choice(['"""', '"', ""])
    return '"' + s + rng.choice(['"', '"', '"', ""])


def literals(rng):
    items = []
    for _ in range(rng.randint(1, 8)):
        r = rng.randrange(3)
        if r == 0:
            items.append(rand_int_lit(rng))
        elif r == 1:
            items.append(rand_float_lit(rng))
        else:
            items.append(rand_string(rng))
    sep = rng.choice([" + ", ", ", "\n    let y = ", " "])
    return "fn main() {\n    let x = " + sep.join(items) + "\n}\n"


def unicode(rng):
    out = []
    if rng.randrange(4) == 0:
        out.append("﻿")
    for _ in range(rng.randint(1, 8)):
        r = rng.randrange(5)
        if r == 0:
            out.append("// " + "".join(rand_char(rng) for _ in range(rng.randint(1, 6))) + "\n")
        elif r == 1:
            out.append("/* " + "".join(rand_char(rng) for _ in range(rng.randint(1, 6))) + " */ ")
        elif r == 2:
            out.append('let s = "' + "".join(rand_char(rng) for _ in range(rng.randint(1, 6))) + '"\n')
        elif r == 3:
            out.append("let " + rand_char(rng) + "x = 1\n")
        else:
            out.append("let x = a" + rand_char(rng) + "b\n")
    return "fn main() {\n" + "".join(out) + "}\n"


def soup(rng):
    return " ".join(rng.choice(VOCAB) for _ in range(rng.randint(1, 60))) + "\n"


def generate(rng, files):
    kind = rng.choices(["mutated", "deep", "literals", "unicode", "soup"], weights=[50, 15, 12, 12, 11])[0]
    if kind == "mutated":
        return kind, mutated(rng, files)
    if kind == "deep":
        return kind, deep(rng)
    if kind == "literals":
        return kind, literals(rng)
    if kind == "unicode":
        return kind, unicode(rng)
    return kind, soup(rng)


def run(cmd):
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=TIMEOUT)
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return "timeout", b"", b""


def rust_panic_site(stderr):
    m = re.search(rb"panicked at ([^:\s]+:\d+)", stderr)
    return m.group(1).decode() if m else None


def build_front(work):
    out = os.path.join(work, "front")
    r = subprocess.run([GRASM, "build", os.path.join(ROOT, "compiler/front/src/main.gr"), "--native", "-o", out],
                       capture_output=True, text=True)
    if r.returncode != 0:
        print("error: compiler/front does not build:")
        print(r.stdout + r.stderr)
        sys.exit(1)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=1500)
    ap.add_argument("--seed", type=int, default=20261026)
    ap.add_argument("--keep", help="write every disagreement here")
    ap.add_argument("--emit", help="write the programs here and compare nothing")
    args = ap.parse_args()
    rng = random.Random(args.seed)
    files = seed_files()
    if not files:
        print("no .gr files to mutate: nothing would be compared")
        sys.exit(1)
    if args.emit:
        os.makedirs(args.emit, exist_ok=True)
        for i in range(args.programs):
            _, text = generate(rng, files)
            with open(os.path.join(args.emit, "f%05d.gr" % i), "w", encoding="utf-8") as f:
                f.write(text)
        print(f"emitted: {args.programs}")
        return
    _lib("fresh").fresh_or_stop(GRASM)
    work = tempfile.mkdtemp(prefix="fuzz_front.")
    try:
        front = os.environ.get("FRONT") or build_front(work)
        # A planted fault for scripts/frontcontrols.sh: the engine skips
        # one program in ten without counting it.
        skip_fault = os.environ.get("FUZZ_FRONT_FAULT") == "skip"
        counts = {"agreed": 0, "declined": 0, "disagreed": 0, "rust_known": 0, "rust_unknown": 0}
        with_diags = 0
        compared_bytes = 0
        by_kind = {}
        shown = 0
        path = os.path.join(work, "p.gr")
        for i in range(args.programs):
            kind, text = generate(rng, files)
            if skip_fault and i % 10 == 3:
                continue
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            rt = run([GRASM, "tokens", path])
            gt = run([front, "tokens", path])
            ra = run([GRASM, "ast", path])
            ga = run([front, "ast", path])
            site = rust_panic_site(rt[2]) or rust_panic_site(ra[2])
            verdict = None
            if site is not None:
                known = any(site.endswith(k) for k in KNOWN_RUST_PANICS)
                verdict = "rust_known" if known else "rust_unknown"
            elif ga[0] == 3 and ga[2].startswith(b"front: declined"):
                verdict = "declined"
            elif rt == gt and ra[0] == ga[0] and ra[1] == ga[1] and ra[0] in (0, 1) and rt[0] in (0, 1):
                verdict = "agreed"
                compared_bytes += len(rt[1]) + len(ra[1])
                if ra[0] == 1:
                    with_diags += 1
            else:
                verdict = "disagreed"
            counts[verdict] += 1
            by_kind.setdefault(kind, {}).setdefault(verdict, 0)
            by_kind[kind][verdict] += 1
            if verdict in ("disagreed", "rust_unknown", "rust_known"):
                if args.keep:
                    os.makedirs(args.keep, exist_ok=True)
                    shutil.copy(path, os.path.join(args.keep, f"{verdict}-{i:05d}.gr"))
                if verdict != "rust_known" and shown < 8:
                    shown += 1
                    print(f"{verdict.upper()}: program {i} ({kind}), seed {args.seed}")
                    if site:
                        print(f"  rust panicked at {site}")
                    elif rt != gt:
                        print(f"  tokens: exit {rt[0]} vs {gt[0]}")
                    else:
                        print(f"  ast: exit {ra[0]} vs {ga[0]}")
                        print("  rust:  " + ra[1][:300].decode("utf-8", "replace").replace("\n", " | "))
                        print("  front: " + ga[1][:300].decode("utf-8", "replace").replace("\n", " | "))
                        print("  front stderr: " + ga[2][:200].decode("utf-8", "replace"))
        total = sum(counts.values())
        for kind in sorted(by_kind):
            parts = "  ".join(f"{v}: {n}" for v, n in sorted(by_kind[kind].items()))
            print(f"  {kind:9} {parts}")
        print(f"programs: {args.programs}   agreed: {counts['agreed']}   declined: {counts['declined']}   "
              f"disagreed: {counts['disagreed']}   rust panics: {counts['rust_known'] + counts['rust_unknown']} "
              f"(known: {counts['rust_known']})")
        print(f"agreed with diagnostics: {with_diags}   bytes compared: {compared_bytes}")
        fail = False
        if total != args.programs:
            print(f"the counts do not add up: {total} programs judged of {args.programs} made")
            fail = True
        if counts["disagreed"] or counts["rust_unknown"]:
            fail = True
        floor = args.programs // 2
        if counts["agreed"] < floor or with_diags < args.programs // 10 or compared_bytes == 0:
            print(f"too little compared: {counts['agreed']} agreed (at least {floor}), {with_diags} with "
                  f"diagnostics (at least {args.programs // 10})")
            fail = True
        sys.exit(1 if fail else 0)
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
