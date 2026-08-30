import hashlib, hmac as hmaclib, os, random, sys, struct

# --- X25519, plainly, with Python's arbitrary-precision integers -------------
# No limbs, no carries: the point of this file is that it shares no structure
# with the implementation under test, so a limb bug cannot be present in both.
P = 2**255 - 19
A24 = 121665

def clamp(k):
    k = bytearray(k)
    k[0] &= 248; k[31] &= 127; k[31] |= 64
    return bytes(k)

def unpack(b):
    return int.from_bytes(b, 'little') & ((1 << 255) - 1)

def pack(n):
    return (n % P).to_bytes(32, 'little')

def scalarmult(scalar, point):
    k = int.from_bytes(clamp(scalar), 'little')
    u = unpack(point)
    x1, x2, z2, x3, z3, swap = u, 1, 0, u, 1, 0
    for t in range(254, -1, -1):
        kt = (k >> t) & 1
        swap ^= kt
        if swap:
            x2, x3 = x3, x2
            z2, z3 = z3, z2
        swap = kt
        a = (x2 + z2) % P; aa = a * a % P
        b = (x2 - z2) % P; bb = b * b % P
        e = (aa - bb) % P
        c = (x3 + z3) % P; d = (x3 - z3) % P
        da = d * a % P; cb = c * b % P
        x3 = pow(da + cb, 2, P)
        z3 = x1 * pow(da - cb, 2, P) % P
        x2 = aa * bb % P
        z2 = e * (aa + A24 * e) % P
    if swap:
        x2, x3 = x3, x2
        z2, z3 = z3, z2
    return pack(x2 * pow(z2, P - 2, P) % P)

# --- ChaCha20-Poly1305, also plainly ----------------------------------------
def rotl(v, n): return ((v << n) | (v >> (32 - n))) & 0xffffffff

def _rounds(w):
    def qr(a, b, c, d):
        w[a] = (w[a] + w[b]) & 0xffffffff; w[d] = rotl(w[d] ^ w[a], 16)
        w[c] = (w[c] + w[d]) & 0xffffffff; w[b] = rotl(w[b] ^ w[c], 12)
        w[a] = (w[a] + w[b]) & 0xffffffff; w[d] = rotl(w[d] ^ w[a], 8)
        w[c] = (w[c] + w[d]) & 0xffffffff; w[b] = rotl(w[b] ^ w[c], 7)
    for _ in range(10):
        qr(0,4,8,12); qr(1,5,9,13); qr(2,6,10,14); qr(3,7,11,15)
        qr(0,5,10,15); qr(1,6,11,12); qr(2,7,8,13); qr(3,4,9,14)
    return w

def chacha_block(key, counter, nonce):
    st = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]
    st += list(struct.unpack('<8I', key))
    st += [counter] + list(struct.unpack('<3I', nonce))
    w = _rounds(st[:])
    return b''.join(struct.pack('<I', (w[i] + st[i]) & 0xffffffff) for i in range(16))

def hchacha(key, nonce16):
    """The same rounds with the final addition left out — that omission is the
    construction, not an optimisation, and is what makes the result one-way."""
    st = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]
    st += list(struct.unpack('<8I', key))
    st += list(struct.unpack('<4I', nonce16))
    w = _rounds(st[:])
    return struct.pack('<4I', *w[0:4]) + struct.pack('<4I', *w[12:16])

def xseal(key, nonce24, aad, plain):
    return seal(hchacha(key, nonce24[:16]), b'\x00' * 4 + nonce24[16:], aad, plain)

def chacha(key, counter, nonce, data):
    out = bytearray()
    for i in range(0, len(data), 64):
        ks = chacha_block(key, counter + i // 64, nonce)
        chunk = data[i:i+64]
        out += bytes(a ^ b for a, b in zip(chunk, ks))
    return bytes(out)

def poly1305(key, msg):
    r = int.from_bytes(key[:16], 'little') & 0x0ffffffc0ffffffc0ffffffc0fffffff
    s = int.from_bytes(key[16:], 'little')
    p = (1 << 130) - 5
    acc = 0
    for i in range(0, len(msg), 16):
        blk = msg[i:i+16]
        n = int.from_bytes(blk + b'\x01', 'little')
        acc = (acc + n) * r % p
    return ((acc + s) & ((1 << 128) - 1)).to_bytes(16, 'little')

def pad16(b): return b'\x00' * ((16 - len(b) % 16) % 16)

def seal(key, nonce, aad, plain):
    otk = chacha_block(key, 0, nonce)[:32]
    ct = chacha(key, 1, nonce, plain)
    mac_data = aad + pad16(aad) + ct + pad16(ct)
    mac_data += struct.pack('<Q', len(aad)) + struct.pack('<Q', len(ct))
    return ct + poly1305(otk, mac_data)

# --- the handshake, from the protocol description rather than from the code --
#
# The `noise` package pins three values so a refactor cannot move them quietly.
# That proves they did not change; it does not prove they were right. This
# derives the same three from the written protocol — the mixing order, the
# labels, the KDF, the AEAD nonces — so agreement means two independent
# readings of the specification landed on the same bytes.
CONSTRUCTION = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s"
IDENTIFIER = b"GrasmWire v1"
BASE = (9).to_bytes(32, 'little')

def _h(d): return hashlib.blake2s(d).digest()
def _hmac(k, m): return hmaclib.new(k, m, hashlib.blake2s).digest()

def kdf(ck, inp, n):
    prk = _hmac(ck, inp)
    out, prev, ctr = b'', b'', 1
    while len(out) < n * 32:
        prev = _hmac(prk, prev + bytes([ctr]))
        out += prev
        ctr += 1
    return [out[i * 32:(i + 1) * 32] for i in range(n)]

class _State:
    def __init__(self, ck, h):
        self.ck, self.h = ck, h
    def mix_hash(self, d):
        self.h = _h(self.h + d)
    def mix_only(self, i):
        self.ck = kdf(self.ck, i, 1)[0]
    def mix_key(self, i):
        o = kdf(self.ck, i, 2)
        self.ck = o[0]
        return o[1]
    def mix_psk(self, p):
        o = kdf(self.ck, p, 3)
        self.ck = o[0]
        self.mix_hash(o[1])
        return o[2]

def handshake(ipriv, rpriv, ieph, reph, psk, timestamp):
    """The transcript hash and both transport keys, initiator's way round."""
    ipub, rpub = scalarmult(ipriv, BASE), scalarmult(rpriv, BASE)
    st = _State(_h(CONSTRUCTION), b'')
    st.h = _h(st.ck + IDENTIFIER)
    st.mix_hash(rpub)

    # -> e, es, s, ss
    iepub = scalarmult(ieph, BASE)
    st.mix_only(iepub)
    st.mix_hash(iepub)
    k1 = st.mix_key(scalarmult(ieph, rpub))
    sealed_static = seal(k1, b'\x00' * 12, st.h, ipub)
    st.mix_hash(sealed_static)
    k2 = st.mix_key(scalarmult(ipriv, rpub))
    sealed_time = seal(k2, b'\x00' * 12, st.h, timestamp)
    st.mix_hash(sealed_time)

    # <- e, ee, se, psk
    repub = scalarmult(reph, BASE)
    st.mix_only(repub)
    st.mix_hash(repub)
    st.mix_only(scalarmult(reph, iepub))
    st.mix_only(scalarmult(reph, ipub))
    k = st.mix_psk(psk)
    sealed_nothing = seal(k, b'\x00' * 12, st.h, b'')
    st.mix_hash(sealed_nothing)

    keys = kdf(st.ck, b'', 2)
    return st.h, keys[0], keys[1]


# --- generate, then compare -------------------------------------------------
#
# Guarded, so this file can be imported as a reference without writing files.
def _main():
    random.seed(int(sys.argv[1]) if len(sys.argv) > 1 else 20260830)
    N = int(sys.argv[2]) if len(sys.argv) > 2 else 200
    OUT = sys.argv[3] if len(sys.argv) > 3 else '/tmp/cryptodiff' 
    cases, want = [], []

    for i in range(N):
        n = random.randrange(1, 200)
        msg = bytes(random.randrange(256) for _ in range(n))
        cases.append("hash " + msg.hex())
        want.append(hashlib.blake2s(msg).hexdigest())

        klen = random.randrange(1, 33)
        key = bytes(random.randrange(256) for _ in range(klen))
        cases.append("keyed " + key.hex() + " " + msg.hex())
        want.append(hashlib.blake2s(msg, key=key).hexdigest())

        hk = bytes(random.randrange(256) for _ in range(random.randrange(1, 80)))
        cases.append("hmac " + hk.hex() + " " + msg.hex())
        want.append(hmaclib.new(hk, msg, hashlib.blake2s).hexdigest())

        sk = bytes(random.randrange(256) for _ in range(32))
        pk = bytes(random.randrange(256) for _ in range(32))
        cases.append("dh " + sk.hex() + " " + pk.hex())
        want.append(scalarmult(sk, pk).hex())

        cases.append("pub " + sk.hex())
        want.append(scalarmult(sk, (9).to_bytes(32, 'little')).hex())

        ak = bytes(random.randrange(256) for _ in range(32))
        an = bytes(random.randrange(256) for _ in range(12))
        # At least one byte each: an empty field would make this line's own
        # format ambiguous, and the empty cases are already covered by the RFC
        # vectors in the package's own test.
        aad = bytes(random.randrange(256) for _ in range(random.randrange(1, 40)))
        smsg = msg if msg else b"\x00"
        cases.append(" ".join(["seal", ak.hex(), an.hex(), aad.hex(), smsg.hex()]))
        want.append(seal(ak, an, aad, smsg).hex())

    # The anti-replay window, over random sequences. The model below is the
    # rule stated plainly — accept a counter once, refuse anything more than
    # 2048 behind the highest seen — with no bitmap and no shifting, so it
    # cannot share an off-by-one with the implementation it is checking.
    for _ in range(30):
        seq, highest, seen = [], -1, set()
        for _ in range(random.randrange(5, 60)):
            style = random.randrange(4)
            if style == 0:
                c = max(0, highest + random.randrange(1, 4))
            elif style == 1:
                c = max(0, highest - random.randrange(0, 2100))
            elif style == 2:
                c = max(0, highest + random.randrange(1, 5000))
            else:
                c = random.randrange(0, 8000)
            seq.append(c)
            highest = max(highest, c)
        model, top, spent = [], 0, set()
        first = True
        for c in seq:
            if first:
                # The window starts at zero with nothing spent, so counter 0
                # is inside it and everything else is ahead of it.
                first = False
            if c > top:
                top = c
                spent = {x for x in spent if top - x < 2048}
                spent.add(c)
                model.append('y')
            elif top - c >= 2048 or c in spent:
                model.append('n')
            else:
                spent.add(c)
                model.append('y')
        cases.append("replay " + ",".join(str(c) for c in seq))
        want.append("".join(model))

    # And one whole handshake, checked against a second reading of the
    # protocol rather than against its own previous output.
    hs_keys = [bytes(random.randrange(256) for _ in range(32)) for _ in range(4)]
    hs_psk = bytes(random.randrange(256) for _ in range(32))
    hs_ts = bytes(random.randrange(256) for _ in range(12))
    ip, rp, ie, re = [clamp(k) for k in hs_keys]
    cases.append(" ".join(["handshake", ip.hex(), rp.hex(), ie.hex(), re.hex(),
                           hs_ts.hex(), hs_psk.hex()]))
    hh, tx, rx = handshake(ip, rp, ie, re, hs_psk, hs_ts)
    want += [hh.hex(), tx.hex(), rx.hex()]

    open(os.path.join(OUT, 'cases.txt'), 'w').write("\n".join(cases) + "\n")
    open(os.path.join(OUT, 'want.txt'), 'w').write("\n".join(want) + "\n")
    print(f"{len(cases)} cases written")


if __name__ == '__main__':
    _main()
