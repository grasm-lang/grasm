"""The country table against the tz database's own files.

`iso3166.tab` and `zone.tab` are updated several times a year and are on
every machine that can tell the time. Checking against them here means a
tzdata update shows up as a failing gate rather than as quietly stale data
in the package.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
ZONEINFO = "/usr/share/zoneinfo"

names = {}
zones = {}
country_file = os.path.join(ZONEINFO, "iso3166.tab")
zone_file = os.path.join(ZONEINFO, "zone.tab")
if not (os.path.exists(country_file) and os.path.exists(zone_file)):
    print("no tz tables on this machine; skipped")
    sys.exit(0)
for line in open(country_file, encoding="utf-8"):
    if line.startswith("#") or not line.strip():
        continue
    code, name = line.rstrip("\n").split("\t", 1)
    names[code] = name
for line in open(zone_file, encoding="utf-8"):
    if line.startswith("#") or not line.strip():
        continue
    parts = line.rstrip("\n").split("\t")
    zones.setdefault(parts[0], []).append(parts[2])

ran = subprocess.run([GRASM, "run", "packages/isotest/src/main.gr"],
                     cwd=ROOT, capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
mine = {}
for line in ran.stdout.split("\n"):
    if not line.strip():
        continue
    code, name, listed = line.rstrip("\n").split("\t")
    mine[code] = (name, listed.split(",") if listed else [])

agree = differ = shown = 0
for code in sorted(names):
    want = (names[code], zones.get(code, []))
    if mine.get(code) == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {code}: tzdata {want} grasm {mine.get(code)}")
unexpected = sorted(set(mine) - set(names))
if unexpected:
    differ += len(unexpected)
    print(f"  codes not in iso3166.tab: {unexpected[:8]}")
print(f"countries: {len(names)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
