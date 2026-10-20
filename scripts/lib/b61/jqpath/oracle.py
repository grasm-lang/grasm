"""jq 1.7.1's path builtins, one case per line: an operation and up to three
JSON texts (hex), all run through one jq process. Each answer is the JSON
[0, result] or [1, error message]:
  get A P      A | getpath(P)            set A P V   A | setpath(P; V)
  del A PS     A | delpaths(PS)          paths A     [A | paths]
  pathsf A S   [A | paths(S)] for a selector name S (a JSON string)
  te A         A | to_entries            fe A        A | from_entries
  pick A PS    A | pick(PS[] as $p | getpath($p))
  ts A         [A | tostream]            fs E        [fromstream(E[])]
  tr E N       [N | truncate_stream(E[])]
The JSON texts go to jq as they are, so number literals reach it intact."""
import subprocess
import sys

PROGRAM = r'''
.op as $op | .a as $a | .b as $b | .c as $c |
try (
  if $op == "get" then [0, ($a | getpath($b))]
  elif $op == "set" then [0, ($a | setpath($b; $c))]
  elif $op == "del" then [0, ($a | delpaths($b))]
  elif $op == "paths" then [0, [$a | paths]]
  elif $op == "pathsf" then [0, [$a | if $b == "values" then paths(values) elif $b == "nulls" then paths(nulls)
      elif $b == "booleans" then paths(booleans) elif $b == "numbers" then paths(numbers) elif $b == "strings" then paths(strings)
      elif $b == "arrays" then paths(arrays) elif $b == "objects" then paths(objects) elif $b == "iterables" then paths(iterables)
      else paths(scalars) end]]
  elif $op == "te" then [0, ($a | to_entries)]
  elif $op == "fe" then [0, ($a | from_entries)]
  elif $op == "pick" then [0, ($a | pick($b[] as $p | getpath($p)))]
  elif $op == "ts" then [0, [$a | tostream]]
  elif $op == "fs" then [0, [fromstream($a[])]]
  else [0, [$b | truncate_stream($a[])]] end
) catch [1, .]
'''

lines = sys.stdin.read().split("\n")
if lines and lines[-1] == "":
    lines.pop()


def jq(parts):
    r = subprocess.run(["jq", "-c", PROGRAM], input="\n".join(parts) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(f"jq exited with {r.returncode}: {r.stderr[-2000:]}")
        sys.exit(1)
    out = r.stdout.split("\n")[:-1]
    if len(out) != len(parts):
        sys.stderr.write("jq answered %d of %d" % (len(out), len(parts)))
        sys.exit(1)
    return out


# jq 1.7.1's jv_dels frees a slice key twice when its bounds are not numbers
# (jv_aux.c: parse_slice has consumed it, then the error path frees it), and
# the damage crashes jq some cases later. A deletion with a slice in its
# paths therefore runs in a jq of its own.
parts, alone = [], []
for line in lines:
    f = line.split(" ")
    args = [bytes.fromhex(h).decode() for h in f[1:]] + ["null"] * 3
    parts.append('{"op":"%s","a":%s,"b":%s,"c":%s}' % (f[0], args[0], args[1], args[2]))
    alone.append(f[0] == "del" and "{" in args[1])
answers = [None] * len(parts)
shared = [k for k in range(len(parts)) if not alone[k]]
for k, a in zip(shared, jq([parts[k] for k in shared])):
    answers[k] = a
for k in range(len(parts)):
    if alone[k]:
        answers[k] = jq([parts[k]])[0]
sys.stdout.write("".join(a + "\n" for a in answers))
