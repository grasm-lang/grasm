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
from scapy.layers.rtp import RTP  # noqa: E402
from scapy.contrib.stun import (STUN, STUNXorMappedAddress, STUNUsername,  # noqa: E402
                                STUNPriority, STUNFingerprint)
from scapy.layers.sctp import (SCTP, SCTPChunkData, SCTPChunkInit, SCTPChunkSACK,  # noqa: E402
                               SCTPChunkHeartbeatReq)
from scapy.layers.dhcp import BOOTP, DHCP  # noqa: E402
from scapy.layers.dhcp6 import (DHCP6_Solicit, DHCP6_Reply, DHCP6_RelayForward,  # noqa: E402
                                DHCP6OptClientId, DHCP6OptElapsedTime, DHCP6OptIA_NA,
                                DHCP6OptRelayMsg, DUID_LL)
from scapy.layers.ipsec import SecurityAssociation, ESP  # noqa: E402
from scapy.layers.inet import IP, UDP  # noqa: E402
from scapy.layers.netbios import NBNSHeader, NBNSQueryRequest, NBNSQueryResponse  # noqa: E402
from scapy.layers.pflog import PFLog  # noqa: E402
import struct  # noqa: E402
import zlib  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)
work = tempfile.mkdtemp()
harness = os.path.join(work, "packettest")
oracle = os.path.join(work, "gostdoracle")
go_built = subprocess.run(["go", "build", "-o", oracle, "."], cwd=os.path.join(ROOT, "scripts/lib/gostd"),
                          capture_output=True, text=True)
if go_built.returncode != 0:
    print(go_built.stderr[-1500:]); sys.exit(1)
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


# --- RTP ----------------------------------------------------------------
for _ in range(300):
    csrc = [rng.randrange(1 << 32) for _ in range(rng.randrange(0, 4))]
    ext = rng.random() < 0.3
    padn = rng.choice([0, 0, 0, 1, 4, 7])
    payload = rb(rng.randrange(0, 30))
    body = b""
    ext_profile, ext_data = 0, b""
    if ext:
        ext_profile = rng.randrange(65536)
        ext_data = rb(4 * rng.randrange(0, 3))
        body += struct.pack(">HH", ext_profile, len(ext_data) // 4) + ext_data
    pad = (bytes(padn - 1) + bytes([padn])) if padn else b""
    hdr = RTP(version=2, padding=1 if padn else 0, extension=1 if ext else 0, marker=rng.randrange(2),
              payload_type=rng.randrange(128), sequence=rng.randrange(65536),
              timestamp=rng.randrange(1 << 32), sourcesync=rng.randrange(1 << 32), sync=csrc)
    raw = bytes(hdr / Raw(body + payload + pad))
    p = RTP(raw)
    text = (f"version={p.version};padding={p.padding};extension={p.extension};marker={p.marker};"
            f"payload_type={p.payload_type};sequence={p.sequence};timestamp={p.timestamp};"
            f"ssrc={p.sourcesync};csrc={','.join(str(c) for c in p.sync)};ext_profile={ext_profile};"
            f"ext_data={ext_data.hex()};payload={payload.hex()};pad={pad.hex()}")
    add("rtp", raw, text)
    add("rtp.rt", raw, raw.hex())

# --- STUN ---------------------------------------------------------------
COOKIE = 0x2112A442
for _ in range(300):
    txid = rng.getrandbits(96)
    method, cls = 1, rng.randrange(4)
    mtype = (method & 0xF) | ((method & 0x70) << 1) | ((method & 0xF80) << 2) | ((cls & 1) << 4) | ((cls & 2) << 7)
    attrs = []
    port = rng.randrange(65536)
    ipaddr = ip4()
    # scapy's xport and xip are the real port and address; it XORs them.
    if rng.random() < 0.7:
        attrs.append(STUNXorMappedAddress(xport=port, xip=ipaddr))
    if rng.random() < 0.5:
        attrs.append(STUNUsername(username=rb(rng.randrange(1, 10)).hex()[: rng.randrange(1, 10)].encode()))
    if rng.random() < 0.4:
        attrs.append(STUNPriority(priority=rng.randrange(1 << 32)))
    msg = STUN(stun_message_type=mtype, transaction_id=txid, attributes=attrs)
    raw = bytes(msg)
    if rng.random() < 0.5:
        # A FINGERPRINT, computed here with zlib's CRC-32 over the message
        # whose length already counts it.
        body = raw[20:]
        head = raw[:2] + struct.pack(">H", len(body) + 8) + raw[4:20]
        crc = zlib.crc32(head + body) ^ 0x5354554E
        raw = head + body + struct.pack(">HHI", 0x8028, 4, crc)
    parsed = []
    at = 20
    xma = ""
    while at < len(raw):
        t, n = struct.unpack(">HH", raw[at:at + 4])
        v = raw[at + 4:at + 4 + n]
        parsed.append(f"{t}:{v.hex()}")
        if t == 0x20:
            xma = f"1:{port}:{socket.inet_aton(ipaddr).hex()}"
        at += 4 + (n + 3) // 4 * 4
    fp = 0
    if len(raw) >= 28 and raw[-8:-6] == b"\x80\x28":
        fp = 1 if zlib.crc32(raw[:-8]) ^ 0x5354554E == struct.unpack(">I", raw[-4:])[0] else 0
    text = (f"type={mtype};class={cls};method={method};cookie={COOKIE};txid={txid:024x};"
            f"attrs={'|'.join(parsed)};xma={xma};fp={fp}")
    add("stun", raw, text)
    add("stun.rt", raw, raw.hex())

# --- SCTP ---------------------------------------------------------------
for _ in range(300):
    chunks = []
    for _ in range(rng.randrange(1, 4)):
        k = rng.choice(["data", "data", "init", "sack", "hb"])
        if k == "data":
            chunks.append(SCTPChunkData(tsn=rng.randrange(1 << 32), stream_id=rng.randrange(65536),
                                        stream_seq=rng.randrange(65536), proto_id=rng.randrange(1 << 32),
                                        beginning=rng.randrange(2), ending=rng.randrange(2),
                                        unordered=rng.randrange(2), data=rb(rng.randrange(0, 20))))
        elif k == "init":
            chunks.append(SCTPChunkInit(init_tag=rng.randrange(1 << 32), a_rwnd=rng.randrange(1 << 32),
                                        n_out_streams=rng.randrange(65536), n_in_streams=rng.randrange(65536),
                                        init_tsn=rng.randrange(1 << 32)))
        elif k == "sack":
            chunks.append(SCTPChunkSACK(cumul_tsn_ack=rng.randrange(1 << 32), a_rwnd=rng.randrange(1 << 32)))
        else:
            chunks.append(SCTPChunkHeartbeatReq())
    pkt = SCTP(sport=rng.randrange(65536), dport=rng.randrange(65536), tag=rng.randrange(1 << 32))
    for c in chunks:
        pkt = pkt / c
    raw = bytes(pkt)
    good = raw
    corrupt = rng.random() < 0.2
    if corrupt:
        raw = raw[:9] + bytes([raw[9] ^ 0x11]) + raw[10:]
    p = SCTP(raw)
    parts = []
    at = 12
    while at < len(raw):
        t, f, n = raw[at], raw[at + 1], struct.unpack(">H", raw[at + 2:at + 4])[0]
        v = raw[at + 4:at + n]
        extra = ""
        if t == 0:
            tsn, sid, ssn, ppid = struct.unpack(">IHHI", v[:12])
            extra = f":{tsn}/{sid}/{ssn}/{ppid}/{v[12:].hex()}"
        parts.append(f"{t}:{f}:{v.hex()}" + extra)
        at += (n + 3) // 4 * 4
    text = (f"sport={p.sport};dport={p.dport};tag={p.tag};chksum={p.chksum};ok={0 if corrupt else 1};"
            f"chunks={'|'.join(parts)}")
    add("sctp", raw, text)
    add("sctp.rt", raw, good.hex())

# --- DHCP ---------------------------------------------------------------
DHCP_OPTS = [("message-type", lambda: rng.randrange(1, 9)), ("lease_time", lambda: rng.randrange(1 << 32)),
             ("router", ip4), ("name_server", ip4), ("hostname", lambda: b"host-" + rb(2).hex().encode()),
             ("server_id", ip4), ("requested_addr", ip4), ("subnet_mask", ip4)]
for _ in range(300):
    opts = []
    for name, val in rng.sample(DHCP_OPTS, rng.randrange(0, 6)):
        opts.append((name, val()))
    if rng.random() < 0.2:
        opts.append("pad")
    opts.append("end")
    pkt = BOOTP(op=rng.choice([1, 2]), xid=rng.randrange(1 << 32), secs=rng.randrange(65536),
                flags=rng.choice([0, 0x8000]), ciaddr=ip4(), yiaddr=ip4(), siaddr=ip4(), giaddr=ip4(),
                chaddr=rb(6)) / DHCP(options=opts)
    raw = bytes(pkt)
    p = BOOTP(raw)
    olist = []
    at = 240
    mtype, lease, has_end = 0, -1, 0
    while at < len(raw):
        c = raw[at]
        if c == 0:
            olist.append("0:")
            at += 1
            continue
        if c == 255:
            has_end = 1
            break
        n = raw[at + 1]
        v = raw[at + 2:at + 2 + n]
        olist.append(f"{c}:{v.hex()}")
        if c == 53 and n == 1 and mtype == 0:
            mtype = v[0]
        if c == 51 and n == 4 and lease == -1:
            lease = struct.unpack(">I", v)[0]
        at += 2 + n
    text = (f"op={p.op};htype={p.htype};hlen={p.hlen};hops={p.hops};xid={p.xid};secs={p.secs};"
            f"flags={int(p.flags)};ciaddr={raw[12:16].hex()};yiaddr={raw[16:20].hex()};"
            f"siaddr={raw[20:24].hex()};giaddr={raw[24:28].hex()};chaddr={raw[28:44].hex()};cookie=1;"
            f"opts={'|'.join(olist)};end={has_end};type={mtype};lease={lease}")
    add("dhcp", raw, text)
    add("dhcp.rt", raw, raw.hex())

# --- DHCPv6 -------------------------------------------------------------
for _ in range(300):
    opts = DHCP6OptClientId(duid=DUID_LL(lladdr=mac())) / DHCP6OptElapsedTime(elapsedtime=rng.randrange(65536))
    if rng.random() < 0.5:
        opts = opts / DHCP6OptIA_NA(iaid=rng.randrange(1 << 32), T1=rng.randrange(1 << 32), T2=rng.randrange(1 << 32))
    kind = rng.choice(["sol", "reply", "relay"])
    if kind == "sol":
        msg = DHCP6_Solicit(trid=rng.randrange(1 << 24)) / opts
    elif kind == "reply":
        msg = DHCP6_Reply(trid=rng.randrange(1 << 24)) / opts
    else:
        inner = DHCP6_Solicit(trid=rng.randrange(1 << 24)) / opts
        msg = DHCP6_RelayForward(hopcount=rng.randrange(32), linkaddr=ip6(), peeraddr=ip6()) / DHCP6OptRelayMsg(message=inner)
    raw = bytes(msg)
    t = raw[0]
    start = 34 if t in (12, 13) else 4
    olist = []
    at = start
    while at < len(raw):
        c, n = struct.unpack(">HH", raw[at:at + 4])
        olist.append(f"{c}:{raw[at + 4:at + 4 + n].hex()}")
        at += 4 + n
    trid = 0 if t in (12, 13) else int.from_bytes(raw[1:4], "big")
    hops = raw[1] if t in (12, 13) else 0
    link = raw[2:18].hex() if t in (12, 13) else ""
    peer = raw[18:34].hex() if t in (12, 13) else ""
    add("dhcpv6", raw, f"type={t};trid={trid};hops={hops};link={link};peer={peer};opts={'|'.join(olist)}")
    add("dhcpv6.rt", raw, raw.hex())

# --- ESP ----------------------------------------------------------------
for _ in range(300):
    sa = SecurityAssociation(ESP, spi=rng.randrange(1, 1 << 32), crypt_algo="NULL", crypt_key=None)
    inner = IP(src=ip4(), dst=ip4()) / UDP(sport=rng.randrange(65536), dport=rng.randrange(65536)) / Raw(rb(rng.randrange(0, 30)))
    enc = sa.encrypt(inner)
    raw = bytes(enc[ESP])
    payload = bytes(inner[UDP])
    padn = raw[-2]
    add("espn", raw, f"spi={sa.spi};seq={struct.unpack('>I', raw[4:8])[0]};payload={payload.hex()};"
                     f"padding={raw[-2 - padn:-2].hex()};nh=17")
    add("espn.rt", raw, raw.hex())
gcm = []
for _ in range(200):
    key = rb(rng.choice([16, 24, 32]) + 4)
    head = rb(8)
    iv = rb(8)
    nh = rng.randrange(256)
    payload = rb(rng.randrange(0, 40))
    gcm.append((key, head + iv + bytes([nh]) + payload, head, nh, payload))
feed = "".join(f"espgcm x{k.hex()} x{b.hex()}\n" for k, b, _, _, _ in gcm)
sealed = subprocess.run([oracle], input=feed, capture_output=True, text=True).stdout.split("\n")[:-1]
for (key, b, head, nh, payload), out in zip(gcm, sealed):
    add("espg.s", key, out, b)
    data = bytes.fromhex(out)
    spi, seq = struct.unpack(">II", head)
    add("espg", key, f"spi={spi};seq={seq};payload={payload.hex()};nh={nh}", data)
    tampered = bytearray(data)
    tampered[rng.randrange(len(tampered))] ^= 1
    add("espg", key, "ERR", bytes(tampered))

# --- NBNS ---------------------------------------------------------------
def enc_suffix(v):
    """scapy's SUFFIX field holds the suffix already encoded as two letters."""
    return ((0x41 + (v >> 4)) << 8) | (0x41 + (v & 15))


for _ in range(300):
    name = "".join(rng.choice("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-") for _ in range(rng.randrange(1, 16)))
    suffix = rng.choice([0x00, 0x20, 0x1C, 0x1B, 0x03])
    if rng.random() < 0.5:
        pkt = NBNSHeader(NAME_TRN_ID=rng.randrange(65536)) / NBNSQueryRequest(QUESTION_NAME=name.encode(), SUFFIX=enc_suffix(suffix))
        raw = bytes(pkt)
        q = f"{name}/{suffix}//32/1"
        add("nbns", raw, f"id={pkt.NAME_TRN_ID};flags={struct.unpack('>H', raw[2:4])[0]};q={q};an=")
    else:
        addr = ip4()
        ttl = rng.randrange(1 << 32)
        pkt = NBNSHeader(NAME_TRN_ID=rng.randrange(65536), RESPONSE=1, QDCOUNT=0, ANCOUNT=1) / \
            NBNSQueryResponse(RR_NAME=name.encode(), SUFFIX=enc_suffix(suffix), TTL=ttl, ADDR_ENTRY=[])
        raw = bytes(pkt)
        try:
            parsed = NBNSHeader(raw)
        except Exception:
            continue
        rdlen = struct.unpack(">H", raw[12 + 34 + 8:12 + 34 + 10])[0]
        rdata = raw[12 + 34 + 10:12 + 34 + 10 + rdlen]
        an = f"{name}/{suffix}/32/1/{ttl}/{rdata.hex()}"
        add("nbns", raw, f"id={pkt.NAME_TRN_ID};flags={struct.unpack('>H', raw[2:4])[0]};q=;an={an}")
    add("nbns.rt", raw, raw.hex())

# --- pflog --------------------------------------------------------------
for _ in range(300):
    v6 = rng.random() < 0.3
    kw = dict(addrfamily=24 if v6 else 2, action=rng.randrange(14), reason=rng.randrange(20),
              iface=("em" + str(rng.randrange(10))).encode(), ruleset=b"" if rng.random() < 0.5 else b"main",
              rulenumber=rng.randrange(-5, 1000), subrulenumber=rng.randrange(-1, 10),
              uid=rng.randrange(-1, 1000), pid=rng.randrange(1 << 31), ruleuid=rng.randrange(-1, 100),
              rulepid=rng.randrange(1 << 31), direction=rng.randrange(3), rewritten=rng.randrange(2),
              naddrfamily=24 if v6 else 2, sport=rng.randrange(65536), dport=rng.randrange(65536))
    if v6:
        kw.update(saddr=ip6(), daddr=ip6())
    else:
        kw.update(saddr=ip4(), daddr=ip4())
    raw = bytes(PFLog(**kw) / Raw(rb(rng.randrange(0, 20))))
    p = PFLog(raw)
    n = 16 if v6 else 4
    fam = socket.AF_INET6 if v6 else socket.AF_INET
    text = (f"hdrlen={p.hdrlen};af={p.addrfamily};action={p.action};reason={p.reason};"
            f"iface={p.iface.rstrip(b'\x00').decode()};ruleset={p.ruleset.rstrip(b'\x00').decode()};"
            f"rule={p.rulenumber};subrule={p.subrulenumber};uid={p.uid};pid={p.pid};ruleuid={p.ruleuid};"
            f"rulepid={p.rulepid};dir={raw[60]};rewritten={p.rewritten};naf={p.naddrfamily};"
            f"saddr={socket.inet_pton(fam, p.saddr).hex()};daddr={socket.inet_pton(fam, p.daddr).hex()};"
            f"sport={p.sport};dport={p.dport};payload={raw[max(p.hdrlen, 100):].hex()}")
    add("pflog", raw, text)
    add("pflog.rt", raw, raw.hex())

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
        only = os.environ.get("ONLY", "")
        if op.startswith(only) and (differ <= 10 or only):
            shown_now = sum(1 for _ in [0])
            print(f"  {op} {a.hex()[:80]}")
            print(f"    scapy {want[:200]}")
            print(f"    grasm {mine[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 32:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
