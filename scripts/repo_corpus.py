"""Refs, reflogs, submodules and dates, from repositories git made itself."""
import os
import random
import shutil
import subprocess
import tempfile

MODULES = """\
[submodule "lib"]
\tpath = vendor/lib
\turl = https://example.com/lib.git
\tbranch = main
[submodule "tools"]
\tpath = tools
\turl = git@example.com:tools.git
\tshallow = true
[submodule "escaping"]
\tpath = ../../.git/hooks
\turl = https://example.com/evil.git
[submodule "optionurl"]
\tpath = fine
\turl = --upload-pack=touch /tmp/x
"""


def run(cwd, *args):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def build(root):
    argv, expected, labels = [], [], []

    def add(args, want, label):
        argv.extend(args)
        expected.append(want)
        labels.append(label)

    repository = os.path.join(root, "repo")
    os.makedirs(repository)
    run(repository, "git", "init", "-q")
    run(repository, "git", "config", "user.email", "t@example.com")
    run(repository, "git", "config", "user.name", "T Tester")
    open(os.path.join(repository, "f"), "w").write("one\n")
    run(repository, "git", "add", "f")
    run(repository, "git", "commit", "-qm", "first")
    run(repository, "git", "tag", "light")
    run(repository, "git", "tag", "-a", "annotated", "-m", "a tag")
    run(repository, "git", "branch", "other")
    open(os.path.join(repository, "f"), "w").write("two\n")
    run(repository, "git", "commit", "-qam", "second")
    run(repository, "git", "checkout", "-q", "other")
    run(repository, "git", "checkout", "-q", "-")
    run(repository, "git", "pack-refs", "--all")

    packed = os.path.join(repository, ".git", "packed-refs")
    if os.path.exists(packed):
        want = run(repository, "git", "show-ref").stdout.rstrip("\n")
        add(["showref", packed], want.encode().hex(),
            "packed refs against show-ref")

    log = os.path.join(repository, ".git", "logs", "HEAD")
    if os.path.exists(log):
        printed = run(repository, "git", "reflog", "--format=%H %gs").stdout
        # `git reflog` prints newest first and the file is oldest first, and
        # the message is prose after the action word.
        want = []
        for line in printed.rstrip("\n").split("\n"):
            sha, _, message = line.partition(" ")
            want.append(sha + " " + message.split(":")[0])
        want.reverse()
        add(["reflog", log], "\n".join(want).encode().hex(),
            "reflog against git reflog")
        # And written back byte for byte.
        add(["reflogwrite", log],
            open(log).read().rstrip("\n").encode().hex(),
            "reflog written back unchanged")

    modules = os.path.join(root, "dotgitmodules")
    open(modules, "w").write(MODULES)
    listed = run(root, "git", "config", "-f", modules, "--list").stdout
    add(["modules", modules], listed.rstrip("\n").encode().hex(),
        "submodules against git config --list")
    add(["modulesafe", modules],
        "lib=safe,tools=safe,escaping=unsafe,optionurl=unsafe"
        .encode().hex(), "which submodule declarations are safe")

    # Abbreviations, against `git rev-parse --short`, which picks the
    # shortest prefix unique in *this* repository.
    ids = run(repository, "git", "rev-list", "--objects", "--all").stdout
    known = []
    for line in ids.rstrip("\n").split("\n"):
        if line:
            known.append(line.split(" ")[0])
    if known:
        listing = os.path.join(root, "objects")
        open(listing, "w").write("\n".join(known) + "\n")
        for full in known:
            want = run(repository, "git", "rev-parse", "--short",
                       full).stdout.strip()
            if not want:
                continue
            add(["abbrev", listing, ":" + full.encode().hex()], want,
                f"abbreviation of {full[:12]}")

    # Dates: every format git prints, over commits made at chosen instants
    # in chosen zones.
    random.seed(31)
    stamps = [(1789298603, "+0400"), (0, "+0000"), (1, "-0930"),
              (946684800, "+0530"), (2147483647, "+1400"),
              (1234567890, "-0500"), (1700000000, "+0000")]
    for _ in range(20):
        stamps.append((random.randrange(0, 2 ** 31),
                       "%s%02d%02d" % (random.choice("+-"),
                                       random.randrange(0, 15),
                                       random.choice([0, 15, 30, 45]))))
    dates = os.path.join(root, "dates")
    os.makedirs(dates)
    run(dates, "git", "init", "-q")
    run(dates, "git", "config", "user.email", "t@example.com")
    run(dates, "git", "config", "user.name", "T")
    for n, (seconds, offset) in enumerate(stamps):
        open(os.path.join(dates, "f"), "w").write(str(n))
        run(dates, "git", "add", "f")
        stamp = "%d %s" % (seconds, offset)
        made = subprocess.run(
            ["git", "commit", "-qm", "x"], cwd=dates, capture_output=True,
            env={**os.environ, "GIT_AUTHOR_DATE": stamp,
                 "GIT_COMMITTER_DATE": stamp})
        if made.returncode != 0:
            continue
        for command, fmt in (("iso", "iso"), ("isostrict", "iso-strict"),
                             ("rfc", "rfc"), ("short", "short"),
                             ("raw", "raw"), ("unix", "unix")):
            printed = run(dates, "git", "log", "-1", "--date=" + fmt,
                          "--format=%ad").stdout.rstrip("\n")
            add([command, ":" + b"-".hex(), ":" + stamp.encode().hex()],
                printed, f"{fmt} of {stamp}")
    return argv, expected, labels


def workspace():
    return tempfile.mkdtemp()


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
