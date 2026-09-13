"""Passwords and escaped literals for the SASLprep and escape gate."""
import unicodedata

# RFC 4013 section 3 gives these as its own examples.
RFC_EXAMPLES = [
    ("I­X", "IX"),
    ("user", "user"),
    ("USER", "USER"),
    ("ª", "a"),
    ("Ⅸ", "IX"),
]
PASSWORDS = [
    "", "hunter2", "pass word", "pa ss", "café",
    "café", "ＡＢＣ", "　space",
    "tab\there", "new\nline", "null\x00byte",
    "العربية",
    "aاb", "العa",
    "​zero", "۝", "￾", "private",
    "\U0001f600", "x" * 200,
]
LITERALS = [
    r"plain", r"a\nb", r"a\tb", r"a\\b", r"a\qb", r"\x41\x42",
    r"é", r"\U0001f600", r"\101\102", r"\1234", r"\0", r"\08",
    r"\u00", r"\x4", r"\U0011ffff", r"\ud800", r"end\\",
    "line\\\ncontinued", r"\a\b\f\v\r", r"\'\"", r"",
]
TEXTS = ["plain", "a\nb", "a\tb", "a\\b", "café", "\U0001f600",
         "null\x00byte", " line", "", "a'b\"c", "ÿ",
         "￿", "high \U0010ffff"]


def comparable(text):
    """Skip inputs where the two Unicode versions normalise differently."""
    return (unicodedata.normalize("NFKC", text)
            == unicodedata.ucd_3_2_0.normalize("NFKC", text))


def saslprep(text, stored):
    """RFC 4013, using the tables Python ships."""
    import stringprep
    mapped = []
    for c in text:
        if stringprep.in_table_b1(c):
            continue
        if stringprep.in_table_c12(c):
            mapped.append(" ")
            continue
        mapped.append(c)
    prepared = unicodedata.ucd_3_2_0.normalize("NFKC", "".join(mapped))
    for c in prepared:
        if (stringprep.in_table_c12(c) or stringprep.in_table_c21(c)
                or stringprep.in_table_c22(c) or stringprep.in_table_c3(c)
                or stringprep.in_table_c4(c) or stringprep.in_table_c5(c)
                or stringprep.in_table_c6(c) or stringprep.in_table_c7(c)
                or stringprep.in_table_c8(c) or stringprep.in_table_c9(c)):
            raise ValueError("prohibited")
        if stored and stringprep.in_table_a1(c):
            raise ValueError("unassigned")
    rtl = [stringprep.in_table_d1(c) for c in prepared]
    if any(rtl):
        if any(stringprep.in_table_d2(c) for c in prepared):
            raise ValueError("bidi")
        if not rtl[0] or not rtl[-1]:
            raise ValueError("bidi")
    return prepared


def system_records():
    """The machine's own passwd and group files, checked against `pwd`/`grp`.

    The files are the input and the modules are the expectation, so this
    checks the parser against an implementation nobody here wrote.
    """
    import grp
    import pwd
    out = []
    try:
        passwd = open("/etc/passwd", encoding="utf-8", errors="replace").read()
        group = open("/etc/group", encoding="utf-8", errors="replace").read()
    except OSError:
        return out
    want = []
    for entry in pwd.getpwall():
        want.append(":".join([entry.pw_name, entry.pw_passwd,
                              str(entry.pw_uid), str(entry.pw_gid),
                              entry.pw_gecos, entry.pw_dir, entry.pw_shell]))
    from_file = [line for line in passwd.split("\n")
                 if line.strip() and not line.strip().startswith("#")]
    # `pwd` may see directory-service users the file does not; compare only
    # the ones the file actually holds.
    names = set(line.split(":", 1)[0] for line in from_file)
    want = [line for line in want if line.split(":", 1)[0] in names]
    if want:
        out.append(("users", passwd, "\n".join(
            line for line in from_file
            if line.split(":", 1)[0] in set(w.split(":", 1)[0]
                                            for w in want))))
    want_groups = []
    for entry in grp.getgrall():
        want_groups.append(":".join([entry.gr_name, entry.gr_passwd,
                                     str(entry.gr_gid),
                                     ",".join(entry.gr_mem)]))
    group_lines = [line for line in group.split("\n")
                   if line.strip() and not line.strip().startswith("#")]
    if group_lines:
        out.append(("groups", group, "\n".join(group_lines)))
    return out


def build():
    argv, expected, labels = [], [], []
    skipped = 0

    def add(command, text, want, label):
        argv.extend([command, ":" + text.encode().hex()])
        expected.append(want.encode().hex())
        labels.append(label)

    for text, _ in RFC_EXAMPLES:
        if not comparable(text):
            skipped += 1
            continue
        try:
            want = saslprep(text, False)
        except Exception:
            want = "?refused?"
        add("query", text, want, f"rfc example {text!r}")
    for text in PASSWORDS:
        if not comparable(text):
            skipped += 1
            continue
        for command, stored in (("query", False), ("stored", True)):
            try:
                want = saslprep(text, stored)
            except Exception:
                want = "?refused?"
            add(command, text, want, f"{command} {text!r}")
    for literal in LITERALS:
        try:
            want = literal.encode("latin-1", "backslashreplace").decode(
                "unicode_escape")
            # A lone surrogate is something Python's strings can hold and
            # this language's cannot, so those are left out and counted
            # rather than compared.
            want.encode("utf-8")
        except UnicodeDecodeError:
            want = "?refused?"
        except UnicodeEncodeError:
            skipped += 1
            continue
        add("unescape", literal, want, f"unescape {literal!r}")
    for text in TEXTS:
        want = text.encode("unicode_escape").decode("ascii")
        add("escape", text, want, f"escape {text!r}")
    for command, source, want in system_records():
        add(command, source, want, f"{command} from this machine")
    # Files that are not well formed.
    add("users", "root:x:0:0:root:/root:/bin/sh\nshort:x:1\n", "",
        "a short passwd line is refused")
    add("groups", "wheel:*:0:root,admin\nbad:x\n", "",
        "a short group line is refused")
    return argv, expected, labels, skipped
