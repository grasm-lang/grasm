"""Unix password hashes against `openssl passwd` and the sha-crypt vectors.

Built natively rather than interpreted: the published vectors run sha512crypt
at 123456 rounds, which Cranelift does in seconds and the interpreter does in
minutes.
"""
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
OPENSSL = "/usr/bin/openssl"

cases = []
skipped = 0

# `openssl passwd` is the reference for the two MD5 schemes. It is in the
# base system on macOS and on every Linux that ships OpenSSL; when it is not,
# those cases are counted as skipped rather than quietly dropped.
if os.path.exists(OPENSSL):
    for password in ("", "a", "password", "correct horse battery staple",
                     "x" * 40, "salé", "$weird$chars$"):
        for salt in ("abcdefgh", "12345678", "ab", "", "0123456789abcdef"):
            for flag in ("-1", "-apr1"):
                ran = subprocess.run([OPENSSL, "passwd", flag, "-salt", salt,
                                      password], capture_output=True,
                                     text=True)
                if ran.returncode != 0:
                    skipped += 1
                    continue
                cases.append((f"openssl {flag} {password!r} {salt!r}",
                              password, ran.stdout.strip()))
else:
    skipped += 70

# The vectors published with the sha-crypt specification, which include a
# salt longer than the maximum and a round count below the minimum, both of
# which are clamped rather than refused.
SPEC = [
    ("$5$saltstring", "Hello world!",
     "$5$saltstring$5B8vYYiY.CVt1RlTTf8KbXBH3hsxY/GNooZaBBGWEc5"),
    ("$5$rounds=10000$saltstringsaltstring", "Hello world!",
     "$5$rounds=10000$saltstringsaltst$3xv.VbSHBb41AL9AvLeujZkZRBAwqFMz2."
     "opqey6IcA"),
    ("$5$rounds=5000$toolongsaltstring", "This is just a test",
     "$5$rounds=5000$toolongsaltstrin$Un/5jzAHMgOGZ5.mWJpuVolil07guHPvOW8m"
     "GRcvxa5"),
    ("$5$rounds=1400$anotherlongsaltstring",
     "a very much longer text to encrypt.  This one even stretches over "
     "morethan one line.",
     "$5$rounds=1400$anotherlongsalts$Rx.j8H.h8HjEDGomFU8bDkXm3XIUnzyxf12o"
     "P84Bnq1"),
    ("$5$rounds=77777$short",
     "we have a short salt string but not a short password",
     "$5$rounds=77777$short$JiO1O3ZpDAxGJeaDIuqCoEFysAe1mZNJRs3pw0KQRd/"),
    ("$5$rounds=123456$asaltof16chars..", "a short string",
     "$5$rounds=123456$asaltof16chars..$gP3VQ/6X7UUEW3HkBn2w1/Ptq2jxPyzV/c"
     "ZKmF/wJvD"),
    ("$5$rounds=10$roundstoolow", "the minimum number is still observed",
     "$5$rounds=1000$roundstoolow$yfvwcWrQ8l/K0DAWyuPMDNHpIVlTQebY9l/gL972"
     "bIC"),
    ("$6$saltstring", "Hello world!",
     "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJue"
     "sI68u4OTLiBFdcbYEdFCoEOfaS35inz1"),
    ("$6$rounds=10000$saltstringsaltstring", "Hello world!",
     "$6$rounds=10000$saltstringsaltst$OW1/O6BYHV6BcXZu8QVeXbDWra3Oeqh0sbH"
     "bbMCVNSnCM/UrjmM0Dp8vOuZeHBy/YTBmSK6H9qs/y3RnOaw5v."),
    ("$6$rounds=5000$toolongsaltstring", "This is just a test",
     "$6$rounds=5000$toolongsaltstrin$lQ8jolhgVRVhY4b5pZKaysCLi0QBxGoNeKQz"
     "Q3glMhwllF7oGDZxUhx1yxdYcz/e1JSbq3y6JMxxl8audkUEm0"),
    ("$6$rounds=1400$anotherlongsaltstring",
     "a very much longer text to encrypt.  This one even stretches over "
     "morethan one line.",
     "$6$rounds=1400$anotherlongsalts$POfYwTEok97VWcjxIiSOjiykti.o/pQs.wPv"
     "MxQ6Fm7I6IoYN3CmLs66x9t0oSwbtEW7o7UmJEiDwGqd8p4ur1"),
    ("$6$rounds=77777$short",
     "we have a short salt string but not a short password",
     "$6$rounds=77777$short$WuQyW2YR.hBNpjjRhpYD/ifIw05xdfeEyQoMxIXbkvr0gg"
     "e1a1x3yRULJ5CCaUeOxFmtlcGZelFl5CxtgfiAc0"),
    ("$6$rounds=123456$asaltof16chars..", "a short string",
     "$6$rounds=123456$asaltof16chars..$BtCwjqMJGx5hrJhZywWvt0RLE8uZ4oPwce"
     "lCjmw2kSYu.Ec6ycULevoBK25fs2xXgMNrCzIMVcgEJAstJeonj1"),
    ("$6$rounds=10$roundstoolow", "the minimum number is still observed",
     "$6$rounds=1000$roundstoolow$kUMsbe306n21p9R.FRkW3IGn.S9NPN0x50YhH1xh"
     "LsPuWGsUSklZt58jaTfF4ZEQpyUNGc0dqbpBYYBaHHrsX."),
]
for setting, password, want in SPEC:
    cases.append((f"spec {setting}", password, want))

work = tempfile.mkdtemp()
harness = os.path.join(work, "uctest")
built = subprocess.run([GRASM, "build", "packages/uctest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

argv = []
for _, password, stored in cases:
    argv += ["like", ":" + password.encode().hex(), stored]
ran = subprocess.run([harness] + argv, capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")

agree = differ = shown = 0
for i, (label, _, stored) in enumerate(cases):
    line = lines[i] if i < len(lines) else "<missing>"
    hashed, _, verified = line.rpartition(" ")
    if hashed == stored and verified == "yes":
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {label}")
            print(f"    want {stored}")
            print(f"    got  {hashed} (verify {verified})")
print(f"hashes: {len(cases)}   agreeing: {agree}   differing: {differ}   "
      f"skipped: {skipped}")
sys.exit(1 if differ else 0)
