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
from scapy.contrib.igmp import IGMP  # noqa: E402
from scapy.contrib.mpls import MPLS  # noqa: E402
from scapy.contrib.lldp import (LLDPDUChassisID, LLDPDUPortID, LLDPDUTimeToLive,  # noqa: E402
                                LLDPDUSystemName, LLDPDUEndOfLLDPDU)
from scapy.layers.l2 import STP  # noqa: E402
from scapy.config import conf  # noqa: E402
# Out-of-order LLDPDUs are built on purpose, to check they are reported.
conf.contribs["LLDP"].strict_mode_disable()
from scapy.contrib.cdp import CDPv2_HDR, CDPMsgDeviceID, CDPMsgPortID, CDPMsgPlatform  # noqa: E402
from scapy.layers.rip import RIP, RIPEntry  # noqa: E402
from scapy.layers.vrrp import VRRP, VRRPv3  # noqa: E402
from scapy.layers.hsrp import HSRP  # noqa: E402
from scapy.contrib.bfd import BFD  # noqa: E402
from scapy.contrib.nsh import NSH  # noqa: E402
from scapy.contrib.erspan import ERSPAN_II, ERSPAN_III  # noqa: E402
from scapy.layers.l2tp import L2TP  # noqa: E402
from scapy.layers.ppp import PPPoE, PPPoED, PPP  # noqa: E402
from scapy.contrib.ospf import OSPF_Hdr, OSPF_Hello, OSPF_LSUpd, OSPF_Router_LSA, OSPF_LSAck, OSPF_LSA_Hdr  # noqa: E402
from scapy.contrib.bgp import (BGPHeader, BGPOpen, BGPUpdate, BGPNotification, BGPPathAttr,  # noqa: E402
                               BGPPAOrigin, BGPPANextHop, BGPPAMultiExitDisc, BGPNLRI_IPv4)
from scapy.contrib.eigrp import EIGRP, EIGRPParam, EIGRPSwVer  # noqa: E402
from scapy.contrib.pim import PIMv2Hdr, PIMv2Hello, PIMv2HelloHoldtime, PIMv2HelloDRPriority  # noqa: E402
from scapy.contrib.isis import ISIS_CommonHdr, ISIS_L1_LAN_Hello, ISIS_AreaTlv, ISIS_GenericTlv  # noqa: E402
from scapy.layers.eap import EAP, EAPOL, EAPOL_KEY  # noqa: E402
from scapy.layers.dot11 import RadioTap  # noqa: E402
from scapy.layers.can import CAN  # noqa: E402
from scapy.layers.usb import USBpcap  # noqa: E402
from scapy.layers.bluetooth import HCI_Hdr, HCI_Command_Hdr, HCI_ACL_Hdr, HCI_Event_Hdr  # noqa: E402
from scapy.layers.radius import Radius, RadiusAttribute  # noqa: E402
from scapy.layers.netflow import (NetflowHeader, NetflowHeaderV5, NetflowRecordV5, NetflowHeaderV9,  # noqa: E402
                                  NetflowFlowsetV9, NetflowTemplateV9, NetflowTemplateFieldV9)
from scapy.contrib.modbus import (ModbusADURequest, ModbusADUResponse,  # noqa: E402
                                  ModbusPDU03ReadHoldingRegistersRequest,
                                  ModbusPDU03ReadHoldingRegistersResponse,
                                  ModbusPDU06WriteSingleRegisterRequest)
from scapy.layers.snmp import SNMP, SNMPget, SNMPnext, SNMPresponse, SNMPset, SNMPbulk, SNMPvarbind  # noqa: E402
from scapy.asn1.asn1 import (ASN1_OID, ASN1_INTEGER, ASN1_STRING, ASN1_NULL, ASN1_IPADDRESS,  # noqa: E402
                             ASN1_COUNTER32, ASN1_GAUGE32, ASN1_TIME_TICKS, ASN1_COUNTER64)
import hashlib  # noqa: E402
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
harness2 = os.path.join(work, "packettest2")
for src, out in [("packages/packettest/src/main.gr", harness), ("packages/packettest2/src/main.gr", harness2)]:
    built = subprocess.run([GRASM, "build", src, "--native", "-o", out], cwd=ROOT, capture_output=True, text=True)
    if built.returncode != 0:
        print(built.stderr[-1500:]); sys.exit(1)
harness3 = os.path.join(work, "packettest3")
built = subprocess.run([GRASM, "build", "packages/packettest3/src/main.gr", "--native", "-o", harness3],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
THIRD = ("radius", "radauth", "radhide", "radreveal", "nf5", "nf9", "mbtcp", "mbrtu", "snmp", "md4")
# The protocols the second harness answers for.
SECOND = ("igmp", "mpls", "lldp", "stp", "cdp", "rip", "vrrp", "bfd", "hsrp", "nsh", "erspan", "l2tp", "pppoe",
          "ospf", "lsack", "bgp", "eigrp", "pim", "isis", "eapol", "radiotap", "can", "usbpcap", "hci")


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


# --- B15: routing, links and LANs ---------------------------------------
def tlvs16(raw, start):
    """LLDP's 7-bit type, 9-bit length TLVs, up to the end TLV."""
    out, at = [], start
    while at < len(raw):
        head = struct.unpack(">H", raw[at:at + 2])[0]
        t, n = head >> 9, head & 511
        out.append(f"{t}:{raw[at + 2:at + 2 + n].hex()}")
        at += 2 + n
        if t == 0:
            break
    return out


for _ in range(250):
    raw = bytes(IGMP(type=rng.choice([0x11, 0x16, 0x17, 0x12]), mrcode=rng.randrange(256), gaddr=ip4()) /
                Raw(rb(rng.choice([0, 0, 4]))))
    good = raw
    if rng.random() < 0.2:
        raw = raw[:2] + bytes([raw[2] ^ 1]) + raw[3:]
    p = IGMP(raw)
    add("igmp", raw, f"type={p.type};mrcode={p.mrcode};chksum={p.chksum};gaddr={socket.inet_aton(p.gaddr).hex()};"
                     f"rest={raw[8:].hex()};ok={1 if checksum(raw) == 0 else 0}")
    add("igmp.rt", raw, good.hex())

for _ in range(250):
    n = rng.randrange(1, 5)
    pkt = None
    for k in range(n):
        e = MPLS(label=rng.randrange(1 << 20), cos=rng.randrange(8), s=1 if k == n - 1 else 0, ttl=rng.randrange(256))
        pkt = e if pkt is None else pkt / e
    raw = bytes(pkt / Raw(rb(rng.randrange(0, 20))))
    stack = []
    for k in range(n):
        v = struct.unpack(">I", raw[4 * k:4 * k + 4])[0]
        stack.append(f"{v >> 12}:{(v >> 9) & 7}:{(v >> 8) & 1}:{v & 255}")
    add("mpls", raw, f"stack={'|'.join(stack)};payload_at={4 * n}")
    add("mpls.rt", raw, raw.hex())

for _ in range(250):
    parts = [LLDPDUChassisID(subtype=4, id=mac()), LLDPDUPortID(subtype=5, id=rb(rng.randrange(1, 8)).hex()),
             LLDPDUTimeToLive(ttl=rng.randrange(65536))]
    if rng.random() < 0.5:
        parts.append(LLDPDUSystemName(system_name=("sw" + str(rng.randrange(100))).encode()))
    if rng.random() < 0.15:
        parts[0], parts[1] = parts[1], parts[0]
    parts.append(LLDPDUEndOfLLDPDU())
    pkt = Ether(dst="01:80:c2:00:00:0e", src=mac(), type=0x88CC)
    for q in parts:
        pkt = pkt / q
    raw = bytes(pkt)[14:]
    tl = tlvs16(raw, 0)
    kinds = [int(t.split(":")[0]) for t in tl]
    ok = 1 if kinds[:3] == [1, 2, 3] else 0
    ttl = next((int(t.split(":")[1][:4], 16) for t in tl if t.startswith("3:")), -1)
    add("lldp", raw, f"tlvs={'|'.join(tl)};ok={ok};ttl={ttl}")
    # Ethernet pads a short frame after the end TLV; that is not LLDPDU.
    end = sum(2 + len(bytes.fromhex(t.split(":")[1])) for t in tl)
    add("lldp.rt", raw, raw[:end].hex())

for _ in range(250):
    tcn = rng.random() < 0.2
    pkt = STP(version=rng.choice([0, 2]), bpdutype=0x80 if tcn else rng.choice([0, 2]), bpduflags=rng.randrange(256),
              rootid=rng.randrange(65536), rootmac=mac(), pathcost=rng.randrange(1 << 32), bridgeid=rng.randrange(65536),
              bridgemac=mac(), portid=rng.randrange(65536), age=rng.randrange(256), maxage=rng.randrange(256),
              hellotime=rng.randrange(256), fwddelay=rng.randrange(256))
    raw = bytes(pkt / Raw(rb(rng.choice([0, 0, 1]))))
    if tcn:
        text = (f"proto={struct.unpack('>H', raw[:2])[0]};version={raw[2]};bpdutype=128;flags=0;rootid=0;rootmac=;"
                f"pathcost=0;bridgeid=0;bridgemac=;portid=0;age=0;maxage=0;hello=0;fwd=0;rest={raw[4:].hex()}")
    else:
        q = STP(raw)
        text = (f"proto={q.proto};version={q.version};bpdutype={q.bpdutype};flags={int(q.bpduflags)};rootid={q.rootid};"
                f"rootmac={q.rootmac.replace(':', '')};pathcost={q.pathcost};bridgeid={q.bridgeid};"
                f"bridgemac={q.bridgemac.replace(':', '')};portid={q.portid};"
                # The timers on the wire are 1/256ths of a second.
                f"age={struct.unpack('>H', raw[27:29])[0]};maxage={struct.unpack('>H', raw[29:31])[0]};"
                f"hello={struct.unpack('>H', raw[31:33])[0]};fwd={struct.unpack('>H', raw[33:35])[0]};"
                f"rest={raw[35:].hex()}")
    add("stp", raw, text)
    add("stp.rt", raw, raw.hex())

for _ in range(250):
    msgs = [CDPMsgDeviceID(val=("dev" + str(rng.randrange(1000))).encode())]
    if rng.random() < 0.7:
        msgs.append(CDPMsgPortID(iface=("Gi0/" + str(rng.randrange(48))).encode()))
    if rng.random() < 0.5:
        msgs.append(CDPMsgPlatform(val=rb(rng.randrange(1, 6)).hex()[: rng.randrange(1, 9)].encode()))
    raw = bytes(CDPv2_HDR(vers=2, ttl=rng.randrange(256), msg=msgs))
    good = raw
    corrupt = rng.random() < 0.2
    if corrupt:
        raw = raw[:2] + bytes([raw[2] ^ 4]) + raw[3:]
    tl, at = [], 4
    while at < len(raw):
        t, n = struct.unpack(">HH", raw[at:at + 4])
        tl.append(f"{t}:{raw[at + 4:at + n].hex()}")
        at += n
    add("cdp", raw, f"vers={raw[0]};ttl={raw[1]};cksum={struct.unpack('>H', raw[2:4])[0]};ok={0 if corrupt else 1};"
                    f"tlvs={'|'.join(tl)}")
    add("cdp.rt", raw, good.hex())

for _ in range(250):
    pkt = RIP(cmd=rng.choice([1, 2]), version=rng.choice([1, 2]))
    entries = []
    for _ in range(rng.randrange(0, 4)):
        e = RIPEntry(AF=2, RouteTag=rng.randrange(65536), addr=ip4(), mask=ip4(), nextHop=ip4(), metric=rng.randrange(1, 17))
        pkt = pkt / e
    raw = bytes(pkt)
    ents = []
    at = 4
    while at + 20 <= len(raw):
        af, tag = struct.unpack(">HH", raw[at:at + 4])
        metric = struct.unpack(">I", raw[at + 16:at + 20])[0]
        ents.append(f"{af}:{tag}:{raw[at + 4:at + 8].hex()}:{raw[at + 8:at + 12].hex()}:{raw[at + 12:at + 16].hex()}:{metric}")
        at += 20
    add("rip", raw, f"cmd={raw[0]};version={raw[1]};entries={'|'.join(ents)}")
    add("rip.rt", raw, raw.hex())

for _ in range(450):
    kind = rng.choice(["v2", "v3", "v3six"])
    if kind == "v2":
        addrs = [ip4() for _ in range(rng.randrange(1, 4))]
        auth = rb(8)
        pkt = VRRP(vrid=rng.randrange(256), priority=rng.randrange(256), addrlist=addrs, authtype=rng.randrange(3),
                   adv=rng.randrange(256), auth1=int.from_bytes(auth[:4], "big"), auth2=int.from_bytes(auth[4:], "big"))
        raw = bytes(pkt)
        op, n, srcdst = "vrrp4", 4, rb(8)
        alist = [socket.inet_aton(x).hex() for x in addrs]
        text = (f"version=2;type=1;vrid={raw[1]};priority={raw[2]};authtype={raw[4]};adv={raw[5]};"
                f"chksum={struct.unpack('>H', raw[6:8])[0]};addrs={','.join(alist)};auth={raw[8 + 4 * len(addrs):].hex()}")
    else:
        six = kind == "v3six"
        if six:
            src, dst = ip6(), ip6()
            addrs = [ip6() for _ in range(rng.randrange(1, 3))]
            outer = IPv6(src=src, dst=dst)
            fam, n = socket.AF_INET6, 16
        else:
            src, dst = ip4(), ip4()
            addrs = [ip4() for _ in range(rng.randrange(1, 4))]
            outer = IP(src=src, dst=dst)
            fam, n = socket.AF_INET, 4
        full = outer / VRRPv3(vrid=rng.randrange(256), priority=rng.randrange(256), adv=rng.randrange(4096), addrlist=addrs)
        raw = bytes(full[VRRPv3])
        op = "vrrp6" if six else "vrrp4"
        srcdst = socket.inet_pton(fam, src) + socket.inet_pton(fam, dst)
        alist = [socket.inet_pton(fam, x).hex() for x in addrs]
        adv = struct.unpack(">H", raw[4:6])[0] & 4095
        text = (f"version=3;type=1;vrid={raw[1]};priority={raw[2]};authtype=0;adv={adv};"
                f"chksum={struct.unpack('>H', raw[6:8])[0]};addrs={','.join(alist)};auth=")
    add(op, raw, text, srcdst)
    add(op + ".rt", raw, raw.hex(), srcdst)

for _ in range(250):
    raw = bytes(HSRP(opcode=rng.randrange(3), state=rng.choice([0, 1, 2, 4, 8, 16]), hellotime=rng.randrange(256),
                     holdtime=rng.randrange(256), priority=rng.randrange(256), group=rng.randrange(256),
                     auth=rb(8), virtualIP=ip4()))
    add("hsrp", raw, f"version={raw[0]};opcode={raw[1]};state={raw[2]};hello={raw[3]};hold={raw[4]};priority={raw[5]};"
                     f"group={raw[6]};auth={raw[8:16].hex()};vip={raw[16:20].hex()}")
    add("hsrp.rt", raw, raw.hex())

for _ in range(250):
    raw = bytes(BFD(version=1, diag=rng.randrange(32), sta=rng.randrange(4), flags=rng.randrange(64) & ~4,
                    detect_mult=rng.randrange(256), my_discriminator=rng.randrange(1 << 32),
                    your_discriminator=rng.randrange(1 << 32), min_tx_interval=rng.randrange(1 << 32),
                    min_rx_interval=rng.randrange(1 << 32), echo_rx_interval=rng.randrange(1 << 32)))
    q = BFD(raw)
    add("bfd", raw, f"version={q.version};diag={q.diag};sta={q.sta};flags={int(q.flags)};detect_mult={q.detect_mult};"
                    f"len={q.len};my={q.my_discriminator};your={q.your_discriminator};tx={q.min_tx_interval};"
                    f"rx={q.min_rx_interval};echo={q.echo_rx_interval};auth=")
    add("bfd.rt", raw, raw.hex())

for _ in range(250):
    ctx = rb(16)
    raw = bytes(NSH(mdtype=1, ttl=rng.randrange(64), nextproto=rng.choice([1, 2, 3]), spi=rng.randrange(1 << 24),
                    si=rng.randrange(256), oam=rng.randrange(2), context_header=ctx) / Raw(rb(rng.randrange(0, 20))))
    q = NSH(raw)
    add("nsh", raw, f"ver={q.ver};oam={q.oam};ttl={q.ttl};mdtype={q.mdtype};nextproto={q.nextproto};spi={q.spi};"
                    f"si={q.si};context={raw[8:24].hex()};payload={raw[24:].hex()}")
    add("nsh.rt", raw, raw.hex())

for _ in range(250):
    payload = rb(rng.randrange(0, 20))
    if rng.random() < 0.5:
        raw = bytes(ERSPAN_II(vlan=rng.randrange(4096), cos=rng.randrange(8), en=rng.randrange(4), t=rng.randrange(2),
                              session_id=rng.randrange(1024), index=rng.randrange(1 << 20)) / Raw(payload))
        q = ERSPAN_II(raw)
        text = (f"ver=1;vlan={q.vlan};cos={q.cos};en={q.en};t={q.t};session={q.session_id};index={q.index};"
                f"ts=0;sgt=0;p=0;ft=0;hw=0;d=0;gra=0;o=0;platform=;payload={raw[8:].hex()}")
    else:
        raw = bytes(ERSPAN_III(vlan=rng.randrange(4096), cos=rng.randrange(8), bso=rng.randrange(4), t=rng.randrange(2),
                               session_id=rng.randrange(1024), timestamp=rng.randrange(1 << 32),
                               sgt_other=rng.randrange(65536), p=rng.randrange(2), ft=rng.randrange(32),
                               hw=rng.randrange(64), d=rng.randrange(2), gra=rng.randrange(4), o=0) / Raw(payload))
        q = ERSPAN_III(raw)
        text = (f"ver=2;vlan={q.vlan};cos={q.cos};en={q.bso};t={q.t};session={q.session_id};index=0;"
                f"ts={q.timestamp};sgt={q.sgt_other};p={q.p};ft={q.ft};hw={q.hw};d={q.d};gra={q.gra};o=0;platform=;"
                f"payload={raw[12:].hex()}")
    add("erspan", raw, text)
    add("erspan.rt", raw, raw.hex())

for _ in range(250):
    choice = rng.choice(["data", "len", "seq", "lenseq", "offset", "control"])
    flags = {"data": 0, "len": 0x400, "seq": 0x80, "lenseq": 0x480, "offset": 0x20, "control": 0xC80}[choice]
    kw = dict(hdr=flags, tunnel_id=rng.randrange(65536), session_id=rng.randrange(65536))
    if flags & 0x80:
        kw.update(ns=rng.randrange(65536), nr=rng.randrange(65536))
    if flags & 0x20:
        kw.update(offset=2)
    payload = rb(rng.randrange(0, 20))
    raw = bytes(L2TP(**kw) / Raw(payload))
    length = struct.unpack(">H", raw[2:4])[0] if flags & 0x400 else 0
    add("l2tp", raw, f"flags={flags};version=2;len={length};tunnel={kw['tunnel_id']};session={kw['session_id']};"
                     f"ns={kw.get('ns', 0)};nr={kw.get('nr', 0)};offset={kw.get('offset', 0)};payload={payload.hex()}")
    add("l2tp.rt", raw, raw.hex())

for _ in range(250):
    if rng.random() < 0.5:
        code = rng.choice([0x09, 0x07, 0x19, 0x65, 0xA7])
        tags = b""
        tl = []
        for _ in range(rng.randrange(0, 4)):
            t = rng.choice([0x0101, 0x0102, 0x0103, 0x0104])
            v = rb(rng.randrange(0, 8))
            tags += struct.pack(">HH", t, len(v)) + v
            tl.append(f"{t}:{v.hex()}")
        raw = bytes(PPPoED(code=code, sessionid=rng.randrange(65536)) / Raw(tags))
    else:
        code = 0
        tl = []
        raw = bytes(PPPoE(sessionid=rng.randrange(65536)) / PPP(proto=0x0021) / Raw(rb(rng.randrange(0, 20))))
    n = struct.unpack(">H", raw[4:6])[0]
    add("pppoe", raw, f"version=1;type=1;code={code};session={struct.unpack('>H', raw[2:4])[0]};len={n};"
                      f"payload={raw[6:6 + n].hex()};tags={'|'.join(tl)}")
    add("pppoe.rt", raw, raw.hex())

# --- B15, second part ---------------------------------------------------
def tlv8(raw, start, end):
    out, at = [], start
    while at < end:
        out.append(f"{raw[at]}:{raw[at + 2:at + 2 + raw[at + 1]].hex()}")
        at += 2 + raw[at + 1]
    return out


for _ in range(250):
    if rng.random() < 0.5:
        nbrs = [ip4() for _ in range(rng.randrange(0, 3))]
        pkt = OSPF_Hdr(type=1, src=ip4(), area=ip4(), authtype=rng.choice([0, 1]), authdata=rng.randrange(1 << 63)) / \
            OSPF_Hello(mask=ip4(), hellointerval=rng.randrange(65536), prio=rng.randrange(256),
                       deadinterval=rng.randrange(1 << 32), router=ip4(), backup=ip4(), neighbors=nbrs)
    else:
        lsas = [OSPF_Router_LSA(id=ip4(), adrouter=ip4(), seq=rng.randrange(1 << 31), age=rng.randrange(3600))
                for _ in range(rng.randrange(1, 3))]
        pkt = OSPF_Hdr(type=4, src=ip4(), area=ip4()) / OSPF_LSUpd(lsalist=lsas)
    raw = bytes(pkt)
    t = raw[1]
    text = (f"version={raw[0]};type={t};len={struct.unpack('>H', raw[2:4])[0]};src={raw[4:8].hex()};"
            f"area={raw[8:12].hex()};chksum={struct.unpack('>H', raw[12:14])[0]};authtype={struct.unpack('>H', raw[14:16])[0]}")
    body = raw[24:]
    if t == 1:
        ncount = (len(body) - 20) // 4
        text += (f";hello={body[0:4].hex()}/{struct.unpack('>H', body[4:6])[0]}/{body[6]}/{body[7]}/"
                 f"{struct.unpack('>I', body[8:12])[0]}/{body[12:16].hex()}/{body[16:20].hex()}/{ncount}")
    else:
        heads, at = [], 4
        for _ in range(struct.unpack(">I", body[:4])[0]):
            age, _opt, kind = struct.unpack(">HBB", body[at:at + 4])
            seq, ck, ln = struct.unpack(">IHH", body[at + 12:at + 20])
            heads.append(f"{age}/{kind}/{body[at + 4:at + 8].hex()}/{seq}/{ck}/{ln}")
            add("lsack", body[at:at + ln], str(ck))
            at += ln
        text += ";lsas=" + ",".join(heads)
    add("ospf", raw, text)
    add("ospf.rt", raw, raw.hex())

for _ in range(250):
    kind = rng.choice(["open", "update", "update", "keepalive", "notify"])
    if kind == "open":
        pkt = BGPHeader(type=1) / BGPOpen(my_as=rng.randrange(65536), hold_time=rng.randrange(65536), bgp_id=ip4())
    elif kind == "update":
        def nl():
            bits = rng.randrange(0, 33)
            addr = socket.inet_ntoa(struct.pack(">I", rng.getrandbits(32) & (((1 << bits) - 1) << (32 - bits)) if bits else 0))
            return BGPNLRI_IPv4(prefix=f"{addr}/{bits}")
        attrs = [BGPPathAttr(type_flags=0x40, type_code=1, attribute=BGPPAOrigin(origin=rng.randrange(3))),
                 BGPPathAttr(type_flags=0x40, type_code=3, attribute=BGPPANextHop(next_hop=ip4()))]
        if rng.random() < 0.5:
            attrs.append(BGPPathAttr(type_flags=0x80, type_code=4, attribute=BGPPAMultiExitDisc(med=rng.randrange(1 << 32))))
        pkt = BGPHeader(type=2) / BGPUpdate(withdrawn_routes=[nl() for _ in range(rng.randrange(0, 3))],
                                            path_attr=attrs, nlri=[nl() for _ in range(rng.randrange(0, 3))])
    elif kind == "notify":
        pkt = BGPHeader(type=3) / BGPNotification(error_code=rng.randrange(1, 7), error_subcode=rng.randrange(10))
    else:
        pkt = BGPHeader(type=4)
    raw = bytes(pkt)
    t = raw[18]
    text = f"type={t};len={len(raw)}"
    body = raw[19:]
    if t == 1:
        text += f";open={body[0]}/{struct.unpack('>H', body[1:3])[0]}/{struct.unpack('>H', body[3:5])[0]}/{body[5:9].hex()}/0"
    if t == 2:
        def plist(b, a, e):
            out = []
            while a < e:
                n = (b[a] + 7) // 8
                out.append(f"{b[a]}/{b[a + 1:a + 1 + n].hex()}")
                a += 1 + n
            return out
        wl = struct.unpack(">H", body[:2])[0]
        al = struct.unpack(">H", body[2 + wl:4 + wl])[0]
        attrs, at = [], 4 + wl
        while at < 4 + wl + al:
            fl, code = body[at], body[at + 1]
            if fl & 16:
                n, h = struct.unpack(">H", body[at + 2:at + 4])[0], 4
            else:
                n, h = body[at + 2], 3
            attrs.append(f"{fl}/{code}/{body[at + h:at + h + n].hex()}")
            at += h + n
        text += (f";withdrawn={','.join(plist(body, 2, 2 + wl))};attrs={','.join(attrs)};"
                 f"nlri={','.join(plist(body, 4 + wl + al, len(body)))}")
    add("bgp", raw, text)
    add("bgp.rt", raw, raw.hex())

for _ in range(250):
    tl = [EIGRPParam(k1=rng.randrange(256), k2=rng.randrange(256), k3=rng.randrange(256), holdtime=rng.randrange(65536))]
    if rng.random() < 0.5:
        tl.append(EIGRPSwVer())
    raw = bytes(EIGRP(opcode=rng.choice([1, 3, 4, 5]), seq=rng.randrange(1 << 32), ack=rng.randrange(1 << 32),
                      asn=rng.randrange(1 << 16), flags=rng.randrange(16), tlvlist=tl))
    parts, at = [], 20
    while at < len(raw):
        t, n = struct.unpack(">HH", raw[at:at + 4])
        parts.append(f"{t}:{raw[at + 4:at + n].hex()}")
        at += n
    f = struct.unpack(">IIII", raw[4:20])
    add("eigrp", raw, f"ver={raw[0]};opcode={raw[1]};chksum={struct.unpack('>H', raw[2:4])[0]};flags={f[0]};"
                      f"seq={f[1]};ack={f[2]};asn={f[3]};tlvs={'|'.join(parts)}")
    add("eigrp.rt", raw, raw.hex())

for _ in range(250):
    if rng.random() < 0.6:
        opts = [PIMv2HelloHoldtime(holdtime=rng.randrange(65536))]
        if rng.random() < 0.5:
            opts.append(PIMv2HelloDRPriority(dr_priority=rng.randrange(1 << 32)))
        # scapy fills in the checksum only with an IP layer underneath.
        raw = bytes((IP(src=ip4(), dst="224.0.0.13") / PIMv2Hdr(type=0) / PIMv2Hello(option=opts))[PIMv2Hdr])
        olist, at = [], 4
        while at < len(raw):
            t, n = struct.unpack(">HH", raw[at:at + 4])
            olist.append(f"{t}:{raw[at + 4:at + 4 + n].hex()}")
            at += 4 + n
        add("pim", raw, f"version=2;type=0;chksum={struct.unpack('>H', raw[2:4])[0]};body={raw[4:].hex()};"
                        f"opts={'|'.join(olist)}")
    else:
        raw = bytes((IP(src=ip4(), dst="224.0.0.13") / PIMv2Hdr(type=3) / Raw(rb(rng.randrange(4, 30))))[PIMv2Hdr])
        add("pim", raw, f"version=2;type=3;chksum={struct.unpack('>H', raw[2:4])[0]};body={raw[4:].hex()};opts=")
    add("pim.rt", raw, raw.hex())

for _ in range(250):
    tlvs = [ISIS_AreaTlv(areas=["49.%04x" % rng.randrange(65536)])]
    if rng.random() < 0.5:
        tlvs.append(ISIS_GenericTlv(type=rng.choice([129, 132, 240]), val=rb(rng.randrange(1, 6))))
    raw = bytes(ISIS_CommonHdr() / ISIS_L1_LAN_Hello(circuittype=rng.randrange(1, 4), sourceid="%04x.%04x.%04x" % tuple(rng.randrange(65536) for _ in range(3)),
                                                     holdingtime=rng.randrange(65536), priority=rng.randrange(128),
                                                     lanid="%04x.%04x.%04x.%02x" % (rng.randrange(65536), rng.randrange(65536), rng.randrange(65536), rng.randrange(256)),
                                                     tlvs=tlvs))
    hl = raw[1]
    add("isis", raw, f"nlpid={raw[0]};hdrlen={hl};pdutype={raw[4] & 31};fixed={raw[8:hl].hex()};"
                     f"tlvs={'|'.join(tlv8(raw, hl, len(raw)))}")
    add("isis.rt", raw, raw.hex())

for _ in range(250):
    if rng.random() < 0.5:
        e = EAP(code=2, id=rng.randrange(256), type=1, identity=("user" + str(rng.randrange(100))).encode())
        raw = bytes(EAPOL(version=rng.choice([1, 2]), type=0) / e)
        eb = raw[4:]
        el = struct.unpack(">H", eb[2:4])[0]
        add("eapol", raw, f"version={raw[0]};type=0;len={struct.unpack('>H', raw[2:4])[0]};"
                          f"eap={eb[0]}/{eb[1]}/{el}/{eb[4]}/{eb[5:el].hex()}")
    else:
        kdata = rb(rng.randrange(0, 30))
        key = EAPOL_KEY(key_descriptor_type=2, key_descriptor_type_version=2, key_type=1,
                        key_ack=rng.randrange(2), has_key_mic=rng.randrange(2), install=rng.randrange(2),
                        secure=rng.randrange(2), key_length=16, key_replay_counter=rng.randrange(1 << 32),
                        key_nonce=rb(32), key_mic=rb(16), key_data=kdata)
        raw = bytes(EAPOL(version=2, type=3) / key)
        kb = raw[4:]
        add("eapol", raw, f"version=2;type=3;len={struct.unpack('>H', raw[2:4])[0]};"
                          f"key={kb[0]}/{struct.unpack('>H', kb[1:3])[0]}/{struct.unpack('>H', kb[3:5])[0]}/"
                          f"{kb[5:13].hex()}/{kb[13:45].hex()}/{kb[77:93].hex()}/{kdata.hex()}")
    add("eapol.rt", raw, raw.hex())

for _ in range(250):
    names = ["TSFT", "Flags", "Rate", "Channel", "dBm_AntSignal", "dBm_AntNoise", "Lock_Quality", "Antenna", "RXFlags", "TXFlags"]
    chosen = [n for n in names if rng.random() < 0.6] or ["Flags"]
    kw = dict(present="+".join(chosen))
    vals = {}
    if "TSFT" in chosen:
        kw["mac_timestamp"] = vals[0] = rng.randrange(1 << 63)
    if "Flags" in chosen:
        kw["Flags"] = vals[1] = rng.randrange(256)
    if "Rate" in chosen:
        kw["Rate"] = vals[2] = rng.randrange(128)
    if "Channel" in chosen:
        kw["ChannelFrequency"] = rng.choice([2412, 2437, 5180, 5745])
        kw["ChannelFlags"] = rng.randrange(65536)
    if "dBm_AntSignal" in chosen:
        kw["dBm_AntSignal"] = rng.randrange(-100, 0)
    if "dBm_AntNoise" in chosen:
        kw["dBm_AntNoise"] = rng.randrange(-100, 0)
    if "Lock_Quality" in chosen:
        kw["Lock_Quality"] = rng.randrange(65536)
    if "Antenna" in chosen:
        kw["Antenna"] = rng.randrange(256)
    if "RXFlags" in chosen:
        kw["RXFlags"] = rng.randrange(65536)
    if "TXFlags" in chosen:
        kw["TXFlags"] = rng.randrange(65536)
    raw = bytes(RadioTap(**kw) / Raw(rb(rng.randrange(0, 20))))
    hl = struct.unpack("<H", raw[2:4])[0]
    present = struct.unpack("<I", raw[4:8])[0]
    order = [("TSFT", 0, "<Q", None), ("Flags", 1, "<B", None), ("Rate", 2, "<B", None), ("Channel", 3, None, None),
             ("dBm_AntSignal", 5, "<b", None), ("dBm_AntNoise", 6, "<b", None), ("Lock_Quality", 7, "<H", None),
             ("Antenna", 11, "<B", None), ("RXFlags", 14, "<H", None), ("TXFlags", 15, "<H", None)]
    fparts = []
    for name, bit, fmt, _ in order:
        if name not in chosen:
            continue
        if name == "Channel":
            v = struct.pack("<HH", kw["ChannelFrequency"], kw["ChannelFlags"])
        else:
            key = "mac_timestamp" if name == "TSFT" else name
            # scapy's Rate is in Mb/s and goes out in 500 kb/s units.
            val = RadioTap().get_field("Rate").i2m(None, kw[key]) if name == "Rate" else kw[key]
            v = struct.pack(fmt, val)
        fparts.append(f"{bit}:{v.hex()}")
    add("radiotap", raw, f"len={hl};present={present};fields={'|'.join(fparts)};frame={raw[hl:].hex()}")
    add("radiotap.rt", raw, raw.hex())

for _ in range(250):
    ext = rng.random() < 0.5
    rtr = rng.random() < 0.2
    err = rng.random() < 0.1
    flags = (4 if ext else 0) | (2 if rtr else 0) | (1 if err else 0)
    ident = rng.randrange(1 << 29) if ext else rng.randrange(1 << 11)
    data = rb(rng.randrange(0, 9))
    raw = bytes(CAN(flags=flags, identifier=ident, length=len(data), data=data))
    add("can", raw, f"ext={1 if ext else 0};rtr={1 if rtr else 0};err={1 if err else 0};id={ident};len={len(data)};data={data.hex()}")
    add("can.rt", raw, raw.hex())

for _ in range(250):
    data = rb(rng.randrange(0, 20))
    h = USBpcap(irpId=rng.randrange(1 << 63), usbd_status=rng.randrange(1 << 31), function=rng.randrange(65536),
                info=rng.randrange(2), bus=rng.randrange(65536), device=rng.randrange(65536),
                endpoint=rng.randrange(256), transfer=rng.choice([0, 1, 3]), dataLength=len(data))
    raw = bytes(h / Raw(data))
    q = USBpcap(raw)
    add("usbpcap", raw, f"hlen={raw[0] | raw[1] << 8};irp={raw[2:10].hex()};status={q.usbd_status};function={q.function};"
                        f"info={q.info};bus={q.bus};device={q.device};endpoint={q.endpoint};transfer={q.transfer};"
                        f"dlen={q.dataLength};data={raw[27:].hex()}")
    add("usbpcap.rt", raw, raw.hex())

for _ in range(250):
    kind = rng.choice(["cmd", "acl", "event"])
    data = rb(rng.randrange(0, 20))
    if kind == "cmd":
        ogf, ocf = rng.randrange(64), rng.randrange(1024)
        raw = bytes(HCI_Hdr(type=1) / HCI_Command_Hdr(ogf=ogf, ocf=ocf) / Raw(data))
        text = f"type=1;ogf={ogf};ocf={ocf};handle=0;pb=0;bc=0;code=0;payload={data.hex()}"
    elif kind == "acl":
        hd, pb, bc = rng.randrange(4096), rng.randrange(4), rng.randrange(4)
        raw = bytes(HCI_Hdr(type=2) / HCI_ACL_Hdr(handle=hd, PB=pb, BC=bc) / Raw(data))
        text = f"type=2;ogf=0;ocf=0;handle={hd};pb={pb};bc={bc};code=0;payload={data.hex()}"
    else:
        code = rng.randrange(256)
        raw = bytes(HCI_Hdr(type=4) / HCI_Event_Hdr(code=code) / Raw(data))
        text = f"type=4;ogf=0;ocf=0;handle=0;pb=0;bc=0;code={code};payload={data.hex()}"
    add("hci", raw, text)
    add("hci.rt", raw, raw.hex())


# --- B16: the enterprise ---------------------------------------------------
for _ in range(250):
    attrs = []
    for _ in range(rng.randrange(0, 4)):
        attrs.append(RadiusAttribute(type=rng.choice([1, 4, 5, 6, 18, 26, 31, 32]), value=rb(rng.randrange(0, 20))))
    pkt = Radius(code=rng.choice([1, 2, 3, 4, 5, 11]), id=rng.randrange(256), authenticator=rb(16), attributes=attrs)
    raw = bytes(pkt)
    parts, at = [], 20
    while at < len(raw):
        parts.append(f"{raw[at]}:{raw[at + 2:at + raw[at + 1]].hex()}")
        at += raw[at + 1]
    add("radius", raw, f"code={raw[0]};id={raw[1]};auth={raw[4:20].hex()};attrs={'|'.join(parts)}")
    add("radius.rt", raw, raw.hex())
    req_auth, secret = rb(16), rb(rng.randrange(1, 20))
    add("radauth", raw, Radius(raw).compute_authenticator(req_auth, secret).hex(), req_auth + secret)
    # User-Password hiding, RFC 2865 5.2, against hashlib's MD5.
    pw = bytes(rng.randrange(1, 256) for _ in range(rng.randrange(1, 40)))
    padded = pw + bytes((16 - len(pw) % 16) % 16)
    out, prev = b"", req_auth
    for k in range(0, len(padded), 16):
        mask = hashlib.md5(secret + prev).digest()
        block = bytes(x ^ y for x, y in zip(padded[k:k + 16], mask))
        out += block
        prev = block
    add("radhide", pw, out.hex(), req_auth + secret)
    add("radreveal", out, pw.hex(), req_auth + secret)

for _ in range(200):
    n = rng.randrange(0, 4)
    pkt = NetflowHeader(version=5) / NetflowHeaderV5(count=n, sysUptime=rng.randrange(1 << 32), unixSecs=rng.randrange(1 << 32),
                                                     flowSequence=rng.randrange(1 << 32))
    for _ in range(n):
        pkt = pkt / NetflowRecordV5(src=ip4(), dst=ip4(), nexthop=ip4(), dpkts=rng.randrange(1 << 32),
                                    dOctets=rng.randrange(1 << 32), srcport=rng.randrange(65536), dstport=rng.randrange(65536),
                                    prot=rng.choice([6, 17, 1]), tcpFlags=rng.randrange(256), src_as=rng.randrange(65536),
                                    dst_as=rng.randrange(65536))
    raw = bytes(pkt)
    recs = []
    for k in range(n):
        r = raw[24 + 48 * k:72 + 48 * k]
        dp, do = struct.unpack(">II", r[16:24])
        sp, dpt = struct.unpack(">HH", r[32:36])
        sa, da = struct.unpack(">HH", r[40:44])
        recs.append(f"{r[0:4].hex()}/{r[4:8].hex()}/{dp}/{do}/{sp}/{dpt}/{r[38]}/{r[37]}/{sa}/{da}")
    h = struct.unpack(">HHIIII", raw[:20])
    add("nf5", raw, f"count={h[1]};uptime={h[2]};secs={h[3]};seq={h[5]};records={'|'.join(recs)}")
    add("nf5.rt", raw, raw.hex())

for _ in range(200):
    fields = [(rng.choice([1, 2, 4, 7, 8, 11, 12]), rng.choice([1, 2, 4])) for _ in range(rng.randrange(1, 5))]
    tmpl = NetflowTemplateV9(templateID=256 + rng.randrange(10), template_fields=[
        NetflowTemplateFieldV9(fieldType=t, fieldLength=l) for t, l in fields])
    pkt = NetflowHeader(version=9) / NetflowHeaderV9(count=2, sysUptime=rng.randrange(1 << 32), unixSecs=rng.randrange(1 << 32),
                                                     packageSequence=rng.randrange(1 << 32), SourceID=rng.randrange(1 << 32)) / \
        NetflowFlowsetV9(templates=[tmpl])
    raw = bytes(pkt)
    width = sum(l for _, l in fields)
    data = rb(width * rng.randrange(1, 4))
    raw += struct.pack(">HH", tmpl.templateID, 4 + len(data)) + data
    sets, at = [], 20
    while at + 4 <= len(raw):
        sid, n = struct.unpack(">HH", raw[at:at + 4])
        content = raw[at + 4:at + n]
        if sid == 0:
            c = 0
            while c + 4 <= len(content):
                tid, cnt = struct.unpack(">HH", content[c:c + 4])
                if tid == 0 and cnt == 0:
                    break
                fl = [f"{struct.unpack('>H', content[c + 4 + 4 * k:c + 6 + 4 * k])[0]}:"
                      f"{struct.unpack('>H', content[c + 6 + 4 * k:c + 8 + 4 * k])[0]}" for k in range(cnt)]
                sets.append(f"T{tid}=" + ",".join(fl))
                c += 4 + 4 * cnt
        else:
            sets.append(f"D{sid}={content.hex()}")
        at += n
    h = struct.unpack(">HHIIII", raw[:20])
    add("nf9", raw, f"count={h[1]};seq={h[4]};source={h[5]};sets={'|'.join(sets)}")
    add("nf9.rt", raw, raw.hex())

def crc16_modbus(b):
    crc = 0xFFFF
    for x in b:
        crc ^= x
        for _ in range(8):
            crc = (crc >> 1) ^ 0xA001 if crc & 1 else crc >> 1
    return crc


for _ in range(250):
    kind = rng.choice(["req", "resp", "write"])
    if kind == "req":
        pdu = ModbusPDU03ReadHoldingRegistersRequest(startAddr=rng.randrange(65536), quantity=rng.randrange(1, 126))
        raw = bytes(ModbusADURequest(transId=rng.randrange(65536), unitId=rng.randrange(256)) / pdu)
    elif kind == "resp":
        pdu = ModbusPDU03ReadHoldingRegistersResponse(registerVal=[rng.randrange(65536) for _ in range(rng.randrange(1, 6))])
        raw = bytes(ModbusADUResponse(transId=rng.randrange(65536), unitId=rng.randrange(256)) / pdu)
    else:
        pdu = ModbusPDU06WriteSingleRegisterRequest(registerAddr=rng.randrange(65536), registerValue=rng.randrange(65536))
        raw = bytes(ModbusADURequest(transId=rng.randrange(65536), unitId=rng.randrange(256)) / pdu)
    t, pr, n = struct.unpack(">HHH", raw[:6])
    add("mbtcp", raw, f"trans={t};proto={pr};unit={raw[6]};fn={raw[7]};data={raw[8:6 + n].hex()}")
    add("mbtcp.rt", raw, raw.hex())
    body = bytes([rng.randrange(1, 248), rng.choice([1, 3, 4, 6, 16])]) + rb(rng.randrange(0, 12))
    crc = crc16_modbus(body)
    frame = body + bytes([crc & 255, crc >> 8])
    add("mbrtu.e", b"", frame.hex(), body)
    add("mbrtu", frame, f"unit={body[0]};fn={body[1]};data={body[2:].hex()}")
    bad = bytearray(frame)
    bad[rng.randrange(len(bad))] ^= 1
    add("mbrtu", bytes(bad), "ERR")


def ber(b, at):
    tag, n = b[at], b[at + 1]
    at += 2
    if n & 0x80:
        k = n & 0x7F
        n = int.from_bytes(b[at:at + k], "big")
        at += k
    return tag, at, at + n


def oid_text(c):
    arcs, acc = [], 0
    for x in c:
        acc = (acc << 7) | (x & 127)
        if x < 128:
            arcs.append(acc)
            acc = 0
    a = 0 if arcs[0] < 40 else 1 if arcs[0] < 80 else 2
    return ".".join(str(v) for v in [a, arcs[0] - 40 * a] + arcs[1:])


VALUES = [lambda: ASN1_INTEGER(rng.randrange(-1000, 1 << 31)), lambda: ASN1_STRING(rb(rng.randrange(0, 10))),
          lambda: ASN1_NULL(0), lambda: ASN1_IPADDRESS(ip4()), lambda: ASN1_COUNTER32(rng.randrange(1 << 32)),
          lambda: ASN1_GAUGE32(rng.randrange(1 << 32)), lambda: ASN1_TIME_TICKS(rng.randrange(1 << 32)),
          lambda: ASN1_COUNTER64(rng.randrange(1 << 63)), lambda: ASN1_OID("1.3.6.1.4.1." + str(rng.randrange(99999)))]
for _ in range(250):
    vbs = [SNMPvarbind(oid=ASN1_OID("1.3.6.1.2.1." + ".".join(str(rng.randrange(300)) for _ in range(rng.randrange(1, 5)))),
                       value=rng.choice(VALUES)()) for _ in range(rng.randrange(0, 4))]
    kind = rng.choice([SNMPget, SNMPnext, SNMPresponse, SNMPset, SNMPbulk])
    if kind is SNMPbulk:
        pdu = SNMPbulk(id=rng.randrange(1 << 31), non_repeaters=rng.randrange(5), max_repetitions=rng.randrange(50), varbindlist=vbs)
    else:
        pdu = kind(id=rng.randrange(1 << 31), varbindlist=vbs)
    raw = bytes(SNMP(version=rng.choice([0, 1]), community=("c" + str(rng.randrange(1000))).encode(), PDU=pdu))
    _, b0, _ = ber(raw, 0)
    _, vs, ve = ber(raw, b0)
    version = int.from_bytes(raw[vs:ve], "big", signed=True)
    _, cs, ce = ber(raw, ve)
    ptag, ps, pe = ber(raw, ce)
    ints = []
    at = ps
    for _ in range(3):
        _, is_, ie = ber(raw, at)
        ints.append(int.from_bytes(raw[is_:ie], "big", signed=True))
        at = ie
    _, ls, le = ber(raw, at)
    out = []
    at = ls
    while at < le:
        _, ss, se = ber(raw, at)
        _, os_, oe = ber(raw, ss)
        vt, vs2, ve2 = ber(raw, oe)
        out.append(f"{oid_text(raw[os_:oe])}/{vt}/{raw[vs2:ve2].hex()}")
        at = se
    add("snmp", raw, f"version={version};community={raw[cs:ce].hex()};pdu={ptag};id={ints[0]};err={ints[1]};"
                     f"idx={ints[2]};vbs={'|'.join(out)}")
    add("snmp.rt", raw, raw.hex())

for n in list(range(0, 70)) + [rng.randrange(0, 300) for _ in range(60)]:
    data = rb(n)
    p = os.path.join(work, "md4in")
    open(p, "wb").write(data)
    digest = subprocess.run(["openssl", "dgst", "-md4", "-binary", "-provider", "legacy", "-provider", "default", p],
                            capture_output=True).stdout
    add("md4", data, digest.hex())


def run(which, picked):
    argv = []
    for op, a, b, _ in picked:
        argv += [op, "x" + a.hex(), "x" + b.hex()]
    lines = []
    for start in range(0, len(argv), 3 * 1000):
        ran = subprocess.run([which] + argv[start:start + 3000], capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


which = [3 if c[0].startswith(THIRD) else 2 if c[0].startswith(SECOND) else 1 for c in cases]
streams = {k: iter(run(h, [c for c, w in zip(cases, which) if w == k]))
           for k, h in [(1, harness), (2, harness2), (3, harness3)]}
got = [next(streams[w], "<missing>") for w in which]

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
if thin or len(families) < 92:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
