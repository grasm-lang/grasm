import ipaddress
import random
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/v6test/src/main.gr")

random.seed(197)
addresses = ["::", "::1", "2001:db8::1", "2001:0db8:0000:0000:0000:0000:0000:0001",
             "2001:DB8::1", "fe80::1%eth0", "::ffff:192.0.2.1",
             "2001:0:0:1:0:0:0:1", "2001:db8:0:1:1:1:1:1", "ff02::1",
             "fd00::1", "1:2:3:4:5:6:7:8", "::2:3:4:5:6:7:8",
             "1:2:3:4:5:6:7::", "64:ff9b::192.0.2.33"]
bad = ["2001:db8::1::2", "2001:db8:::1", "12345::", "", ":::",
       "1:2:3:4:5:6:7:8:9", "1:2:3:4:5:6:7", "::ffff:999.0.2.1", "gg::"]
for _ in range(400):
    addresses.append(str(ipaddress.IPv6Address(random.getrandbits(128))))
for _ in range(100):
    prefix = random.randint(0, 128)
    addresses.append(str(ipaddress.IPv6Address(random.getrandbits(128))))
cidrs = []
for _ in range(200):
    prefix = random.randint(0, 128)
    cidrs.append(f"{ipaddress.IPv6Address(random.getrandbits(128))}/{prefix}")
argv = []
for a in addresses + bad:
    argv += ["render", a if a else "-"]
for a in addresses:
    argv += ["private", a]
for c in cidrs:
    argv += ["cidr", c]
got = subprocess.run([GRASM, "run", HARNESS, "--"] + argv,
                     capture_output=True, text=True)
if got.returncode != 0:
    print(got.stderr[-2000:]); sys.exit(1)
lines = got.stdout.split("\n")
agree = differ = shown = 0
at = 0
def check(label, want, line):
    global agree, differ, shown
    if str(want) == line:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 12:
            print(f"  {label}\n    python {want}\n    grasm  {line}")
for a in addresses + bad:
    try:
        want = str(ipaddress.IPv6Address(a.split("%")[0]))
    except Exception:
        want = "error"
    check(f"render {a!r}", want, lines[at]); at += 1
for a in addresses:
    address = ipaddress.IPv6Address(a.split("%")[0])
    want = str(address.is_private and not address.is_global or
               address.is_loopback or address.is_link_local or
               address.is_unspecified or
               address in ipaddress.ip_network("fc00::/7")).lower()
    want = str(address.is_loopback or address.is_link_local or
               address.is_unspecified or
               address in ipaddress.ip_network("fc00::/7")).lower()
    check(f"private {a}", want, lines[at]); at += 1
for c in cidrs:
    network = ipaddress.ip_network(c, strict=False)
    want = f"{network} {network[-1]}"
    check(f"cidr {c}", want, lines[at]); at += 1
print(f"cases: {agree + differ}   agreeing: {agree}   differing: {differ}")

sys.exit(1 if differ else 0)
