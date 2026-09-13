"""Signatures made by `ssh-keygen -Y sign`, for the signature gate."""
import os
import shutil
import subprocess
import tempfile

MESSAGES = [("plain", "hello world\n"), ("empty", ""),
            ("binary", "".join(chr(c) for c in range(1, 127))),
            ("long", "x" * 5000)]
KINDS = ["ed25519", "rsa", "ecdsa"]
SIGNATURE_ALGORITHM = {"ed25519": "ssh-ed25519", "rsa": "rsa-sha2-512",
                       "ecdsa": "ecdsa-sha2-nistp256"}


def build(root):
    argv, expected, labels = [], [], []
    for kind in KINDS:
        subprocess.run(["ssh-keygen", "-q", "-t", kind, "-N", "", "-C",
                        "signer@example", "-f", os.path.join(root, "k_" + kind)],
                       capture_output=True)
    for name, text in MESSAGES:
        path = os.path.join(root, "m_" + name)
        open(path, "w").write(text)
        for kind in KINDS:
            for namespace in ("file", "git"):
                ran = subprocess.run(
                    ["ssh-keygen", "-Y", "sign", "-f",
                     os.path.join(root, "k_" + kind), "-n", namespace,
                     "-O", "hashalg=sha512", path],
                    capture_output=True, text=True)
                made = path + ".sig"
                if ran.returncode != 0 or not os.path.exists(made):
                    continue
                signature = os.path.join(
                    root, "s_%s_%s_%s.sig" % (name, kind, namespace))
                os.replace(made, signature)
                public = open(os.path.join(root, "k_" + kind + ".pub")
                              ).read().split()
                listed = subprocess.run(
                    ["ssh-keygen", "-lf",
                     os.path.join(root, "k_" + kind + ".pub")],
                    capture_output=True, text=True).stdout.split()
                # Only ed25519 is verified here; the rest are parsed and
                # reported rather than claimed.
                verdict = "yes" if kind == "ed25519" else "no"
                argv += ["sig", signature, path]
                expected.append("%s sha512 %s %s %s %s" % (
                    namespace, public[0], SIGNATURE_ALGORITHM[kind],
                    listed[1], verdict))
                labels.append("%s signature over %s in %s"
                              % (kind, name, namespace))
    # A good signature over the wrong message, and something that is not one.
    good = os.path.join(root, "s_plain_ed25519_file.sig")
    if os.path.exists(good):
        listed = subprocess.run(
            ["ssh-keygen", "-lf", os.path.join(root, "k_ed25519.pub")],
            capture_output=True, text=True).stdout.split()
        argv += ["sig", good, os.path.join(root, "m_long")]
        expected.append("file sha512 ssh-ed25519 ssh-ed25519 %s no"
                        % listed[1])
        labels.append("a good signature over the wrong message")
    argv += ["sig", ":" + b"not a signature".hex(),
             os.path.join(root, "m_plain")]
    expected.append("?refused?")
    labels.append("something that is not a signature")
    return argv, expected, labels


def workspace():
    return tempfile.mkdtemp()


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
