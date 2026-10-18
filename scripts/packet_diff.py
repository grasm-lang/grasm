"""Packet headers against scapy.

scapy builds every packet from random field values, doing the layout,
lengths and checksums itself. Each Grasm package then has to decode
scapy's bytes into scapy's own reading of them, and re-encode what it
decoded into scapy's bytes exactly -- so a checksum computed over the
wrong range, or a length counted in the wrong unit, shows up on the second
half even when the first passes. Damaged checksums are included, and the
decoder has to say so.
"""
import logging
import os
import random
import socket
import subprocess
import sys
import tempfile

logging.getLogger("scapy").setLevel(logging.ERROR)
from scapy.all import ARP, ICMP, Dot1AD, Dot1Q, Ether, GRE, Raw, CookedLinux, CookedLinuxV2  # noqa: E402
from scapy.contrib.geneve import GENEVE, GeneveOptions  # noqa: E402
from scapy.layers.inet6 import (IPv6, ICMPv6EchoRequest, ICMPv6EchoReply, ICMPv6ND_NS,  # noqa: E402
                                ICMPv6ND_NA, ICMPv6ND_RS, ICMPv6ND_RA, ICMPv6ND_Redirect,
                                ICMPv6NDOptSrcLLAddr, ICMPv6NDOptDstLLAddr, ICMPv6NDOptMTU,
                                ICMPv6NDOptPrefixInfo, ICMPv6DestUnreach, ICMPv6PacketTooBig,
                                ICMPv6TimeExceeded)
from scapy.layers.vxlan import VXLAN  # noqa: E402
from scapy.utils import checksum  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)
work = tempfile.mkdtemp()
harness = os.path.join(work, "packettest")
built = subprocess.run([GRASM, "build", "packages/packettest/src/main.gr", "--native", "-o",
                        harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def mac():
    return ":".join(f"{rng.randrange(256):02x}" for _ in range(6))


def ip4():
    return ".".join(str(rng.randrange(256)) for _ in range(4))


def ip6():
    return socket.inet_ntop(socket.AF_INET6, rb(16))


def n0(v):
    return 0 if v is None else int(v)


cases = []   # (op, a, b, expected)


def add(op, a, expected, b=b""):
    cases.append((op, a, b, expected))


# --- ICMP ---------------------------------------------------------------
for _ in range(400):
    t = rng.choice([0, 8, 3, 5, 11, 12, 13, 14, 17, 18, rng.randrange(256)])
    kw = {"type": t, "code": rng.choice([0, 0, 1, 4, rng.randrange(256)])}
    if t in (0, 8, 13, 14, 17, 18):
        kw.update(id=rng.randrange(65536), seq=rng.randrange(65536))
    if t == 5:
        kw.update(gw=ip4())
    if t == 12:
        kw.update(ptr=rng.randrange(256))
    raw = bytes(ICMP(**kw) / Raw(rb(rng.randrange(0, 40))))
    good = raw
    if rng.random() < 0.2:
        raw = raw[:2] + bytes([raw[2] ^ 0x5a]) + raw[3:]
    p = ICMP(raw)
    text = (f"type={p.type};code={p.code};chksum={p.chksum};rest={raw[4:8].hex()};"
            f"body={raw[8:].hex()};ok={1 if checksum(raw) == 0 else 0}")
    if p.type in (0, 8):
        text += f";id={p.id};seq={p.seq}"
    add("icmp", raw, text)
    add("icmp.rt", raw, good.hex())
add("icmp", b"\x08\x00\x00", "ERR")

# --- ICMPv6 -------------------------------------------------------------
ND = [ICMPv6ND_NS, ICMPv6ND_NA, ICMPv6ND_RS, ICMPv6ND_RA, ICMPv6ND_Redirect]
for _ in range(400):
    src, dst = ip6(), ip6()
    kind = rng.choice(["echo", "reply", "ns", "na", "rs", "ra", "redirect", "unreach", "toobig",
                       "exceeded"])
    if kind == "echo":
        m = ICMPv6EchoRequest(id=rng.randrange(65536), seq=rng.randrange(65536), data=rb(rng.randrange(0, 20)))
    elif kind == "reply":
        m = ICMPv6EchoReply(id=rng.randrange(65536), seq=rng.randrange(65536), data=rb(rng.randrange(0, 20)))
    elif kind == "ns":
        m = ICMPv6ND_NS(tgt=ip6())
    elif kind == "na":
        m = ICMPv6ND_NA(tgt=ip6(), R=rng.randrange(2), S=rng.randrange(2), O=rng.randrange(2))
    elif kind == "rs":
        m = ICMPv6ND_RS()
    elif kind == "ra":
        m = ICMPv6ND_RA(chlim=rng.randrange(256), routerlifetime=rng.randrange(65536),
                        M=rng.randrange(2), O=rng.randrange(2))
    elif kind == "redirect":
        m = ICMPv6ND_Redirect(tgt=ip6(), dst=ip6())
    elif kind == "unreach":
        m = ICMPv6DestUnreach(code=rng.randrange(7)) / Raw(rb(rng.randrange(0, 30)))
    elif kind == "toobig":
        m = ICMPv6PacketTooBig(mtu=rng.randrange(1 << 32)) / Raw(rb(rng.randrange(0, 30)))
    else:
        m = ICMPv6TimeExceeded(code=rng.randrange(2)) / Raw(rb(rng.randrange(0, 30)))
    if kind in ("ns", "na", "rs", "ra", "redirect"):
        for _ in range(rng.randrange(0, 3)):
            m = m / rng.choice([ICMPv6NDOptSrcLLAddr(lladdr=mac()), ICMPv6NDOptDstLLAddr(lladdr=mac()),
                                ICMPv6NDOptMTU(mtu=rng.randrange(1 << 32)),
                                ICMPv6NDOptPrefixInfo(prefix=ip6(), prefixlen=rng.randrange(129))])
    pkt = IPv6(src=src, dst=dst) / m
    full = bytes(pkt)
    raw = full[40:]
    good = raw
    corrupt = rng.random() < 0.2
    if corrupt:
        raw = raw[:2] + bytes([raw[2] ^ 0x33]) + raw[3:]
    opts = []
    if raw[0] in (133, 134, 135, 136, 137):
        layer = IPv6(full).payload.payload
        while layer and layer.__class__.__name__.startswith("ICMPv6NDOpt"):
            ob = bytes(layer)[: layer.len * 8]
            opts.append(f"{ob[0]}:{ob[1]}:{ob[2:].hex()}")
            layer = layer.payload
    addrs = socket.inet_pton(socket.AF_INET6, src) + socket.inet_pton(socket.AF_INET6, dst)
    text = (f"type={raw[0]};code={raw[1]};cksum={raw[2] * 256 + raw[3]};body={raw[4:].hex()};"
            f"ok={0 if corrupt else 1};opts={'|'.join(opts)}")
    add("icmpv6", raw, text, addrs)
    add("icmpv6.rt", raw, good.hex(), addrs)

# --- ARP ----------------------------------------------------------------
for _ in range(300):
    v6 = rng.random() < 0.2
    if v6:
        a = ARP(op=rng.randrange(1, 5), hwsrc=mac(), hwdst=mac(), ptype=0x86DD, plen=16,
                psrc=ip6(), pdst=ip6())
    else:
        a = ARP(op=rng.randrange(1, 5), hwsrc=mac(), psrc=ip4(), hwdst=mac(), pdst=ip4())
    raw = bytes(a)
    p = ARP(raw)
    fam = socket.AF_INET6 if v6 else socket.AF_INET
    text = (f"hwtype={p.hwtype};ptype={p.ptype};hwlen={p.hwlen};plen={p.plen};op={p.op};"
            f"hwsrc={bytes.fromhex(p.hwsrc.replace(':', '')).hex()};"
            f"psrc={socket.inet_pton(fam, p.psrc).hex()};"
            f"hwdst={bytes.fromhex(p.hwdst.replace(':', '')).hex()};"
            f"pdst={socket.inet_pton(fam, p.pdst).hex()}")
    add("arp", raw, text)
    add("arp.rt", raw, raw.hex())
add("arp", bytes(ARP())[:20], "ERR")

# --- VLAN ---------------------------------------------------------------
for _ in range(300):
    frame = Ether(src=mac(), dst=mac())
    layers = []
    for _ in range(rng.randrange(0, 4)):
        cls = rng.choice([Dot1Q, Dot1AD])
        layers.append(cls(prio=rng.randrange(8), dei=rng.randrange(2), vlan=rng.randrange(4096)))
    pkt = frame
    for layer in layers:
        pkt = pkt / layer
    pkt = pkt / Raw(rb(rng.randrange(0, 20)))
    raw = bytes(pkt)
    tags = []
    at = 12
    while int.from_bytes(raw[at:at + 2], "big") in (0x8100, 0x88A8, 0x9100):
        tpid = int.from_bytes(raw[at:at + 2], "big")
        parsed = (Dot1AD if tpid == 0x88A8 else Dot1Q)(raw[at + 2:])
        tags.append(f"{tpid}:{parsed.prio}:{parsed.dei}:{parsed.vlan}:{parsed.type}")
        at += 4
    add("vlan", raw, f"tags={'|'.join(tags)};payload_at={at + 2}")
    add("vlan.rt", raw, raw.hex())

# --- GRE ----------------------------------------------------------------
for _ in range(300):
    kw = dict(chksum_present=rng.randrange(2), key_present=rng.randrange(2),
              seqnum_present=rng.randrange(2), recursion_control=rng.randrange(8),
              strict_route_source=rng.randrange(2), proto=rng.choice([0x0800, 0x86DD, 0x6558, rng.randrange(65536)]))
    if kw["key_present"]:
        kw["key"] = rng.randrange(1 << 32)
    if kw["seqnum_present"]:
        kw["sequence_number"] = rng.randrange(1 << 32)
    raw = bytes(GRE(**kw) / Raw(rb(rng.randrange(0, 30))))
    p = GRE(raw)
    names = ["chksum_present", "routing_present", "key_present", "seqnum_present",
             "strict_route_source", "recursion_control", "flags", "version", "proto", "chksum",
             "offset", "key", "sequence_number"]
    hlen = 4 + 4 * ((p.chksum_present or p.routing_present) + p.key_present + p.seqnum_present)
    text = ";".join(f"{n}={n0(getattr(p, n))}" for n in names) + f";payload={raw[hlen:].hex()}"
    add("gre", raw, text)
    add("gre.rt", raw, raw.hex())

# --- VXLAN --------------------------------------------------------------
for _ in range(300):
    flags = rng.choice([0x08, 0x08, 0x88, 0x0C])
    kw = dict(flags=flags, vni=rng.randrange(1 << 24))
    if flags & 0x80:
        kw.update(gpflags=rng.randrange(256), gpid=rng.randrange(65536))
    if flags & 0x04:
        kw.update(NextProtocol=rng.randrange(1, 5))
    raw = bytes(VXLAN(**kw) / Raw(rb(rng.randrange(0, 30))))
    p = VXLAN(raw)
    add("vxlan", raw, f"flags={raw[0]};vni={p.vni};payload={raw[8:].hex()}")
    add("vxlan.rt", raw, raw.hex())

# --- Geneve -------------------------------------------------------------
for _ in range(300):
    opts = [GeneveOptions(classid=rng.randrange(65536), type=rng.randrange(256),
                          data=rb(4 * rng.randrange(0, 4))) for _ in range(rng.randrange(0, 3))]
    g = GENEVE(oam=rng.randrange(2), critical=rng.randrange(2), proto=rng.choice([0x6558, 0x0800]),
               vni=rng.randrange(1 << 24), options=opts)
    raw = bytes(g / Raw(rb(rng.randrange(0, 20))))
    p = GENEVE(raw)
    olist = "|".join(f"{o.classid}:{o.type}:{n0(o.reserved)}:{bytes(o.data).hex()}" for o in p.options)
    text = (f"version={p.version};oam={p.oam};critical={p.critical};reserved={p.reserved};"
            f"proto={p.proto};vni={p.vni};reserved2={p.reserved2};options={olist};"
            f"payload={raw[8 + 4 * p.optionlen:].hex()}")
    add("geneve", raw, text)
    add("geneve.rt", raw, raw.hex())

# --- SLL ----------------------------------------------------------------
for _ in range(300):
    src = rb(8)
    if rng.random() < 0.5:
        h = CookedLinux(pkttype=rng.randrange(5), lladdrtype=rng.choice([1, 772, rng.randrange(65536)]),
                        lladdrlen=rng.randrange(9), src=src, proto=rng.choice([0x0800, 0x86DD]))
        raw = bytes(h / Raw(rb(rng.randrange(0, 20))))
        p = CookedLinux(raw)
        text = (f"pkttype={p.pkttype};lladdrtype={p.lladdrtype};lladdrlen={p.lladdrlen};"
                f"src={raw[6:14].hex()};proto={p.proto};ifindex=0;reserved=0;payload={raw[16:].hex()}")
        add("sll1", raw, text)
        add("sll1.rt", raw, raw.hex())
    else:
        h = CookedLinuxV2(proto=rng.choice([0x0800, 0x86DD]), ifindex=rng.randrange(1 << 32),
                          lladdrtype=rng.choice([1, 772]), pkttype=rng.randrange(5),
                          lladdrlen=rng.randrange(9), src=src)
        raw = bytes(h / Raw(rb(rng.randrange(0, 20))))
        p = CookedLinuxV2(raw)
        text = (f"pkttype={p.pkttype};lladdrtype={p.lladdrtype};lladdrlen={p.lladdrlen};"
                f"src={raw[12:20].hex()};proto={p.proto};ifindex={p.ifindex};reserved={p.reserved};"
                f"payload={raw[20:].hex()}")
        add("sll2", raw, text)
        add("sll2.rt", raw, raw.hex())

argv = []
for op, a, b, _ in cases:
    argv += [op, "x" + a.hex(), "x" + b.hex()]
got = []
for start in range(0, len(argv), 3 * 1000):
    ran = subprocess.run([harness] + argv[start:start + 3000], capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    got += ran.stdout.split("\n")[:-1]

families = {}
agree = differ = 0
for i, (op, a, b, want) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    fam = families.setdefault(op, [0, 0])
    if mine == want:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 10:
            print(f"  {op} {a.hex()[:80]}")
            print(f"    scapy {want[:200]}")
            print(f"    grasm {mine[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 16:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
