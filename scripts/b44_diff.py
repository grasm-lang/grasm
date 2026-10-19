"""OpenType and AAT font tables against fontTools, on the fonts macOS ships.

CFF outlines (name-keyed and CID-keyed, every charstring operator the
system's fonts use), GPOS positioning lookups, GDEF glyph classes,
carets and mark sets, item variation stores with HVAR, VVAR and MVAR,
gvar tuple variations (point counts measured from glyf), trak tracking,
feat features, meta and ltag, vmtx and VORG, and BASE baselines -- each
read here from the raw table and compared line for line with what
fontTools makes of the same table.
"""
import glob
import os
import random
import shutil
import subprocess
import sys
import tempfile

from fontTools.ttLib import TTCollection, TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b44")
harness = os.path.join(work, "b44test")
built = subprocess.run([GRASM, "build", "packages/b44test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
ORACLE = os.path.join(ROOT, "scripts/lib/b44/oracle.py")
cases = []
unreadable = []

# Every font (and the first two faces of each collection), with its tables.
fonts = []
for p in sorted(glob.glob("/System/Library/Fonts/**/*.tt[fc]", recursive=True) + glob.glob("/System/Library/Fonts/**/*.otf", recursive=True)):
    try:
        faces = TTCollection(p, lazy=True).fonts[:2] if p.lower().endswith(".ttc") else [TTFont(p, lazy=True)]
    except Exception:
        continue
    for i, f in enumerate(faces):
        fonts.append((p, i, set(f.keys()), f["maxp"].numGlyphs if "maxp" in f else 0))


def having(*tags, small=None):
    return [f for f in fonts if all(t in f[2] for t in tags) and (small is None or f[3] <= small)]


def add(fam, op, font, spec=False):
    path, index, _tags, glyphs = font
    d = tempfile.mkdtemp(dir=work)
    r = subprocess.run([sys.executable, ORACLE, op, path, str(index), d], capture_output=True, text=True, timeout=1800)
    if r.returncode != 0:
        # A table fontTools cannot read gives nothing to compare with.
        unreadable.append(f"{os.path.basename(path)}#{index} {op}: {r.stderr.strip().splitlines()[-1][:120]}")
        return
    want = r.stdout
    target = os.path.join(d, "spec") if spec else os.path.join(d, sorted(x for x in os.listdir(d))[0])

    def check(got, want=want, target=target, label=f"{os.path.basename(path)}#{index}"):
        if got != "ok":
            return False, label, got
        mine = open(target + ".out").read()
        if mine == want:
            return True, "", ""
        a, b = mine.split("\n"), want.split("\n")
        k = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
        return False, f"{label} line {k}: {(b[k] if k < len(b) else '<end>')[:200]}", (a[k] if k < len(a) else "<end>")[:200]
    cases.append((op, target, check, fam))


def sample(pool, n):
    return rng.sample(pool, min(n, len(pool)))


for f in sample(having("CFF "), 9):
    add("cff", f"cff:{max(1, f[3] // 120)}", f)
for f in sample(having("GPOS"), 10):
    add("gpos", "gpos", f)
for f in sample(having("GDEF"), 10):
    add("gdef", "gdef", f)
var = [(f, t) for f in fonts for t in ("HVAR", "VVAR", "MVAR") if t in f[2]]
for f, t in rng.sample(var, min(12, len(var))):
    add("varstore", f"var:{t}:{f[3]}", f)
for f in sample(having("gvar", "glyf", "fvar", small=3000), 8):
    add("gvar", "gvar", f, spec=True)
for f in sample(having("trak"), 8):
    add("trak", "trak", f)
for f in sample(having("feat"), 10):
    add("feat", "feat", f)
meta_pool = having("ltag") + sample([f for f in having("meta") if "ltag" not in f[2]], 6)
for f in sample(meta_pool, 10):
    add("meta", "meta", f, spec=True)
for f in sample(having("vhea", "vmtx"), 8):
    add("vertical", f"vertical:{f[3]}", f, spec=True)
for f in sample(having("BASE"), 8):
    add("base", "base", f)


def finish():
    answers = []
    for i in range(0, len(cases), 50):
        argv = []
        for op, path, _, _ in cases[i:i + 50]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])
    if len(answers) != len(cases):
        print(f"{len(answers)} answers for {len(cases)} cases"); sys.exit(1)
    families, differ = {}, 0
    for (op, path, check, fam), got in zip(cases, answers):
        try:
            ok, want, mine = check(got)
        except Exception as e:
            ok, want, mine = False, "an answer the reference can read", f"{type(e).__name__}: {str(e)[:200]}"
        f = families.setdefault(fam, [0, 0])
        if ok:
            f[0] += 1
        else:
            f[1] += 1
            differ += 1
            if differ <= 10:
                print(f"  {fam} {op}")
                print(f"    want  {want}")
                print(f"    grasm {mine}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad:
        print("disagreeing:", " ".join(bad), f"(SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    for u in unreadable:
        print("left out, fontTools cannot read it: " + u)
    thin = [k for k, (a, d) in families.items() if a + d < 8]
    if thin or len(families) < int(os.environ.get("FAMILIES", "10")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
