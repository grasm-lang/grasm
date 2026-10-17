"""Diffs from this repository's own history, for the patch-id gate.

Everything here is **bytes**, not text. A diff is not guaranteed to be UTF-8:
`git show` of a commit that touches a file git does not treat as text emits
those bytes, and this corpus is built from this repository's own last forty
commits, so committing one binary fixture is enough. That is exactly what
happened -- the image and compression fixtures added this week made
`subprocess.run(..., text=True)` raise `UnicodeDecodeError: 'utf-8' codec
can't decode byte 0xc8 in position 36864`, and the gate failed on its own
harness rather than on anything Grasm did.

Reading the real history is worth keeping: a corpus of hand-written diffs
would not have the shapes that break a patch-id. The cost is that the corpus
moves when the history does, which is why nothing here may assume the
content is text.
"""
import os
import shutil
import subprocess
import tempfile


def run(*args):
    return subprocess.run(args, capture_output=True).stdout


def build(root):
    """Write each diff to a file and return the arguments and expectations.

    Diffs are passed by path because a real one runs to megabytes and does
    not fit in an argument list.
    """
    # Only the commit ids are decoded; they are hex.
    commits = run("git", "log", "--format=%H", "-40").decode().split()
    diffs = []
    for commit in commits:
        text = run("git", "show", "--format=", "--no-color", commit)
        if text.strip():
            diffs.append((f"show {commit[:8]}", text))
    for newer, older in zip(commits, commits[3:]):
        text = run("git", "diff", "--no-color", older, newer)
        if text.strip():
            diffs.append((f"diff {older[:8]}..{newer[:8]}", text))
    if diffs:
        base = diffs[0][1]
        # The same change at a different offset, and with a message in
        # front: both must hash the same as the original.
        diffs.append(("the same hunk moved", base.replace(b"@@ -1,",
                                                          b"@@ -101,")))
        diffs.append(("with a commit message in front",
                      b"subject\n\nbody\n\n" + base))
    argv, expected, labels = [], [], []
    # A diff that is not valid UTF-8 cannot be hashed by a Grasm program
    # today: `bytes.to_str()` replaces each invalid byte with U+FFFD without
    # saying so, so `patchid` is handed a different diff from the one git
    # read. See docs/known-issues.md. These are counted and reported rather
    # than dropped, so the limitation cannot grow without anyone noticing.
    skipped = []
    for n, (label, text) in enumerate(diffs):
        try:
            text.decode("utf-8")
        except UnicodeDecodeError:
            skipped.append(label)
            continue
        path = os.path.join(root, "diff-%03d.patch" % n)
        open(path, "wb").write(text)
        ran = subprocess.run(["git", "patch-id"], input=text,
                             capture_output=True)
        # The id is hex, so only it is decoded.
        fields = ran.stdout.decode("ascii", "replace").split()
        want = fields[0] if fields else "?"
        argv += ["id", path]
        expected.append(want)
        labels.append(label)
    return argv, expected, labels, skipped


def workspace():
    return tempfile.mkdtemp()


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
