"""Cases for gitwiretest, with git's answers.

`build()` returns (argv, expected, labels). Every command is three words: a
name and two hex arguments, each prefixed with `:` so an empty one survives.
Answers are hex.

Every answer comes from running git: `check-ref-format`, `config
--get-color`, `shortlog` reading a log on its standard input, `diff --raw`
over repositories built here, and `bundle create` / `bundle list-heads`.
Git runs with no global or system configuration, outside any repository
unless one is made for it, and many at a time.
"""
import concurrent.futures
import os
import random
import shutil
import subprocess
import tempfile

WORK = tempfile.mkdtemp(prefix="gitwire_")
ENV = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1",
           HOME=WORK, GIT_CEILING_DIRECTORIES=os.path.dirname(WORK), LC_ALL="C",
           GIT_AUTHOR_NAME="A U Thor", GIT_AUTHOR_EMAIL="author@example.com",
           GIT_COMMITTER_NAME="C O Mitter", GIT_COMMITTER_EMAIL="committer@example.com",
           GIT_AUTHOR_DATE="1700000000 +0000", GIT_COMMITTER_DATE="1700000000 +0000")


def git(args, cwd=WORK, stdin=None):
    return subprocess.run(["git"] + args, cwd=cwd, env=ENV, input=stdin,
                          capture_output=True)


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
        self.labels.append(f"{what} {(raw_a if raw_a is not None else a)!r:.120} {b!r}")


def parallel(fn, items):
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
        return list(pool.map(fn, items))


# ---------------------------------------------------------------------------

def refnames(c, rng):
    alphabet = ["a", "b", ".", "/", "@", "{", "*", ":", "~", " ", "lock",
                ".lock", "é", "\t", "\\", "[", "?", "^", "-", "}"]
    names = set()
    for n in range(1, 4):
        for _ in range(3000):
            names.add("".join(rng.choice("ab./@{*") for _ in range(n)))
    while len(names) < 6000:
        names.add("".join(rng.choice(alphabet) for _ in range(rng.randint(1, 7))))
    names.update(["refs/heads/main", "main", "@", "a/@", "HEAD", "refs/heads/HEAD",
                  "refs/heads/a.lock", "a.lock/b", "refs/heads/.x", "a/b.", "a//b",
                  "/a/b", "a/b/", "refs/*/x", "refs/*/*", "a/@{b", "a/b@c", "a/.b"])
    cases = []
    for name in sorted(names):
        if name.startswith("-") or "\x00" in name:
            continue
        for flags in rng.sample(["", "o", "p", "op", "n", "on", "opn"], 3):
            cases.append((name, flags))

    def run(case):
        name, flags = case
        args = ["check-ref-format"]
        args += ["--allow-onelevel"] if "o" in flags else []
        args += ["--refspec-pattern"] if "p" in flags else []
        args += ["--normalize"] if "n" in flags else []
        r = git(args + [name])
        if r.returncode != 0:
            return "\x00?"
        return r.stdout.decode()[:-1] if "n" in flags else name

    for (name, flags), want in zip(cases, parallel(run, cases)):
        c.add("rfn", name, flags, th(want))
    branches = sorted(n for n in names if "@{" not in n and n != "@")[:3000]

    def branch(name):
        return "yes" if git(["check-ref-format", "--branch", name]).returncode == 0 else "no"

    for name, want in zip(branches, parallel(branch, branches)):
        c.add("rbr", name, "", want)


def colors(c, rng):
    words = ["red", "RED", "Blue", "brightgreen", "BrightCyan", "normal", "default",
             "reset", "RESET", "bold", "BOLD", "nobold", "no-bold", "no-", "no",
             "dim", "italic", "ul", "blink", "reverse", "strike", "noul", "no-strike",
             "0", "7", "8", "15", "16", "255", "256", "-1", "-2", "+5", "010", "0x1",
             "#ff0000", "#F0a", "#12345g", "#1234", "#abcdef0", "bright", "brightnormal",
             "black", "white", "yellow", "magenta", "99999999999999999999", "-0"]
    seps = [" ", "  ", "\t", "\n", " \r"]
    specs = set(["", "   ", "red blue green"])
    while len(specs) < 6000:
        n = rng.randint(1, 4)
        spec = rng.choice(["", " "]) + rng.choice(seps).join(
            rng.choice(words) for _ in range(n)) + rng.choice(["", " "])
        specs.add(spec)
    specs = sorted(specs)

    def run(spec):
        r = git(["config", "--get-color", "", spec])
        return r.stdout.decode() if r.returncode == 0 else "?"

    for spec, want in zip(specs, parallel(run, specs)):
        c.add("gcol", spec, "", th(want))


def mailmaps(c, rng):
    lines = ["Jane Doe <jane@example.com>", "jane doe <JANE@example.com>",
             "<new@example.com> <jane@example.com>", "Proper <p@x.com> Bob <bob@x.com>",
             "Other <o@x.com> BOB <BOB@X.COM>", "Named <n@x.com> <bob@x.com>",
             "# Jane Doe <jane@example.com>", " # not a comment <c@x.com>",
             "Hash <a#b@x.com>", "Spaced < s@x.com >", "<> <empty@x.com>",
             "Empty <e@x.com> <>", "X <x@x.com> Y", "Z <z@x.com> junk <q@x.com",
             "\tTabbed Name\t <t@x.com>", "Late Name <jane@example.com>",
             "Ünï <u@x.com> ünï <U@X.COM>", "Only <only@x.com> Bob <bob@x.com> extra"]
    contacts = ["Jane Doe <jane@example.com>", "x <JANE@EXAMPLE.COM>", "Bob <bob@x.com>",
                "bob <BOB@x.com>", "Robert <bob@x.com>", "<bob@x.com>", "a <a#b@x.com>",
                "s < s@x.com >", "s <s@x.com>", "e <empty@x.com>", "e <>", "t <t@x.com>",
                "q <x@x.com>", "ünï <U@x.com>", "ÜNÏ <u@x.com>", "c <c@x.com>",
                "Nobody <nobody@x.com>", "only <only@x.com>"]
    cases = []
    for _ in range(400):
        text = "\n".join(rng.choice(lines) for _ in range(rng.randint(0, 6))) + rng.choice(["", "\n"])
        for contact in rng.sample(contacts, 6):
            cases.append((text, contact))

    def run(case):
        text, contact = case
        d = tempfile.mkdtemp(dir=WORK)
        git(["init", "-q"], cwd=d)
        with open(os.path.join(d, ".mailmap"), "w") as f:
            f.write(text)
        r = git(["check-mailmap", contact], cwd=d)
        shutil.rmtree(d)
        return r.stdout.decode().rstrip("\n") if r.returncode == 0 else None

    for (text, contact), want in zip(cases, parallel(run, cases)):
        if want is not None:
            c.add("mmap", text, contact, th(want))


def shortlogs(c, rng):
    names = ["A U Thor", "Jane Doe", "jane doe", "Zoë Ünïcode", "Bob", " Spaced ",
             "", "O'Brien", "李雷", "Name  With  Spaces", "Tab\tName"]
    emails = ["a@example.com", "jane@example.com", "JANE@example.com", "bob@b.org",
              "", "x@y", "li@example.cn"]
    subjects = ["Fix the thing", "[PATCH] Add feature", "[PATCH v2 1/3] Refactor",
                "   leading space", "trailing space   ", "[PATCH]", "[PATCHES",
                "Merge branch 'x'", "", "tab\tinside", "\tstarts with tab",
                "[PATCH no close", "ünïcode subject"]
    mailmaps = ["", "Jane Doe <jane@example.com>\n",
                "Jane Doe <jane@example.com> <JANE@example.com>\n",
                "Proper <proper@example.com> Bob <bob@b.org>\n",
                "<new@example.com> <a@example.com>\n# comment\n",
                "A Author <a@example.com>\nJane Doe <jane@example.com> jane doe <jane@example.com>\n"]
    cases = []
    for _ in range(700):
        lines = []
        raw = rng.random() < 0.2
        for _ in range(rng.randint(0, 8)):
            name, email = rng.choice(names), rng.choice(emails)
            ident = f"{name} <{email}>"
            if rng.random() < 0.05:
                ident = name
            lines.append("commit " + "%040x" % rng.getrandbits(160))
            if rng.random() < 0.2:
                lines.append("Merge: abc def")
            if raw:
                lines.append(f"author {ident} 1700000000 +0000")
                lines.append(f"committer {rng.choice(names)} <{rng.choice(emails)}> 1700000000 +0000")
            else:
                lines.append(f"Author: {ident}")
                if rng.random() < 0.5:
                    lines.append(f"Commit: {rng.choice(names)} <{rng.choice(emails)}>")
                lines.append("Date:   Tue Nov 14 22:13:20 2023 +0000")
            lines.append("")
            if rng.random() < 0.9:
                for _ in range(rng.randint(1, 2)):
                    lines.append("    " + rng.choice(subjects))
                if rng.random() < 0.3:
                    lines.append("")
                    lines.append("    body line")
            lines.append("")
        log = "\n".join(lines) + rng.choice(["", "\n"])
        mailmap = rng.choice(mailmaps)
        flags = "".join(f for f in "snec" if rng.random() < 0.4)
        cases.append((log, mailmap, flags))

    def run(case):
        log, mailmap, flags = case
        d = tempfile.mkdtemp(dir=WORK)
        if mailmap:
            with open(os.path.join(d, ".mailmap"), "w") as f:
                f.write(mailmap)
        args = ["shortlog"] + ["-" + f for f in flags]
        r = git(args, cwd=d, stdin=log.encode())
        shutil.rmtree(d)
        return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else "?"

    for (log, mailmap, flags), want in zip(cases, parallel(run, cases)):
        c.add("slog", log, mailmap + "\x02" + flags, th(want))


FILE_NAMES = ["a", "b c", "tab\tname", "new\nline", 'quo"te', "back\\slash",
              "résumé.txt", "中文", "dir/sub/file", "#hash", "ctl\x01x", "del\x7fx",
              "dir/other", "z", "long name with spaces.txt", "é/ü"]


def content(rng, lines=20):
    return "".join("line %d %d\n" % (i, rng.randint(0, 3)) for i in range(lines))


def make_change(repo, rng):
    files = [f for f in git(["ls-files", "-z"], cwd=repo).stdout.decode().split("\0") if f]
    for _ in range(rng.randint(1, 5)):
        op = rng.choice(["add", "modify", "delete", "rename", "copy", "chmod", "symlink"])
        if op == "add" or not files:
            name = rng.choice(FILE_NAMES) + rng.choice(["", "2", "_new"])
            path = os.path.join(repo, name)
            if os.path.isdir(path):
                continue
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(content(rng))
            files.append(name)
            continue
        name = rng.choice(files)
        path = os.path.join(repo, name)
        if not os.path.lexists(path):
            continue
        if op == "modify" and not os.path.islink(path):
            with open(path, "a") as f:
                f.write("more %d\n" % rng.randint(0, 9))
        elif op == "delete":
            os.remove(path)
            files.remove(name)
        elif op in ("rename", "copy") and not os.path.islink(path):
            target = name + ("_moved" if op == "rename" else "_copy")
            if os.path.lexists(os.path.join(repo, target)):
                continue
            shutil.copy(path, os.path.join(repo, target))
            if op == "rename":
                os.remove(path)
                files.remove(name)
            with open(os.path.join(repo, target), "a") as f:
                f.write("x\n")
            files.append(target)
        elif op == "chmod" and not os.path.islink(path):
            os.chmod(path, 0o755)
        elif op == "symlink":
            os.remove(path)
            os.symlink("target", path)
    git(["add", "-A"], cwd=repo)
    git(["commit", "-q", "--allow-empty", "-m", "change"], cwd=repo)


def z_canonical(data):
    fields = data.split(b"\0")
    out, at = [], 0
    while at < len(fields) and fields[at]:
        header = fields[at].decode()
        colons = len(header) - len(header.lstrip(":"))
        parts = header[colons:].split(" ")
        status = parts[-1]
        score = -1
        if colons == 1 and len(status) > 1:
            status, score = status[0], int(status[1:])
        path = fields[at + 1].decode()
        at += 2
        new = ""
        if status in ("R", "C"):
            new = fields[at].decode()
            at += 1
        out.append(",".join(parts[:colons]) + " " + parts[colons] + " " +
                   ",".join(parts[colons + 1:2 * colons + 1]) + " " + parts[2 * colons + 1] +
                   " " + status + " " + str(score) + "\x02" + path + "\x02" + new)
    return "\x01".join(out)


def rawdiffs(c, rng):
    variants = [["--no-abbrev", "-M", "-C"], ["-M", "-C", "--find-copies-harder"],
                ["--no-abbrev", "-B", "-M"], ["--abbrev=12"], ["--no-abbrev"],
                ["--no-abbrev", "-M30%", "-C30%"]]
    for n in range(50):
        repo = os.path.join(WORK, "raw%d" % n)
        git(["init", "-q", repo])
        make_change(repo, rng)
        for _ in range(rng.randint(1, 3)):
            make_change(repo, rng)
        count = int(git(["rev-list", "--count", "HEAD"], cwd=repo).stdout)
        for v in rng.sample(variants, 3):
            base = "HEAD~%d" % rng.randint(1, count - 1) if count > 1 else "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            text = git(["diff", "--raw"] + v + [base, "HEAD"], cwd=repo).stdout
            zdata = git(["diff", "--raw", "-z"] + v + [base, "HEAD"], cwd=repo).stdout
            want = th(z_canonical(zdata))
            c.add("rawp", text.decode(), "", want)
            c.add("rawz", "", "", want, raw_a=zdata)
            c.add("rawr", text.decode(), "", text.hex())
    # A merge, for the combined form.
    for n in range(6):
        repo = os.path.join(WORK, "merge%d" % n)
        git(["init", "-q", "-b", "main", repo])
        with open(os.path.join(repo, "shared"), "w") as f:
            f.write(content(rng))
        with open(os.path.join(repo, "résumé"), "w") as f:
            f.write(content(rng))
        git(["add", "-A"], cwd=repo)
        git(["commit", "-q", "-m", "base"], cwd=repo)
        git(["checkout", "-q", "-b", "side"], cwd=repo)
        for name in ["shared", "résumé"]:
            with open(os.path.join(repo, name), "a") as f:
                f.write("side\n")
        git(["commit", "-q", "-am", "side"], cwd=repo)
        git(["checkout", "-q", "main"], cwd=repo)
        with open(os.path.join(repo, "shared"), "w") as f:
            f.write("main\n" + content(rng))
        git(["commit", "-q", "-am", "main"], cwd=repo)
        git(["merge", "-q", "-m", "merge", "side"], cwd=repo)
        for name in ["shared", "résumé"]:
            with open(os.path.join(repo, name), "a") as f:
                f.write("resolved\n")
        git(["add", "-A"], cwd=repo)
        git(["commit", "-q", "-m", "merge", "--allow-empty"], cwd=repo)
        text = git(["diff-tree", "-c", "--raw", "--no-abbrev", "-r", "HEAD"], cwd=repo).stdout
        zdata = git(["diff-tree", "-c", "--raw", "--no-abbrev", "-r", "-z", "HEAD"], cwd=repo).stdout
        # diff-tree prints the commit id first; the raw lines follow.
        text = b"\n".join(l for l in text.split(b"\n") if l.startswith(b":")) + b"\n"
        zdata = zdata[zdata.index(b"\0") + 1:] if not zdata.startswith(b":") else zdata
        if text.strip():
            want = th(z_canonical(zdata))
            c.add("rawp", text.decode(), "", want)
            c.add("rawz", "", "", want, raw_a=zdata)
            c.add("rawr", text.decode(), "", text.hex())
    for bad in [":100644 100644 abc def M", ":100644 abc def M\tx", "no colon\tx",
                ":100644 100644 abc def R\tonly-one", ':100644 100644 abc def M\t"\\q"',
                ':100644 100644 abc def Mx\tx', ':100644 100644 abc def M\t"unterminated']:
        c.add("rawp", bad, "", th("?"))


def bundles(c, rng):
    for n in range(24):
        fmt = "sha256" if n % 3 == 2 else "sha1"
        repo = os.path.join(WORK, "bundle%d" % n)
        git(["init", "-q", "-b", "main", "--object-format=" + fmt, repo])
        for i in range(rng.randint(3, 6)):
            with open(os.path.join(repo, "f%d" % i), "w") as f:
                f.write(content(rng))
            git(["add", "-A"], cwd=repo)
            git(["commit", "-q", "-m", rng.choice(["subject %d" % i, "ünï subject", "tab\there"])], cwd=repo)
        git(["tag", "v1", "HEAD~1"], cwd=repo)
        git(["branch", "other", "HEAD~2"], cwd=repo)
        specs = [["--all"], ["HEAD~2..main"], ["main", "^HEAD~1", "v1"], ["--branches"],
                 ["--version=3", "--all"], ["--filter=blob:none", "--all"]]
        for spec in specs:
            path = os.path.join(WORK, "b%d_%d.bundle" % (n, specs.index(spec)))
            r = git(["bundle", "create", "-q", path] + spec, cwd=repo)
            if r.returncode != 0:
                continue
            data = open(path, "rb").read()
            offset = data.index(b"\n\nPACK") + 2
            header = data[:offset].decode()
            lines = header.split("\n")
            version = 3 if lines[0] == "# v3 git bundle" else 2
            filt = ""
            prereqs = []
            for line in lines[1:]:
                if line.startswith("@filter="):
                    filt = line[8:]
                if line.startswith("-"):
                    oid = line[1:].split(" ")[0]
                    subject = git(["log", "-1", "--format=%s", oid], cwd=repo).stdout.decode().rstrip("\n")
                    prereqs.append(oid + " " + subject)
            heads = git(["bundle", "list-heads", path], cwd=WORK).stdout.decode().rstrip("\n").split("\n")
            want = "\x02".join([str(version), fmt, filt, "\x01".join(prereqs),
                                "\x01".join(x for x in heads if x), str(offset)])
            c.add("bndl", "", "", th(want), raw_a=data)
            c.add("bndw", "", "", data[:offset].hex(), raw_a=data)
            mutations = [data.replace(b"# v2", b"# v4", 1), data.replace(b"\n\nPACK", b"\n \nPACK", 1),
                         data.replace(b"refs/heads/main", b"refs/heads/main   ", 1),
                         data.replace(b"# v3 git bundle\n", b"# v3 git bundle\n@unknown=1\n", 1),
                         data.replace(b"# v2 git bundle\n", b"# v2 git bundle\n@object-format=sha1\n", 1),
                         data[:offset - 1], data[:20], b"",
                         data.replace(b"\n-", b"\n-zz", 1)]
            first_oid = header.split("\n")[1].lstrip("-@")[:10]
            mutations.append(data.replace(first_oid.encode(), first_oid.upper().encode(), 1))
            for m in mutations:
                mpath = path + ".m"
                open(mpath, "wb").write(m)
                r = git(["bundle", "list-heads", mpath], cwd=WORK)
                want = r.stdout.decode().rstrip("\n").replace("\n", "\x01") if r.returncode == 0 else "?"
                c.add("bnlh", "", "", th(want), raw_a=m)


def build():
    rng = random.Random(4242)
    c = Cases()
    refnames(c, rng)
    colors(c, rng)
    mailmaps(c, rng)
    shortlogs(c, rng)
    rawdiffs(c, rng)
    bundles(c, rng)
    shutil.rmtree(WORK, ignore_errors=True)
    return c.argv, c.expected, c.labels
