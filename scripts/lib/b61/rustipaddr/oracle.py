"""Rust 1.99's std::net addresses, one call per line; every call goes into
one generated program compiled with rustc -O:
  p <kind> <text hex>          <kind>::from_str for ip, v4, v6, sock, sock4, sock6;
                               answers format!("{}") of the result or E:<hex of the error>
  f4 <u32> <spec hex>          format!("{:<spec>}", Ipv4Addr::from_bits)
  f6 <u128 hex> <spec hex>     the same for Ipv6Addr
  fs4 <u32> <port> <spec hex>  SocketAddrV4
  fs6 <u128 hex> <port> <scope> <spec hex>   SocketAddrV6 (flowinfo 0)
  c4 <u32>                     the stable Ipv4Addr predicates as 0/1 digits, then
                               to_ipv6_compatible and to_ipv6_mapped
  c6 <u128 hex>                the stable Ipv6Addr predicates, then to_ipv4_mapped,
                               to_ipv4 ("-" for None) and to_canonical
Text answers are hex."""
import os
import subprocess
import sys
import tempfile

lines = sys.stdin.read().split("\n")
if lines and lines[-1] == "":
    lines.pop()


def rust_str(t):
    return '"' + "".join(c if c.isascii() and c.isalnum() or c in " .,:;<>^+-#*?_=!@%&()[]~|/$" else "\\u{%x}" % ord(c) for c in t) + '"'


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


KINDS = {"ip": "IpAddr", "v4": "Ipv4Addr", "v6": "Ipv6Addr", "sock": "SocketAddr", "sock4": "SocketAddrV4", "sock6": "SocketAddrV6"}
calls = []
for line in lines:
    f = line.split(" ")
    op = f[0]
    if op == "p":
        calls.append(f"parse::<{KINDS[f[1]]}>({rust_str(unhex(f[2]))})")
    elif op == "f4":
        calls.append(f"hx(&format!({rust_str('{:' + unhex(f[2]) + '}')}, Ipv4Addr::from_bits({f[1]}u32)))")
    elif op == "f6":
        calls.append(f"hx(&format!({rust_str('{:' + unhex(f[2]) + '}')}, Ipv6Addr::from_bits(0x{f[1]}u128)))")
    elif op == "fs4":
        calls.append(f"hx(&format!({rust_str('{:' + unhex(f[3]) + '}')}, SocketAddrV4::new(Ipv4Addr::from_bits({f[1]}u32), {f[2]}u16)))")
    elif op == "fs6":
        calls.append(f"hx(&format!({rust_str('{:' + unhex(f[4]) + '}')}, SocketAddrV6::new(Ipv6Addr::from_bits(0x{f[1]}u128), {f[2]}u16, 0, {f[3]}u32)))")
    elif op == "c4":
        calls.append(f"c4(Ipv4Addr::from_bits({f[1]}u32))")
    else:
        calls.append(f"c6(Ipv6Addr::from_bits(0x{f[1]}u128))")

src = ["use std::net::*;", "use std::io::Write;", "use std::str::FromStr;",
       "fn hx(s: &str) -> String { if s.is_empty() { \"-\".to_string() } else { s.bytes().map(|b| format!(\"{:02x}\", b)).collect() } }",
       "fn parse<T: FromStr + std::fmt::Display>(s: &str) -> String where T::Err: std::fmt::Display { match T::from_str(s) { Ok(v) => hx(&v.to_string()), Err(e) => format!(\"E:{}\", hx(&e.to_string())) } }",
       "fn b(x: bool) -> &'static str { if x { \"1\" } else { \"0\" } }",
       "fn o4(x: Option<Ipv4Addr>) -> String { match x { Some(v) => v.to_string(), None => \"-\".to_string() } }",
       "fn c4(a: Ipv4Addr) -> String { hx(&format!(\"{}{}{}{}{}{}{} {} {}\", b(a.is_unspecified()), b(a.is_loopback()), b(a.is_private()), b(a.is_link_local()), b(a.is_multicast()), b(a.is_broadcast()), b(a.is_documentation()), a.to_ipv6_compatible(), a.to_ipv6_mapped())) }",
       "fn c6(a: Ipv6Addr) -> String { let i = IpAddr::V6(a); hx(&format!(\"{}{}{}{}{}{}{}{} {} {} {} {:?}\", b(a.is_unspecified()), b(a.is_loopback()), b(a.is_unique_local()), b(a.is_unicast_link_local()), b(a.is_multicast()), b(i.is_unspecified()), b(i.is_loopback()), b(i.is_multicast()), o4(a.to_ipv4_mapped()), o4(a.to_ipv4()), a.to_canonical(), a.segments())) }"]
chunk = 250
for k in range(0, len(calls), chunk):
    src.append(f"fn part{k // chunk}(o: &mut Vec<String>) {{")
    for c in calls[k:k + chunk]:
        src.append(f"    o.push({c});")
    src.append("}")
src.append("fn main() { let mut o = Vec::new();")
for k in range(0, len(calls), chunk):
    src.append(f"    part{k // chunk}(&mut o);")
src.append("    let mut out = std::io::stdout().lock(); for l in o { writeln!(out, \"{}\", l).unwrap(); } }")
work = tempfile.mkdtemp(prefix="rustipaddr")
open(os.path.join(work, "cases.rs"), "w").write("\n".join(src) + "\n")
built = subprocess.run(["rustc", "-O", "cases.rs", "-o", "cases"], cwd=work, capture_output=True, text=True)
if built.returncode != 0:
    sys.stderr.write(built.stderr[-3000:])
    sys.exit(1)
sys.stdout.write(subprocess.run([os.path.join(work, "cases")], capture_output=True, text=True).stdout)
