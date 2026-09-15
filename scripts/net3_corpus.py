"""Cases for nettest3, with Python's answers.

`build()` returns (argv, expected, labels). Every command is three words:
a name and two hex arguments, each prefixed with `:` so an empty one
survives. Text answers are hex, so a newline in one cannot split a line.
"""
import calendar
import datetime
import email._parseaddr as parseaddr
import email.utils
import ftplib
import io
import itertools
import mailbox
import random
import socket


def h(text):
    return ":" + text.encode("utf-8").hex()


def th(text):
    return text.encode("utf-8").hex()


class Cases:
    def __init__(self):
        self.argv, self.expected, self.labels = [], [], []

    def add(self, what, a, b, want):
        self.argv += [what, h(a), h(b)]
        self.expected.append(want)
        self.labels.append(f"{what} {a!r} {b!r}")


def inet(c, rng):
    fixed = ["1", "1.2", "1.2.3", "1.2.3.4", "0x7f.1", "017.0.0.1",
             "08.1.1.1", "1.2.3.4 ", "1.2.3.4 x", " 1.2.3.4", "1.2.3.4\n",
             "1.2.3.4\t9", "0x", "0x.1", "", "1..2", "1.2.3.4.", "4294967295",
             "4294967296", "0xffffffff", "0x100000000", "1.16777215",
             "1.16777216", "255.255.65535", "256.1", "0X1F",
             "00000000000000000001", "0x00000000000000000001", "1.2.3.4.5",
             "+1", "-1", "1e2", "0xg", "18446744073709551616.1",
             "18446744073709551617", "0x10000000000000000.1",
             "0x1ffffffffffffffff", "1.2.3.4é", "1.2.3.4\x0b",
             "1.2.3.4\x1c", "0001.1.1.1", "1.2.3.0255", ".1.2.3", "010.0.0.1",
             "0xA.0xb.0Xc.0xD", "0.0", "00", "0x0", "0x.0x.0x.0x",
             "255.255.255.255", "256.255.255.255", "1.2.65536"]
    alphabet = "0123456789xX.abf "
    strings = set(fixed)
    for n in range(0, 5):
        for t in itertools.product("01x.9", repeat=n):
            strings.add("".join(t))
    while len(strings) < 9000:
        strings.add("".join(rng.choice(alphabet)
                            for _ in range(rng.randint(1, 14))))
    # Mostly four parts, so the strict reading has something to accept.
    while len(strings) < 14000:
        parts = [rng.choice(["0", "00", "010", "255", "256", "0255", "1",
                             str(rng.randint(0, 300)), "0x1f", ""])
                 for _ in range(rng.choice([3, 4, 4, 4, 5]))]
        strings.add(".".join(parts) + rng.choice(["", "", "", " ", "x", "."]))
    for s in sorted(strings):
        try:
            want = socket.inet_aton(s).hex()
        except OSError:
            want = th("?")
        c.add("aton", s, "", want)
        try:
            want = socket.inet_pton(socket.AF_INET, s).hex()
        except OSError:
            want = th("?")
        c.add("pton", s, "", want)
    for values in [[1, 2, 3, 4], [0, 0, 0, 0], [255, 255, 255, 255], [1, 2, 3],
                   [], [10, 0, 0, 1, 5]]:
        try:
            want = socket.inet_ntoa(bytes(values))
        except OSError:
            want = "?"
        c.add("ntoa", "\x01".join(map(str, values)), "", th(want))


DAY_WORDS = ["Mon,", "Tue", "wed,", "THU", "Fri,", "Sat", "Sunday,", "Fri,09",
             "x,12", ","]
MONTH_WORDS = ["Jan", "feb", "MAR", "April", "may", "June", "jul", "Aug",
               "sept", "Oct", "november", "DEC", "Foo"]
DAY_NUMBERS = ["1", "09", "31", "0", "32", "-1", "1_0", "12,", "x", "+5"]
YEARS = ["2001", "01", "68", "69", "99", "100", "1999,", "0", "-5", "10000",
         "9999", "1", "2_001", ",", "20x1"]
TIMES = ["01:08:47", "1:2", "23:59:59", "24:00:00", "25:99:99", "12.30",
         "12.30.15", "1.2.3.4", "12:30:15,", "12", "a:b", "12::", ":", "12:3_0",
         "-1:00:00", "1:2:3:4"]
ZONES = ["+0000", "-0000", "-0800", "+0530", "EST", "est", "PDT", "GMT", "UT",
         "Z", "XYZ", "", "+05_00", "-5", "+2400", "-2359", "0", "+99", "1e3",
         "+999999", "-12345"]


def parsed(text):
    try:
        r = parseaddr._parsedate_tz(text)
    except Exception:
        return "?", "?", "no"
    if r is None:
        return "?", "?", "no"
    zone = "none" if r[9] is None else str(r[9])
    fields = ",".join(str(x) for x in r[:6]) + "," + zone
    try:
        stamp = str(email.utils.mktime_tz(email.utils.parsedate_tz(text)))
    except Exception:
        stamp = "?"
    try:
        email.utils.parsedate_to_datetime(text)
        check = "ok"
    except Exception:
        check = "no"
    return fields, stamp, check


def dates(c, rng):
    texts = ["Fri, 09 Nov 2001 01:08:47 -0000", "Fri, 09 Nov 2001 01:08:47 +0000",
             "Mon, 20 Nov 1995 19:12:08 -0500", "20 Nov 1995 19:12:08 EST",
             "Friday, 09-Nov-01 01:08:47 GMT", "Fri,09 Nov 2001 01:08:47 -0800",
             "Nov 09 2001 01:08:47", "09 Nov 01 01.08.47", "Tue, 1 Jul 2003 10:52:37 +0200",
             "Wed, 31 Feb 2001 25:99:00 +0000", "", "   ", "garbage",
             "Thu, 01 Jan 1970 00:00:00 GMT", "Sat, 1 Jan 0001 00:00:00 +0100",
             "Fri, 31 Dec 9999 23:59:59 -0100", "1 Jan 2000 00:00:00+0100",
             "1 Jan 2000 00:00:00-0100", "1 Jan 2000 +0100 00:00:00",
             "2000 Jan 1 00:00", "Jan 1, 2000 00:00:00", "1 Jan 2000, 00:00:00,",
             "1\tJan\x0b2000\x1c00:00", "1 Jan 2000 00:00 est", "01-Jan-2000 00:00 -",
             "Mon, 1 Jan 2000"]
    seen = set()
    for t in texts:
        seen.add(t)
    while len(seen) < 6000:
        seen.add("%s%s %d %s %d %02d:%02d:%02d %s" % (
            rng.choice(["", "Mon, ", "tue,", "Wed "]), "", rng.randint(0, 32),
            rng.choice(MONTH_WORDS[:12]), rng.choice([rng.randint(0, 99),
                                                      rng.randint(1900, 2100)]),
            rng.randint(0, 25), rng.randint(0, 61), rng.randint(0, 61),
            rng.choice(ZONES)))
    while len(seen) < 18000:
        words = []
        if rng.random() < 0.5:
            words.append(rng.choice(DAY_WORDS))
        body = [rng.choice(DAY_NUMBERS), rng.choice(MONTH_WORDS),
                rng.choice(YEARS), rng.choice(TIMES), rng.choice(ZONES)]
        if rng.random() < 0.2:
            rng.shuffle(body)
        if rng.random() < 0.1:
            body = ["-".join(body[:3])] + body[3:]
        words += body[:rng.randint(2, 5)]
        sep = rng.choice([" ", "  ", "\t", " \n "])
        seen.add(sep.join(w for w in words))
    for t in sorted(seen):
        fields, stamp, check = parsed(t)
        c.add("dparse", t, "", fields)
        c.add("dstamp", t, "", stamp)
        c.add("dcheck", t, "", check)
    moments = [0, 1, -1, 86399, 86400, 951782400, 1004396927, 2147483647,
               -2208988800, -62135596800, -62135596801, 253402300799,
               253402300800]
    moments += [rng.randint(-62135596800, 253402300799) for _ in range(400)]
    for s in moments:
        for gmt in (False, True):
            try:
                want = email.utils.formatdate(s, usegmt=gmt)
            except Exception:
                want = "?"
            c.add("dfmt", str(s), "gmt" if gmt else "plain", th(want))
        for off in [0, 3600, -18000, 19800, 86399, -86399, 86400, 45, -3661,
                    rng.randint(-86399, 86399)]:
            try:
                dt = datetime.datetime.fromtimestamp(
                    s, datetime.timezone(datetime.timedelta(seconds=off)))
                want = email.utils.format_datetime(dt)
            except Exception:
                want = "?"
            c.add("dfmtat", str(s), str(off), th(want))


def maildirs(c, rng):
    infos = ["", "2,", "2,S", "1,abc", "2,SRF", "xyz", "2,TPD", "2,SS", "2,a2,",
             "2,FRS", ":2,S"]
    flags = ["", "S", "R", "SR", "SSR", "T", "FPT", "z", "2,", "DFPRST"]
    for n in range(0, 4):
        for t in itertools.product("DFRST,2", repeat=n):
            infos.append("2," + "".join(t))
            flags.append("".join(t))
    infos = sorted(set(infos))
    flags = sorted(set(flags))
    for f in flags:
        m = mailbox.MaildirMessage()
        m.set_flags(f)
        c.add("mset", f, "", th(m.get_info()))
    for info in infos:
        for f in rng.sample(flags, 6):
            m = mailbox.MaildirMessage()
            m.set_info(info)
            m.add_flag(f)
            c.add("madd", info, f, th(m.get_info()))
            m = mailbox.MaildirMessage()
            m.set_info(info)
            m.remove_flag(f)
            c.add("mrem", info, f, th(m.get_info()))
        for subdir in ("new", "cur"):
            m = mailbox.MaildirMessage()
            m.set_info(info)
            m.set_subdir(subdir)
            box = mailbox.mboxMessage(m)
            present = "yes" if ("Status" in box or "X-Status" in box) else "no"
            c.add("mtombox", info, subdir, th(box.get("Status", "") + "\x01" +
                                              box.get("X-Status", "") + "\x01" +
                                              present))
        mh = mailbox.MHMessage(mailbox.MaildirMessage())
        m = mailbox.MaildirMessage()
        m.set_info(info)
        c.add("mtomh", info, "", th(",".join(mailbox.MHMessage(m).get_sequences())))
        c.add("mtobabyl", info, "", th(",".join(mailbox.BabylMessage(m).get_labels())))
    statuses = ["", "R", "O", "RO", "OR", "x", "DFA"]
    for status in statuses:
        for xstatus in ["", "D", "F", "A", "DFA", "R", "O", "q"]:
            box = mailbox.mboxMessage()
            if status or rng.random() < 0.5:
                box["Status"] = status
            if xstatus or rng.random() < 0.5:
                box["X-Status"] = xstatus
            m = mailbox.MaildirMessage(box)
            c.add("mfrommbox", status, xstatus, th(m.get_subdir() + "\x01" + m.get_info()))
    for n in range(0, 4):
        for seqs in itertools.permutations(["unseen", "replied", "flagged", "other"], n):
            mh = mailbox.MHMessage()
            for s in seqs:
                mh.add_sequence(s)
            m = mailbox.MaildirMessage(mh)
            c.add("mfrommh", "\x01".join(seqs), "", th(m.get_subdir() + "\x01" + m.get_info()))
    names = ["1700000000.M12P34Q5.host:2,RS", "a:b:2,S", "plain", ":2,S", "a:",
             "a!2,S", "", "x::y"]
    for name in names:
        for colon in (":", "!"):
            key = name.split(colon)[0]
            info = name.split(colon)[-1] if colon in name else ""
            c.add("mkey", name, colon, th(key))
            c.add("minfo", name, colon, th(info))
    for s in [0, 1, -1, 86400 * 31 + 5, 1700000000, -62135596800, 253402300799,
              951782400] + [rng.randint(-62135596800, 253402300799) for _ in range(200)]:
        box = mailbox.mboxMessage(mailbox.MaildirMessage())
        m = mailbox.MaildirMessage()
        m.set_date(s)
        box = mailbox.mboxMessage(m)
        c.add("mfrom", str(s), "", th(box.get_from()))
    for host in ["host", "a/b", "c:d", "x/:y", ""]:
        now = 1700000000.25
        uniq = "%s.M%sP%sQ%s.%s" % (int(now), int(now % 1 * 1e6), 4242, 7,
                                    host.replace("/", r"\057").replace(":", r"\072"))
        c.add("muniq", "1700000000,250000,4242,7", host, th(uniq))
    listings = [["new/a:2,S", "cur/b", "cur/.hidden", "new/a:2,R", "cur/c:2,"],
                [], ["cur/x!2,S", "new/x"]]
    for entries in listings:
        for colon in (":", "!"):
            toc = {}
            for entry in entries:
                sub, name = entry.split("/", 1)
                if name.startswith("."):
                    continue
                toc[name.split(colon)[0]] = sub + "/" + name
            c.add("mtable", "\x01".join(entries), colon,
                  th("\x01".join(k + "=" + v for k, v in toc.items())))


class Lines:
    def __init__(self, lines):
        self.lines = list(lines)

    def __call__(self):
        if not self.lines:
            raise EOFError
        return self.lines.pop(0)


def ftp(c, rng):
    streams = ["220 ok\r\n", "220-a\r\n220 b\r\n", "a\rb\nc\r\nd", "\r\n\r\n",
               "x" * 8191 + "\n", "x" * 8192 + "\n", "x" * 8192, "x" * 8193,
               "\n", "", "one\r", "a\r\r\nb"]
    for s in streams:
        f = ftplib.FTP()
        f.file = io.TextIOWrapper(io.BytesIO(s.encode("utf-8")), "utf-8",
                                  None, None)
        out = []
        try:
            while True:
                out.append(f.getline())
        except EOFError:
            want = "\x01".join(out)
        except ftplib.Error:
            want = "?"
        c.add("flines", s, "", th(want))
    replies = [["220 hi"], ["220-hi", "there", "220 done"],
               ["220-hi", "221 no", "220-again", "220 end"], ["220-hi"],
               ["220-hi", "220"], ["2", "x"], ["", "y"], ["123-", "123 x"],
               ["220-a", "220-b", "220 c", "next"]]
    for lines in replies:
        for at in range(0, len(lines) + 1):
            f = ftplib.FTP()
            f.getline = Lines(lines[at:])
            try:
                want = f.getmultiline() + "\x02" + str(at + len(lines[at:]) -
                                                      len(f.getline.lines))
            except EOFError:
                want = "?"
            c.add("freply", "\x01".join(lines), str(at), th(want))
    for r in ["150 ok", "250 ok", "450 busy", "550 no", "650", "", "x", "3"]:
        f = ftplib.FTP()
        f.getline = Lines([r])
        try:
            f.getresp()
            kind = "ok"
        except ftplib.error_temp:
            kind = "temp"
        except ftplib.error_perm:
            kind = "perm"
        except (ftplib.error_proto, EOFError):
            kind = "proto"
        c.add("fclass", r, "", kind)
    parts150 = ["150 ", "150", "151 ", "150 Opening (12 bytes)", " (", "1234",
                " BYTES)", " bytes)", "x", "\n", "(9 bytes)", " (0",
                "0 bytes)", " (12 bytes) and (34 bytes)", "\r"]
    seen = set()
    while len(seen) < 3000:
        seen.add("".join(rng.choice(parts150) for _ in range(rng.randint(1, 6))))
    for r in sorted(seen | {"150 Opening BINARY mode (1234 bytes)",
                            "150 x (1 bytes) (2 bytes)"}):
        try:
            v = ftplib.parse150(r)
            want = "none" if v is None else str(v)
        except ftplib.Error:
            want = "?"
        c.add("f150", r, "", want)
    parts227 = ["227 ", "227", "228 ", "Entering (", "192,168,1,2,19,137",
                ")", "1,2", ",", "3", "x", "\n", "007", "1,2,3,4,5"]
    seen = set()
    while len(seen) < 3000:
        seen.add("".join(rng.choice(parts227) for _ in range(rng.randint(1, 6))))
    while len(seen) < 5000:
        numbers = [str(rng.choice([0, 1, 10, 127, 192, 255, 256, 999,
                                   rng.randint(0, 300)])).zfill(rng.choice([1, 1, 3]))
                   for _ in range(rng.choice([5, 6, 6, 6, 7]))]
        seen.add(rng.choice(["227 Entering Passive Mode (", "227 =", "227 ",
                             "226 ("]) + rng.choice([",", ", "]).join(numbers) +
                 rng.choice([")", ").", "", ",x"]))
    for r in sorted(seen):
        try:
            host, port = ftplib.parse227(r)
            want = host + "\x01" + str(port)
        except ftplib.Error:
            want = "?"
        c.add("f227", r, "", th(want))
    parts229 = ["229 ", "229", "(", ")", "|", "|||", "6446", "!", "x", " ",
                "-5", "1_0", "Entering Extended Passive Mode "]
    seen = set()
    while len(seen) < 3000:
        seen.add("".join(rng.choice(parts229) for _ in range(rng.randint(1, 7))))
    while len(seen) < 6000:
        d = rng.choice("|!#||")
        e = d if rng.random() < 0.9 else rng.choice("|!#")
        middle = d * rng.choice([2, 3, 3, 3, 4]) + rng.choice(
            ["6446", "0", "65535", " 21 ", "-5", "1_0", "", "x", "99999"]) + e
        seen.add(rng.choice(["229 ", "229", "229 Entering (", "229 ("]) +
                 rng.choice(["(", "", "(("]) + middle +
                 rng.choice([")", "", "))", ") x"]))
    for r in sorted(seen | {"229 Entering Extended Passive Mode (|||6446|)"}):
        try:
            host, port = ftplib.parse229(r, ("10.0.0.1", 21))
            want = host + "\x01" + str(port)
        except (ftplib.Error, ValueError, IndexError):
            want = "?"
        c.add("f229", r, "10.0.0.1", th(want))
    parts257 = ["257 ", "257", "258 ", "\"", "\"\"", "/usr", " created", "x"]
    seen = set()
    while len(seen) < 2000:
        seen.add("".join(rng.choice(parts257) for _ in range(rng.randint(1, 6))))
    for r in sorted(seen):
        try:
            want = ftplib.parse257(r)
        except ftplib.Error:
            want = "?"
        c.add("f257", r, "", th(want))
    for r in ["213 1234", "213  99 ", "213", "213 x", "550 no", "2131", "213 -1",
              "213 1_000", "213\t5\r"]:
        try:
            s = r[3:].strip()
            want = str(int(s)) if r[:3] == "213" else "none"
        except ValueError:
            want = "?"
        c.add("f213", r, "", want)
    mlsd = ["type=file;size=12;modify=20200101000000; a b.txt",
            "Type=dir;Perm=el; sub", "type=file;type=dir; x", "nofacts",
            "size=1; name\r\n", " leading", "a=1;b; c", "=v;; x", ";", "",
            "type=file;size=12 no-trailing-semicolon", "a=1=2; x\n\r"]
    for line in mlsd:
        f = ftplib.FTP()
        f.retrlines = lambda cmd, cb, line=line: cb(line)
        f.sendcmd = lambda cmd: "200"
        name, entry = next(f.mlsd())
        c.add("fmlsd", line, "", th(name + "\x02" + "\x01".join(
            k + "=" + v for k, v in entry.items())))
    for host, port in [("192.168.1.2", 5001), ("10.0.0.1", 0), ("1.2.3.4", 65535),
                       ("h", -1), ("a.b", 257)]:
        f = ftplib.FTP()
        sent = []
        f.voidcmd = lambda cmd: sent.append(cmd)
        f.sendport(host, port)
        c.add("fport", host, str(port), th(sent[0]))
        for af, family in [(socket.AF_INET, 1), (socket.AF_INET6, 2)]:
            f.af = af
            sent.clear()
            f.sendeprt(host, port)
            c.add("feprt", host, f"{family},{port}", th(sent[0]))
    for cmd in ["USER x", "RETR a\r\nDELE b", "LIST\n", "NOOP\r", ""]:
        f = ftplib.FTP()
        out = []

        class Sock:
            def sendall(self, data):
                out.append(data.decode())
        f.sock = Sock()
        try:
            f.putline(cmd)
            want = out[0]
        except ValueError:
            want = "?"
        c.add("fcmd", cmd, "", th(want))
    for line in ["PASS secret\r\n", "pass x", "PASS \r\n", "USER bob", "Pass no",
                 "PASS a\n\r\n"]:
        f = ftplib.FTP()
        c.add("fmask", line, "", th(eval(f.sanitize(line))))


def services(c, rng):
    # Each lookup scans the file in the C library, about 3 ms, so every name
    # is asked for with and without a protocol, the rarer protocols for a
    # sample of names, and every port that appears in the file plus the
    # well-known range and a sample of the rest.
    names = set()
    ports = set(range(0, 1100))
    for line in open("/etc/services", encoding="utf-8", errors="replace"):
        body = line.split("#")[0].split()
        names.update(body[:1] + body[2:])
        if len(body) > 1 and body[1].split("/")[0].isdigit():
            ports.add(int(body[1].split("/")[0]))
    ports.update(rng.sample(range(1100, 65536), 1500))
    names.update(["HTTP", "nonexistent", "", "80", "http/tcp"])
    names = sorted(names)
    rare = set(rng.sample(names, 300)) | {"rtmp", "nbp", "HTTP", "http"}
    for name in names:
        # The C library caches a found entry by name, and a later lookup of
        # the same name under a protocol other than tcp or udp is answered
        # from that cache whatever the protocol — `4talk` over `ddp` is 3284
        # once `4talk` over `udp` has been found, and not found before. So
        # the rare protocols are asked first, while the answer still comes
        # from the file.
        protos = (["ddp", "sctp", "TCP"] if name in rare else []) + ["", "tcp", "udp"]
        for proto in protos:
            try:
                args = (name, proto) if proto else (name,)
                want = str(socket.getservbyname(*args)) if name else "?"
            except OSError:
                want = "?"
            c.add("svcname", name, proto, want)
    for port in sorted(ports):
        for proto in ["", "tcp", "udp"]:
            try:
                want = socket.getservbyport(*((port, proto) if proto else (port,)))
            except OSError:
                want = "?"
            c.add("svcport", str(port), proto, th(want))
    protos = set()
    for line in open("/etc/protocols"):
        body = line.split("#")[0].split()
        protos.update(body[:1] + body[2:])
    for name in sorted(protos | {"TCPX", "nothing"}):
        try:
            want = str(socket.getprotobyname(name))
        except OSError:
            want = "?"
        c.add("protoname", name, "", want)


def build():
    rng = random.Random(3650)
    c = Cases()
    inet(c, rng)
    dates(c, rng)
    maildirs(c, rng)
    ftp(c, rng)
    services(c, rng)
    return c.argv, c.expected, c.labels
