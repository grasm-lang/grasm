# Getting the package set to 650

The tracker `docs/packages-to-600.md` was, continued. That one closed at 600
of 600 with nothing left under "what is left"; this picks the count up there.

## The rules, unchanged

Every rule from `docs/packages-to-200.md` still applies, along with the seventh
from the 600 plan:

1. It is something a real program needs.
2. It compiles, is formatted, and lints clean.
3. Its types are prefixed — the whole set stays at zero collisions.
4. A demo asserts against it, rather than printing.
5. Demos are grouped, a handful of packages to one.
6. Where an implementation exists elsewhere, it is checked against that
   implementation — Python's standard library, `git`, `ssh-keygen`, `openssl`
   — and not against expectations written beside the code.
7. A package that already exists under another name is not written again.

One more is added, because this stretch leans on it:

8. **A table is generated from the implementation it will be checked
   against, never typed out.** `scripts/gen_dbcs.py` writes the double-byte
   codec tables by decoding every byte pair with Python's own codec, and the
   generator is checked in beside the packages it produced, so the table can
   be regenerated rather than trusted.

## Where it stands

| | |
|---|---|
| At the close of the 600 plan | 600 |
| Now | 623 |
| Target | 650 |
| Remaining | 27 |

The count is directories under `packages/` that are not `*-demo`. Test
harnesses are counted.

## Batches

### Done

- [x] **What text actually arrives in** — `cp932` `cp949` `big5hkscs`
  `ebcdic`. The Shift-JIS Windows writes, Korean with every modern syllable,
  Hong Kong's supplementary characters, and five mainframe code pages.
  107,765 cases against Python's codecs — every character each one knows,
  in both directions.
- [x] **Python's text tools** — `ndiff` `formatfield` `stringtemplate`
  `expandvars` `z85`, and the harness `texttest2`. 37,750 cases against
  `difflib`, `string` and `os.path`, including every string of up to five
  characters from `{}!:[]a0` through `Formatter.parse`.
- [x] **Network odds and ends** — `inetaton` `netdb` `rfc2822date`
  `maildir` `ftpwire`, and the harness `nettest3`. 146,551 cases against
  `socket`, `email.utils`, `mailbox` and `ftplib`, service lookups against
  this machine's own `/etc/services`.
- [x] **Python's data formats** — `pyliteral` `pymarshal` `prettyrepr`
  `floatmath` `getopt`, and the harness `pydatatest`. 91,589 cases against
  `ast.literal_eval`, `marshal`, `pprint`, `math` and `getopt`. `shelvefile`
  was dropped from the plan — `shelve` sits on whichever `dbm` the machine
  has — and `prettyrepr` took its place. Two defects found on the way, both
  outside this batch: the interpreter's write through a reference-valued
  field, and `shortestfloat` at powers of two.

### Next

Each line has an implementation to be checked against on this machine.

- **More of git** — `gitrefname` (`git check-ref-format`), `gitbundle` (`git
  bundle list-heads`), `gitcolor` (`git config --get-color`), `gitrawdiff`
  (`git diff --raw`), `shortlog` (`git shortlog`)
- **Unix tools** — `posixcksum` (`cksum`, `sum`), `sshcert` (`ssh-keygen
  -L`), `opensslenc` (`openssl enc`), `terminfo` (derived from `tic` itself)
- **Other languages' formats** — `rubymarshal` (`ruby`), `perlpack` (`perl`),
  `pytokenize` (`tokenize`)
