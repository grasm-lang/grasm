"""RubyGems versions against Ruby's own Gem::Version.

The system Ruby ships RubyGems 3.0, whose comparison predates
`canonical_segments` and answers differently; the current
`lib/rubygems/version.rb` is vendored beside this script and loaded over
it, so the oracle is the RubyGems people run today.
"""
import json
import os
import random
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/gemtest/src/main.gr")
VENDORED = os.path.join(ROOT, "scripts/fixtures/rubygems-version.rb")

if not shutil.which("ruby"):
    print("pairs: 0   agreeing: 0   skipped: ruby is not installed")
    sys.exit(0)
random.seed(293)
PIECES = ["1", "0", "2", "10", "01", "a", "b", "pre", "beta", "rc",
          ".", "-", "1a", "0", "9"]
def version():
    out = "1"
    for _ in range(random.randint(0, 5)):
        out += random.choice(PIECES)
    return out
KNOWN = ["1.0", "1.0.0", "1", "1.0.0.0", "1.0.a", "1.0.0.a", "1.0.0.a.0",
         "1.0.pre", "1.0-pre", "1.0.pre.1", "1.0.0.rc1", "2.0", "1.9",
         "1.10", "1.0.0.beta.2", "1.0.0.beta.10", "0", "1.0.0-1",
         "1.0.0.pre.1", "1.0.a1", "1.0.1a"]
cases = []
for a in KNOWN:
    for b in KNOWN:
        cases.append((a, b))
cases += [(version(), version()) for _ in range(1200)]
argv = []
for a, b in cases:
    argv += [a, b]
lines = []
STEP = 800
for at in range(0, len(argv), STEP * 2):
    chunk = argv[at:at + STEP * 2]
    got = subprocess.run([GRASM, "run", HARNESS, "--"] + chunk,
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    lines += got.stdout.split("\n")[:len(chunk) // 2]
script = r'''
require "rubygems"
load ARGV[0]
require "json"
STDIN.each_line do |line|
  a, b = JSON.parse(line)
  begin
    va = Gem::Version.new(a)
    vb = Gem::Version.new(b)
    puts "#{va <=> vb} #{va.prerelease? ? "pre" : "rel"} #{va.release}"
  rescue ArgumentError
    puts "error"
  end
end
'''
payload = "\n".join(json.dumps([a, b]) for a, b in cases)
ruby = subprocess.run(["ruby", "-W0", "-e", script, VENDORED],
                      input=payload, capture_output=True, text=True)
if ruby.returncode != 0:
    print(ruby.stderr[-2000:]); sys.exit(1)
want = ruby.stdout.split("\n")
agree = differ = shown = 0
for i, (a, b) in enumerate(cases):
    if lines[i] == want[i]:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 10:
            print(f"  {a!r} vs {b!r}: ruby {want[i]!r}, grasm {lines[i]!r}")
print(f"pairs: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
