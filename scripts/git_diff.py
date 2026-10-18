"""git's index formats and pretty formats against git.

Repositories are built with `git commit-tree`, so every message, date and
time zone is exactly what the test chose: messages with leading blank
lines, multi-line subjects, trailing white space and CRLF; dates beyond
2038 (commit-graph keeps 34 bits); commits dated before their parents (so
corrected commit dates differ from commit times); octopus merges (the
commit-graph's EDGE chunk). Objects go into several packs by repeated
`git repack -d`, then a multi-pack-index and a commit-graph are written.

- `packidx` against `git show-index`, on git's version 2 indexes, version
  1 indexes and version 2 with every offset forced into the 64-bit table
  (`git index-pack --index-version=...`); and indexes it writes, read back
  by `git show-index`.
- `packrev` against the order of offsets `git show-index` reports, and
  byte for byte against the .rev git wrote.
- `midx` against each pack's own index.
- `commitgraph` against `git log`: parents, trees, commit times, and
  topological levels and corrected dates computed from them.
- `gitlogfmt` against `git log -1 --format`.
"""
import os
import random
import re
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="gitdiff")
harness = os.path.join(work, "gittest")
built = subprocess.run([GRASM, "build", "packages/gittest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
US = "\x1f"
cases = []
bases = []
ENV = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null", LC_ALL="C", TZ="UTC")


def git(repo, *args, data=None, env=None):
    out = subprocess.run(["git", "-C", repo, *args], input=data, capture_output=True, env=env or ENV)
    if out.returncode != 0:
        raise SystemExit(f"git {' '.join(args)}: {out.stderr.decode()[-500:]}")
    return out.stdout


def show_index(data):
    out = subprocess.run(["git", "show-index"], input=data, capture_output=True, env=ENV)
    if out.returncode != 0:
        return None
    entries = []
    for line in out.stdout.decode().splitlines():
        parts = line.split()
        crc = int(parts[2].strip("()"), 16) if len(parts) > 2 else -1
        entries.append((parts[1], int(parts[0]), crc))
    return sorted(entries)


MESSAGES = ["subject\n", "subject\n\nbody line\n", "\n\nleading blank\n\nbody\n", "multi\nline subject\n\nbody",
            "trailing spaces   \n  \nbody\n", "no newline", "subject\n\n\n\nbody after blanks\n",
            "crlf subject\r\n\r\nbody\r\n", "Ünïcödé subject\n", "a..b...c..txt -- [x] (y)\n",
            "  indented subject\n\n  indented body\n", "", "\n", "subject\n\nbody\n\n", "Fix #12: it's /done/!\n\nSigned-off-by: A <a@b>\n"]
ZONES = ["+0000", "+0530", "-0800", "+1245", "-0930", "+1400", "-1200", "+0100"]


def stamp(base, width):
    return f"@{base + rng.randrange(0, width)} {rng.choice(ZONES)}"


def build_repo(path, wide):
    # A narrow repository keeps every date within 3 * 2^30 of the others, so
    # corrected-date offsets stay below 2^32 (some above 2^31, in GDO2); its
    # base is 0 or 2^32, the latter for commit times needing all 34 bits.
    # A wide one spans 2^33 and meets git's own overflow defect.
    base, width = (0, 1 << 33) if wide else (rng.choice([0, 1 << 32]), 3 << 30)
    bases.append(base)
    git(path, "init", "-q")
    commits, parents_of = [], {}
    tree = None
    for k in range(rng.randrange(4, 18)):
        with open(os.path.join(path, f"f{k % 5}"), "w") as fh:
            fh.write(f"{k} {rng.random()}\n")
        git(path, "add", "-A")
        tree = git(path, "write-tree").decode().strip()
        parents = []
        if commits:
            parents = [commits[-1]] if rng.random() < 0.7 else []
            if rng.random() < 0.25 and len(commits) >= 2:
                parents = rng.sample(commits, min(len(commits), rng.randrange(2, 5)))
        env = dict(ENV, GIT_AUTHOR_NAME=rng.choice(["A U Thor", "Zoë", "x"]),
                   GIT_AUTHOR_EMAIL=rng.choice(["a@example.com", "first.last@host.org", "noat"]),
                   GIT_COMMITTER_NAME="C O Mitter", GIT_COMMITTER_EMAIL="c@example.com",
                   GIT_AUTHOR_DATE=stamp(base, width), GIT_COMMITTER_DATE=stamp(base, width))
        args = ["commit-tree", tree]
        for p in parents:
            args += ["-p", p]
        oid = git(path, *args, "-F", "-", data=rng.choice(MESSAGES).encode(), env=env).decode().strip()
        commits.append(oid)
        git(path, "update-ref", f"refs/heads/b{k}", oid)
        if rng.random() < 0.35:
            git(path, "repack", "-q", "-d")
    git(path, "repack", "-q", "-d")
    return commits


def hexf(p):
    with open(p, "rb") as fh:
        return fh.read().hex()


for r in range(12):
    repo = os.path.join(work, f"r{r}")
    os.makedirs(repo)
    wide = r >= 10
    commits = build_repo(repo, wide)
    packdir = os.path.join(repo, ".git/objects/pack")
    git(repo, "multi-pack-index", "write")
    git(repo, "commit-graph", "write", "--reachable", *(["--changed-paths"] if r % 2 else []))
    idxs = sorted(f for f in os.listdir(packdir) if f.endswith(".idx"))
    per_pack = {}
    for name in idxs:
        data = open(os.path.join(packdir, name), "rb").read()
        entries = show_index(data)
        per_pack[name] = entries
        cases.append(("idx", data.hex(), "-", "v2" + US + ",".join(f"{n}:{o}:{c}" for n, o, c in entries)))
        pack = os.path.join(packdir, name[:-4] + ".pack")
        for version, label in [("1", "v1"), ("2,0", "v2")]:
            out = os.path.join(work, f"tmp-{r}-{label}.idx")
            subprocess.run(["git", "index-pack", f"--index-version={version}", "-o", out, pack], capture_output=True,
                           env=ENV, check=True)
            alt = open(out, "rb").read()
            ents = show_index(alt)
            crcs = [(n, o, -1 if version == "1" else c) for n, o, c in ents]
            cases.append(("idx", alt.hex(), "-", label + US + ",".join(f"{n}:{o}:{c}" for n, o, c in crcs)))
        rev = os.path.join(packdir, name[:-4] + ".rev")
        if os.path.exists(rev):
            offsets = [o for _, o, _ in entries]
            order = sorted(range(len(entries)), key=lambda i: entries[i][1])
            cases.append(("rev", hexf(rev), ",".join(map(str, offsets)), ",".join(map(str, order)) + US + "true"))
        spec = [(n, rng.choice([o, o + (1 << 31) + rng.randrange(1 << 33)]), c) for n, o, c in entries]
        rng.shuffle(spec)
        checksum = bytes(rng.getrandbits(8) for _ in range(20)).hex()
        cases.append(("idxw", ",".join(f"{n}:{o}:{c}" for n, o, c in spec), checksum,
                      ("SHOWINDEX", sorted(spec))))
    packs_sorted = sorted(idxs)
    objects = []
    for pid, name in enumerate(packs_sorted):
        for n, o, _ in per_pack[name]:
            objects.append((n, pid, o))
    objects.sort()
    cases.append(("midx", hexf(os.path.join(packdir, "multi-pack-index")), "-",
                  ",".join(packs_sorted) + US + ",".join(f"{n}:{p}:{o}" for n, p, o in objects) + US))
    log = git(repo, "log", "--all", "--format=%H %T %ct %P").decode().split("\n")
    info = {}
    for line in log:
        if line:
            f = line.split(" ")
            info[f[0]] = (f[1], int(f[2]), f[3:] if len(f) > 3 and f[3] else [])
    level, corrected, true_corrected = {}, {}, {}

    def lev(c):
        if c not in level:
            ps = info[c][2]
            level[c] = 1 + max((lev(p) for p in ps), default=0)
            # What git 2.48 stores: an offset of 2^32 or more keeps only its
            # low word, and children are computed from what was kept (git
            # commit-graph verify then complains, see cgv).
            ideal = max([info[c][1]] + [cor(p) + 1 for p in ps])
            true_corrected[c] = ideal
            off = ideal - info[c][1]
            corrected[c] = info[c][1] + (off & 0xFFFFFFFF if off > 0x7FFFFFFF else off)
        return level[c]

    def cor(c):
        lev(c)
        return corrected[c]

    def cor_true(c):
        lev(c)
        return true_corrected[c]

    sys.setrecursionlimit(10000)
    want = ",".join(f"{c}:{info[c][0]}:{' '.join(info[c][2])}:{lev(c)}:{info[c][1]}:{cor(c)}" for c in sorted(info))
    if not wide:
        # Above 2^32 git's own corrected dates go wrong (git commit-graph
        # verify agrees, see cgv); there the corrected field is not compared.
        cases.append(("cg", hexf(os.path.join(repo, ".git/objects/info/commit-graph")), "-",
                      ("NOCORR", want) if bases[-1] else want))
    verify = subprocess.run(["git", "-C", repo, "commit-graph", "verify"], capture_output=True, text=True, env=ENV)
    complaints = re.findall(r"commit-graph generation for commit (\w+) is (\d+) < (\d+)", verify.stderr)
    cases.append(("cgv", hexf(os.path.join(repo, ".git/objects/info/commit-graph")), "-",
                  ",".join(f"{o}:{h}:{w}" for o, h, w in complaints)))
    formats = ["%H %h %T %t %P %p", "%an <%ae> %al", "%ad|%aD|%at|%ai|%aI|%as", "%cn %ce %cl %cd %cD %ct %ci %cI %cs",
               "%s", "%f", "%b", "%B", "%%%n%x41%x7a", "%q %Z %", "[%s]%n[%b]", "%aN %aE %aL", "%e", "%h%x", "%ax"]
    for c in commits:
        raw = git(repo, "cat-file", "commit", c)
        for fmt in rng.sample(formats, 5):
            out = git(repo, "log", "-1", f"--format=tformat:{fmt}", c)
            if out.endswith(b"\n"):
                out = out[:-1]
            cases.append(("log", raw.hex(), c + US + fmt.encode().hex(), out.hex()))


def run(picked):
    argv = []
    for op, a, b, _ in picked:
        argv += [op, a, b]
    lines, start = [], 0
    while start < len(argv):
        chunk, size = [], 0
        while start < len(argv) and (size < 200000 or not chunk):
            chunk += argv[start:start + 3]
            size += sum(len(x) for x in argv[start:start + 3])
            start += 3
        try:
            out = subprocess.run([harness] + chunk, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("the harness did not finish in 300 s"); sys.exit(1)
        if out.returncode != 0:
            print(out.stderr[-2000:]); sys.exit(1)
        lines += out.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, a, b, want) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    if isinstance(want, tuple) and want[0] == "NOCORR":
        strip = lambda t: ",".join(e.rsplit(":", 1)[0] for e in t.split(","))
        ok = strip(mine) == strip(want[1])
        shown = want[1]
    elif isinstance(want, tuple):
        back = show_index(bytes.fromhex(mine)) if mine and mine != "<missing>" else None
        ok = back == want[1]
        shown = f"show-index read {back[:3] if back else back}"
    else:
        ok = mine == want
        shown = want
    fam = families.setdefault(op, [0, 0])
    if ok:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 8:
            print(f"  {op} {b[:120]!r}")
            if op == "log":
                print(f"    grasm {bytes.fromhex(mine).decode(errors='replace')[:200]!r}")
                print(f"    git   {bytes.fromhex(want).decode(errors='replace')[:200]!r}")
            else:
                print(f"    grasm {mine[:300]}")
                print(f"    want  {str(shown)[:300]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < (10 if k in ("midx", "cg", "cgv") else 30)]
if thin or len(families) < 7:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
# The hard paths have to have happened: an overflow offset (GDO2) and an
# octopus merge (EDGE) in a fully compared commit-graph.
fully = [a for op, a, _, w in cases if op == "cg" and not isinstance(w, tuple)]
gdo2 = sum(1 for a in fully if "47444f32" in a[:200])
edge = sum(1 for a in fully if "45444745" in a[:200])
if not gdo2 or not edge:
    print(f"no compared commit-graph had GDO2 ({gdo2}) or EDGE ({edge}) chunks")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}   graphs with GDO2: {gdo2}, with EDGE: {edge}")
sys.exit(1 if differ else 0)
