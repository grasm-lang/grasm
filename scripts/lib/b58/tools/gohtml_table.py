"""Print Go 1.25's html entity table (src/html/entity.go) as Grasm functions
returning packed "name:hex[,hex]" strings, for packages/gohtml.
usage: python3 gohtml_table.py /path/to/go/src/html/entity.go"""
import re
import sys

# Go comments out the two names whose replacement is wider than the name
# ("nLt;" and "nGt;"); only live lines count.
src = "\n".join(l for l in open(sys.argv[1]).read().splitlines() if not l.strip().startswith("//"))
entries = []
for m in re.finditer(r'"([^"]+)":\s*\'\\U([0-9A-Fa-f]{8})\'', src):
    entries.append((m.group(1), [int(m.group(2), 16)]))
for m in re.finditer(r'"([^"]+)":\s*\{\'\\u([0-9A-Fa-f]{4})\',\s*\'\\u([0-9A-Fa-f]{4})\'\}', src):
    entries.append((m.group(1), [int(m.group(2), 16), int(m.group(3), 16)]))
entries.sort()
items = [f"{n}:{','.join('%x' % c for c in cs)}" for n, cs in entries]
groups = [items[i:i + 450] for i in range(0, len(items), 450)]
out = []
for gi, g in enumerate(groups):
    lines = []
    cur = ""
    for it in g:
        if len(cur) + len(it) + 1 > 100:
            lines.append(cur)
            cur = ""
        cur += it + " "
    lines.append(cur)
    body = " +\n".join(('    return "' if k == 0 else '        "') + l + '"' for k, l in enumerate(lines))
    out.append(f"fn entity_group_{gi}() -> str {{\n{body}\n}}\n")
print(f"// {len(entries)} names from Go 1.25's src/html/entity.go.\n")
print("\n".join(out))
print("fn entity_text() -> str {\n    return " + " + ".join(f"entity_group_{i}()" for i in range(len(groups))) + "\n}")
