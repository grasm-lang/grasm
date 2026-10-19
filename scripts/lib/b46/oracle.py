"""The Python half of scripts/b46_diff.py: python3.13 oracle.py <op>
<case file>...; one line per case file, as packages/b46test answers.

`entry`: importlib.metadata's entry points. `reqfile`: pip's own
requirements-file parser, each line serialised as pip reads it (the
shared `--use-feature` list is part of what it acts on)."""
import contextlib
import io
import json
import os
import sys

sys.argv[0] = "pip"
from importlib.metadata import EntryPoints  # noqa: E402

h = lambda s: s.encode().hex()
FIELDS = ["config_settings", "constraints", "editables", "extra_index_urls", "features_enabled", "find_links", "global_options", "hashes", "index_url",
          "no_binary", "no_index", "only_binary", "pre", "prefer_binary", "require_hashes", "requirements", "trusted_hosts"]


def entry(text):
    try:
        eps = list(EntryPoints._from_text(text))
    except TypeError:
        return "TYPEERROR"
    out = []
    for e in eps:
        try:
            p = h(e.module) + "," + (h(e.attr) if e.attr is not None else "N") + "," + ".".join(h(x) for x in e.extras)
        except AssertionError:
            p = "ASSERT"
        out.append(h(e.group) + "," + h(e.name) + "," + h(e.value) + "," + p)
    return ";".join(out)


def opts_json(o):
    d = {}
    for f in FIELDS:
        d[f] = sorted(getattr(o.format_control, f)) if f in ("no_binary", "only_binary") else getattr(o, f, None)
    return json.dumps(d, ensure_ascii=False)


def reqfile(lines):
    from pip._internal.req.req_file import RequirementsFileParser, get_line_parser
    from pip._internal.exceptions import RequirementsFileParseError, InstallationError, CommandError
    import pip._internal.cli.cmdoptions as cmdoptions
    # A fresh shared --use-feature list for each case, as a fresh pip has.
    cmdoptions.use_new_feature.keywords["default"].clear()
    root = lines[0]
    for l in lines[1:]:
        k, name, value = l.split("\t")
        if k == "E":
            os.environ[name] = bytes.fromhex(value).decode() if value != "-" else ""
    out = []
    try:
        with contextlib.redirect_stderr(io.StringIO()):
            for line in RequirementsFileParser(None, get_line_parser(None)).parse(root, False):
                flags = ("R" if line.is_requirement else "-") + ("E" if getattr(line, "is_editable", False) else "-") + ("C" if line.constraint else "-")
                out.append("\t".join(["LINE", json.dumps(line.filename, ensure_ascii=False), str(line.lineno), flags,
                                      json.dumps(getattr(line, "requirement", ""), ensure_ascii=False), opts_json(line.opts)]))
    except RequirementsFileParseError as e:
        return "ERR RequirementsFileParseError " + json.dumps(str(e), ensure_ascii=False)
    except InstallationError as e:
        return "ERR InstallationError " + json.dumps(str(e), ensure_ascii=False)
    except CommandError as e:
        return "ERR CommandError " + json.dumps(str(e), ensure_ascii=False)
    return "\n".join(out)


op = sys.argv[1]
for path in sys.argv[2:]:
    lines = [l for l in open(path, encoding="utf-8").read().split("\n") if l]
    if op == "reqfile":
        print(h(reqfile(lines)))
    else:
        print(";".join(h(entry(bytes.fromhex(l).decode() if l != "-" else "")) for l in lines))
