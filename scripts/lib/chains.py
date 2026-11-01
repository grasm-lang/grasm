"""Programs whose expressions are as deep as they are long.

    python3 scripts/lib/chains.py OUTDIR

The nesting cap ([G-NEST-LIMIT]) counts neither a `^` chain nor an `else
if` chain -- both parsers read them in a loop -- so a program can nest
either as deep as it is long, and every pass after the parser has to walk
them without recursing per term, and every pass has to survive what the cap
does count down to the cap. This writes, for `diagdiff`, a `^` chain and an
`else if` chain of 6,000, 20,000 and 50,000 terms each, the `else if` chain
once as a value and once as statements; and every shape `fuzz_front.py`'s
deep generator makes -- brackets, blocks, unary, binary, `^` and `??`
chains, pipelines, postfix chains, `else if` chains, types, patterns,
nested interpolations, closures, `if`s, maps, calls -- at every depth it
makes them, around the cap and past it; and prints their paths, one a line. (`grasm check` takes all of them; a `^` chain of 100,000 aborts
it, docs/known-issues.md.)
"""

import importlib.util
import os
import sys

LENGTHS = (6000, 20000, 50000)


def program(shape, n):
    head = "fn main() {\n    let a = true\n"
    if shape == "pow":
        return head + "    let x = 2" + " ^ 2" * n + "\n    print(\"{x}\")\n}\n"
    if shape == "elseif":
        return head + "    let x = if a { 1 }" + " else if a { 2 }" * n + " else { 3 }\n    print(\"{x}\")\n}\n"
    # `else if` as statements, in a function whose last expression it is not.
    return head + "    if a {\n        print(\"1\")\n    }" + " else if a {\n        print(\"2\")\n    }" * n + "\n}\n"


class Fixed:
    """A stand-in for fuzz_front's random source: the depth, the shape and
    the operator chosen, not drawn."""

    def __init__(self, depth, kind, sub):
        self.depth, self.kind, self.sub, self.calls = depth, kind, sub, 0

    def choice(self, seq):
        self.calls += 1
        return self.depth if self.calls == 1 else seq[self.sub % len(seq)]

    def randrange(self, n):
        return self.kind


def deep_programs():
    """Every shape `fuzz_front.py`'s deep generator makes, at every depth it
    makes it, each operator a shape chooses among: (name, source)."""
    spec = importlib.util.spec_from_file_location(
        "fuzz_front", os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "fuzz_front.py"))
    ff = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ff)
    out = []
    for kind in range(17):
        for sub in range(7 if kind in (3, 4, 8) else 1):
            for depth in ff.DEPTHS:
                out.append((f"deep{kind:02d}-{sub}-{depth}", ff.deep(Fixed(depth, kind, sub))))
    return out


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    paths = []
    for shape in ("pow", "elseif", "elseifstmt"):
        for n in LENGTHS:
            path = os.path.abspath(os.path.join(out, f"{shape}{n}.gr"))
            with open(path, "w") as f:
                f.write(program(shape, n))
            paths.append(path)
    for name, src in deep_programs():
        path = os.path.abspath(os.path.join(out, name + ".gr"))
        with open(path, "w") as f:
            f.write(src)
        paths.append(path)
    print("\n".join(paths))


if __name__ == "__main__":
    main()
