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
| Now | 611 |
| Target | 650 |
| Remaining | 39 |

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

### Next

Each line has an implementation to be checked against on this machine.

- **Network odds and ends** — `inetaton` (`socket.inet_aton`), `netdb`
  (`/etc/services` and `socket.getservbyname`), `rfc2822date`
  (`email.utils`), `maildir` (`mailbox.MaildirMessage`), `ftpwire`
  (`ftplib.parse227` and friends)
- **Python's data formats** — `pyliteral` (`ast.literal_eval`), `pymarshal`
  (`marshal`), `shelvefile` (`shelve`), `getopt` (`getopt.gnu_getopt`),
  `floatmath` (`math.fsum`, `nextafter`, `ulp`)
- **More of git** — `gitrefname` (`git check-ref-format`), `gitbundle` (`git
  bundle list-heads`), `gitcolor` (`git config --get-color`), `gitrawdiff`
  (`git diff --raw`), `shortlog` (`git shortlog`)
- **Unix tools** — `posixcksum` (`cksum`, `sum`), `sshcert` (`ssh-keygen
  -L`), `opensslenc` (`openssl enc`), `terminfo` (derived from `tic` itself)
- **Other languages' formats** — `rubymarshal` (`ruby`), `perlpack` (`perl`),
  `pytokenize` (`tokenize`)
