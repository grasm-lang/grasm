"""Rust 1.99's format!, one call per line: <spec as hex> <value> <count>...
where the value is i<decimal i64>, u<decimal u64>, f<16 hex digits of an
f64> or s<hex of a str>, and each count is a usize argument. A spec with
`.*` takes its precision from the first count; otherwise `N$` names count
N (the value being argument 0). All the calls go into one generated
program, compiled with rustc; answers are the output as hex."""
import os
import subprocess
import sys
import tempfile

lines = sys.stdin.read().split("\n")
if lines and lines[-1] == "":
    lines.pop()


def rust_str(t):
    return '"' + "".join(c if c.isascii() and c.isalnum() or c in " .,:;<>^+-#*?_=!@%&()[]~|/" else "\\u{%x}" % ord(c) for c in t) + '"'


def rust_fmt(t):
    # A format string: braces stay as written, everything else escaped.
    return '"' + "".join(c if c.isascii() and c.isalnum() or c in " .,:;<>^+-#*?_=!@%&()[]~|/{}$" else "\\u{%x}" % ord(c) for c in t) + '"'


def value(tok):
    k, v = tok[0], tok[1:]
    if k == "i":
        n = int(v)
        return "i64::MIN" if n == -2 ** 63 else f"({n}i64)"
    if k == "u":
        return f"{int(v)}u64"
    if k == "f":
        return f"f64::from_bits(0x{v}u64)"
    return rust_str(bytes.fromhex(v).decode("utf-8") if v != "-" else "")


calls = []
for line in lines:
    f = line.split(" ")
    spec = bytes.fromhex(f[0]).decode("utf-8") if f[0] != "-" else ""
    counts = [f"{int(c)}usize" for c in f[2:]]
    if ".*" in spec:
        calls.append(f"format!({rust_fmt('{:' + spec + '}')}, {counts[0]}, {value(f[1])})")
    else:
        calls.append(f"format!({rust_fmt('{0:' + spec + '}')}, {', '.join([value(f[1])] + counts)})")

src = ["use std::io::Write;", "fn hx(s: &str) -> String { if s.is_empty() { \"-\".to_string() } else { s.bytes().map(|b| format!(\"{:02x}\", b)).collect() } }"]
chunk = 250
for k in range(0, len(calls), chunk):
    src.append(f"fn part{k // chunk}(o: &mut Vec<String>) {{")
    for c in calls[k:k + chunk]:
        src.append(f"    o.push(hx(&{c}));")
    src.append("}")
src.append("fn main() { let mut o = Vec::new();")
for k in range(0, len(calls), chunk):
    src.append(f"    part{k // chunk}(&mut o);")
src.append("    let mut out = std::io::stdout().lock(); for l in o { writeln!(out, \"{}\", l).unwrap(); } }")
work = tempfile.mkdtemp(prefix="rustfmt")
open(os.path.join(work, "cases.rs"), "w").write("\n".join(src) + "\n")
built = subprocess.run(["rustc", "-O", "cases.rs", "-o", "cases"], cwd=work, capture_output=True, text=True)
if built.returncode != 0:
    sys.stderr.write(built.stderr[-3000:])
    sys.exit(1)
sys.stdout.write(subprocess.run([os.path.join(work, "cases")], capture_output=True, text=True).stdout)
