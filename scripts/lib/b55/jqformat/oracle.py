# The oracle for packages/jqformat: jq 1.7.1's @-formats. A line is
# "jq FORMAT HEX-OF-JSON"; jq reads each value from the JSON text itself, so
# number literals reach it as written. The answer is the result in base64,
# or E: and the error message in base64.
import subprocess
import sys

lines = [l.split()[1:] for l in sys.stdin if l.strip()]
feed = "".join('{"f":"%s","v":%s}\n' % (f, bytes.fromhex(h).decode("utf-8")) for f, h in lines)
prog = '.f as $f | try (.v | format($f) | @base64) catch ("E:" + (. | tostring | @base64))'
out = subprocess.run(["jq", "-r", prog], input=feed, capture_output=True, text=True).stdout
sys.stdout.write(out)
