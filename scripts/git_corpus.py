"""Indexes, attributes and mailmaps for the git gate, built by git itself."""
import os
import shutil
import subprocess
import tempfile


def run(cwd, *args):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


ATTRIBUTES = """\
# what this repository says about its own files
*.txt text
*.bin -text
*.md text=auto
*.png binary
build/* -diff
!never
[attr]binary -diff -merge -text
*.generated linguist-generated
docs/**/*.txt !text
"""

ATTR_PATHS = ["a.txt", "a.bin", "a.md", "a.png", "build/x.o", "other",
              "docs/deep/a.txt", "x.generated", "deep/dir/a.txt"]
ATTR_NAMES = ["text", "diff", "merge", "binary", "linguist-generated"]

MAILMAP = """\
# the canonical names
Proper Name <proper@example.com>
<new@example.com> <old@example.com>
Both Fixed <both@example.com> <wrong@example.com>
Shared One <one@example.com> Machine Account <shared@example.com>
Shared Two <two@example.com> Other Person <shared@example.com>
"""

MAILMAP_LINES = [
    "Proper Name <proper@example.com>",
    "Anything <proper@example.com>",
    "Someone <old@example.com>",
    "Someone <OLD@EXAMPLE.COM>",
    "Someone <wrong@example.com>",
    "Machine Account <shared@example.com>",
    "Other Person <shared@example.com>",
    "Unknown Person <shared@example.com>",
    "Nobody <nobody@example.com>",
]


def build_repositories(root):
    """One repository per index version, plus one mid-merge."""
    made = []
    for version in (2, 3, 4):
        path = os.path.join(root, "v%d" % version)
        os.makedirs(os.path.join(path, "deep/nested/dir"))
        run(path, "git", "init", "-q")
        run(path, "git", "config", "index.version", str(version))
        run(path, "git", "config", "user.email", "t@example.com")
        run(path, "git", "config", "user.name", "T")
        open(os.path.join(path, "a.txt"), "w").write("hello\n")
        open(os.path.join(path, "b binary.bin"), "wb").write(bytes(range(64)))
        open(os.path.join(path, "deep/nested/dir/c.txt"), "w").write("deep\n")
        open(os.path.join(path, "deep/nested/d.txt"), "w").write("also\n")
        open(os.path.join(path, "unicode-café.txt"), "w").write("x\n")
        os.symlink("a.txt", os.path.join(path, "link"))
        os.chmod(os.path.join(path, "a.txt"), 0o755)
        run(path, "git", "add", "-A")
        made.append(("index version %d" % version, path))
    # A repository stopped in the middle of a conflicted merge, so its index
    # holds three entries for one path.
    path = os.path.join(root, "conflict")
    os.makedirs(path)
    run(path, "git", "init", "-q")
    run(path, "git", "config", "user.email", "t@example.com")
    run(path, "git", "config", "user.name", "T")
    open(os.path.join(path, "f.txt"), "w").write("base\n")
    run(path, "git", "add", "-A")
    run(path, "git", "commit", "-qm", "base")
    run(path, "git", "checkout", "-qb", "other")
    open(os.path.join(path, "f.txt"), "w").write("theirs\n")
    run(path, "git", "commit", "-qam", "theirs")
    run(path, "git", "checkout", "-q", "-")
    open(os.path.join(path, "f.txt"), "w").write("ours\n")
    run(path, "git", "commit", "-qam", "ours")
    run(path, "git", "merge", "other")
    made.append(("index with a conflict", path))
    return made


def build():
    root = tempfile.mkdtemp()
    argv, expected, labels = [], [], []

    def add(args, want, label):
        argv.extend(args)
        expected.append(want)
        labels.append(label)

    for label, path in build_repositories(root):
        data = open(os.path.join(path, ".git", "index"), "rb").read()
        stage = run(path, "git", "ls-files", "--stage").stdout.rstrip("\n")
        add(["stage", ":" + data.hex()], stage.encode().hex(), label)
        add(["checksum", ":" + data.hex()], "yes", label + " checksum")
        damaged = bytearray(data)
        damaged[20] ^= 0xff
        add(["checksum", ":" + bytes(damaged).hex()], "no",
            label + " damaged")
        paths = set(line.split("\t", 1)[1] for line in stage.split("\n")
                    if line)
        add(["count", ":" + data.hex()], str(len(paths)), label + " count")
    add(["stage", ":" + b"not an index at all".hex()], "?refused?",
        "not an index")
    add(["stage", ":" + b"DIRC\x00\x00\x00\x09\x00\x00\x00\x01".hex()],
        "?refused?", "unknown index version")

    # Attributes, against `git check-attr` in a repository holding the file.
    attr_repo = os.path.join(root, "attrs")
    os.makedirs(os.path.join(attr_repo, "docs/deep"))
    os.makedirs(os.path.join(attr_repo, "build"))
    os.makedirs(os.path.join(attr_repo, "deep/dir"))
    run(attr_repo, "git", "init", "-q")
    open(os.path.join(attr_repo, ".gitattributes"), "w").write(ATTRIBUTES)
    for name in ATTR_NAMES:
        for path in ATTR_PATHS:
            got = run(attr_repo, "git", "check-attr", name, "--",
                      path).stdout.rstrip("\n")
            want = got.rsplit(": ", 1)[1] if ": " in got else "unspecified"
            add(["attr", ":" + ATTRIBUTES.encode().hex(),
                 ":" + path.encode().hex(), ":" + name.encode().hex()],
                want.encode().hex(), f"check-attr {name} {path}")

    # Mailmap, against `git check-mailmap`.
    map_repo = os.path.join(root, "mailmap")
    os.makedirs(map_repo)
    run(map_repo, "git", "init", "-q")
    open(os.path.join(map_repo, ".mailmap"), "w").write(MAILMAP)
    for line in MAILMAP_LINES:
        got = run(map_repo, "git", "check-mailmap", line).stdout.rstrip("\n")
        if not got:
            continue
        add(["mailmap", ":" + MAILMAP.encode().hex(),
             ":" + line.encode().hex()], got.encode().hex(),
            f"check-mailmap {line!r}")
    return argv, expected, labels, root


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
