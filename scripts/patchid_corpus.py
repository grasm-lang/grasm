"""Diffs from this repository's own history, for the patch-id gate."""
import os
import shutil
import subprocess
import tempfile


def run(*args):
    return subprocess.run(args, capture_output=True, text=True).stdout


def build(root):
    """Write each diff to a file and return the arguments and expectations.

    Diffs are passed by path because a real one runs to megabytes and does
    not fit in an argument list.
    """
    commits = run("git", "log", "--format=%H", "-40").split()
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
        diffs.append(("the same hunk moved", base.replace("@@ -1,",
                                                          "@@ -101,")))
        diffs.append(("with a commit message in front",
                      "subject\n\nbody\n\n" + base))
    argv, expected, labels = [], [], []
    for n, (label, text) in enumerate(diffs):
        path = os.path.join(root, "diff-%03d.patch" % n)
        open(path, "w").write(text)
        ran = subprocess.run(["git", "patch-id"], input=text,
                             capture_output=True, text=True)
        want = ran.stdout.split()[0] if ran.stdout.split() else "?"
        argv += ["id", path]
        expected.append(want)
        labels.append(label)
    return argv, expected, labels


def workspace():
    return tempfile.mkdtemp()


def clean(root):
    shutil.rmtree(root, ignore_errors=True)
