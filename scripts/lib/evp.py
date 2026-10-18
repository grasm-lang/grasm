"""OpenSSL's libcrypto, in-process through ctypes, as an oracle.

Running `openssl enc` once per case costs a process per case; this calls
EVP directly, so a gate can afford tens of thousands of cases. Ciphers are
fetched by name (the legacy provider is loaded too, for Blowfish, CAST5,
IDEA, SEED and RC2), MACs through EVP_MAC with OSSL_PARAM arrays.

Every call returns bytes, or None where OpenSSL refuses: a bad length, a
tag that does not verify, an unwrap whose integrity check fails.
"""
import ctypes
import glob
import os

_CANDIDATES = ["/opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib", "/usr/local/opt/openssl@3/lib/libcrypto.3.dylib"]
_path = next((p for p in _CANDIDATES if os.path.exists(p)), None) or (glob.glob("/opt/homebrew/Cellar/openssl@3/*/lib/libcrypto.3.dylib") or [None])[0]
if _path is None:
    raise SystemExit("no OpenSSL 3 libcrypto found")
L = ctypes.CDLL(_path)

vp, cp, ci, sz = ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_size_t
L.OSSL_PROVIDER_load.restype = vp
L.OSSL_PROVIDER_load.argtypes = [vp, cp]
L.OpenSSL_version.restype = cp
L.OpenSSL_version.argtypes = [ci]
L.EVP_CIPHER_fetch.restype = vp
L.EVP_CIPHER_fetch.argtypes = [vp, cp, cp]
L.EVP_CIPHER_free.argtypes = [vp]
L.EVP_CIPHER_CTX_new.restype = vp
L.EVP_CIPHER_CTX_free.argtypes = [vp]
L.EVP_CipherInit_ex.argtypes = [vp, vp, vp, cp, cp, ci]
L.EVP_CipherInit_ex2.argtypes = [vp, vp, cp, cp, ci, vp]
L.EVP_CipherUpdate.argtypes = [vp, vp, ctypes.POINTER(ci), cp, ci]
L.EVP_CipherFinal_ex.argtypes = [vp, vp, ctypes.POINTER(ci)]
L.EVP_CIPHER_CTX_set_padding.argtypes = [vp, ci]
L.EVP_CIPHER_CTX_ctrl.argtypes = [vp, ci, ci, vp]
L.EVP_CIPHER_CTX_set_params.argtypes = [vp, vp]
L.EVP_MAC_fetch.restype = vp
L.EVP_MAC_fetch.argtypes = [vp, cp, cp]
L.EVP_MAC_free.argtypes = [vp]
L.EVP_MAC_CTX_new.restype = vp
L.EVP_MAC_CTX_new.argtypes = [vp]
L.EVP_MAC_CTX_free.argtypes = [vp]
L.EVP_MAC_init.argtypes = [vp, cp, sz, vp]
L.EVP_MAC_update.argtypes = [vp, cp, sz]
L.EVP_MAC_final.argtypes = [vp, vp, ctypes.POINTER(sz), sz]
L.EVP_MAC_finalXOF.argtypes = [vp, vp, sz]
L.ERR_clear_error.argtypes = []

L.OSSL_PROVIDER_load(None, b"default")
LEGACY = bool(L.OSSL_PROVIDER_load(None, b"legacy"))
VERSION = L.OpenSSL_version(0).decode()

SET_IVLEN, GET_TAG, SET_TAG = 0x9, 0x10, 0x11


class _Param(ctypes.Structure):
    _fields_ = [("key", cp), ("data_type", ctypes.c_uint), ("data", vp), ("data_size", sz), ("return_size", sz)]


def _params(items):
    """OSSL_PARAMs from (name, value) pairs: str is UTF-8, bytes an octet
    string, int an unsigned integer (size_t), bool an int."""
    arr = (_Param * (len(items) + 1))()
    keep = []
    for i, (k, v) in enumerate(items):
        if isinstance(v, bool):
            buf = ctypes.c_int(1 if v else 0); t, n = 1, ctypes.sizeof(buf)
        elif isinstance(v, int):
            buf = ctypes.c_size_t(v); t, n = 2, ctypes.sizeof(buf)
        elif isinstance(v, str):
            raw = v.encode(); buf = ctypes.create_string_buffer(raw, len(raw) + 1); t, n = 4, len(raw)
        else:
            buf = ctypes.create_string_buffer(bytes(v), max(len(v), 1)); t, n = 5, len(v)
        keep.append(buf)
        arr[i] = _Param(k.encode(), t, ctypes.cast(ctypes.byref(buf), vp), n, 0)
    arr[len(items)] = _Param(None, 0, None, 0, 0)
    return arr, keep


class _Ctx:
    def __init__(self, name):
        self.cipher = L.EVP_CIPHER_fetch(None, name.encode(), None)
        if not self.cipher:
            L.ERR_clear_error()
            raise KeyError(name)
        self.ctx = L.EVP_CIPHER_CTX_new()

    def __enter__(self):
        return self

    def __exit__(self, *a):
        L.EVP_CIPHER_CTX_free(self.ctx)
        L.EVP_CIPHER_free(self.cipher)
        L.ERR_clear_error()

    def update(self, data, out=True):
        n = ci(0)
        if not out:
            return L.EVP_CipherUpdate(self.ctx, None, ctypes.byref(n), data, len(data)) == 1
        buf = ctypes.create_string_buffer(len(data) + 64)
        if L.EVP_CipherUpdate(self.ctx, buf, ctypes.byref(n), data, len(data)) != 1:
            return None
        return buf.raw[:n.value]

    def final(self):
        buf = ctypes.create_string_buffer(64)
        n = ci(0)
        if L.EVP_CipherFinal_ex(self.ctx, buf, ctypes.byref(n)) != 1:
            return None
        return buf.raw[:n.value]

    def ctrl(self, op, arg, ptr=None):
        return L.EVP_CIPHER_CTX_ctrl(self.ctx, op, arg, ptr) == 1


def cipher(name, key, iv, data, encrypt=True, padding=False, params=()):
    """A whole message through a cipher, or None if OpenSSL refuses."""
    with _Ctx(name) as c:
        enc = 1 if encrypt else 0
        if params:
            # Parameters such as the key length have to be in place before
            # the key is: set them on a keyless init, then give the key.
            arr, keep = _params(list(params))
            if L.EVP_CipherInit_ex2(c.ctx, c.cipher, None, None, enc, arr) != 1:
                return None
            if L.EVP_CipherInit_ex2(c.ctx, None, key, iv, enc, None) != 1:
                return None
        elif L.EVP_CipherInit_ex2(c.ctx, c.cipher, key, iv, enc, None) != 1:
            return None
        L.EVP_CIPHER_CTX_set_padding(c.ctx, 1 if padding else 0)
        body = c.update(data)
        if body is None:
            return None
        tail = c.final()
        return None if tail is None else body + tail


def aead_seal(name, key, nonce, aad_parts, plaintext, tag_len=16, ccm=False, fixed_nonce=False):
    """Ciphertext and tag. `aad_parts` is a list: SIV takes each as its own
    component, the others concatenate them. `fixed_nonce` skips setting the
    nonce length, for modes whose nonce is not adjustable (SIV has none)."""
    with _Ctx(name) as c:
        if L.EVP_CipherInit_ex(c.ctx, c.cipher, None, None, None, 1) != 1:
            return None
        if nonce is not None and not fixed_nonce and not c.ctrl(SET_IVLEN, len(nonce)):
            return None
        if (ccm or "OCB" in name) and not c.ctrl(SET_TAG, tag_len):
            return None
        if L.EVP_CipherInit_ex(c.ctx, None, None, key, nonce, 1) != 1:
            return None
        if ccm:
            n = ci(0)
            if L.EVP_CipherUpdate(c.ctx, None, ctypes.byref(n), None, len(plaintext)) != 1:
                return None
        for a in aad_parts:
            if (len(a) or "SIV" in name) and not c.update(a, out=False):
                return None
        body = c.update(plaintext) if (plaintext or ccm or "SIV" in name) else b""
        if body is None:
            return None
        tail = c.final()
        if tail is None:
            return None
        tag = ctypes.create_string_buffer(16)
        if not c.ctrl(GET_TAG, tag_len, ctypes.cast(tag, vp)):
            return None
        return body + tail, tag.raw[:tag_len]


def aead_open(name, key, nonce, aad_parts, ciphertext, tag, ccm=False, fixed_nonce=False):
    """The plaintext, or None if the tag does not verify."""
    with _Ctx(name) as c:
        if L.EVP_CipherInit_ex(c.ctx, c.cipher, None, None, None, 0) != 1:
            return None
        if nonce is not None and not fixed_nonce and not c.ctrl(SET_IVLEN, len(nonce)):
            return None
        tagbuf = ctypes.create_string_buffer(bytes(tag), max(len(tag), 1))
        # OCB takes the tag's length first and refuses a tag of another length.
        if "OCB" in name and not c.ctrl(SET_TAG, len(tag)):
            return None
        if not c.ctrl(SET_TAG, len(tag), ctypes.cast(tagbuf, vp)):
            return None
        if L.EVP_CipherInit_ex(c.ctx, None, None, key, nonce, 0) != 1:
            return None
        if ccm:
            n = ci(0)
            if L.EVP_CipherUpdate(c.ctx, None, ctypes.byref(n), None, len(ciphertext)) != 1:
                return None
        for a in aad_parts:
            if (len(a) or "SIV" in name) and not c.update(a, out=False):
                return None
        body = c.update(ciphertext) if (ciphertext or ccm or "SIV" in name) else b""
        if body is None:
            return None
        if ccm:
            return body
        tail = c.final()
        return None if tail is None else body + tail


def mac(name, key, data, params=(), size=None, xof=False):
    """A MAC through EVP_MAC; `size` reads that many bytes (KMAC)."""
    m = L.EVP_MAC_fetch(None, name.encode(), None)
    if not m:
        raise KeyError(name)
    ctx = L.EVP_MAC_CTX_new(m)
    try:
        items = list(params)
        if size is not None:
            items.append(("size", size))
        if xof:
            items.append(("xof", True))
        arr, keep = _params(items)
        if L.EVP_MAC_init(ctx, key, len(key), arr) != 1:
            return None
        if data and L.EVP_MAC_update(ctx, data, len(data)) != 1:
            return None
        cap = size or 64
        out = ctypes.create_string_buffer(cap)
        if xof:
            return out.raw[:cap] if L.EVP_MAC_finalXOF(ctx, out, cap) == 1 else None
        n = sz(0)
        if L.EVP_MAC_final(ctx, out, ctypes.byref(n), cap) != 1:
            return None
        return out.raw[:n.value]
    finally:
        L.EVP_MAC_CTX_free(ctx)
        L.EVP_MAC_free(m)
        L.ERR_clear_error()
