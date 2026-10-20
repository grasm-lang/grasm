"""Writes packages/unicodeblock's tables from Java 17's Character.UnicodeBlock
(scripts/lib/b47/java/Blocks.java and BlockNames.java), and prints the table
lines so the gate can check the package still carries them."""
import collections, os, subprocess, sys
JAVA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "java")
subprocess.run(["javac", "-d", JAVA, os.path.join(JAVA, "Blocks.java"), os.path.join(JAVA, "BlockNames.java")], check=True)
ranges = [l.split() for l in subprocess.run(["java", "-cp", JAVA, "Blocks"], capture_output=True, text=True, check=True).stdout.splitlines()]
names = [l.split() for l in subprocess.run(["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", JAVA, "BlockNames"],
                                           capture_output=True, text=True, check=True).stdout.splitlines()]
spaced = collections.defaultdict(list)
consts = []
for key, block in names:
    if block not in consts:
        consts.append(block)
    if "~" in key:
        spaced[block].append(key.replace("~", " "))
derived = set()
for b in consts:
    derived.add(b)
    for s in spaced[b]:
        derived.add(s); derived.add(s.replace(" ", ""))
extras = [(k.replace("~", " "), b) for k, b in names if k.replace("~", " ") not in derived]
lines = []
lines.append("    let blocks = [" + ", ".join(
    f'("{b}", [{", ".join(chr(34) + s + chr(34) for s in sorted(spaced[b]))}])' for b in consts) + "]")
lines.append("    let ranges = [" + ", ".join(f"(0x{a}, 0x{z}, \"{n}\")" for a, z, n in ranges) + "]")
lines.append("    let extras = [" + ", ".join(f'("{k}", "{b}")' for k, b in extras) + "]")
if "--print" in sys.argv:
    print("\n".join(lines))
    sys.exit(0)
out = sys.argv[1]
src = open(out).read()
start = src.index("    // BEGIN TABLES")
end = src.index("    // END TABLES")
src = src[:start] + "    // BEGIN TABLES (scripts/lib/b47/gen_blocks.py, from Java 17)\n" + "\n".join(lines) + "\n" + src[end:]
open(out, "w").write(src)
