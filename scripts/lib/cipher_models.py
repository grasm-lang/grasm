"""Reference models of SM4, Camellia, ARIA and SEED, parameterised by their
S-boxes, for scripts/gen_cipher_tables.py: a table found in libcrypto is
accepted only if the model built on it encrypts as OpenSSL does.
"""
M32, M64, M128 = (1 << 32) - 1, (1 << 64) - 1, (1 << 128) - 1
def rotl32(x, n): return ((x << n) | (x >> (32 - n))) & M32
def rotl128(x, n): n %= 128; return ((x << n) | (x >> (128 - n))) & M128 if n else x
def rotr128(x, n): return rotl128(x, 128 - n)
def i2b(x, n): return x.to_bytes(n, "big")
def b2i(b): return int.from_bytes(b, "big")

# --- SM4 ---
SM4_FK = [0xA3B1BAC6, 0x56AA3350, 0x677D9197, 0xB27022DC]
def sm4_ck(i): return b2i(bytes(((4 * i + j) * 7) % 256 for j in range(4)))
def sm4_tau(s, x): return b2i(bytes(s[b] for b in i2b(x, 4)))
def sm4_enc(s, key, block):
    k = [b2i(key[4*i:4*i+4]) ^ SM4_FK[i] for i in range(4)]
    rk = []
    for i in range(32):
        b = sm4_tau(s, k[i+1] ^ k[i+2] ^ k[i+3] ^ sm4_ck(i))
        k.append(k[i] ^ b ^ rotl32(b, 13) ^ rotl32(b, 23)); rk.append(k[-1])
    x = [b2i(block[4*i:4*i+4]) for i in range(4)]
    for i in range(32):
        b = sm4_tau(s, x[i+1] ^ x[i+2] ^ x[i+3] ^ rk[i])
        x.append(x[i] ^ b ^ rotl32(b, 2) ^ rotl32(b, 10) ^ rotl32(b, 18) ^ rotl32(b, 24))
    return b"".join(i2b(v, 4) for v in reversed(x[-4:]))

# --- Camellia ---
def sqrt_frac_digits(p):
    # hex digits 2..17 of the fractional part of sqrt(p)
    from math import isqrt
    r = isqrt(p << (2 * 4 * 20))       # sqrt(p) * 16^20
    frac = r & ((1 << 80) - 1)          # 20 hex digits of fraction
    return (frac >> 12) & M64           # drop first digit, keep next 16
CAM_SIGMA = [sqrt_frac_digits(p) for p in (2, 3, 5, 7, 11, 13)]
def cam_boxes(s1):
    rot8 = lambda v, n: ((v << n) | (v >> (8 - n))) & 255
    return s1, [rot8(s1[x], 1) for x in range(256)], [rot8(s1[x], 7) for x in range(256)], [s1[rot8(x, 1)] for x in range(256)]
def cam_f(sb, x, k):
    s1, s2, s3, s4 = sb
    t = list(i2b(x ^ k, 8))
    t = [s1[t[0]], s2[t[1]], s3[t[2]], s4[t[3]], s2[t[4]], s3[t[5]], s4[t[6]], s1[t[7]]]
    t1, t2, t3, t4, t5, t6, t7, t8 = t
    y = [t1^t3^t4^t6^t7^t8, t1^t2^t4^t5^t7^t8, t1^t2^t3^t5^t6^t8, t2^t3^t4^t5^t6^t7,
         t1^t2^t6^t7^t8, t2^t3^t5^t7^t8, t3^t4^t5^t6^t8, t1^t4^t5^t6^t7]
    return b2i(bytes(y))
def cam_fl(x, k):
    x1, x2, k1, k2 = x >> 32, x & M32, k >> 32, k & M32
    x2 ^= rotl32(x1 & k1, 1); x1 ^= (x2 | k2)
    return (x1 << 32) | x2
def cam_flinv(y, k):
    y1, y2, k1, k2 = y >> 32, y & M32, k >> 32, k & M32
    y1 ^= (y2 | k2); y2 ^= rotl32(y1 & k1, 1)
    return (y1 << 32) | y2
def cam_keys(sb, key):
    if len(key) == 16: kl, kr = b2i(key), 0
    elif len(key) == 24: kl, kr = b2i(key[:16]), (b2i(key[16:]) << 64) | (~b2i(key[16:]) & M64)
    else: kl, kr = b2i(key[:16]), b2i(key[16:])
    S = CAM_SIGMA
    d1, d2 = (kl ^ kr) >> 64, (kl ^ kr) & M64
    d2 ^= cam_f(sb, d1, S[0]); d1 ^= cam_f(sb, d2, S[1])
    d1 ^= kl >> 64; d2 ^= kl & M64
    d2 ^= cam_f(sb, d1, S[2]); d1 ^= cam_f(sb, d2, S[3])
    ka = (d1 << 64) | d2
    d1, d2 = (ka ^ kr) >> 64, (ka ^ kr) & M64
    d2 ^= cam_f(sb, d1, S[4]); d1 ^= cam_f(sb, d2, S[5])
    kb = (d1 << 64) | d2
    def halves(v, n): r = rotl128(v, n); return [r >> 64, r & M64]
    if len(key) == 16:
        plan = [("L",0),("A",0),("L",15),("A",15),("FL","A",30),("L",45),("A",45,"hi"),("L",60,"lo"),("A",60),("FL","L",77),("L",94),("A",94),("L",111),("A",111)]
    else:
        plan = [("L",0),("B",0),("R",15),("A",15),("FL","R",30),("B",30),("L",45),("A",45),("FL","L",60),("R",60),("B",60),("L",77),("FL","A",77),("R",94),("A",94),("L",111),("B",111)]
    regs = {"L": kl, "R": kr, "A": ka, "B": kb}
    out = []
    for step in plan:
        if step[0] == "FL":
            out.append(("fl", halves(regs[step[1]], step[2])))
        elif len(step) == 3:
            h = halves(regs[step[0]], step[1]); out.append(("k", [h[0] if step[2] == "hi" else h[1]]))
        else:
            out.append(("k", halves(regs[step[0]], step[1])))
    flat, fls = [], []
    for kind, v in out:
        if kind == "k": flat += v
        else: flat.append(("FL", v))
    return flat
def cam_enc(s1, key, block):
    sb = cam_boxes(s1)
    seq = cam_keys(sb, key)
    kw1, kw2 = seq[0], seq[1]
    kw3, kw4 = seq[-2], seq[-1]
    body = seq[2:-2]
    m = b2i(block); d1, d2 = (m >> 64) ^ kw1, (m & M64) ^ kw2
    i = 0
    rnd = 0
    while i < len(body):
        item = body[i]
        if isinstance(item, tuple):
            d1 = cam_fl(d1, item[1][0]); d2 = cam_flinv(d2, item[1][1]); i += 1; continue
        if rnd % 2 == 0: d2 ^= cam_f(sb, d1, item)
        else: d1 ^= cam_f(sb, d2, item)
        rnd += 1; i += 1
    d2 ^= kw3; d1 ^= kw4
    return i2b((d2 << 64) | d1, 16)

# --- ARIA ---
ARIA_C = [0x517cc1b727220a94fe13abe8fa9a6ee0, 0x6db14acc9e21c820ff28b1d5ef5de2b0, 0xdb92371d2126e9700324977504e8c90e]
ARIA_A = [[3,4,6,8,9,13,14],[2,5,7,8,9,12,15],[1,4,6,10,11,12,15],[0,5,7,10,11,13,14],[0,2,5,8,11,14,15],[1,3,4,9,10,14,15],
          [0,2,7,9,10,12,13],[1,3,6,8,11,12,13],[0,1,4,7,10,13,15],[0,1,5,6,11,12,14],[2,3,5,6,8,13,15],[2,3,4,7,9,12,14],
          [1,2,6,7,9,11,12],[0,3,6,7,8,10,13],[0,3,4,5,9,11,14],[1,2,4,5,8,10,15]]
def inv(s): r = [0]*256; [r.__setitem__(s[i], i) for i in range(256)]; return r
def aria_diff(x):
    b = i2b(x, 16); return b2i(bytes([b[a]^b[c]^b[d]^b[e]^b[f]^b[g]^b[h] for a,c,d,e,f,g,h in ARIA_A]))
def aria_sl(sb, x, odd):
    sb1, sb2, sb3, sb4 = sb
    order = [sb1, sb2, sb3, sb4] if odd else [sb3, sb4, sb1, sb2]
    b = i2b(x, 16); return b2i(bytes(order[i % 4][b[i]] for i in range(16)))
def aria_enc(s1, s2, key, block):
    sb = (s1, s2, inv(s1), inv(s2))
    fo = lambda d, rk: aria_diff(aria_sl(sb, d ^ rk, True))
    fe = lambda d, rk: aria_diff(aria_sl(sb, d ^ rk, False))
    kl = b2i(key[:16]); kr = b2i(key[16:].ljust(16, b"\0"))
    n = {16: 12, 24: 14, 32: 16}[len(key)]
    ck = {16: (0, 1, 2), 24: (1, 2, 0), 32: (2, 0, 1)}[len(key)]
    w0 = kl; w1 = fo(w0, ARIA_C[ck[0]]) ^ kr; w2 = fe(w1, ARIA_C[ck[1]]) ^ w0; w3 = fo(w2, ARIA_C[ck[2]]) ^ w1
    W = [w0, w1, w2, w3]
    ek = []
    for rot in [lambda v: rotr128(v, 19), lambda v: rotr128(v, 31), lambda v: rotl128(v, 61), lambda v: rotl128(v, 31)]:
        for i in range(4):
            ek.append(W[i] ^ rot(W[(i + 1) % 4]))
    ek.append(W[0] ^ rotl128(W[1], 19))
    p = b2i(block)
    for i in range(n - 1):
        p = fo(p, ek[i]) if i % 2 == 0 else fe(p, ek[i])
    return i2b(aria_sl(sb, p ^ ek[n - 1], False) ^ ek[n], 16)

# --- SEED ---
SEED_M = [0xfc, 0xf3, 0xcf, 0x3f]
def seed_g(s1, s2, x):
    y0, y1, y2, y3 = s1[x & 255], s2[(x >> 8) & 255], s1[(x >> 16) & 255], s2[x >> 24]
    m0, m1, m2, m3 = SEED_M
    z3 = (y0 & m3) ^ (y1 & m0) ^ (y2 & m1) ^ (y3 & m2)
    z2 = (y0 & m2) ^ (y1 & m3) ^ (y2 & m0) ^ (y3 & m1)
    z1 = (y0 & m1) ^ (y1 & m2) ^ (y2 & m3) ^ (y3 & m0)
    z0 = (y0 & m0) ^ (y1 & m1) ^ (y2 & m2) ^ (y3 & m3)
    return (z3 << 24) | (z2 << 16) | (z1 << 8) | z0
def seed_keys(s1, s2, key):
    a, b, c, d = [b2i(key[4*i:4*i+4]) for i in range(4)]
    ks = []
    for i in range(16):
        kc = rotl32(0x9e3779b9, i)
        ks.append((seed_g(s1, s2, (a + c - kc) & M32), seed_g(s1, s2, (b - d + kc) & M32)))
        if i % 2 == 0:
            ab = (a << 32) | b; ab = ((ab >> 8) | (ab << 56)) & M64; a, b = ab >> 32, ab & M32
        else:
            cd = (c << 32) | d; cd = ((cd << 8) | (cd >> 56)) & M64; c, d = cd >> 32, cd & M32
    return ks
def seed_f(s1, s2, k, r0, r1):
    r0 ^= k[0]; r1 ^= k[1]; r1 ^= r0
    r1 = seed_g(s1, s2, r1); r0 = (r0 + r1) & M32; r0 = seed_g(s1, s2, r0)
    r1 = (r1 + r0) & M32; r1 = seed_g(s1, s2, r1); r0 = (r0 + r1) & M32
    return r0, r1
def seed_enc(s1, s2, key, block):
    ks = seed_keys(s1, s2, key)
    l0, l1, r0, r1 = [b2i(block[4*i:4*i+4]) for i in range(4)]
    for i in range(16):
        t0, t1 = seed_f(s1, s2, ks[i], r0, r1)
        l0, l1 = l0 ^ t0, l1 ^ t1
        if i < 15: l0, l1, r0, r1 = r0, r1, l0, l1
    return b"".join(i2b(v, 4) for v in (l0, l1, r0, r1))
