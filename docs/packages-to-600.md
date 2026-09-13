# Getting the package set to 600

The tracker that `docs/packages-to-200.md` was, continued past the point it
was written for. That document closed at 200 of 200; the count kept moving
because packages kept being needed, and this picks the count up where it
left off.

## The rules, unchanged

Every rule in `docs/packages-to-200.md` still applies, and the fifth one is
the reason this is a list of batches rather than a list of packages:

1. It is something a real program needs.
2. It compiles, is formatted, and lints clean.
3. Its types are prefixed — the whole set has to stay at zero collisions.
4. A demo asserts against it, rather than printing.
5. Demos are grouped, about six packages to one.
6. Where a standard exists, it is checked against the standard.

One rule is added, because the count is now large enough for it to matter:

7. **A package that already exists under another name does not get written
   again.** `difflib.unified_diff` was on the list for this push until
   `packages/diffview` turned out to have `unified` already. Checking the
   whole name list before starting a batch costs a minute and has caught
   seven duplicates so far.

## Where it stands

| | |
|---|---|
| At the close of the 200 plan | 513 |
| Now | 579 |
| Target | 600 |
| Remaining | 21 |

The count is directories under `packages/` that are not `*-demo`. Test
harnesses are directories and are counted; they are real programs that the
gates run, and several of them are the only consumer a format parser has.

## Batches

### Done

- [x] **Bytes written down** — `structpack` `quotedprintable` `encodings`
  and the harnesses that check them, each byte-identical to the library
  that owns the format
- [x] **Holding something** — five packages about waiting, which is what a
  poll is for
- [x] **Decided long ago** — `md5` `uuidnamed` `base32hex` `filemode`
  `htmlentities` `closematch`. Things nobody would choose today and
  everybody still has to read: a broken hash, the UUID versions built on
  it, an alphabet chosen so encodings sort, the ten characters `ls`
  prints, HTML's 2231 named references, and the suggestion ranking every
  "did you mean" copies. 590 cases against `hashlib`, `uuid`, `base64`,
  `stat`, `html` and `difflib`.
- [x] **Compression that compresses** — `deflate` `gzip` `xzfile`. `zlib`
  could read a deflate stream and could only write stored blocks, so every
  producer in the tree emitted files nothing made smaller. Now there is an
  encoder, the `.gz` container both ways, and a reader for `.xz` down to the
  range decoder. 202 cases against `zlib`, `gzip` and `lzma`, corrupted
  inputs included.
- [x] **Who a machine lets in** — `unixcrypt` `sshkey` `crc64`. md5crypt,
  apr1, sha256crypt and sha512crypt against `openssl passwd` and the
  sha-crypt specification's own vectors; OpenSSH public keys and both
  fingerprint formats against `ssh-keygen -l`; and the CRC-64 `.xz` checks
  with by default, which `xzfile` could previously read and not verify.
  97 cases.
- [x] **Two more ways to write Japanese** — `eucjp` `iso2022jp` `iso3166`.
  The Unix encoding of JIS X 0208 and the seven-bit one Japanese mail is
  still in, both going through the table `shiftjis` already carried rather
  than copying it; and the country codes, names and zones, taken from the tz
  database's own tables. 27,789 cases against Python's codecs and against
  `iso3166.tab`.
- [x] **Agreeing with somebody else** — `mersenne` `gettextmo`. MT19937
  seeded the way Python seeds it, for when "random" in a paper or a fixture
  means that exact generator; and the compiled `.mo` catalogues every
  translation in the world is stored in, plural rules evaluated rather than
  guessed. 1,356 cases against `random` and `gettext`.
- [x] **Reading what Python wrote** — `picklefile` `formatspec`. The pickle
  format's data half, with the four opcodes that call Python refused; and
  the mini-language after the colon in a format field, integers and strings.
  15,868 cases against `pickle` and `format()`.
- [x] **Formats the other end still speaks** — `xmlrpcwire` `difftext`.
  XML-RPC request and response bodies, where the type of every value is
  written down; and unified and context diffs byte-identical to
  `difflib`'s, because a hunk header that is off by one is a diff `patch`
  rejects. 404 cases against `xmlrpc.client` and `difflib`.
- [x] **Names, and numbers as names** — `shortestfloat` `idna`. The shortest
  decimal that reads back as the same double, done with exact integers
  rather than floating point; and IDNA2003 with nameprep, whose tables
  were generated from Python's own `stringprep`. `punycode` gained a
  decoder and lost a bug. 22,774 cases against `repr`, `%.*f` and
  `encodings.idna`.
- [x] **Seven bits, four ways** — `utf7` `utf32` `iso2022kr` `hzgb`. The
  encoding IMAP mailbox names are built on, the fixed-width one without
  surrogates, the Korean escape encoding whose designator appears once,
  and the tilde-based one invented for Usenet in 1989. The two CJK ones go
  through the tables `euckr` and `gb18030` already carry. 700 cases
  against Python's codecs.
- [x] **What git keeps beside a repository** — `gitindex` `gitattributes`
  `mailmap`. The staging area as a file, at all three index versions and
  mid-conflict; what the repository says about each path, macros and the
  difference between unset and unspecified; and the file that says two
  commits were by the same person. 72 cases against `git ls-files
  --stage`, `git check-attr` and `git check-mailmap`.

### Next

Each line is a batch of about six, chosen so that everything in it has an
implementation elsewhere to be checked against. A name in brackets is the
thing it is checked against.

- **Float presentations for `formatspec`** — `shortestfloat` exists now, so
  the `f`, `e`, `g` and `%` types it refuses can be implemented. That is an
  edit rather than a package and does not move the count.

`terminfo` was on this list and came off it. The plan was to check it
against `infocmp`, whose `-s i` flag is documented to print capabilities in
the order the compiled file stores them; the `infocmp` on this machine
ignores the flag and prints alphabetically, so there is no way to derive the
index-to-name table from an outside source here. Writing the table from
memory is exactly what rule 6 exists to prevent.
- **Container formats** — `debfile`, `oggmeta`, `isobmff`

## What this still does not include

The same three: a web framework, an ORM, a game engine. Each is a project
rather than a package, and the count is not a reason to change that.
