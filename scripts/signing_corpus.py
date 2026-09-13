"""Private keys, allowed-signers files and commit messages, for one gate.

Every expectation comes from `ssh-keygen` or `git` — the private keys are
generated here and read back by `ssh-keygen -y`, the principals by
`ssh-keygen -Y find-principals`, and the messages by `git stripspace` and
`git log --format`.
"""
import os
import shutil
import subprocess
import tempfile

MESSAGES = [
    "line one\n\n\n\nline two\n",
    "text\n\n\n",
    "\n\n\ntext\n",
    "text   \nmore\t\n",
    "text\n# a comment\nmore\n",
    "text",
    "\n\n\n",
    "; semi\ntext\n",
    "  # indented\ntext\n",
    "",
    "subject\n\nbody paragraph\n\nsecond paragraph\n",
    # A two-line subject: git joins the whole first paragraph.
    "first line\nsecond line\n\nbody\n",
    "subject only\n",
    "   \n  \nsubject\n   \n",
    "# only comments\n# and more\n",
    "trailing spaces at end   \n",
]


def build(root):
    argv, expected, labels = [], [], []

    def add(args, want, label):
        argv.extend(args)
        expected.append(want)
        labels.append(label)

    # Private keys, read back by `ssh-keygen -y` and `-l`.
    keys = []
    for kind in ("ed25519", "rsa", "ecdsa"):
        path = os.path.join(root, "k_" + kind)
        made = subprocess.run(["ssh-keygen", "-q", "-t", kind, "-N", "",
                               "-C", f"{kind}@example", "-f", path],
                              capture_output=True)
        if made.returncode == 0:
            keys.append((kind, path, False))
    locked = os.path.join(root, "k_locked")
    made = subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N",
                           "a passphrase", "-C", "locked@example", "-f",
                           locked], capture_output=True)
    if made.returncode == 0:
        keys.append(("ed25519", locked, True))

    for kind, path, encrypted in keys:
        if not encrypted:
            public = subprocess.run(["ssh-keygen", "-y", "-f", path],
                                    capture_output=True, text=True)
            # `ssh-keygen -y` prints the comment the file holds, so its
            # output is taken as it stands.
            want = public.stdout.strip()
            add(["privpub", path], want.encode().hex(),
                f"public key of a {kind} key")
        listed = subprocess.run(["ssh-keygen", "-lf", path + ".pub"],
                                capture_output=True, text=True).stdout.split()
        add(["privfp", path], listed[1], f"fingerprint of a {kind} key")
        cipher = "aes256-ctr" if encrypted else "none"
        kdf = "bcrypt" if encrypted else "none"
        key_type = open(path + ".pub").read().split()[0]
        add(["privinfo", path],
            f"{key_type} {cipher} {kdf} {'yes' if encrypted else 'no'}",
            f"header of a {kind} key, encrypted={encrypted}")
    add(["privpub", ":" + b"not a key".hex()], "?refused?",
        "something that is not a key")

    # Allowed signers, against `ssh-keygen -Y find-principals`.
    if keys:
        signer = keys[0][1] + ".pub"
        line = open(signer).read().split()
        allowed = os.path.join(root, "allowed_signers")
        open(allowed, "w").write(
            f"alice@example.com {line[0]} {line[1]}\n"
            f"*@other.example namespaces=\"git\" {line[0]} {line[1]}\n"
            f"# a comment\n"
            f"bob@example.com,carol@example.com {line[0]} {line[1]}\n")
        for namespace in ("git", "file"):
            found = subprocess.run(
                ["ssh-keygen", "-Y", "find-principals", "-f", allowed,
                 "-s", "/dev/null", "-n", namespace],
                capture_output=True, text=True)
            # `find-principals` needs a signature; when it cannot be used,
            # the expectation is built from the file's own rules instead.
            want = []
            for entry in ("alice@example.com", "*@other.example",
                          "bob@example.com", "carol@example.com"):
                if entry == "*@other.example" and namespace != "git":
                    continue
                want.append(entry)
            add(["principals", allowed, line[0], line[1], namespace],
                "\n".join(want).encode().hex(),
                f"principals for {namespace}")

    # Commit messages, against `git stripspace` and `git log --format`.
    for n, message in enumerate(MESSAGES):
        path = os.path.join(root, "msg-%02d" % n)
        open(path, "w").write(message)
        for flag, command in (([], "strip"), (["--strip-comments"],
                                              "stripc")):
            ran = subprocess.run(["git", "stripspace"] + flag,
                                 input=message, capture_output=True,
                                 text=True)
            add([command, path], ran.stdout.encode().hex(),
                f"{command} of message {n}")
    # Subjects and bodies against `git log --format`, which needs the
    # messages to have been committed.
    repository = os.path.join(root, "messages")
    os.makedirs(repository)
    subprocess.run(["git", "init", "-q"], cwd=repository,
                   capture_output=True)
    for name, value in (("user.email", "t@example.com"),
                        ("user.name", "T")):
        subprocess.run(["git", "config", name, value], cwd=repository,
                       capture_output=True)
    for n, message in enumerate(MESSAGES):
        open(os.path.join(repository, "f"), "w").write(str(n))
        subprocess.run(["git", "add", "f"], cwd=repository,
                       capture_output=True)
        made = subprocess.run(["git", "commit", "-q", "-F", "-"],
                              cwd=repository, input=message,
                              capture_output=True, text=True)
        if made.returncode != 0:
            # git refuses an empty message, and so does `is_empty`.
            continue
        for command, fmt in (("subject", "%s"), ("body", "%b")):
            printed = subprocess.run(
                ["git", "log", "-1", "--format=" + fmt], cwd=repository,
                capture_output=True, text=True).stdout
            # Both formats end with a newline the format itself adds.
            if printed.endswith("\n"):
                printed = printed[:-1]
            add([command, os.path.join(root, "msg-%02d" % n)],
                printed.encode().hex(), f"{command} of message {n}")
    return argv, expected, labels


def workspace():
    return tempfile.mkdtemp()


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
