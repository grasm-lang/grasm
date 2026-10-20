"""Rust 1.99's std::time::Duration, one call per line; every call goes into
one generated program compiled with rustc -O:
  dbg <secs> <nanos> <spec hex>      format!("{:<spec>?}", Duration::new(secs, nanos))
  tf <f64 bits hex>                  Duration::try_from_secs_f64
  f64 <s> <n>                        as_secs_f64
  add|sub|sadd|ssub|absdiff <s> <n> <s> <n>
  mul|div|smul <s> <n> <u32>
  mulf|divf <s> <n> <f64 bits hex>   mul_f64, div_f64 (a panic answers P:<hex of its message>)
  ddf <s> <n> <s> <n>                div_duration_f64
A duration answers D:<secs>:<nanos>, None for None, E:<hex> for an error,
a float F:<16 hex digits>."""
import os
import subprocess
import sys
import tempfile

lines = sys.stdin.read().split("\n")
if lines and lines[-1] == "":
    lines.pop()


def rust_fmt(t):
    return '"' + "".join(c if c.isascii() and c.isalnum() or c in " .,:;<>^+-#*?_=!@%&()[]~|/{}$" else "\\u{%x}" % ord(c) for c in t) + '"'


def dur(s, n):
    return f"Duration::new({s}u64, {n}u32)"


calls = []
for line in lines:
    f = line.split(" ")
    op = f[0]
    if op == "dbg":
        spec = bytes.fromhex(f[3]).decode() if f[3] != "-" else ""
        calls.append(f"hx(&format!({rust_fmt('{:' + spec + '?}')}, {dur(f[1], f[2])}))")
    elif op == "tf":
        calls.append(f"tf(f64::from_bits(0x{f[1]}u64))")
    elif op == "f64":
        calls.append(f"fl({dur(f[1], f[2])}.as_secs_f64())")
    elif op in ("add", "sub"):
        calls.append(f"od({dur(f[1], f[2])}.checked_{op}({dur(f[3], f[4])}))")
    elif op in ("sadd", "ssub"):
        calls.append(f"d({dur(f[1], f[2])}.saturating_{op[1:]}({dur(f[3], f[4])}))")
    elif op == "absdiff":
        calls.append(f"d({dur(f[1], f[2])}.abs_diff({dur(f[3], f[4])}))")
    elif op in ("mul", "div"):
        calls.append(f"od({dur(f[1], f[2])}.checked_{op}({f[3]}u32))")
    elif op == "smul":
        calls.append(f"d({dur(f[1], f[2])}.saturating_mul({f[3]}u32))")
    elif op in ("mulf", "divf"):
        calls.append(f"guard(|| d({dur(f[1], f[2])}.{op[:3]}_f64(f64::from_bits(0x{f[3]}u64))))")
    else:
        calls.append(f"fl({dur(f[1], f[2])}.div_duration_f64({dur(f[3], f[4])}))")

src = ["use std::time::Duration;", "use std::io::Write;",
       "fn hx(s: &str) -> String { if s.is_empty() { \"-\".to_string() } else { s.bytes().map(|b| format!(\"{:02x}\", b)).collect() } }",
       "fn d(x: Duration) -> String { format!(\"D:{}:{}\", x.as_secs(), x.subsec_nanos()) }",
       "fn od(x: Option<Duration>) -> String { match x { Some(v) => d(v), None => \"None\".to_string() } }",
       "fn fl(x: f64) -> String { format!(\"F:{:016x}\", x.to_bits()) }",
       "fn tf(x: f64) -> String { match Duration::try_from_secs_f64(x) { Ok(v) => d(v), Err(e) => format!(\"E:{}\", hx(&e.to_string())) } }",
       "fn guard<F: FnOnce() -> String + std::panic::UnwindSafe>(f: F) -> String { match std::panic::catch_unwind(f) { Ok(s) => s, Err(p) => { let m = if let Some(s) = p.downcast_ref::<String>() { s.clone() } else if let Some(s) = p.downcast_ref::<&str>() { s.to_string() } else { String::new() }; format!(\"P:{}\", hx(&m)) } } }"]
chunk = 250
for k in range(0, len(calls), chunk):
    src.append(f"fn part{k // chunk}(o: &mut Vec<String>) {{")
    for c in calls[k:k + chunk]:
        src.append(f"    o.push({c});")
    src.append("}")
src.append("fn main() { std::panic::set_hook(Box::new(|_| {})); let mut o = Vec::new();")
for k in range(0, len(calls), chunk):
    src.append(f"    part{k // chunk}(&mut o);")
src.append("    let mut out = std::io::stdout().lock(); for l in o { writeln!(out, \"{}\", l).unwrap(); } }")
work = tempfile.mkdtemp(prefix="rustduration")
open(os.path.join(work, "cases.rs"), "w").write("\n".join(src) + "\n")
built = subprocess.run(["rustc", "-O", "cases.rs", "-o", "cases"], cwd=work, capture_output=True, text=True)
if built.returncode != 0:
    sys.stderr.write(built.stderr[-3000:])
    sys.exit(1)
sys.stdout.write(subprocess.run([os.path.join(work, "cases")], capture_output=True, text=True).stdout)
