"""pcapng, 802.11, 802.15.4, Bluetooth LE and IKEv2.

- `pcapng`: random files (one or two sections, either byte order, several
  interfaces with their own resolutions and offsets, Enhanced, Simple and
  obsolete packet blocks, name, statistics and custom blocks) read by
  tshark; every packet's interface, time, lengths, comments, direction,
  interface name and description and bytes must match. Then the file
  `write` makes of what `read` read is given to tshark, which must read
  it the same.
- `dot11`: scapy builds management, control and data frames from random
  fields, QoS and HT control included; the header, the management fixed
  fields and elements, and the frame check sequence must come back.
- `dot15d4`: scapy builds 2003 and 2006 frames of every type, with and
  without security; tshark reads 2015 frames, whose addressing follows the
  PAN ID compression table.
- `btle`: tshark reads advertising PDUs of every type and data PDUs; the
  CRC-24 is held to scapy's for random seeds.
- `ikev2`: scapy builds messages from random chains of SA, KE, Nonce,
  Notify, ID, AUTH, CERTREQ, Vendor ID, Delete, traffic selector, CP and
  Encrypted payloads.

Every family also re-encodes what it decoded and must get the same bytes.
"""
import ipaddress
import logging
import os
import random
import socket
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

logging.getLogger("scapy").setLevel(logging.ERROR)
from scapy.all import Raw  # noqa: E402
from scapy.layers.dot11 import (Dot11, Dot11FCS, Dot11QoS, Dot11Elt, Dot11Beacon, Dot11ProbeResp,  # noqa: E402
                                Dot11ProbeReq, Dot11AssoReq, Dot11AssoResp, Dot11ReassoReq, Dot11ReassoResp,
                                Dot11Disas, Dot11Deauth, Dot11Auth, Dot11Action)
from scapy.layers.dot15d4 import (Dot15d4FCS, Dot15d4Data, Dot15d4Cmd, Dot15d4Beacon,  # noqa: E402
                                  Dot15d4AuxSecurityHeader)
from scapy.layers.bluetooth4LE import BTLE  # noqa: E402
from scapy.contrib import ikev2 as ike  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="packet4")
harness = os.path.join(work, "packettest4")
built = subprocess.run([GRASM, "build", "packages/packettest4/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


def word(n):
    return "".join(rng.choice("abcdefghijklmnopqrstuvwxyz0123456789 ") for _ in range(n)).strip() or "x"


def mac():
    return ":".join(f"{rng.randrange(256):02x}" for _ in range(6))


def ip4():
    return ".".join(str(rng.randrange(256)) for _ in range(4))


def ip6():
    raw = rb(16)
    if rng.random() < 0.6:
        k = rng.randrange(8)
        n = rng.randrange(1, 8 - k + 1)
        raw = raw[:2 * k] + bytes(2 * n) + raw[2 * (k + n):]
    return str(ipaddress.IPv6Address(raw))


cases = []   # (op, a, b, want): want is a string or a function of the answer giving None or a reason


def add(op, a, want, b=b""):
    cases.append((op, a, b, want))


def tshark(path, fields):
    ran = subprocess.run(["tshark", "-r", path, "-T", "fields", "-E", "separator=|"] + sum([["-e", f] for f in fields], []),
                         capture_output=True, text=True)
    return ran.stdout.split("\n")[:-1]


# --- pcapng --------------------------------------------------------------
PCAPNG_FIELDS = ["frame.interface_id", "frame.time_epoch", "frame.len", "frame.cap_len", "frame.comment", "frame.p2p_dir",
                 "frame.interface_name", "frame.interface_description", "data.data"]


def pcapng_file():
    out = b""
    for _ in range(rng.choice([1, 1, 2])):
        E = rng.choice("<>")

        def opt(code, val):
            return struct.pack(E + "HH", code, len(val)) + val + bytes((4 - len(val) % 4) % 4)

        def opts(items):
            return b"".join(opt(c, v) for c, v in items) + (opt(0, b"") if items and rng.random() < 0.8 else b"")

        def block(t, body):
            n = 12 + len(body)
            return struct.pack(E + "II", t, n) + body + struct.pack(E + "I", n)

        def pad(b):
            return b + bytes((4 - len(b) % 4) % 4)

        shb_opts = [(1, word(rng.randrange(1, 20)).encode()) for _ in range(rng.randrange(0, 2))]
        if rng.random() < 0.5:
            shb_opts.append((3, b"Darwin"))
        out += block(0x0A0D0D0A, struct.pack(E + "IHHq", 0x1A2B3C4D, 1, 0, -1) + opts(shb_opts))
        snaps = []
        for _ in range(rng.randrange(1, 4)):
            snap = rng.choice([0, 65535, 262144, rng.randrange(1, 120)])
            snaps.append(snap)
            items = []
            if rng.random() < 0.6:
                items.append((2, word(rng.randrange(1, 12)).encode()))
            if rng.random() < 0.5:
                items.append((3, word(rng.randrange(1, 30)).encode()))
            r = rng.random()
            if r < 0.4:
                items.append((9, bytes([rng.randrange(0, 10)])))
            elif r < 0.7:
                items.append((9, bytes([128 + rng.randrange(0, 31)])))
            if rng.random() < 0.3:
                items.append((14, struct.pack(E + "q", rng.randrange(0, 10 ** 7))))
            out += block(1, struct.pack(E + "HHI", 147, 0, snap) + opts(items))
        for _ in range(rng.randrange(0, 9)):
            # tshark lists custom blocks among the frames; the package keeps them apart.
            kind = rng.choice(["epb", "epb", "epb", "spb", "opb", "nrb", "isb"])
            if kind in ("epb", "opb"):
                i = rng.randrange(len(snaps))
                cap = rng.randrange(0, 80)
                if snaps[i]:
                    cap = min(cap, snaps[i])
                orig = cap + rng.choice([0, 0, rng.randrange(1, 50)])
                ts = rng.choice([rng.randrange(1 << 32), rng.randrange(1 << 45), rng.randrange(1 << 52)])
                items = [(1, word(rng.randrange(1, 15)).encode()) for _ in range(rng.randrange(0, 3))]
                if rng.random() < 0.5:
                    items.append((2, struct.pack(E + "I", rng.randrange(4) + 4 * rng.randrange(4))))
                if kind == "epb":
                    head = struct.pack(E + "IIIII", i, ts >> 32, ts & 0xFFFFFFFF, cap, orig)
                    out += block(6, head + pad(rb(cap)) + opts(items))
                else:
                    head = struct.pack(E + "HHIIII", i, rng.randrange(100), ts >> 32, ts & 0xFFFFFFFF, cap, orig)
                    out += block(2, head + pad(rb(cap)) + opts([o for o in items if o[0] == 1]))
            elif kind == "spb":
                orig = rng.randrange(0, 60)
                cap = min(orig, snaps[0]) if snaps[0] else orig
                out += block(3, struct.pack(E + "I", orig) + pad(rb(cap)))
            elif kind == "nrb":
                recs = b""
                for _ in range(rng.randrange(0, 3)):
                    if rng.random() < 0.5:
                        v = socket.inet_aton(ip4()) + b"".join(word(6).encode() + b"\0" for _ in range(rng.randrange(1, 3)))
                        recs += struct.pack(E + "HH", 1, len(v)) + pad(v)
                    else:
                        v = rb(16) + word(8).encode() + b"\0"
                        recs += struct.pack(E + "HH", 2, len(v)) + pad(v)
                out += block(4, recs + struct.pack(E + "HH", 0, 0) + opts([(1, b"names")] if rng.random() < 0.3 else []))
            elif kind == "isb":
                ts = rng.randrange(1 << 40)
                items = [(4, struct.pack(E + "Q", rng.randrange(1000)))] if rng.random() < 0.5 else []
                out += block(5, struct.pack(E + "III", rng.randrange(len(snaps)), ts >> 32, ts & 0xFFFFFFFF) + opts(items))
            else:
                out += block(0x00000BAD, struct.pack(E + "I", 32473) + pad(rb(rng.randrange(0, 20))))
    return out


def check_pcapng_rt(want):
    def check(got):
        if got == "ERR":
            return "refused"
        path = os.path.join(work, f"rt{rng.random()}.pcapng")
        open(path, "wb").write(bytes.fromhex(got))
        lines = "#".join(tshark(path, PCAPNG_FIELDS))
        return None if lines == want else f"tshark reads the rewritten file as {lines[:200]}"
    return check


files = [pcapng_file() for _ in range(220)]
paths = []
for k, data in enumerate(files):
    p = os.path.join(work, f"f{k}.pcapng")
    open(p, "wb").write(data)
    paths.append(p)
with ThreadPoolExecutor(8) as pool:
    readings = list(pool.map(lambda p: "#".join(tshark(p, PCAPNG_FIELDS)), paths))
for data, want in zip(files, readings):
    add("pcapng", data, want)
    add("pcapng.rt", data, check_pcapng_rt(want))

# --- 802.11 --------------------------------------------------------------
MGMT_FIXED = {8: 12, 5: 12, 0: 4, 2: 10, 1: 6, 3: 6, 11: 6, 10: 2, 12: 2, 13: 1, 14: 1}


def elements():
    els = []
    for _ in range(rng.randrange(0, 5)):
        eid = rng.choice([0, 1, 3, 50, 5, 48, 221, rng.randrange(256)])
        info = word(rng.randrange(0, 20)).encode() if eid == 0 else bytes([rng.randrange(1, 15)]) if eid == 3 else \
            rb(rng.randrange(0, 12))
        els.append((eid, info))
    return els


def mgmt_body(sub):
    """scapy's bytes for a management body and the fields it holds."""
    f = dict(ts=-1, iv=-1, cap=-1, li=-1, ap="", st=-1, aid=-1, rs=-1, al=-1, tr=-1, cat=-1)
    if sub in (8, 5):
        f["ts"], f["iv"], f["cap"] = rng.randrange(1 << 62), rng.randrange(65536), rng.randrange(65536)
        layer = (Dot11Beacon if sub == 8 else Dot11ProbeResp)(timestamp=f["ts"], beacon_interval=f["iv"], cap=f["cap"])
    elif sub == 0:
        f["cap"], f["li"] = rng.randrange(65536), rng.randrange(65536)
        layer = Dot11AssoReq(cap=f["cap"], listen_interval=f["li"])
    elif sub == 2:
        f["cap"], f["li"], f["ap"] = rng.randrange(65536), rng.randrange(65536), mac()
        layer = Dot11ReassoReq(cap=f["cap"], listen_interval=f["li"], current_AP=f["ap"])
    elif sub in (1, 3):
        f["cap"], f["st"], f["aid"] = rng.randrange(65536), rng.randrange(65536), rng.randrange(65536)
        layer = (Dot11AssoResp if sub == 1 else Dot11ReassoResp)(cap=f["cap"], status=f["st"], AID=f["aid"])
    elif sub == 11:
        f["al"], f["tr"], f["st"] = rng.randrange(65536), rng.randrange(65536), rng.randrange(65536)
        layer = Dot11Auth(algo=f["al"], seqnum=f["tr"], status=f["st"])
    elif sub in (10, 12):
        f["rs"] = rng.randrange(65536)
        layer = (Dot11Disas if sub == 10 else Dot11Deauth)(reason=f["rs"])
    elif sub == 13:
        f["cat"] = rng.randrange(256)
        layer = Dot11Action(category=f["cat"])
    elif sub == 14:
        f["cat"] = rng.randrange(256)
        layer = Raw(bytes([f["cat"]]))
    else:
        layer = Dot11ProbeReq() if sub == 4 else None
    els, rest = [], b""
    if sub in (10, 12, 13, 14):
        rest = rb(rng.randrange(0, 10))
        body = bytes(layer) + rest
    else:
        els = elements()
        body = (bytes(layer) if layer is not None else b"") + b"".join(bytes(Dot11Elt(ID=i, info=v)) for i, v in els)
    ssid = next((v.decode() for i, v in els if i == 0), "-")
    ch = next((v[0] for i, v in els if i == 3 and len(v) >= 1), -1)
    rates = [b % 128 for i, v in els if i in (1, 50) for b in v]
    # scapy's `cap` is a 16-bit flags field, written big-endian; the field is little-endian.
    cap = -1 if f["cap"] < 0 else int.from_bytes(f["cap"].to_bytes(2, "big"), "little")
    want = (f"ts={f['ts']};iv={f['iv']};cap={cap};li={f['li']};ap={f['ap']};st={f['st']};aid={f['aid']};rs={f['rs']};"
            f"al={f['al']};tr={f['tr']};cat={f['cat']};el={','.join(f'{i}:{v.hex()}' for i, v in els)};rest={rest.hex()};"
            f"ssid={ssid};ch={ch};rates={','.join(map(str, rates))}")
    return body, want


for _ in range(900):
    kind = rng.choice([0, 0, 1, 2, 2])
    sub = rng.choice([0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14]) if kind == 0 else \
        rng.choice([4, 5, 8, 9, 10, 11, 12, 13, 14, 15]) if kind == 1 else rng.randrange(16)
    flags = rng.randrange(256) & ~0x40
    version = rng.choice([0, 0, 0, rng.randrange(4)])
    a1, a2, a3, a4 = mac(), mac(), mac(), mac()
    sc, dur = rng.randrange(65536), rng.randrange(65536)
    hdr = Dot11(type=kind, subtype=sub, proto=version, FCfield=flags, ID=dur, addr1=a1, addr2=a2, addr3=a3, SC=sc, addr4=a4)
    qos = has_qos = kind == 2 and sub >= 8 and sub != 13
    htc = flags & 0x80 and (kind == 0 or has_qos)
    payload = b""
    qv = -1
    if has_qos:
        tid, eosp, ack, amsdu, txop = rng.randrange(16), rng.randrange(2), rng.randrange(4), rng.randrange(2), rng.randrange(256)
        payload += bytes(Dot11QoS(TID=tid, EOSP=eosp, Ack_Policy=ack, A_MSDU_Present=amsdu, TXOP=txop))
        qv = tid + 16 * eosp + 32 * ack + 128 * amsdu + 256 * txop
    hv = -1
    if htc:
        hv = rng.randrange(1 << 32)
        payload += struct.pack("<I", hv)
    mgmt_want = None
    if kind == 0:
        body, mgmt_want = mgmt_body(sub)
    else:
        body = rb(rng.randrange(0, 30))
    raw = bytes(hdr) + payload + body
    two = kind != 1 or sub in (4, 5, 8, 9, 10, 11, 14, 15)
    three = kind != 1
    four = kind == 2 and flags & 3 == 3
    add("dot11", raw, f"v={version};t={kind};s={sub};f={flags};d={dur};a1={a1};a2={a2 if two else ''};a3={a3 if three else ''};"
                      f"a4={a4 if four else ''};sc={sc if three else -1};q={qv};h={hv};body={body.hex()}")
    add("dot11.rt", raw, raw.hex())
    if mgmt_want is not None:
        add("dot11.mgmt", raw, mgmt_want)
    if kind == 2:
        ds = flags & 3
        src, dst, bss = [(a2, a1, a3), (a2, a3, a1), (a3, a1, a2), (a4, a3, "")][ds]
        add("dot11.ds", raw, f"{src}|{dst}|{bss}")
    if rng.random() < 0.3:
        with_fcs = bytes(Dot11FCS(type=kind, subtype=sub, proto=version, FCfield=flags, ID=dur, addr1=a1, addr2=a2,
                                  addr3=a3, SC=sc, addr4=a4) / Raw(payload + body))
        add("dot11.fcs", with_fcs, "True")
        bad = bytearray(with_fcs)
        bad[rng.randrange(len(bad))] ^= 1 << rng.randrange(8)
        add("dot11.fcs", bytes(bad), "False")
        add("dot11.addfcs", with_fcs[:-4], with_fcs.hex())

# --- 802.15.4 ------------------------------------------------------------


def wpan_addr(mode):
    return rng.randrange(65536) if mode == 2 else rng.randrange(1 << 64)


def addr_text(mode, v):
    if mode == 2:
        return f"0x{v:04x}"
    if mode == 3:
        return ":".join(f"{b:02x}" for b in v.to_bytes(8, "big"))
    return ""


for _ in range(700):
    ft = rng.choice([0, 1, 1, 2, 3])
    sec = ft != 2 and rng.random() < 0.4
    # 2003 frames keep their security material in the payload.
    ver = 1 if sec else rng.randrange(2)
    pend, ackr, seq = rng.randrange(2), rng.randrange(2), rng.randrange(256)
    if ft == 2:
        dm, sm, comp = 0, 0, 0
    elif ft == 0:
        dm, sm, comp = 0, rng.choice([2, 3]), 0
    else:
        dm, sm = rng.choice([2, 3]), rng.choice([0, 2, 3])
        comp = rng.randrange(2) if sm else 0
    dpan, span = rng.randrange(65536), rng.randrange(65536)
    da, sa = wpan_addr(dm), wpan_addr(sm)
    fcf = dict(fcf_frametype=ft, fcf_security=sec, fcf_pending=pend, fcf_ackreq=ackr, fcf_panidcompress=comp,
               fcf_destaddrmode=dm, fcf_srcaddrmode=sm, fcf_framever=ver, seqnum=seq)
    lvl = km = 0
    ctr, ks, ki = -1, b"", -1
    aux = {}
    if sec:
        lvl, km, ctr = rng.randrange(8), rng.randrange(4), rng.randrange(1 << 32)
        auxkw = dict(sec_sc_seclevel=lvl, sec_sc_keyidmode=km, sec_framecounter=ctr)
        if km:
            ki = rng.randrange(256)
            auxkw["sec_keyid_keyindex"] = ki
        if km == 2:
            v = rng.randrange(1 << 32)
            auxkw["sec_keyid_keysource"] = v
            ks = v.to_bytes(4, "little")
        if km == 3:
            v = rng.randrange(1 << 64)
            auxkw["sec_keyid_keysource"] = v
            ks = v.to_bytes(8, "little")
        aux = dict(aux_sec_header=Dot15d4AuxSecurityHeader(**auxkw))
    payload = rb(rng.randrange(0, 20))
    cmd = "-"
    beacon_want = None
    if ft == 1:
        pkt = Dot15d4FCS(**fcf) / Dot15d4Data(dest_panid=dpan, dest_addr=da, src_panid=span, src_addr=sa, **aux) / Raw(payload)
    elif ft == 3:
        cid = rng.randrange(1, 256)
        cmd = str(cid)
        pkt = Dot15d4FCS(**fcf) / Dot15d4Cmd(dest_panid=dpan, dest_addr=da, src_panid=span, src_addr=sa, cmd_id=cid, **aux) / Raw(payload)
        payload = bytes([cid]) + payload
    elif ft == 0:
        bo, so, fcap = rng.randrange(16), rng.randrange(16), rng.randrange(16)
        ble, pc, ap, gp = rng.randrange(2), rng.randrange(2), rng.randrange(2), rng.randrange(2)
        shorts = [rng.randrange(65536) for _ in range(rng.randrange(0, 4))]
        longs = [rng.randrange(1 << 64) for _ in range(rng.randrange(0, 3))]
        pkt = Dot15d4FCS(**fcf) / Dot15d4Beacon(src_panid=span, src_addr=sa, sf_beaconorder=bo, sf_sforder=so, sf_finalcapslot=fcap,
                                                sf_battlifeextend=ble, sf_pancoord=pc, sf_assocpermit=ap, gts_spec_permit=gp,
                                                gts_spec_desccount=0, pa_num_short=len(shorts), pa_num_long=len(longs),
                                                pa_short_addresses=shorts, pa_long_addresses=longs, **aux) / Raw(payload)
        raw_wo = bytes(pkt)
        beacon_want = (f"bo={bo};so={so};fcs={fcap};ble={ble};pc={pc};ap={ap};gp={gp};ps={','.join(addr_text(2, x) for x in shorts)};"
                       f"pl={','.join(addr_text(3, x) for x in longs)};payload={payload.hex()}")
        sf = struct.pack("<H", bo + 16 * so + 256 * fcap + 4096 * ble + 16384 * pc + 32768 * ap)
        pend_spec = bytes([len(shorts) + 16 * len(longs)])
        payload = sf + bytes([128 * gp]) + pend_spec + b"".join(struct.pack("<H", x) for x in shorts) + \
            b"".join(struct.pack("<Q", x) for x in longs) + payload
    else:
        pkt = Dot15d4FCS(**fcf) / Raw(payload)
    raw = bytes(pkt)
    dp = dpan if dm else -1
    sp = span if sm and not (comp and dm) else -1
    if ft == 0:
        sp = span
    add("wpan", raw, f"t={ft};sec={int(sec)};pend={pend};ack={ackr};comp={comp};dm={dm};ver={ver};sm={sm};seq={seq};dp={dp};"
                     f"da={addr_text(dm, da)};sp={sp};sa={addr_text(sm, sa)};lvl={lvl};km={km};ctr={ctr};ks={ks.hex()};ki={ki};"
                     f"cmd={cmd};pl={payload.hex()}")
    add("wpan.rt", raw, raw.hex())
    add("wpan.fcs", raw, "True")
    if beacon_want:
        add("wpan.beacon", raw, beacon_want)
    bad = bytearray(raw)
    bad[rng.randrange(len(bad))] ^= 1 << rng.randrange(8)
    add("wpan.fcs", bytes(bad), "False")


def kermit(d):
    c = 0
    for x in d:
        c ^= x
        for _ in range(8):
            c = (c >> 1) ^ 0x8408 if c & 1 else c >> 1
    return c


def pcap(pkts, linktype):
    out = struct.pack("<IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 65535, linktype)
    for p in pkts:
        out += struct.pack("<IIII", 0, 0, len(p), len(p)) + p
    return out


# 2015 frames, built byte by byte and read by tshark.
v2 = []
for _ in range(700):
    ver = rng.choice([2, 2, 2, 0, 1])
    ft = rng.choice([1, 1, 3, 0])
    dm, sm = rng.choice([0, 2, 3]), rng.choice([0, 2, 3])
    if ver < 2 and dm == 0 and sm == 0:
        dm = 2
    comp = rng.randrange(2)
    if ver < 2 and not (dm and sm):
        comp = 0
    sec = ver > 0 and rng.random() < 0.35
    supp = ver == 2 and rng.random() < 0.3
    fc = ft | (sec << 3) | (rng.randrange(2) << 4) | (rng.randrange(2) << 5) | (comp << 6) | (supp << 8) | \
        (dm << 10) | (ver << 12) | (sm << 14)
    out = struct.pack("<H", fc) + (b"" if supp else bytes([rng.randrange(256)]))
    d, s = dm != 0, sm != 0
    if ver < 2:
        dpp, spp = d, s and not (comp and d)
    elif not d and not s:
        dpp, spp = bool(comp), False
    elif d and not s:
        dpp, spp = not comp, False
    elif not d:
        dpp, spp = False, not comp
    elif dm == 3 and sm == 3:
        dpp, spp = not comp, False
    else:
        dpp, spp = True, not comp
    if dpp:
        out += rb(2)
    out += rb({0: 0, 2: 2, 3: 8}[dm])
    if spp:
        out += rb(2)
    out += rb({0: 0, 2: 2, 3: 8}[sm])
    if sec:
        km = rng.randrange(4)
        csupp = ver == 2 and rng.random() < 0.4
        out += bytes([rng.randrange(8) | (km << 3) | (csupp << 5)])
        if not csupp:
            out += rb(4)
        out += rb({0: 0, 1: 0, 2: 4, 3: 8}[km]) + (rb(1) if km else b"")
    out += rb(rng.randrange(0, 12))
    out += struct.pack("<H", kermit(out))
    v2.append(out)
path = os.path.join(work, "wpan.pcap")
open(path, "wb").write(pcap(v2, 195))
WPAN = ["wpan.frame_type", "wpan.seq_no", "wpan.dst_pan", "wpan.dst16", "wpan.dst64", "wpan.src_pan", "wpan.src16", "wpan.src64",
        "wpan.aux_sec.sec_level", "wpan.aux_sec.key_id_mode", "wpan.aux_sec.frame_counter", "wpan.aux_sec.key_index",
        "wpan.aux_sec.frame_counter_suppression", "wpan.version"]
for raw, want in zip(v2, tshark(path, WPAN)):
    add("wpan.ts", raw, want)
    add("wpan.rt", raw, raw.hex())

# --- Bluetooth LE --------------------------------------------------------
ADV_AA = struct.pack("<I", 0x8E89BED6)


def addr():
    return rb(6)


def ad_data():
    # Well-formed values only: tshark misreads a short manufacturer field or
    # an odd-length UUID list and loses its place in what follows.
    out = b""
    for _ in range(rng.randrange(0, 4)):
        kind = rng.choice([1, 2, 3, 8, 9, 10, 22, 255])
        n = {1: 1, 10: 1, 2: 2 * rng.randrange(0, 3), 3: 2 * rng.randrange(1, 3), 22: rng.randrange(2, 6),
             255: rng.randrange(2, 8)}.get(kind, rng.randrange(1, 8))
        val = word(n).encode()[:n].ljust(n, b"a") if kind in (8, 9) else rb(n)
        out += bytes([len(val) + 1, kind]) + val
    return out


adv = []
for _ in range(700):
    k = rng.randrange(7)
    head = k | (rng.randrange(2) << 5) | (rng.randrange(2) << 6) | (rng.randrange(2) << 7)
    if k == 5:
        body = addr() + addr() + rb(4) + rb(3) + rb(1) + rb(8) + rb(5) + bytes([rng.randrange(256)])
    elif k in (1, 3):
        body = addr() + addr()
    else:
        body = addr() + ad_data()
    pdu = bytes([head, len(body)]) + body
    crc = BTLE.compute_crc(pdu)
    if rng.random() < 0.1:
        crc = bytes([crc[0] ^ 1]) + crc[1:]
    adv.append(ADV_AA + pdu + crc)
path = os.path.join(work, "adv.pcap")
open(path, "wb").write(pcap(adv, 251))
BTLE_FIELDS = ["btle.access_address", "btle.advertising_header.pdu_type", "btle.advertising_header.randomized_tx",
               "btle.advertising_header.randomized_rx", "btle.length", "btle.advertising_address", "btle.target_address",
               "btle.initiator_address", "btle.scanning_address", "btle.crc.incorrect",
               "btle.link_layer_data.access_address", "btle.link_layer_data.crc_init", "btle.link_layer_data.window_size",
               "btle.link_layer_data.window_offset", "btle.link_layer_data.interval", "btle.link_layer_data.latency",
               "btle.link_layer_data.timeout", "btle.link_layer_data.channel_map", "btle.link_layer_data.hop",
               "btle.link_layer_data.sleep_clock_accuracy", "btcommon.eir_ad.entry.type", "btcommon.eir_ad.entry.length"]
for raw, line in zip(adv, tshark(path, BTLE_FIELDS)):
    f = line.split("|")
    target = f[6] or f[7]
    want = "|".join(f[:6] + [target, f[8], "1" if f[9] else ""] + f[10:])
    add("btle", raw, want)
    add("btle.rt", raw, raw[4:-3].hex())
data = []
for _ in range(300):
    aa = struct.pack("<I", rng.randrange(1 << 32))
    if aa == ADV_AA:
        continue
    body = rb(rng.randrange(0, 30))
    pdu = bytes([rng.randrange(1, 4) | (rng.randrange(2) << 2) | (rng.randrange(2) << 3) | (rng.randrange(2) << 4), len(body)]) + body
    data.append(aa + pdu + BTLE.compute_crc(pdu, rng.randrange(1 << 24)))
path = os.path.join(work, "data.pcap")
open(path, "wb").write(pcap(data, 251))
for raw, want in zip(data, tshark(path, ["btle.data_header.llid", "btle.data_header.next_expected_sequence_number",
                                         "btle.data_header.sequence_number", "btle.data_header.more_data",
                                         "btle.data_header.length", "btle.access_address"])):
    add("btle", raw, want)
    add("btle.rt", raw, raw[4:-3].hex())
for _ in range(300):
    pdu, init = rb(rng.randrange(0, 40)), rng.randrange(1 << 24)
    swapped = int.from_bytes(init.to_bytes(3, "little"), "big")
    add("btle.crc", pdu, BTLE.compute_crc(pdu, swapped).hex(), init.to_bytes(3, "little"))

# --- IKEv2 ---------------------------------------------------------------


def ike_payload(last):
    flags = rng.choice([0, 0, 0x80])
    crit = f"{1 if flags else 0}/0/"
    k = rng.choice(["sa", "ke", "nonce", "notify", "id", "auth", "certreq", "vendor", "delete", "ts", "cp", "sk"] if last else
                   ["sa", "ke", "nonce", "notify", "id", "auth", "certreq", "vendor", "delete", "ts", "cp"])
    if k == "sa":
        props, texts = None, []
        n = rng.randrange(1, 3)
        for i in range(n):
            spi = rb(rng.choice([0, 4, 8]))
            trans, ttexts = None, []
            for _ in range(rng.randrange(1, 4)):
                tt, tid = rng.randrange(1, 6), rng.randrange(1, 40)
                if tt == 1 and rng.random() < 0.6:
                    kl = rng.choice([128, 192, 256])
                    # scapy writes the attribute only when told the transform's length.
                    t = ike.IKEv2_Transform(transform_type=tt, transform_id=tid, length=12, key_length=kl)
                    ttexts.append(f"{tt}.{tid}.14:{kl}")
                else:
                    t = ike.IKEv2_Transform(transform_type=tt, transform_id=tid)
                    ttexts.append(f"{tt}.{tid}.")
                trans = t if trans is None else trans / t
            proto = rng.choice([1, 2, 3])
            p = ike.IKEv2_Proposal(proposal=i + 1, proto=proto, SPI=spi, trans_nb=len(ttexts), trans=trans)
            props = p if props is None else props / p
            texts.append(f"{i + 1},{proto},{spi.hex()},[{' '.join(ttexts)}]")
        return ike.IKEv2_SA(flags=flags, prop=props), "33/" + crit + " ".join(texts)
    if k == "ke":
        g, d = rng.choice([14, 19, 20, 21, 31]), rb(rng.randrange(1, 40))
        return ike.IKEv2_KE(flags=flags, group=g, ke=d), f"34/{crit}{g}:{d.hex()}"
    if k == "nonce":
        d = rb(rng.randrange(1, 40))
        return ike.IKEv2_Nonce(flags=flags, nonce=d), f"40/{crit}{d.hex()}"
    if k == "notify":
        t, spi, d = rng.choice([14, 24, 16388, 16389, 16390, 16404]), rb(rng.choice([0, 0, 4, 8])), rb(rng.randrange(0, 20))
        p = rng.choice([0, 1, 3])
        return ike.IKEv2_Notify(flags=flags, proto=p, SPIsize=len(spi), SPI=spi, type=t, notify=d), \
            f"41/{crit}{p},{spi.hex()},{t},{d.hex()}"
    if k == "id":
        t = rng.choice([1, 2, 3, 11])
        cls = rng.choice([ike.IKEv2_IDi, ike.IKEv2_IDr])
        code = 35 if cls is ike.IKEv2_IDi else 36
        if t == 1:
            a = ip4()
            return cls(flags=flags, IDtype=1, ID=a), f"{code}/{crit}1:{socket.inet_aton(a).hex()}"
        v = word(rng.randrange(1, 20)).encode()
        return cls(flags=flags, IDtype=t, ID=v), f"{code}/{crit}{t}:{v.hex()}"
    if k == "auth":
        m, d = rng.choice([1, 2, 3, 14]), rb(rng.randrange(1, 40))
        return ike.IKEv2_AUTH(flags=flags, auth_type=m, load=d), f"39/{crit}{m}:{d.hex()}"
    if k == "certreq":
        d = rb(20 * rng.randrange(0, 3))
        return ike.IKEv2_CERTREQ(flags=flags, cert_encoding=4, cert_authority=d), f"38/{crit}4:{d.hex()}"
    if k == "vendor":
        d = rb(rng.randrange(1, 20))
        return ike.IKEv2_VendorID(flags=flags, vendorID=d), f"43/{crit}{d.hex()}"
    if k == "delete":
        spis = [rb(4) for _ in range(rng.randrange(0, 4))]
        return ike.IKEv2_Delete(flags=flags, proto=3, SPIsize=4, SPInum=len(spis), SPI=spis), \
            f"42/{crit}3,4,{'.'.join(s.hex() for s in spis)}"
    if k == "ts":
        sels, texts = [], []
        for _ in range(rng.randrange(1, 3)):
            proto, sp, ep = rng.choice([0, 6, 17]), rng.randrange(65536), rng.randrange(65536)
            if rng.random() < 0.5:
                a, b = ip4(), ip4()
                sels.append(ike.IPv4TrafficSelector(IP_protocol_ID=proto, start_port=sp, end_port=ep,
                                                    starting_address_v4=a, ending_address_v4=b))
                texts.append(f"7,{proto},{sp},{ep},{a},{b}")
            else:
                a, b = ip6(), ip6()
                # scapy's default length for an IPv6 selector is 20; RFC 7296 says 40.
                sels.append(ike.IPv6TrafficSelector(IP_protocol_ID=proto, length=40, start_port=sp, end_port=ep,
                                                    starting_address_v6=a, ending_address_v6=b))
                texts.append(f"8,{proto},{sp},{ep},{a},{b}")
        cls = rng.choice([ike.IKEv2_TSi, ike.IKEv2_TSr])
        code = 44 if cls is ike.IKEv2_TSi else 45
        return cls(flags=flags, number_of_TSs=len(sels), traffic_selector=sels), f"{code}/{crit}{' '.join(texts)}"
    if k == "cp":
        attrs, texts = [], []
        for _ in range(rng.randrange(0, 3)):
            if rng.random() < 0.5:
                a = ip4()
                attrs.append(ike.ConfigurationAttribute(type=1, length=4, value=a))
                texts.append(f"1={socket.inet_aton(a).hex()}")
            else:
                attrs.append(ike.ConfigurationAttribute(type=rng.choice([3, 8]), length=0))
                texts.append(f"{attrs[-1].type}=")
        cfg = rng.choice([1, 2])
        return ike.IKEv2_CP(flags=flags, CFGType=cfg, attributes=attrs), f"47/{crit}{cfg}:{','.join(texts)}"
    d = rb(rng.randrange(8, 60))
    inside = rng.choice([35, 39, 33, 41])
    return ike.IKEv2_Encrypted(flags=flags, next_payload=inside, load=d), (f"46/{crit}{d.hex()}", inside)


for _ in range(700):
    spi_i, spi_r = rb(8), rb(8)
    x, fl, mid = rng.choice([34, 35, 36, 37, rng.randrange(256)]), rng.randrange(256), rng.randrange(1 << 31)
    msg = ike.IKEv2(init_SPI=spi_i, resp_SPI=spi_r, exch_type=x, flags=fl, id=mid)
    texts, kinds, inside = [], [], 0
    n = rng.randrange(0, 6)
    for i in range(n):
        layer, text = ike_payload(i == n - 1)
        if isinstance(text, tuple):
            text, inside = text
        msg = msg / layer
        texts.append(text)
        kinds.append(int(text.split("/")[0]))
    raw = bytes(msg)
    want = (f"spii={spi_i.hex()};spir={spi_r.hex()};np={kinds[0] if kinds else 0};v=2.0;x={x};f={fl};id={mid};len={len(raw)};"
            f"i={1 if fl & 8 else 0};r={1 if fl & 32 else 0};inside={inside};" + ";".join(texts))
    add("ike", raw, want)
    add("ike.rt", raw, raw.hex())

# --- run -----------------------------------------------------------------
lines = []
batch, size = [], 0


def flush():
    global batch, size
    if not batch:
        return
    argv = []
    for op, a, b, _ in batch:
        argv += [op, "x" + a.hex(), "x" + b.hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=300)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines.extend(ran.stdout.split("\n")[:-1])
    batch, size = [], 0


for c in cases:
    batch.append(c)
    size += 2 * (len(c[1]) + len(c[2])) + 20
    if size > 200000 or len(batch) >= 1500:
        flush()
flush()

families = {}
agree = differ = 0
only = os.environ.get("ONLY", "")
for i, (op, a, b, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    if callable(want):
        why = want(mine)
    elif mine == want:
        why = None
    else:
        at = next((k for k in range(min(len(mine), len(want))) if mine[k] != want[k]), min(len(mine), len(want)))
        why = f"from {at}: want {want[max(0, at - 60):at + 120]!r}\n    got       {mine[max(0, at - 60):at + 120]!r}"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 10 or (only and fam[1] <= 10)):
            print(f"  {op} {a.hex()[:100]}")
            print(f"    {why[:300]}")
            if callable(want):
                print(f"    grasm {mine[:300]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 17:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
