"""Audio and video containers and codecs against FFmpeg (and Apple's
afconvert where it reads or writes the format).

Every case starts from media FFmpeg (or afconvert) writes from fresh random
signals and parameters, or from bytes the harness writes for FFmpeg to read:

- G.711: every 16-bit sample encoded and every code decoded, against
  FFmpeg's pcm_mulaw and pcm_alaw.
- IMA ADPCM: WAV blocks and QuickTime packets FFmpeg and afconvert encode,
  decoded sample for sample; streams the harness encodes, decoded by FFmpeg
  (and afconvert) to exactly the samples the harness predicts.
- .au, AIFF/AIFF-C and CAF in every encoding the writers offer, decoded
  against FFmpeg's decoders; files the harness writes, read back by FFmpeg.
- YUV4MPEG2 in every colour space, against FFmpeg's raw frames.
- ASS events against FFmpeg's ASS demuxer packets.
- FLAC across compression levels, LPC settings, stereo modes, depths,
  rates and channel counts, against FFmpeg's decoder.
- Ogg pages and packets (Opus, Vorbis, FLAC, two streams in one file),
  OpusHead, Opus packet durations and Vorbis comments against FFmpeg's
  Ogg demuxer, and Ogg Opus files the harness rewrites with new tags.
- ADTS frames and the AudioSpecificConfig FFmpeg writes into MP4; ADTS
  streams the harness re-frames, decoded by FFmpeg.
- MP3 and MP2 frames, Xing and LAME tags against FFmpeg's demuxer.
- MP4, MOV and fragmented MP4 sample tables against FFmpeg's mov demuxer.
- Matroska and WebM frames against FFmpeg's Matroska demuxer, for files
  FFmpeg writes and files the harness writes with every lacing.
"""
import hashlib
import json
import math
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b39")
harness = os.path.join(work, "b39test")
built = subprocess.run([GRASM, "build", "packages/b39test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
HAVE_AFCONVERT = os.path.exists("/usr/bin/afconvert")

cases = []   # (op, path, check, family); check(answer) -> (ok, want, got)
counter = [0]


def path_for(ext):
    counter[0] += 1
    return os.path.join(work, f"m{counter[0]}.{ext}")


def write(data, ext="bin"):
    p = path_for(ext)
    with open(p, "wb") as f:
        f.write(data)
    return p


def run(cmd, data=None):
    r = subprocess.run(cmd, input=data, capture_output=True)
    if r.returncode != 0:
        raise RuntimeError(" ".join(cmd) + "\n" + r.stderr.decode(errors="replace")[-800:])
    return r.stdout


def ffmpeg(*args, data=None):
    return run(["ffmpeg", "-v", "error", "-nostdin", "-y"] + list(args), data)


def probe(path, *what):
    out = run(["ffprobe", "-v", "error", "-of", "json"] + list(what) + [path])
    return json.loads(out)


def decode_pcm(path, fmt="s32le", extra=()):
    """Every sample FFmpeg decodes from a file, interleaved."""
    raw = ffmpeg("-i", path, *extra, "-f", fmt, "-")
    code = {"s32le": "<%di", "s16le": "<%dh", "f64le": "<%dd"}[fmt]
    size = {"s32le": 4, "s16le": 2, "f64le": 8}[fmt]
    return list(struct.unpack(code % (len(raw) // size), raw))


def signal(n, channels, bits=16, kind=None):
    """Interleaved integer samples: tones, noise, silence, clipping, steps."""
    top = (1 << (bits - 1)) - 1
    kind = kind or rng.choice(["tone", "noise", "mix", "silence", "square", "ramp"])
    out = []
    f = [rng.uniform(0.001, 0.2) for _ in range(channels)]
    amp = rng.uniform(0.05, 1.0)
    for i in range(n):
        for c in range(channels):
            if kind == "tone":
                v = amp * math.sin(i * f[c] * 2 * math.pi)
            elif kind == "noise":
                v = rng.uniform(-amp, amp)
            elif kind == "mix":
                v = 0.6 * amp * math.sin(i * f[c]) + rng.uniform(-0.3, 0.3) * amp
            elif kind == "silence":
                v = 0.0
            elif kind == "square":
                v = 1.0 if (i // 37) % 2 else -1.0
            else:
                v = ((i * 7 + c * 1000) % 2000) / 1000.0 - 1.0
            out.append(max(-top - 1, min(top, int(round(v * top)))))
    return out


def pack(samples, fmt):
    return struct.pack(f"<{len(samples)}{fmt}", *samples)


def md5(b):
    return hashlib.md5(b).hexdigest()


def equal(want):
    return lambda got: (got == want, want, got)


def ints(v):
    return ",".join(str(x) for x in v)


# --- G.711 and IMA ADPCM ------------------------------------------------------

EMPTY = write(b"")
everything = pack(list(range(-32768, 32768)), "h")
for law, fmt in (("ulaw", "mulaw"), ("alaw", "alaw")):
    enc = ffmpeg("-f", "s16le", "-ar", "8000", "-ac", "1", "-i", "-", "-f", fmt, "-", data=everything)
    dec = ffmpeg("-f", fmt, "-ar", "8000", "-ac", "1", "-i", "-", "-f", "s16le", "-", data=bytes(range(256)))
    cases.append((f"g711:{law}", EMPTY, equal(md5(enc) + "|" + ints(struct.unpack("<256h", dec))), "g711"))


def riff_chunks(d, start=12):
    out, i = {}, start
    while i + 8 <= len(d):
        cid, n = d[i:i + 4], struct.unpack("<I", d[i + 4:i + 8])[0]
        out.setdefault(cid, d[i + 8:i + 8 + n])
        i += 8 + n + (n & 1)
    return out


def caf_chunks(d):
    out, i = {}, 8
    while i + 12 <= len(d):
        cid, n = d[i:i + 4], struct.unpack(">q", d[i + 4:i + 12])[0]
        if n < 0:
            n = len(d) - i - 12
        out.setdefault(cid, d[i + 12:i + 12 + n])
        i += 12 + n
    return out


def rms(a, b):
    n = min(len(a), len(b))
    return math.sqrt(sum((a[i] - b[i]) ** 2 for i in range(n)) / max(n, 1))


for _ in range(14):
    ch = rng.choice([1, 2])
    n = rng.randrange(200, 3000)
    pcm = pack(signal(n, ch), "h")
    src = write(pcm, "raw")
    block = rng.choice([64, 128, 256, 512, 1024, 2048, 4096])
    wav = path_for("wav")
    ffmpeg("-f", "s16le", "-ar", "22050", "-ac", str(ch), "-i", src, "-c:a", "adpcm_ima_wav", "-block_size", str(block), wav)
    d = open(wav, "rb").read()
    c = riff_chunks(d)
    ba, nch = struct.unpack("<H", c[b"fmt "][12:14])[0], struct.unpack("<H", c[b"fmt "][2:4])[0]
    cases.append((f"imawav:{ba}:{nch}", write(c[b"data"]), equal(ints(decode_pcm(wav, "s16le"))), "ima-wav"))


def caf_ima(d, ch, rate=22050.0):
    desc = struct.pack(">d4sIIIII", rate, b"ima4", 0, 34 * ch, 64, ch, 0)
    data = b"\0\0\0\0" + d
    return (b"caff\0\x01\0\0" + b"desc" + struct.pack(">q", len(desc)) + desc + b"data" + struct.pack(">q", len(data)) + data)


def afconvert_samples(path):
    out = path_for("wav")
    run(["afconvert", "-f", "WAVE", "-d", "LEI16", path, out])
    data = riff_chunks(open(out, "rb").read())[b"data"]
    return list(struct.unpack(f"<{len(data) // 2}h", data))


def padded_to_packet(want, data, ch):
    """FFmpeg and Apple's decoder stop at the file's valid frames; the
    padding after them is still in the packets, and the harness decodes
    every whole packet. The answer must be exactly that long and match
    the decoders up to the valid frames."""
    whole = len(data) // (34 * ch) * 64 * ch

    def check(got):
        if got == "ERR":
            return False, "samples", got
        mine = [int(x) for x in got.split("|")[-2 if "|" in got else 0].split(",") if x]
        ok = mine[:len(want)] == want and len(mine) == whole and len(want) <= whole
        return ok, ints(want[:12]) + f" ({len(want)} of {whole})", ints(mine[:12]) + f" ({len(mine)})"
    return check


for i in range(14):
    ch = rng.choice([1, 2])
    n = rng.randrange(200, 3000)
    src = write(pack(signal(n, ch), "h"), "raw")
    caf = path_for("caf")
    if HAVE_AFCONVERT and i % 2:
        wav = path_for("wav")
        ffmpeg("-f", "s16le", "-ar", "22050", "-ac", str(ch), "-i", src, wav)
        run(["afconvert", "-f", "caff", "-d", "ima4", wav, caf])
    else:
        ffmpeg("-f", "s16le", "-ar", "22050", "-ac", str(ch), "-i", src, "-c:a", "adpcm_ima_qt", caf)
    data = caf_chunks(open(caf, "rb").read())[b"data"][4:]
    check = padded_to_packet(afconvert_samples(caf) if HAVE_AFCONVERT and i % 2 else decode_pcm(caf, "s16le"), data, ch)
    cases.append((f"imaqt:{ch}", write(data), check, "ima-qt"))


def ima_wav_file(d, ch, ba, frames):
    per = (ba - 4 * ch) * 2 // ch + 1
    fmt = struct.pack("<HHIIHHHH", 0x11, ch, 22050, 22050 * ba // per, ba, 4, 2, per)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"fact" + struct.pack("<II", 4, frames) + \
        b"data" + struct.pack("<I", len(d)) + d + (b"\0" if len(d) % 2 else b"")
    return b"RIFF" + struct.pack("<I", len(body)) + body


def check_ima_encode(src_samples, wav_or_caf, ffmpeg_reference, apple):
    """Our stream must decode in FFmpeg (and afconvert) to the samples we
    predict, and stay as close to the input as FFmpeg's own encoder does."""
    def check(got):
        if got == "ERR":
            return False, "an encoding", got
        hx, mine = got.split("|")
        mine = [int(x) for x in mine.split(",")] if mine else []
        path = write(wav_or_caf(bytes.fromhex(hx)), "caf" if apple else "wav")
        theirs = decode_pcm(path, "s16le")
        if theirs != mine:
            return False, "FFmpeg decodes " + ints(theirs[:12]), ints(mine[:12])
        if apple and HAVE_AFCONVERT:
            theirs = afconvert_samples(path)
            if theirs != mine:
                return False, "afconvert decodes " + ints(theirs[:12]), ints(mine[:12])
        ours, ref = rms(src_samples, mine), rms(src_samples, ffmpeg_reference)
        if ours > ref * 1.25 + 2:
            return False, f"error no worse than FFmpeg's {ref:.1f}", f"{ours:.1f}"
        return True, "", ""
    return check


for _ in range(14):
    ch = rng.choice([1, 2])
    n = rng.randrange(100, 3000)
    s = signal(n, ch)
    src = write(pack(s, "h"), "raw")
    ba = 4 * ch * rng.randrange(2, 130)
    ref_path = path_for("wav")
    ffmpeg("-f", "s16le", "-ar", "22050", "-ac", str(ch), "-i", src, "-c:a", "adpcm_ima_wav", "-block_size", "512", ref_path)
    ref = decode_pcm(ref_path, "s16le")
    cases.append((f"imaencwav:{ch}:{ba}", src,
                  check_ima_encode(s, lambda d, ch=ch, ba=ba, n=n: ima_wav_file(d, ch, ba, n), ref, None), "ima-wav-encode"))

for _ in range(14):
    ch = rng.choice([1, 2])
    n = rng.randrange(100, 3000)
    s = signal(n, ch)
    src = write(pack(s, "h"), "raw")
    ref_path = path_for("caf")
    ffmpeg("-f", "s16le", "-ar", "22050", "-ac", str(ch), "-i", src, "-c:a", "adpcm_ima_qt", ref_path)
    ref = decode_pcm(ref_path, "s16le")
    cases.append((f"imaencqt:{ch}", src, check_ima_encode(s, lambda d, ch=ch: caf_ima(d, ch), ref, True), "ima-qt-encode"))


# --- .au, AIFF and CAF --------------------------------------------------------

def bits64(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


def source(ch, n, kind):
    """A raw input file for FFmpeg and the arguments that read it."""
    if kind == "float":
        v = [rng.uniform(-1, 1) for _ in range(n * ch)]
        v[0:2] = [0.0, -0.0][:len(v)]
        return write(pack(v, "d"), "raw"), ["-f", "f64le"]
    return write(pack(signal(n, ch, 32), "i"), "raw"), ["-f", "s32le"]


def want_samples(path, shift, is_float):
    """FFmpeg's decoded samples as the harness prints them: integers at the
    file's own width, or floats as their bits."""
    if is_float:
        return "", ",".join(str(bits64(x)) for x in decode_pcm(path, "f64le"))
    return ints(x >> shift for x in decode_pcm(path, "s32le")), ""


def check_decoded(path, shift, is_float, fields=None):
    ints_want, floats_want = want_samples(path, shift, is_float)
    st = probe(path, "-show_streams")["streams"][0]

    def check(got):
        if got == "ERR":
            return False, "a decoding", got
        parts = got.split("|")
        head, samples, floats = parts[0].split(","), parts[-2], parts[-1]
        if fields:
            for k, v in fields(st).items():
                if head[k] != v:
                    return False, f"field {k} = {v}", f"{head[k]} in {parts[0]}"
        if samples != ints_want or floats != floats_want:
            return False, (ints_want or floats_want)[:120], (samples or floats)[:120]
        return True, "", ""
    return check


AU = [("pcm_mulaw", 1, 16, False), ("pcm_alaw", 27, 16, False), ("pcm_s8", 2, 24, False), ("pcm_s16be", 3, 16, False),
      ("pcm_s24be", 4, 8, False), ("pcm_s32be", 5, 0, False), ("pcm_f32be", 6, 0, True), ("pcm_f64be", 7, 0, True)]
for i in range(24):
    codec, enc, shift, is_float = AU[i % len(AU)]
    ch, rate = rng.choice([1, 2, 3, 6]), rng.choice([8000, 11025, 22050, 44100, 48000, 96000, 12345])
    src, fmt = source(ch, rng.randrange(1, 900), "float" if is_float else "int")
    au = path_for("au")
    ffmpeg(*fmt, "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", codec, "-metadata", f"title=t{i}", au)
    cases.append(("au", au, check_decoded(au, shift, is_float, lambda st, enc=enc: {0: str(enc), 1: st["sample_rate"], 2: str(st["channels"])}), "au"))

AU_ENC = [(1, "pcm_mulaw", 16, 16), (27, "pcm_alaw", 16, 16), (2, "pcm_s8", 8, 24), (3, "pcm_s16be", 16, 16), (4, "pcm_s24be", 24, 8),
          (5, "pcm_s32be", 32, 0)]
for i in range(18):
    enc, codec, bits, shift = AU_ENC[i % len(AU_ENC)]
    ch, rate = rng.choice([1, 2, 5]), rng.choice([8000, 16000, 44100, 7777])
    s = signal(rng.randrange(1, 900), ch, bits)
    note = bytes(rng.randrange(32, 127) for _ in range(rng.randrange(0, 30))).replace(b"=", b"-")

    def check(got, s=s, ch=ch, rate=rate, codec=codec, shift=shift, enc=enc):
        if got == "ERR":
            return False, "a file", got
        path = write(bytes.fromhex(got), "au")
        st = probe(path, "-show_streams")["streams"][0]
        head = (st["codec_name"], st["sample_rate"], st["channels"])
        if head != (codec, str(rate), ch):
            return False, str((codec, rate, ch)), str(head)
        theirs = [x >> shift for x in decode_pcm(path, "s32le")]
        if enc in (1, 27):
            fmt = "mulaw" if enc == 1 else "alaw"
            ref = ffmpeg("-f", "s16le", "-ar", str(rate), "-ac", str(ch), "-i", "-", "-f", fmt, "-", data=pack(s, "h"))
            mine = bytes.fromhex(got)[-len(s):]
            return mine == ref, md5(ref), md5(mine)
        return theirs == s, ints(s[:10]), ints(theirs[:10])
    cases.append((f"auenc:{enc}:{rate}:{ch}:{note.hex()}", write(pack(s, "i"), "raw"), check, "au-encode"))


def to_wav(src, fmt, rate, ch):
    wav = path_for("wav")
    ffmpeg(*fmt, "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", "pcm_s32le" if fmt[1] == "s32le" else "pcm_f64le", wav)
    return wav


# FFmpeg's aiff muxer, and afconvert: (writer, codec or format, shift, float)
AIFF = [("ff", "pcm_s16be", 16, False), ("ff", "pcm_s24be", 8, False), ("ff", "pcm_s32be", 0, False), ("ff", "pcm_s8", 24, False),
        ("ff", "pcm_f32be", 0, True), ("ff", "pcm_f64be", 0, True), ("ff", "pcm_mulaw", 16, False), ("ff", "pcm_alaw", 16, False),
        ("ff", "pcm_s16le", 16, False)]
if HAVE_AFCONVERT:
    AIFF += [("AIFF", "BEI8", 24, False), ("AIFF", "BEI16", 16, False), ("AIFF", "BEI24", 8, False), ("AIFF", "BEI32", 0, False),
             ("AIFC", "BEF32", 0, True), ("AIFC", "BEF64", 0, True), ("AIFC", "ulaw", 16, False), ("AIFC", "alaw", 16, False)]
for i in range(34):
    writer, codec, shift, is_float = AIFF[i % len(AIFF)]
    ch = rng.choice([1, 2, 4])
    rate = rng.choice([8000, 11025, 22050, 32000, 44100, 48000, 88200, 96000, 192000, 12345, 7350, 1000])
    src, fmt = source(ch, rng.randrange(1, 900), "float" if is_float else "int")
    aif = path_for("aif")
    if writer == "ff":
        ffmpeg(*fmt, "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", codec, "-f", "aiff", aif)
    else:
        run(["afconvert", "-f", writer, "-d", codec, to_wav(src, fmt, rate, ch), aif])
    cases.append(("aiff", aif, check_decoded(aif, shift, is_float, lambda st: {
        0: str(st["channels"]), 3: str(bits64(float(st["sample_rate"])))}), "aiff"))

for i in range(16):
    bits = [8, 16, 24, 32, 16][i % 5]
    le = i % 5 == 4
    ch, rate = rng.choice([1, 2, 3]), rng.choice([8000, 22050, 44100, 48000, 96000, 4000, 192000])
    s = signal(rng.randrange(1, 900), ch, bits)

    def check(got, s=s, ch=ch, rate=rate, bits=bits):
        if got == "ERR":
            return False, "a file", got
        path = write(bytes.fromhex(got), "aif")
        st = probe(path, "-show_streams")["streams"][0]
        if (st["sample_rate"], st["channels"]) != (str(rate), ch):
            return False, f"{rate} Hz, {ch} channels", f"{st['sample_rate']} Hz, {st['channels']} channels"
        theirs = [x >> (32 - bits) for x in decode_pcm(path, "s32le")]
        if theirs != s:
            return False, "FFmpeg decodes " + ints(s[:10]), ints(theirs[:10])
        if HAVE_AFCONVERT:
            out = path_for("caf")
            run(["afconvert", "-f", "caff", "-d", "LEI32", path, out])
            raw = caf_chunks(open(out, "rb").read())[b"data"][4:]
            apple = [x >> (32 - bits) for x in struct.unpack(f"<{len(raw) // 4}i", raw)]
            if apple != s:
                return False, "afconvert decodes " + ints(s[:10]), ints(apple[:10])
        return True, "", ""
    cases.append((f"aiffenc:{ch}:{bits64(float(rate))}:{bits}:{1 if le else 0}", write(pack(s, "i"), "raw"), check, "aiff-encode"))

CAF = [("ff", c, sh, fl) for c, sh, fl in [("pcm_s16le", 16, False), ("pcm_s16be", 16, False), ("pcm_s24le", 8, False),
                                            ("pcm_s24be", 8, False), ("pcm_s32le", 0, False), ("pcm_s32be", 0, False),
                                            ("pcm_f32le", 0, True), ("pcm_f64be", 0, True), ("pcm_s8", 24, False),
                                            ("pcm_mulaw", 16, False), ("pcm_alaw", 16, False), ("adpcm_ima_qt", 16, False)]]
if HAVE_AFCONVERT:
    CAF += [("caff", c, sh, fl) for c, sh, fl in [("LEI16", 16, False), ("BEI16", 16, False), ("LEI24", 8, False), ("BEI32", 0, False),
                                                  ("LEF32", 0, True), ("BEF64", 0, True), ("ulaw", 16, False), ("alaw", 16, False),
                                                  ("ima4", 16, False), ("I8", 24, False)]]
for i in range(44):
    writer, codec, shift, is_float = CAF[i % len(CAF)]
    ch = rng.choice([1, 2]) if "ima" in codec else rng.choice([1, 2, 3, 6])
    rate = rng.choice([8000, 11025, 22050, 44100, 48000, 96000, 12345])
    src, fmt = source(ch, rng.randrange(1, 900), "float" if is_float else "int")
    caf = path_for("caf")
    tags = {f"k{j}": "".join(rng.choice("abcxyz é=") for _ in range(rng.randrange(0, 8))) for j in range(rng.randrange(0, 3))}
    if writer == "ff":
        meta = [x for k, v in tags.items() for x in ("-metadata", f"{k}={v}")]
        ffmpeg(*fmt, "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", codec, *meta, caf)
    else:
        run(["afconvert", "-f", "caff", "-d", codec, to_wav(src, fmt, rate, ch), caf])
    if "ima" in codec:
        # FFmpeg cannot open afconvert's IMA4 files; Apple's decoder reads them.
        want = afconvert_samples(caf) if codec == "ima4" else decode_pcm(caf, "s16le")
        data = caf_chunks(open(caf, "rb").read())[b"data"][4:]
        cases.append(("caf", caf, padded_to_packet(want, data, ch), "caf"))
        continue
    base = check_decoded(caf, shift, is_float, lambda st: {0: str(bits64(float(st["sample_rate"]))), 3: str(st["channels"])})
    want_tags = sorted((probe(caf, "-show_format")["format"].get("tags") or {}).items())

    def check(got, base=base, want_tags=want_tags):
        ok, w, g = base(got)
        if not ok:
            return ok, w, g
        info = got.split("|")[1]
        mine = sorted((bytes.fromhex(kv.split("=")[0]).decode(), bytes.fromhex(kv.split("=")[1]).decode()) for kv in info.split(";") if kv)
        return mine == want_tags, str(want_tags), str(mine)
    cases.append(("caf", caf, check, "caf"))

for i in range(16):
    bits = [8, 16, 24, 32][i % 4]
    le = i % 8 >= 4
    ch, rate = rng.choice([1, 2, 5]), rng.choice([8000, 22050, 44100, 48000, 96000, 4000])
    s = signal(rng.randrange(1, 900), ch, bits)

    def check(got, s=s, ch=ch, rate=rate, bits=bits):
        if got == "ERR":
            return False, "a file", got
        path = write(bytes.fromhex(got), "caf")
        st = probe(path, "-show_streams")["streams"][0]
        if (st["sample_rate"], st["channels"]) != (str(rate), ch):
            return False, f"{rate} Hz, {ch} channels", f"{st['sample_rate']} Hz, {st['channels']} channels"
        theirs = [x >> (32 - bits) for x in decode_pcm(path, "s32le")]
        if theirs != s:
            return False, "FFmpeg decodes " + ints(s[:10]), ints(theirs[:10])
        if HAVE_AFCONVERT:
            out = path_for("wav")
            run(["afconvert", "-f", "WAVE", "-d", "LEI32", path, out])
            raw = riff_chunks(open(out, "rb").read())[b"data"]
            apple = [x >> (32 - bits) for x in struct.unpack(f"<{len(raw) // 4}i", raw)]
            if apple != s:
                return False, "afconvert decodes " + ints(s[:10]), ints(apple[:10])
        return True, "", ""
    cases.append((f"cafenc:{bits64(float(rate))}:{ch}:{bits}:{1 if le else 0}", write(pack(s, "i"), "raw"), check, "caf-encode"))



# --- YUV4MPEG2, ASS and FLAC ---------------------------------------------------

y4m_odd = [0]
Y4M = ["gray", "gray9", "gray10", "gray12", "gray16", "yuv420p", "yuv422p", "yuv444p", "yuva444p", "yuv411p", "yuv420p9",
       "yuv420p10", "yuv420p12", "yuv420p14", "yuv420p16", "yuv422p10", "yuv444p12", "yuv444p16", "yuv422p16"]
for i in range(30):
    pix = Y4M[i % len(Y4M)]
    w, h = rng.randrange(2, 70), rng.randrange(2, 50)
    if pix in ("yuv411p",):
        w = w + (-w % 4)
    rate = rng.choice(["25", "30000/1001", "24000/1001", "7/3", "60", "1"])
    frames = rng.randrange(1, 5)
    vf = ["noise=alls=60:allf=t"] if i % 3 else ["drawgrid=w=3:h=3:c=white", "noise=alls=20:allf=t"]
    if i % 3 == 0:
        vf.append(rng.choice(["setfield=tff", "setfield=bff", "setfield=prog"]))
    if i % 4 == 0:
        vf.append(f"setsar={rng.randrange(1, 20)}/{rng.randrange(1, 20)}")
    if pix == "yuv420p" and i % 2:
        extra = ["-chroma_sample_location", rng.choice(["left", "topleft", "center"])]
    else:
        extra = []
    y = path_for("y4m")
    # testsrc2 rounds its size to even; a flat colour keeps odd sizes.
    ffmpeg("-f", "lavfi", "-i", f"color=c=0x{rng.randrange(1 << 24):06x}:s={w}x{h}:r={rate}", "-frames:v", str(frames),
           "-vf", ",".join(vf), "-pix_fmt", pix, *extra, "-strict", "-1", y)
    with open(y, "rb") as f:
        head = f.readline().split()
    y4m_odd[0] += int(head[1][1:]) % 2 or int(head[2][1:]) % 2
    raw = ffmpeg("-i", y, "-f", "rawvideo", "-pix_fmt", pix, "-")
    st = probe(y, "-show_streams")["streams"][0]

    def check(got, raw=raw, st=st, frames=frames):
        if got == "ERR":
            return False, "a parse", got
        head, n, digest, same = got.split("|")
        f = head.split(",")
        num, den = (int(x) for x in f[2].split(":"))
        g = math.gcd(num, den) or 1
        want = (str(st["width"]), str(st["height"]), st["r_frame_rate"], str(frames), md5(raw), "same")
        mine = (f[0], f[1], f"{num // g}/{den // g}", n, digest, same)
        return want == mine, str(want), str(mine)
    cases.append(("y4m", y, check, "y4m"))


def ass_time(cs, short):
    h, m, sec, c = cs // 360000, cs // 6000 % 60, cs // 100 % 60, cs % 100
    frac = str(c) if short and c < 10 else f"{c:02d}"
    return f"{h}:{m:02d}:{sec:02d}.{frac}"


def ass_script():
    styles = ["Default", "Alt", "Sign"]
    nl = rng.choice(["\n", "\r\n"])
    lines = ["﻿[Script Info]" if rng.random() < 0.3 else "[Script Info]", "; a comment", "ScriptType: v4.00+",
             f"PlayResX: {rng.randrange(100, 2000)}", "", "[V4+ Styles]",
             "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, "
             "StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding"]
    for st in styles:
        lines.append(f"Style: {st},Arial,{rng.randrange(10, 60)},&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1")
    if rng.random() < 0.5:
        lines += ["", "[Fonts]", "fontname: x.ttf", "M3@<=A" * rng.randrange(1, 5)]
    lines += ["", "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"]
    words = ["Hello", "world", "{\\b1}bold{\\b0}", "a, b, c", "{\\pos(10,20)}", "é ü", "\\N", "x,y", "  spaced  ", ""]
    for _ in range(rng.randrange(1, 25)):
        start = rng.randrange(0, 3600000) if rng.random() < 0.2 else rng.randrange(0, 6000)
        end = start + rng.randrange(0, 1000)
        kind = "Comment" if rng.random() < 0.15 else "Dialogue"
        layer = rng.choice(["0", "1", "12", "Marked=0"])
        text = " ".join(rng.choice(words) for _ in range(rng.randrange(0, 5)))
        # FFmpeg reads Layer ten bytes past the line's start, as though
        # "Dialogue: " were always written with exactly one space.
        sep = " " if kind == "Dialogue" else rng.choice([" ", "  ", ""])
        lines.append(f"{kind}:{sep}{layer},{ass_time(start, rng.random() < 0.2)},{ass_time(end, rng.random() < 0.2)},"
                     f"{rng.choice(styles)},{rng.choice(['', 'Bob', 'Ann'])},0000,0000,0000,,{text}")
    return nl.join(lines) + nl


for _ in range(24):
    path = write(ass_script().encode(), "ass")
    want = []
    for p in probe(path, "-show_packets", "-show_data")["packets"]:
        raw = bytes.fromhex("".join(l[10:49].replace(" ", "") for l in p.get("data", "").strip().split("\n") if l))
        want.append((p["pts"], p.get("duration", 0), raw.decode()))

    def check(got, want=want):
        if got == "ERR":
            return False, "events", got
        events = [e.split(",") for e in got.split("|")[1].split(";") if e]
        mine, order = [], 0
        for idx, (kind, start, end, _fmt, fields) in enumerate(events):
            if kind != "Dialogue":
                continue
            f = bytes.fromhex(fields).decode().split("\x01")
            layer = int("".join(__import__("itertools").takewhile(str.isdigit, f[0].lstrip()))) if f[0].lstrip()[:1].isdigit() else 0
            mine.append((int(start), idx, int(end) - int(start), f"{order},{layer}," + ",".join(f[3:])))
            order += 1
        mine = [(s, d, t) for s, _, d, t in sorted(mine)]
        k = next((k for k in range(min(len(mine), len(want))) if mine[k] != want[k]), min(len(mine), len(want)))
        return mine == want, f"{len(want)} events, #{k} " + str(want[k:k + 1]), f"{len(mine)} events, #{k} " + str(mine[k:k + 1])
    cases.append(("ass", path, check, "ass"))

def crc8_table():
    t = []
    for i in range(256):
        c = i
        for _ in range(8):
            c = ((c << 1) ^ 7) & 255 if c & 128 else (c << 1) & 255
        t.append(c)
    return t


def crc16_table():
    t = []
    for i in range(256):
        c = i << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x8005) & 0xffff if c & 0x8000 else (c << 1) & 0xffff
        t.append(c)
    return t


CRC8, CRC16 = crc8_table(), crc16_table()


def flac_header(d, at):
    """(header length, block size) of a FLAC frame header at `at` whose
    CRC-8 holds, or (0, 0)."""
    if at + 6 > len(d) or d[at] != 0xFF or d[at + 1] & 0xFE != 0xF8:
        return 0, 0
    bs, sr = d[at + 2] >> 4, d[at + 2] & 15
    first, ones = d[at + 4], 0
    while ones < 7 and first & (0x80 >> ones):
        ones += 1
    # The sync and codes, the UTF-8-coded number, then any block size and
    # sample rate the codes defer to the end of the header.
    tail = at + 4 + (ones or 1)
    n = tail - at + {6: 1, 7: 2}.get(bs, 0) + {12: 1, 13: 2, 14: 2}.get(sr, 0)
    if bs == 0 or at + n >= len(d):
        return 0, 0
    c = 0
    for b in d[at:at + n]:
        c = CRC8[c ^ b]
    if c != d[at + n]:
        return 0, 0
    block = {1: 192, 6: d[tail] + 1, 7: int.from_bytes(d[tail:tail + 2], "big") + 1}.get(bs)
    block = block or (576 << (bs - 2) if bs <= 5 else 256 << (bs - 8))
    return n, block


def flac_frames_sound(path):
    """Whether every frame of a FLAC file has a header CRC-8 and a frame
    CRC-16 that hold. A frame ends where its running CRC-16 matches the
    next two bytes and a sound header (or the end) follows. A frame with a
    corrupt header would pass as part of the one before it -- a CRC-16
    over a message and its own CRC restarts from zero -- so the frames'
    block sizes must also add up to STREAMINFO's sample count."""
    d = open(path, "rb").read()
    at, last, total = 4, False, 0
    while not last:
        last = d[at] & 128
        if d[at] & 127 == 0:
            total = ((d[at + 4 + 13] & 15) << 32) | int.from_bytes(d[at + 4 + 14:at + 4 + 18], "big")
        at += 4 + int.from_bytes(d[at + 1:at + 4], "big")
    counted = 0
    while at < len(d):
        n, block = flac_header(d, at)
        if not n:
            return False
        c, e, found = 0, at, False
        while e + 2 <= len(d):
            if e > at + n and c == int.from_bytes(d[e:e + 2], "big") and (e + 2 == len(d) or flac_header(d, e + 2)[0]):
                found = True
                break
            c = ((c << 8) & 0xffff) ^ CRC16[(c >> 8) ^ d[e]]
            e += 1
        if not found:
            return False
        counted += block
        at = e + 2
    return total == 0 or counted == total


flac_corrupt = [0]
flac_ffmpeg_empty = [0]
FLAC_RATES = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 192000, 12345, 7350, 64000, 100]
for i in range(40):
    deep = i % 3 == 0
    ch = rng.choice([1, 2, 2, 2, 3, 6, 8])
    n = rng.randrange(1, 20000)
    s = signal(n, ch, 24 if deep else 16)
    if i % 7 == 3:
        s = [x - x % 8 for x in s]   # wasted low bits
    src = write(pack([x << (8 if deep else 16) for x in s], "i"), "raw")
    rate = rng.choice(FLAC_RATES)
    opts = ["-compression_level", str(rng.randrange(0, 13))]
    if rng.random() < 0.5:
        opts += ["-lpc_type", str(rng.randrange(0, 4))]
    if ch == 2 and rng.random() < 0.6:
        opts += ["-ch_mode", str(rng.randrange(0, 4))]
    if rng.random() < 0.4:
        opts += ["-frame_size", str(rng.choice([16, 192, 576, 1152, 2304, 4096, 4097, 300, 65535]))]
    if rng.random() < 0.3:
        opts += ["-max_prediction_order", str(rng.randrange(1, 33)), "-strict", "-2"]
    if rng.random() < 0.3:
        opts += ["-exact_rice_parameters", "1", "-min_partition_order", "0", "-max_partition_order", str(rng.randrange(0, 9))]
    fl = path_for("flac")
    ffmpeg("-f", "s32le", "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", "flac", "-sample_fmt", "s32" if deep else "s16",
           *(["-bits_per_raw_sample", "24"] if deep else []), *opts, fl)
    # FFmpeg's encoder now and then writes a frame header whose CRC-8 its
    # own decoder rejects when asked to check; such a file is no oracle.
    if not flac_frames_sound(fl) or \
            subprocess.run(["ffmpeg", "-v", "error", "-err_detect", "crccheck", "-i", fl, "-f", "null", "-"], capture_output=True).stderr:
        flac_corrupt[0] += 1
        continue
    st = probe(fl, "-show_streams")["streams"][0]
    bits = int(st.get("bits_per_raw_sample") or 16)
    # FLAC is lossless: the samples the encoder was given are the answer.
    # FFmpeg's decoder must agree when it decodes the file at all (it
    # yields nothing for some valid files, such as 8 channels in blocks of
    # 65535).
    dec = [x >> (32 - bits) for x in decode_pcm(fl, "s32le")]
    if dec and dec != s:
        raise RuntimeError(f"FFmpeg's FLAC decoder does not return its encoder's input for {fl}")
    flac_ffmpeg_empty[0] += 0 if dec else 1
    want = f"{st['sample_rate']},{st['channels']},{bits}," + "{frames}" + f",{len(s)}|" + md5(pack(s, "i"))

    def check(got, want=want):
        if got == "ERR":
            return False, want, got
        head, digest = got.split("|")
        f = head.split(",")
        mine = ",".join(f[:3]) + ",{frames}," + f[4] + "|" + digest
        return mine == want, want, mine
    cases.append(("flac", fl, check, "flac"))



# --- Ogg, Opus and Vorbis comments ------------------------------------------------

def tone_file(seconds, ch, rate=48000):
    return write(pack(signal(int(seconds * rate), ch, 16, rng.choice(["tone", "mix", "noise"])), "h"), "raw")


def packets_by_stream(path):
    d = probe(path, "-show_packets", "-show_data_hash", "MD5", "-show_streams")
    per = {}
    for p in d["packets"]:
        per.setdefault(p["stream_index"], []).append((int(p["size"]), p["data_hash"].split(":")[1], p.get("duration")))
    return d["streams"], per


HEADERS = {"opus": 2, "vorbis": 3, "flac": 2}
# FFmpeg renames these comments to its own keys when it reads them.
CONV = {"ALBUMARTIST": "album_artist", "TRACKNUMBER": "track", "DISCNUMBER": "disc", "DESCRIPTION": "comment"}


def ffmpeg_tags(comments):
    """The tags FFmpeg makes of Vorbis comments: empty values dropped, a
    repeated name's values joined with ';' under its last spelling."""
    out = {}
    for k, v in comments:
        if not v:
            continue
        k = CONV.get(k.upper(), k)
        match = next((x for x in out if x.lower() == k.lower()), None)
        if match is not None:
            v = out.pop(match) + ";" + v
        out[k] = v
    return out


def decode_tags(summary):
    vendor, comments = summary.split("|", 1)
    pairs = [kv.split("=") for kv in comments.split(";") if kv]
    return bytes.fromhex(vendor).decode(), [(bytes.fromhex(k).decode(), bytes.fromhex(v).decode()) for k, v in pairs]


def ogg_encode(kind, ch, out, extra=()):
    src = tone_file(rng.uniform(0.05, 1.5), ch)
    layout = {1: "mono", 2: "stereo", 3: "3.0", 4: "quad", 5: "5.0", 6: "5.1", 7: "6.1", 8: "7.1"}[ch]
    args = ["-f", "s16le", "-ar", "48000", "-ch_layout", layout, "-i", src]
    if kind == "opus":
        args += ["-c:a", "libopus", "-application", rng.choice(["voip", "audio", "lowdelay"]),
                 "-frame_duration", rng.choice(["2.5", "5", "10", "20", "40", "60", "80", "100", "120"]),
                 "-b:a", str(rng.choice([8000, 16000, 64000, 128000, 256000]) * ch), "-vbr", rng.choice(["on", "off"])]
        if ch > 2 or rng.random() < 0.2:
            args += ["-mapping_family", str(rng.choice([1, 255]) if ch <= 8 else 255)]
    elif kind == "vorbis":
        # FFmpeg's own Vorbis encoder writes stereo only.
        args += ["-c:a", "vorbis", "-strict", "-2", "-q:a", str(rng.randrange(1, 10)), "-ac", "2"]
    else:
        args += ["-c:a", "flac", "-frame_size", str(rng.choice([1152, 4096, 16384, 65535])), "-sample_fmt", rng.choice(["s16", "s32"])]
    meta = []
    for j in range(rng.randrange(0, 5)):
        k = rng.choice(["TITLE", "artist", "Album", "foo", "ALBUMARTIST", "track", f"k{j}", "x y"])
        v = "".join(rng.choice("abé=;: Z9") for _ in range(rng.randrange(0, 10)))
        meta += ["-metadata", f"{k}={v}"]
    ffmpeg(*args, *meta, "-page_duration", str(rng.choice([1000, 20000, 1000000])), *extra, "-f", "ogg", out)


def check_ogg(path):
    streams, per = packets_by_stream(path)

    def check(got):
        if got == "ERR":
            return False, "pages", got
        pages, packets = got.split("|")
        pages = [tuple(int(x) for x in g.split(",")) for g in pages.split(";")]
        serials = []
        for flags, granule, serial, seq, nseg in pages:
            if serial not in serials:
                if not flags & 2:
                    return False, "the first page of a stream marked BOS", f"serial {serial} flags {flags}"
                serials.append(serial)
        for serial in serials:
            mine = [g for g in pages if g[2] == serial]
            if [g[3] for g in mine] != list(range(mine[0][3], mine[0][3] + len(mine))) or not mine[-1][0] & 4:
                return False, "consecutive sequence numbers and an EOS page", str(mine[:4])
        by_serial = {}
        for k in packets.split(";"):
            serial, size, granule, digest = k.split(",")
            by_serial.setdefault(int(serial), []).append((int(size), digest))
        for idx, serial in enumerate(serials):
            want = [(s, h) for s, h, _ in per.get(idx, [])]
            mine = by_serial[serial][HEADERS[streams[idx]["codec_name"]]:]
            if mine != want:
                k = next((k for k in range(min(len(mine), len(want))) if mine[k] != want[k]), min(len(mine), len(want)))
                return False, f"stream {idx}: {len(want)} packets, #{k} {want[k:k + 1]}", f"{len(mine)} packets, #{k} {mine[k:k + 1]}"
        return True, "", ""
    return check


for i in range(30):
    kind = ["opus", "vorbis", "flac"][i % 3]
    ch = rng.choice([1, 2, 6]) if kind == "opus" else rng.choice([1, 2])
    path = path_for("ogg")
    if i % 5 == 4:
        # Two logical streams multiplexed in one file.
        a, b = path_for("ogg"), path_for("ogg")
        ogg_encode(kind, ch, a)
        ogg_encode(rng.choice(["opus", "vorbis", "flac"]), rng.choice([1, 2]), b)
        ffmpeg("-i", a, "-i", b, "-map", "0:a", "-map", "1:a", "-c", "copy", "-f", "ogg", path)
    else:
        ogg_encode(kind, ch, path)
    cases.append(("ogg", path, check_ogg(path), "ogg"))


def check_opus(path):
    d = probe(path, "-show_packets", "-show_streams", "-show_data")
    st = d["streams"][0]
    extradata = bytes.fromhex("".join(l[10:49].replace(" ", "") for l in st["extradata"].strip().split("\n")))
    ver, chans, pre, rate, gain, fam = struct.unpack("<BBHIhB", extradata[8:19])
    head = [ver, chans, pre, rate, gain, fam]
    if fam:
        head += [extradata[19], extradata[20]]
        mapping = list(extradata[21:21 + chans])
    else:
        head += [1, 1 if chans == 2 else 0]
        mapping = [0] if chans == 1 else [0, 1]
    want_head = ",".join(str(x) for x in head) + "," + ints(mapping)
    durations = [int(p["duration"]) for p in d["packets"]]
    tags = st.get("tags", {})

    def check(got):
        if got == "ERR":
            return False, "an OpusHead", got
        h, same, vendor, comments, durs = got.split("|")
        mine = [int(x) for x in durs.split(",") if x]
        if h != want_head or same != "same" or chans != st["channels"] or pre != st.get("initial_padding", pre):
            return False, want_head + " same", h + " " + same
        if mine[:-1] != durations[:-1] or len(mine) != len(durations):
            return False, ints(durations[:20]), ints(mine[:20])
        _, cs = decode_tags(vendor + "|" + comments)
        if ffmpeg_tags(cs) != tags:
            return False, str(tags), str(ffmpeg_tags(cs))
        return True, "", ""
    return check


for i in range(24):
    path = path_for("opus")
    ogg_encode("opus", rng.choice([1, 2, 2, 3, 6, 8]), path)
    cases.append(("opus", path, check_opus(path), "opus"))

for i in range(14):
    path = path_for("ogg")
    ogg_encode("vorbis", rng.choice([1, 2]), path)
    tags = probe(path, "-show_streams")["streams"][0].get("tags", {})

    def check(got, tags=tags):
        if got == "ERR":
            return False, "comments", got
        vendor, comments, same = got.split("|")
        _, cs = decode_tags(vendor + "|" + comments)
        return ffmpeg_tags(cs) == tags and same == "same", str(tags) + " same", str(ffmpeg_tags(cs)) + " " + same
    cases.append(("vorbistags", path, check, "vorbis-comment"))

KEY_CHARS = "ABCXYZabcxyz019 _-.!}"
for i in range(20):
    path = path_for("opus")
    ogg_encode("opus", rng.choice([1, 2]), path)
    _, before = packets_by_stream(path)
    vendor = "".join(rng.choice("abc é9") for _ in range(rng.randrange(0, 12)))
    comments = []
    for _ in range(rng.randrange(0, 8)):
        k = "".join(rng.choice(KEY_CHARS) for _ in range(rng.randrange(1, 8)))
        if comments and rng.random() < 0.3:
            k = comments[-1][0].lower()    # a repeated name, in another case
        v = "".join(rng.choice("vé=;:x ") for _ in range(rng.randrange(0, 10)))
        comments.append((k, v))
    spec = (vendor + "\n" + "\n".join(f"{k}={v}" for k, v in comments)).encode().hex() if comments else vendor.encode().hex()

    def check(got, before=before, comments=comments):
        if got == "ERR":
            return False, "an Ogg Opus file", got
        out = write(bytes.fromhex(got), "opus")
        st, after = packets_by_stream(out)
        if [(s, h) for s, h, _ in after[0]] != [(s, h) for s, h, _ in before[0]]:
            return False, f"{len(before[0])} packets as before", f"{len(after[0])} packets"
        tags = st[0].get("tags", {})
        return tags == ffmpeg_tags(comments), str(tags), str(ffmpeg_tags(comments))
    cases.append((f"opusretag:{spec}", path, check, "opus-retag"))



# --- ADTS and MPEG audio frames ------------------------------------------------------

LAYOUTS = {1: "mono", 2: "stereo", 3: "3.0", 4: "4.0", 5: "5.0", 6: "5.1", 8: "7.1"}


def extradata_of(path):
    st = probe(path, "-show_streams", "-show_data")["streams"][0]
    return bytes.fromhex("".join(l[10:49].replace(" ", "") for l in st.get("extradata", "").strip().split("\n") if l))


for i in range(26):
    rate = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350][i % 13]
    ch = rng.choice(list(LAYOUTS))
    # FFmpeg's ADTS probe wants a run of frames before it believes a file.
    src = write(pack(signal(max(int(rate * rng.uniform(0.02, 0.5)), 1024 * 6), ch), "h"), "raw")
    aac = path_for("aac")
    tags = ["-write_id3v2", "1", "-metadata", "title=x"] if i % 4 == 1 else []
    tags += ["-write_apetag", "1"] if i % 4 == 2 else []
    ffmpeg("-f", "s16le", "-ar", str(rate), "-ch_layout", LAYOUTS[ch], "-i", src, "-c:a", "aac",
           "-profile:a", rng.choice(["aac_low", "mpeg2_aac_low"]), "-b:a", str(rng.choice([16000, 64000, 192000])), *tags, aac)
    d = probe(aac, "-show_packets", "-show_streams")
    st = d["streams"][0]
    want_frames = [(int(p["pos"]), int(p["size"])) for p in d["packets"]]
    m4a = path_for("m4a")
    ffmpeg("-i", aac, "-c", "copy", m4a)
    asc = extradata_of(m4a).hex()

    def check(got, want_frames=want_frames, st=st, asc=asc):
        if got == "ERR":
            return False, "frames", got
        frames, mine_asc = got.split("|")
        f = [x.split(",") for x in frames.split(";")]
        mine = [(int(x[0]), int(x[1])) for x in f]
        if mine != want_frames:
            return False, f"{len(want_frames)} frames {want_frames[:3]}", f"{len(mine)} frames {mine[:3]}"
        rate_i, cc = int(f[0][5]), int(f[0][6])
        rates = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350]
        if str(rates[rate_i]) != st["sample_rate"] or (cc and (8 if cc == 7 else cc) != st["channels"]):
            return False, f"{st['sample_rate']} Hz {st['channels']} ch", f"index {rate_i} config {cc}"
        if cc and mine_asc != asc:
            return False, "AudioSpecificConfig " + asc, mine_asc
        return True, "", ""
    cases.append(("adts", aac, check, "adts"))

    def recheck(got, aac=aac):
        if got == "ERR":
            return False, "frames", got
        out = write(bytes.fromhex(got), "aac")
        a = decode_pcm(aac, "s16le")
        b = decode_pcm(out, "s16le")
        return a == b, f"{len(a)} samples {md5(pack(a, 'h'))}", f"{len(b)} samples {md5(pack(b, 'h'))}"
    cases.append(("adtsrewrap", aac, recheck, "adts-rewrap"))

MP3_RATES = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000]
for i in range(40):
    layer2 = i % 4 == 3
    rate = rng.choice([16000, 22050, 24000, 32000, 44100, 48000] if layer2 else MP3_RATES)
    ch = rng.choice([1, 2])
    # FFmpeg's MPEG audio probe wants two frames in a row past any tag.
    n = max(int(rate * rng.uniform(0.05, 0.8)), 1152 * 4)
    src = write(pack(signal(n, ch), "h"), "raw")
    out = path_for("mp2" if layer2 else "mp3")
    if layer2:
        br = rng.choice([32, 64, 96, 128, 160, 192]) if rate >= 32000 else rng.choice([32, 64, 96, 128, 160])
        ffmpeg("-f", "s16le", "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", "mp2", "-b:a", f"{br}k", "-f", "mp2", out)
    else:
        mode = rng.choice([["-b:a", rng.choice(["8k", "32k", "64k", "128k", "320k"])], ["-q:a", str(rng.randrange(0, 10))],
                           ["-abr", "1", "-b:a", "96k"]])
        extra = ["-write_xing", rng.choice(["0", "1", "1"]), "-id3v2_version", rng.choice(["0", "3", "4"])]
        if rng.random() < 0.3:
            extra += ["-write_id3v1", "1", "-metadata", "title=t"]
        if ch == 2 and rng.random() < 0.5:
            extra += ["-joint_stereo", rng.choice(["0", "1"])]
        ffmpeg("-f", "s16le", "-ar", str(rate), "-ac", str(ch), "-i", src, "-c:a", "libmp3lame", *mode, *extra, out)
    d = probe(out, "-show_packets", "-show_streams")
    st = d["streams"][0]
    want_frames = [(int(p["pos"]), int(p["size"])) for p in d["packets"]]
    # FFmpeg logs the LAME tag's encoder delay and padding as it reads them.
    pad = re.search(rb"pad (\d+) (\d+)", subprocess.run(["ffprobe", "-v", "debug", out], capture_output=True).stderr)
    pad = (int(pad.group(1)), int(pad.group(2))) if pad else None

    def check(got, want_frames=want_frames, st=st, pad=pad):
        if got == "ERR":
            return False, "frames", got
        frames, tag = got.split("|")
        f = [x.split(",") for x in frames.split(";")]
        mine = [(int(x[0]), int(x[1])) for x in f]
        if tag != "none":
            mine = mine[1:]     # the frame that holds the Xing tag is not audio
        if mine != want_frames:
            k = next((k for k in range(min(len(mine), len(want_frames))) if mine[k] != want_frames[k]), 0)
            return False, f"{len(want_frames)} frames, #{k} {want_frames[k:k + 1]}", f"{len(mine)} frames, #{k} {mine[k:k + 1]}"
        h = f[-1]
        if (h[5], 1 if h[6] == "3" else 2) != (st["sample_rate"], st["channels"]):
            return False, f"{st['sample_rate']} Hz {st['channels']} ch", f"{h[5]} Hz mode {h[6]}"
        if tag != "none":
            t = tag.split(",")
            rate, tb = int(st["sample_rate"]), int(st["time_base"].split("/")[1])
            start = int(st.get("start_pts", 0)) * rate // tb
            if t[6] and int(t[7]) != start:
                return False, f"start {start}", f"start {t[7]}"
            # The tag counts the audio frames, and LAME's delay and padding
            # are FFmpeg's. (FFmpeg's stream duration can be an estimate
            # from the file's size, so the duration is not compared.)
            if int(t[1]) >= 0 and int(t[1]) != len(want_frames):
                return False, f"{len(want_frames)} frames counted", f"{t[1]} in the tag"
            if t[6] and (int(t[4]), int(t[5])) != pad:
                return False, f"delay and padding {pad}", f"{t[4]}, {t[5]}"
        return True, "", ""
    cases.append(("mp3", out, check, "mpeg-audio"))



# --- MP4, MOV and fragmented MP4 -------------------------------------------------------

def av_inputs(seconds, video=True, audio=True, rate=44100, ch=2, size="64x48", fps=10):
    args = []
    if video:
        args += ["-f", "lavfi", "-i", f"testsrc2=s={size}:r={fps}:d={seconds}"]
    if audio:
        args += ["-f", "lavfi", "-i", f"sine=f={rng.randrange(100, 2000)}:r={rate}:d={seconds}"]
    return args + (["-ac", str(ch)] if audio else [])


MP4_VARIANTS = [
    ("mp4", ["-c:v", "libx264", "-bf", "2", "-g", "5", "-c:a", "aac"], []),
    ("mp4", ["-c:v", "libx264", "-bf", "0", "-c:a", "aac"], ["-movflags", "+faststart"]),
    ("mp4", ["-c:v", "mpeg4", "-bf", "2", "-c:a", "libmp3lame"], []),
    ("mov", ["-c:v", "mjpeg", "-c:a", "pcm_s16le"], []),
    ("mov", ["-c:v", "png", "-c:a", "pcm_s24be"], []),
    ("mov", ["-c:v", "qtrle", "-c:a", "pcm_f32le"], []),
    ("mov", ["-c:v", "libx264", "-c:a", "alac"], []),
    ("mp4", ["-c:v", "libx264", "-bf", "3", "-c:a", "aac"], ["-movflags", "+frag_keyframe+empty_moov"]),
    ("mp4", ["-c:v", "libx264", "-bf", "1", "-c:a", "aac"], ["-movflags", "+frag_keyframe+empty_moov+default_base_moof"]),
    ("mp4", ["-c:v", "libx264", "-c:a", "aac"], ["-movflags", "+frag_keyframe+separate_moof+omit_tfhd_offset", "-frag_duration", "200000"]),
    ("mp4", ["-c:v", "libx264", "-c:a", "aac"], ["-movflags", "+frag_every_frame+empty_moov"]),
    ("mp4", ["-c:v", "libx264", "-bf", "2", "-c:a", "aac"], ["-movflags", "+negative_cts_offsets", "-use_editlist", "0"]),
    ("mp4", ["-c:v", "libx265", "-tag:v", "hvc1", "-c:a", "flac", "-strict", "-2"], []),
    ("mp4", ["-c:v", "libvpx-vp9", "-c:a", "libopus"], []),
    ("ipod", ["-c:a", "aac"], []),
    ("mov", ["-c:v", "libx264", "-c:a", "pcm_f64be"], ["-movflags", "+frag_keyframe"]),
    ("ismv", ["-c:v", "libx264", "-c:a", "aac"], []),
]
PCM_TAGS = {"lpcm", "sowt", "twos", "in24", "in32", "fl32", "fl64", "raw ", "ulaw", "alaw"}


def ffmpeg_view(samples, tag, table):
    """Samples as FFmpeg's mov demuxer presents them: uncompressed audio
    from the sample table regrouped into packets of up to 1024 samples
    within each chunk; and where composition offsets go negative, so that
    no packet is presented before it is decoded, decode times moved back
    in the sample table, presentation times moved forward in fragments."""
    out = []
    if tag in PCM_TAGS:
        for s in samples[:table]:
            last = out[-1] if out else None
            if last and last[6] == s[6] and last[7] < 1024:
                out[-1] = (last[0], last[1] + s[1], last[2], last[3], last[4] + s[4], True, last[6], last[7] + 1)
            else:
                out.append(s + (1,))
        out = [x[:7] for x in out]
    else:
        out = list(samples[:table])
    frag = list(samples[table:])
    # FFmpeg leaves the last *two* ctts entries out of the shift -- each
    # entry is a run of one offset -- as a guard against files that end badly
    # (mov_read_ctts updates the shift only `if (i+2<entries)`). This model
    # left out one, which agrees with ffprobe until the second-to-last run
    # holds the most negative offset: a five-frame `+negative_cts_offsets`
    # file has ctts 0, 4096, -2048 x2, 0, and ffprobe moves nothing back
    # where the model moved the first decode time to -2048.
    by_dts = sorted(out, key=lambda s: s[2])
    tail = len(by_dts)
    for _ in range(2):
        run = by_dts[tail - 1][3] - by_dts[tail - 1][2] if tail else None
        while tail > 0 and by_dts[tail - 1][3] - by_dts[tail - 1][2] == run:
            tail -= 1
    back = max([0] + [s[2] - s[3] for s in by_dts[:tail]])
    forward = max([0] + [s[2] - s[3] for s in frag])
    return [(s[0], s[1], s[2] - back, s[3], s[4], s[5]) for s in out] + \
        [(s[0], s[1], s[2], s[3] + forward, s[4], s[5]) for s in frag]


# Each variant twice at random sizes, then one fixed file: five frames of
# `+negative_cts_offsets` (variant 11), whose ctts ends 0, 4096, -2048 x2, 0
# -- the shape where FFmpeg's guard leaves out the second-to-last run too. A
# random seed reached it once in several hundred runs.
MP4_PINNED = [(11, 1.0, 8000, 1, "64x48", 5)]
for i in range(len(MP4_VARIANTS) * 2 + len(MP4_PINNED)):
    if i < len(MP4_VARIANTS) * 2:
        fmt, codecs, flags = MP4_VARIANTS[i % len(MP4_VARIANTS)]
        seconds = round(rng.uniform(0.3, 2.5), 2)
        inputs = av_inputs(seconds, video=fmt != "ipod", rate=rng.choice([8000, 22050, 44100, 48000]), ch=rng.choice([1, 2]),
                           size=rng.choice(["64x48", "32x32", "96x64"]), fps=rng.choice([5, 10, 25]))
    else:
        v, seconds, rate, ch, size, fps = MP4_PINNED[i - len(MP4_VARIANTS) * 2]
        i = v
        fmt, codecs, flags = MP4_VARIANTS[v]
        inputs = av_inputs(seconds, video=fmt != "ipod", rate=rate, ch=ch, size=size, fps=fps)
    out = path_for(f"v{i % len(MP4_VARIANTS)}." + ("mp4" if fmt in ("mp4", "ipod", "ismv") else "mov"))
    ffmpeg(*inputs, *codecs, *flags, "-strict", "-1", "-f", fmt, out)
    d = probe(out, "-ignore_editlist", "1", "-show_packets", "-show_streams")
    streams = d["streams"]
    per = {}
    for p in d["packets"]:
        per.setdefault(p["stream_index"], []).append(
            (int(p["pos"]), int(p["size"]), int(p["dts"]), int(p["pts"]), int(p.get("duration", 0)), "K" in p["flags"]))

    def check(got, streams=streams, per=per):
        if got == "ERR":
            return False, "tracks", got
        tracks = got.split("/")
        if len(tracks) != len(streams):
            return False, f"{len(streams)} tracks", f"{len(tracks)}"
        for idx, (t, st) in enumerate(zip(tracks, streams)):
            head, samples = t.split(":")
            h = head.split(",")
            tag = bytes.fromhex(h[5]).decode(errors="replace")
            if tag != st["codec_tag_string"] or st["time_base"] != f"1/{h[2]}":
                return False, f"{st['codec_tag_string']} 1/{st['time_base']}", f"{tag} 1/{h[2]}"
            if st["codec_type"] == "video" and (int(h[6]), int(h[7])) != (st["width"], st["height"]):
                return False, f"{st['width']}x{st['height']}", f"{h[6]}x{h[7]}"
            mine = []
            for x in samples.split(";"):
                f = x.split(",")
                mine.append((int(f[0]), int(f[1]), int(f[2]), int(f[3]), int(f[4]), f[5] == "true", int(f[6])))
            mine = ffmpeg_view(mine, tag, int(h[10]))
            want = sorted(per.get(idx, []))
            mine = sorted(mine)
            # FFmpeg fills a duration of 0 from the codec's frame size, and its
            # MPEG audio parser restates every packet's duration as a whole
            # frame, the stts's trimmed last frames included.
            if len(mine) == len(want):
                parsed = st.get("codec_name") in ("mp3", "mp2")
                mine = [m if m[4] and not parsed else m[:4] + (w[4],) + m[5:] for m, w in zip(mine, want)]
            if mine != want:
                k = next((k for k in range(min(len(mine), len(want))) if mine[k] != want[k]), min(len(mine), len(want)))
                return False, f"stream {idx}: {len(want)} samples, #{k} {want[k:k + 1]}", f"{len(mine)} samples, #{k} {mine[k:k + 1]}"
        return True, "", ""
    cases.append(("mp4", out, check, "isobmff"))



# --- Matroska and WebM -------------------------------------------------------------------

def ass_file():
    path = write(ass_script().encode(), "ass")
    return path


MKV_VARIANTS = [
    ("matroska", ["-c:v", "libx264", "-bf", "2", "-g", "6", "-c:a", "aac"], []),
    ("matroska", ["-c:v", "ffv1", "-c:a", "flac"], ["-cluster_time_limit", "300"]),
    ("matroska", ["-c:v", "mjpeg", "-c:a", "pcm_s16le"], ["-cluster_size_limit", "4000"]),
    ("matroska", ["-c:v", "libx264", "-c:a", "libopus"], []),
    ("matroska", ["-c:v", "mpeg4", "-c:a", "vorbis", "-strict", "-2", "-ac", "2"], ["-cues_to_front", "1", "-reserve_index_space", "2000"]),
    ("webm", ["-c:v", "libvpx", "-deadline", "realtime", "-c:a", "libopus"], []),
    ("webm", ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-c:a", "vorbis", "-strict", "-2", "-ac", "2"], ["-live", "1"]),
    ("matroska", ["-c:v", "libx264", "-c:a", "libmp3lame"], ["PIPE"]),
    ("matroska", ["-c:v", "libx264", "-c:a", "aac", "-c:s", "ass"], ["SUBS"]),
]
for i in range(len(MKV_VARIANTS) * 3):
    fmt, codecs, flags = MKV_VARIANTS[i % len(MKV_VARIANTS)]
    seconds = round(rng.uniform(0.3, 2.5), 2)
    inputs = av_inputs(seconds, rate=rng.choice([8000, 44100, 48000]), ch=rng.choice([1, 2]),
                       size=rng.choice(["64x48", "32x32", "96x64"]), fps=rng.choice([5, 10, 25]))
    out = path_for(f"v{i % len(MKV_VARIANTS)}." + ("webm" if fmt == "webm" else "mkv"))
    maps = []
    if "SUBS" in flags:
        inputs += ["-i", ass_file()]
        maps = ["-map", "0", "-map", "1", "-map", "2"]
    if "PIPE" in flags:
        # Written to a pipe, the Segment's size is unknown.
        data = ffmpeg(*inputs, *codecs, "-f", fmt, "-")
        with open(out, "wb") as f:
            f.write(data)
    else:
        ffmpeg(*inputs, *maps, *codecs, *[x for x in flags if x != "SUBS"], "-f", fmt, out)
    d = probe(out, "-show_packets", "-show_streams", "-show_data_hash", "MD5")
    streams = d["streams"]
    per = {}
    for p in d["packets"]:
        per.setdefault(p["stream_index"], []).append(
            (int(p["pts"]) if "pts" in p else -1, int(p["size"]), p["data_hash"].split(":")[1], "K" in p["flags"]))

    def check(got, streams=streams, per=per):
        if got == "ERR":
            return False, "frames", got
        info, tracks, frames = got.split("|")
        tracks = [t.split(",") for t in tracks.split(";")]
        scale = int(info.split(",")[1])
        if len(tracks) != len(streams):
            return False, f"{len(streams)} tracks", f"{len(tracks)}"
        index = {int(t[0]): k for k, t in enumerate(tracks)}
        # FFmpeg dates a track with a CodecDelay that much earlier, rounded.
        delay = {int(t[0]): (int(t[4]) * 2 + scale) // (2 * scale) for t in tracks}
        mine = {}
        for x in frames.split(";") if frames else []:
            f = x.split(",")
            track, ts = int(f[0]), int(f[1])
            mine.setdefault(index[track], []).append((ts - delay[track] if ts >= 0 else -1, int(f[3]), f[4], f[2] == "true"))
        for k, st in enumerate(streams):
            t = tracks[k]
            if st["codec_type"] == "video" and (int(t[5]), int(t[6])) != (st["width"], st["height"]):
                return False, f"{st['width']}x{st['height']}", f"{t[5]}x{t[6]}"
            a, b = per.get(k, []), mine.get(k, [])
            if a != b:
                j = next((j for j in range(min(len(a), len(b))) if a[j] != b[j]), min(len(a), len(b)))
                return False, f"stream {k}: {len(a)} packets, #{j} {a[j:j + 1]}", f"{len(b)} frames, #{j} {b[j:j + 1]}"
        return True, "", ""
    cases.append(("mkv", out, check, "matroska"))

for i in range(30):
    ntracks = rng.randrange(1, 4)
    lines, expected = [], {}
    for t in range(1, ntracks + 1):
        dd = rng.choice([0, 0, 1000000, 20000000, 33366667, 7])
        if t == 3:
            lines.append(f"T {t} 17 S_TEXT/UTF8 - {dd} 0 0 0 0 0")
        else:
            rate = rng.choice([8000.0, 44100.0, 48000.0])
            lines.append(f"T {t} 2 A_PCM/INT/LIT - {dd} 0 0 {bits64(rate)} {rng.choice([1, 2])} 16")
        expected[t] = (dd, [])
    ts = rng.randrange(0, 1000)
    last_ts = {}
    for _ in range(rng.randrange(1, 40)):
        t = rng.randrange(1, ntracks + 1)
        # FFmpeg re-times a block that shares its track's last timestamp.
        if last_ts.get(t) == ts:
            ts += 1
        last_ts[t] = ts
        lacing = rng.choice([0, 1, 2, 3])
        n = 1 if lacing == 0 else rng.randrange(1, 9)
        if lacing == 2:
            size = rng.randrange(1, 300)
            frames = [bytes(rng.randrange(256) for _ in range(size)) for _ in range(n)]
        else:
            frames = [bytes(rng.randrange(256) for _ in range(rng.choice([1, 2, 254, 255, 256, 510, 600, rng.randrange(1, 900)])))
                      for _ in range(n)]
        if t == 3:
            frames = [bytes(rng.choice(b"abc xyz") for _ in range(len(f))) for f in frames]
        key = rng.random() < 0.7
        lines.append(f"B {t} {ts} {1 if key else 0} {lacing} " + ",".join(f.hex() for f in frames))
        dd = expected[t][0]
        block = dd * n // 1000000
        for j, f in enumerate(frames):
            when = ts if j == 0 else (ts + block * j // n if block > 0 else -1)
            expected[t][1].append((when, len(f), md5(f), key))
        ts += rng.choice([0, 1, 7, 40, 1000, 40000])
    spec = write("\n".join(lines).encode(), "txt")

    def check(got, expected=expected):
        if got == "ERR":
            return False, "a file", got
        hx, own = got.split("#")
        # The harness reads its own file back: every frame, its time (-1
        # when the file leaves it unknown) and its block's key flag.
        mine = {}
        for x in own.split("|")[2].split(";"):
            f = x.split(",")
            mine.setdefault(int(f[0]), []).append((int(f[1]), int(f[3]), f[4], f[2] == "true"))
        for t in expected:
            if mine.get(t, []) != expected[t][1]:
                return False, f"track {t} read back as written", f"{mine.get(t, [])[:2]} for {expected[t][1][:2]}"
        path = write(bytes.fromhex(hx), "mkv")
        d = probe(path, "-show_packets", "-show_streams", "-show_data_hash", "MD5")
        per = {}
        for p in d["packets"]:
            per.setdefault(p["stream_index"], []).append(
                (int(p["pts"]) if "pts" in p else -1, int(p["size"]), p["data_hash"].split(":")[1], "K" in p["flags"]))
        for k, t in enumerate(sorted(expected)):
            b = per.get(k, [])
            # PCM and text are intra-only, so FFmpeg marks every packet a key
            # frame; and it interpolates a time the file leaves unknown.
            a = [(b[j][0] if w == -1 and j < len(b) else w, size, h, True) for j, (w, size, h, _key) in enumerate(expected[t][1])]
            if b != a:
                j = next((j for j in range(min(len(a), len(b))) if a[j] != b[j]), min(len(a), len(b)))
                return False, f"track {t}: {len(a)} frames, #{j} {a[j:j + 1]}", f"FFmpeg reads {len(b)}, #{j} {b[j:j + 1]}"
        return True, "", ""
    cases.append(("mkvenc:matroska", spec, check, "matroska-encode"))


# --- run ------------------------------------------------------------------------

def finish():
    answers = []
    for i in range(0, len(cases), 100):
        argv = []
        for op, path, _, _ in cases[i:i + 100]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            for op, path, _, _ in cases[i:i + 100]:
                one = subprocess.run([harness, op, path], capture_output=True, text=True)
                if one.returncode != 0:
                    print(f"harness crashed on {op} {path}")
                    break
            print(ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])

    families, differ = {}, 0
    only = os.environ.get("ONLY", "")
    for (op, path, check, fam), got in zip(cases, answers):
        try:
            ok, want, mine = check(got)
        except Exception as e:  # an answer the oracle cannot even read is a disagreement
            ok, want, mine = False, "an answer FFmpeg can read", f"{type(e).__name__}: {str(e)[:200]}"
        f = families.setdefault(fam, [0, 0])
        if ok:
            f[0] += 1
        else:
            f[1] += 1
            differ += 1
            if fam.startswith(only) and (differ <= 8 or (only and f[1] <= 8)):
                print(f"  {fam} {op[:60]} {path}")
                print(f"    want  {str(want)[:300]}")
                print(f"    grasm {str(mine)[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad:
        print("disagreeing:", " ".join(bad), f"(SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    if flac_corrupt[0] or flac_ffmpeg_empty[0]:
        print(f"FLAC files FFmpeg wrote with a bad CRC, skipped: {flac_corrupt[0]}; "
              f"FLAC files FFmpeg could not decode, checked against the encoder's input: {flac_ffmpeg_empty[0]}")
    # G.711 runs every input in one case; every other family needs ten.
    thin = [k for k, (a, d) in families.items() if a + d < 10 and k != "g711"]
    if y4m_odd[0] < 5:
        print(f"only {y4m_odd[0]} YUV4MPEG2 files of odd size; chroma rounding went untested"); sys.exit(1)
    if thin or len(families) < int(os.environ.get("FAMILIES", "24")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"media kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
