"""Cases for unixtest, with the answers of the tools themselves.

`build()` returns (argv, expected, labels). Every command is three words: a
name and two hex arguments, each prefixed with `:` so an empty one survives.

`cksum` and `sum` read files written here; `openssl enc` encrypts and
decrypts; `ssh-keygen -s` issues certificates and `ssh-keygen -L` lists
them; `infocmp -1` prints every entry in the system's terminfo directory.
The files stay until the driver exits, because the harness reads them.
"""
import atexit
import concurrent.futures
import os
import random
import shutil
import subprocess
import tempfile

WORK = tempfile.mkdtemp(prefix="unixwire_")
atexit.register(shutil.rmtree, WORK, True)
ENV = dict(os.environ, TZ="UTC", LC_ALL="C")


def h(text):
    return ":" + text.encode("utf-8").hex()


def hb(raw):
    return ":" + raw.hex()


def th(text):
    return text.encode("utf-8").hex()


class Cases:
    def __init__(self):
        self.argv, self.expected, self.labels = [], [], []

    def add(self, what, a, b, want, raw_a=None):
        self.argv += [what, hb(raw_a) if raw_a is not None else h(a), h(b)]
        self.expected.append(want)
        self.labels.append(f"{what} {(raw_a if raw_a is not None else a)!r:.100} {b!r:.100}")


def parallel(fn, items):
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
        return list(pool.map(fn, items))


def run(args, data=None, cwd=WORK):
    return subprocess.run(args, input=data, capture_output=True, env=ENV, cwd=cwd)


def checksums(c, rng):
    sizes = list(range(0, 40)) + [255, 256, 257, 511, 512, 513, 1023, 1024,
                                  1025, 1535, 1536, 4096, 65535, 65536, 65537,
                                  100000, 1048576]
    cases = []
    for n, size in enumerate(sizes):
        path = os.path.join(WORK, "file%d.bin" % n)
        kind = n % 3
        if kind == 0:
            data = bytes(rng.getrandbits(8) for _ in range(size))
        elif kind == 1:
            data = bytes([255]) * size
        else:
            data = bytes(i % 256 for i in range(size))
        open(path, "wb").write(data)
        for algorithm in [0, 1, 2, 3, 9]:
            cases.append((path, algorithm))

    def answer(case):
        path, algorithm = case
        if algorithm == 9:
            args = ["sum", path]
        elif algorithm == 0:
            args = ["cksum", path]
        else:
            args = ["cksum", "-o", str(algorithm), path]
        return run(args).stdout.decode().rstrip("\n")

    for (path, algorithm), want in zip(cases, parallel(answer, cases)):
        c.add("ck", path, str(algorithm), th(want))


CIPHERS = ["aes-128-cbc", "aes-192-cbc", "aes-256-cbc", "aes-128-ecb",
           "aes-192-ecb", "aes-256-ecb", "aes-128-ctr", "aes-192-ctr", "aes-256-ctr"]


def encryption(c, rng):
    cases = []
    for _ in range(700):
        cipher = rng.choice(CIPHERS)
        size = rng.choice([0, 1, 15, 16, 17, 31, 32, 33, 64, rng.randint(0, 100)])
        data = bytes(rng.getrandbits(8) for _ in range(size))
        password = rng.choice(["secret", "pässwörd", "a", "with space", "x" * 70])
        mode = rng.choice(["pbkdf2", "pbkdf2", "legacy", "raw"])
        digest = rng.choice(["sha256", "sha256", "sha1", "md5"])
        iterations = rng.choice([1, 2, 17, 100]) if rng.random() < 0.95 else 10000
        nosalt = rng.random() < 0.2
        nopad = rng.random() < 0.15
        keylen = int(cipher[4:7]) // 8
        key = bytes(rng.getrandbits(8) for _ in range(rng.choice([keylen, keylen, keylen - 3])))
        iv = bytes(rng.getrandbits(8) for _ in range(rng.choice([16, 16, 10])))
        cases.append((cipher, data, password, mode, digest, iterations, nosalt, nopad, key, iv))

    def openssl(case, decrypt_input=None, wrong=False):
        cipher, data, password, mode, digest, iterations, nosalt, nopad, key, iv = case
        args = ["openssl", "enc", "-d" if decrypt_input is not None else "-e", "-" + cipher]
        if mode == "raw":
            args += ["-K", key.hex()]
            if not cipher.endswith("ecb"):
                args += ["-iv", iv.hex()]
        else:
            args += ["-pass", "pass:" + (password + "x" if wrong else password), "-md", digest]
            if mode == "pbkdf2":
                args += ["-pbkdf2", "-iter", str(iterations)]
            if nosalt:
                args += ["-nosalt"]
        if nopad:
            args += ["-nopad"]
        return run(args, data if decrypt_input is None else decrypt_input)

    def spec(case, salt_hex, wrong=False):
        cipher, data, password, mode, digest, iterations, nosalt, nopad, key, iv = case
        keylen = int(cipher[4:7]) // 8
        if mode == "raw":
            k = (key + bytes(keylen))[:keylen]
            v = b"" if cipher.endswith("ecb") else (iv + bytes(16))[:16]
            return ",".join([cipher, "", "", "0", "1", digest, "0" if nopad else "1",
                             k.hex(), v.hex() if v else ""])
        pw = (password + "x" if wrong else password).encode()
        return ",".join([cipher, pw.hex(), salt_hex, "1" if mode == "pbkdf2" else "0",
                         str(iterations), digest, "0" if nopad else "1", "", ""])

    def both(case):
        enc = openssl(case)
        out = []
        if enc.returncode != 0:
            return (case, None, None, None)
        sealed = enc.stdout
        salted = case[3] != "raw" and not case[6]
        salt = sealed[8:16] if salted else b""
        dec_wrong = openssl(case, decrypt_input=sealed, wrong=True) if case[3] != "raw" else None
        return (case, sealed, salt, dec_wrong)

    for case, sealed, salt, wrong in parallel(both, cases):
        cipher, data = case[0], case[1]
        if sealed is None:
            # OpenSSL refused: -nopad with a partial block.
            c.add("enc", "", spec(case, "00" * 8 if case[3] != "raw" and not case[6] else ""),
                  th("?"), raw_a=data)
            continue
        salt_hex = salt.hex()
        c.add("enc", "", spec(case, salt_hex), sealed.hex(), raw_a=data)
        c.add("dec", "", spec(case, "-" if salt else ""), data.hex(), raw_a=sealed)
        if wrong is not None:
            want = wrong.stdout.hex() if wrong.returncode == 0 else th("?")
            c.add("dec", "", spec(case, "-" if salt else "", wrong=True), want, raw_a=sealed)
        if len(sealed) > 17:
            cut = sealed[:-1]
            r = openssl(case, decrypt_input=cut)
            c.add("dec", "", spec(case, "-" if salt else ""),
                  r.stdout.hex() if r.returncode == 0 else th("?"), raw_a=cut)


def certificates(c, rng):
    d = os.path.join(WORK, "certs")
    os.makedirs(d)
    keys = []
    for name, args in [("ca_ed", ["-t", "ed25519"]), ("ca_ec", ["-t", "ecdsa", "-b", "256"]),
                       ("ca_rsa", ["-t", "rsa", "-b", "2048"]), ("ca_ec384", ["-t", "ecdsa", "-b", "384"]),
                       ("u_ed", ["-t", "ed25519"]), ("u_ec256", ["-t", "ecdsa", "-b", "256"]),
                       ("u_ec384", ["-t", "ecdsa", "-b", "384"]), ("u_ec521", ["-t", "ecdsa", "-b", "521"]),
                       ("u_rsa", ["-t", "rsa", "-b", "2048"])]:
        run(["ssh-keygen", "-q", "-N", "", "-C", name + "@host", "-f", os.path.join(d, name)] + args)
        keys.append(name)
    cas = ["ca_ed", "ca_ec", "ca_rsa", "ca_ec384"]
    users = [k for k in keys if k.startswith("u_")]
    option_sets = [[], ["-O", "no-pty"], ["-O", "clear"], ["-O", "force-command=/bin/echo hi"],
                   ["-O", "source-address=10.0.0.0/8,192.168.1.1"], ["-O", "verify-required"],
                   ["-O", "clear", "-O", "permit-pty"], ["-O", "extension:custom@example.com=val"],
                   ["-O", "critical:custom@example.com=value"], ["-O", "no-x11-forwarding", "-O", "no-user-rc"],
                   ["-O", "extension:flag@example.com"]]
    validities = ["always:forever", "20240101000000:20250101123456", "always:20300101",
                  "19700102:forever", "20240229235959:20240301000001"]
    cases = []
    for n in range(120):
        ca, user = rng.choice(cas), rng.choice(users)
        path = os.path.join(d, "c%d" % n)
        shutil.copy(os.path.join(d, user + ".pub"), path + ".pub")
        args = ["ssh-keygen", "-q", "-s", os.path.join(d, ca), "-I",
                rng.choice(["id", "alice's key", "with \"quote\"", "", "ünï"]),
                "-V", rng.choice(validities), "-z", str(rng.choice([0, 1, 42, 2 ** 32, 2 ** 64 - 1]))]
        principals = rng.sample(["alice", "bob", "root", "host.example.com", "ünï"], rng.randint(0, 3))
        if principals:
            args += ["-n", ",".join(principals)]
        if rng.random() < 0.3:
            args += ["-h"]
        else:
            args += rng.choice(option_sets)
        if ca == "ca_rsa":
            args += ["-t", rng.choice(["rsa-sha2-256", "rsa-sha2-512"])]
        r = run(args + [path + ".pub"])
        if r.returncode == 0:
            cases.append((ca, path + "-cert.pub"))

    def listing(case):
        ca, path = case
        r = run(["ssh-keygen", "-L", "-f", path])
        return r.stdout.decode() if r.returncode == 0 else "?"

    for (ca, path), want in zip(cases, parallel(listing, cases)):
        line = open(path).read()
        c.add("cert", line, path, th(want))
        c.add("certv", line, "", "unverifiable" if ca == "ca_ec384" else "yes")
        if ca == "ca_ec384":
            continue
        import base64
        kind, blob = line.split()[0], bytearray(base64.b64decode(line.split()[1]))
        for spot in [len(blob) - 5, len(blob) // 2]:
            bad = bytearray(blob)
            bad[spot] ^= 1
            bad_path = path + ".bad%d" % spot
            text = kind + " " + base64.b64encode(bytes(bad)).decode() + "\n"
            open(bad_path, "w").write(text)
            r = run(["ssh-keygen", "-L", "-f", bad_path])
            c.add("certv", text, "", "yes" if r.returncode == 0 else "no")


def terminfos(c, rng):
    root = "/usr/share/terminfo"
    entries = []
    for sub in sorted(os.listdir(root)):
        full = os.path.join(root, sub)
        if os.path.isdir(full):
            for name in sorted(os.listdir(full)):
                entries.append((sub, name))
    cases = [(e, x) for e in entries for x in ("", "x")]

    def dump(case):
        (sub, name), x = case
        args = ["infocmp", "-1"] + (["-x"] if x else []) + [name]
        r = run(args)
        return r.stdout.decode("latin-1") if r.returncode == 0 else None

    for ((sub, name), x), want in zip(cases, parallel(dump, cases)):
        if want is None or not want.startswith("#\tReconstructed via infocmp from file: " +
                                               os.path.join(root, sub, name)):
            continue
        c.add("ti", os.path.join(root, sub, name), x, want.encode("latin-1").hex())


def build():
    rng = random.Random(6464)
    c = Cases()
    checksums(c, rng)
    encryption(c, rng)
    certificates(c, rng)
    terminfos(c, rng)
    return c.argv, c.expected, c.labels
