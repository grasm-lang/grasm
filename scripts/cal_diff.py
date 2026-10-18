"""Calendars against ICU, through Node's Intl.DateTimeFormat.

Every calendar converts batches of day numbers -- random ones across
three thousand years, and runs of consecutive days around the hard
places: year ends, leap months, the Umm al-Qura table's edges, era
changes, the 1582 Gregorian changeover -- and each date has to agree with
ICU's era, year, month and day, and convert back to the same day.
Hebrew months are compared by ICU's English names, Chinese ones with
ICU's "bis" for a leap month.
"""
import json
import os
import random
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="caldiff")
harness = os.path.join(work, "caltest")
built = subprocess.run([GRASM, "build", "packages/caltest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

WIDE = (-900000, 220000)       # about 434 to 2572
RANGES = {"chinese": (-25478, 47491), "dangi": (-25478, 47491), "japanese": (-470000, 200000)}
HARD = {"islamic-umalqura": [-31826, 74809, 74838, 74839], "japanese": [-141427, 18017, 6947, -483000 + 500],
        "buddhist": [-141427, -141428], "roc": [-21184, -21185], "chinese": [19403, 19432, 19462],
        "dangi": [19403, 19432, 19462], "hebrew": [19800, 19830, 20160, 20520], "persian": [20166, 20531, 20897]}
# Every Rosh Hashanah in the tested range, by the textbook's own formula
# (Calendrical Calculations): the Hebrew year start is where an
# implementation of its postponement rules goes wrong, and only in a few
# years in two hundred.


def hebrew_elapsed(y):
    months = (235 * y - 234) // 19
    parts = 12084 + 13753 * months
    day = 29 * months + parts // 25920
    return day + 1 if (3 * (day + 1)) % 7 < 3 else day


def rosh_hashanah(y):
    ny0, ny1, ny2 = hebrew_elapsed(y - 1), hebrew_elapsed(y), hebrew_elapsed(y + 1)
    corr = 2 if ny2 - ny1 == 356 else 1 if ny1 - ny0 == 382 else 0
    return -1373427 + hebrew_elapsed(y) + corr - 719163


HARD["hebrew"] += [rosh_hashanah(y) + k for y in range(4200, 6330) for k in (-1, 0, 1)]
CALS = ["coptic", "ethiopic", "ethioaa", "persian", "indian", "islamic-civil", "islamic-tbla", "islamic-umalqura",
        "hebrew", "japanese", "buddhist", "roc", "chinese", "dangi"]
batches = []
for cal in CALS:
    lo, hi = RANGES.get(cal, WIDE)
    for _ in range(12):
        days = [rng.randrange(lo, hi) for _ in range(60)]
        start = rng.randrange(lo, hi - 400)
        days += list(range(start, start + 400, rng.choice([1, 1, 3])))
        batches.append((cal, days))
    hard = HARD.get(cal, [])
    if hard:
        near = sorted({x for h in hard for x in (range(h - 40, h + 40) if len(hard) < 50 else [h]) if lo <= x < hi})
        for k in range(0, len(near), 1500):
            batches.append((cal, near[k:k + 1500]))

ran = subprocess.run(["node", "scripts/lib/node/cal.js"], input=json.dumps(batches), capture_output=True, text=True,
                     cwd=ROOT, check=True)
want = json.loads(ran.stdout)
argv = []
for cal, days in batches:
    argv += [cal, ",".join(map(str, days))]
lines, start = [], 0
while start < len(argv):
    out = subprocess.run([harness] + argv[start:start + 40], capture_output=True, text=True, timeout=300)
    if out.returncode != 0:
        print(out.stderr[-2000:]); sys.exit(1)
    lines += out.stdout.split("\n")[:-1]
    start += 40
families, agree, differ, shown = {}, 0, 0, 0
for (cal, days), mine, theirs in zip(batches, lines, want):
    fam = families.setdefault(cal, [0, 0])
    for d, m, t in zip(days, mine.split(","), theirs.split(",")):
        if m == t:
            agree += 1
            fam[0] += 1
        else:
            differ += 1
            fam[1] += 1
            if shown < 12:
                shown += 1
                print(f"  {cal} day {d}: grasm {m!r} icu {t!r}")
if len(lines) != len(batches):
    print("the harness answered", len(lines), "of", len(batches)); sys.exit(1)
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 1000]
if thin or len(families) < len(CALS):
    print(f"too few dates to mean anything: {thin}"); sys.exit(1)
print(f"dates: {agree + differ}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
