# Changelog

Anything marked `stable` in `spec/stability.md` will not change incompatibly
before 2.0. Anything marked `provisional` may change in a minor release with a
migration note here; anything `experimental` may change in any release. See
`docs/stability.md`.

## Unreleased — since 1.0.0

### Added

- **The encodings text actually arrives in, as opposed to the ones it is
  labelled with.** `cp932` is the Shift-JIS Windows writes — with NEC's
  circled numbers and IBM's extended kanji, and with Microsoft's mapping of
  the six characters where it and JIS disagree, which is the "wave dash
  problem". `cp949` is Korean with all 11,172 modern syllables, where EUC-KR
  has 2,350. `big5hkscs` is Traditional Chinese as Hong Kong writes it, with
  about 1,700 characters outside the Basic Multilingual Plane and four byte
  pairs that are a letter and a combining mark. `ebcdic` is five mainframe
  code pages, and splits record files on EBCDIC's own newline, which does not
  decode to `\n`. None of the tables were typed: `scripts/gen_dbcs.py`
  decodes every byte pair with Python's own codec and writes the package.
  107,765 cases in both directions; `dbcsdiff` is a gate.
  `docs/packages-to-650.md` is the tracker from here.

### Changed

- **`formatspec` formats floats now.** It refused `f`, `e`, `g`, `%` and the
  bare precision because doing them means correctly-rounded decimal
  conversion, and claiming that without doing it would have been worse than
  leaving it out. `shortestfloat` supplied the conversion, so the caveat is
  gone and the gate grew from 15,519 cases to 19,567 — every presentation
  type over a hundred doubles including the infinities and the subnormals.
  `g` is the one worth reading twice: its precision counts significant
  digits, a precision of zero means one, and the choice between fixed and
  scientific is made on the exponent *after* rounding, so `{:.2g}` of 99.9
  is `1e+02`. A bare precision with no type is `g` that switches to an
  exponent one step sooner and keeps a digit after the point, which is why
  `{:.1}` of zero is `0e+00` where `{:.1g}` is `0`.

### Added

- **Five packages that finish reading a git repository without git.**
  `gitrefs` reads `packed-refs` and loose refs, with loose ones winning —
  not a preference: `git pack-refs` writes the packed file and deletes the
  loose files afterwards, so a loose ref that still exists is either newer or
  the only copy. It also keeps the `^` line a packed annotated tag carries,
  which `git show-ref` does not print and which a tool resolving a tag to a
  commit needs. `gitreflog` reads where a ref has been, parsing from the tab
  backwards because a name may contain spaces and most do. `gitmodules` reads
  what a repository says about the repositories inside it, and asks the two
  questions that come from real vulnerabilities: does the path stay inside
  the working tree, and is the URL a location rather than an option.
  `gitdate` renders the timestamp in an object the six ways git renders it,
  in the writer's own offset rather than the reader's. `gitobjectid` computes
  the shortest abbreviation unique in a given set of objects, which is what
  `git rev-parse --short` prints and why a stored short id eventually stops
  working. 156 cases against `git show-ref`, `git reflog`, `git config
  --list`, `git log --date=` and `git rev-parse --short`; `repodiff` is a
  gate.

  **That is 600 packages.**

- **The three files around an SSH signature.** `sshprivkey` reads
  `-----BEGIN OPENSSH PRIVATE KEY-----`, a format documented in OpenSSH's
  own source and no RFC. An encrypted key still parses: the header and the
  public key are never encrypted, so "is this key encrypted, and whose is
  it?" — the question most callers actually have — is answerable without a
  passphrase, and this never needs to hold one. `allowedsigners` reads the
  file that turns a verified signature into a name, keeping the `namespaces`
  option as the restriction it is rather than a label, and splitting options
  on the commas outside quotes. `gitstripspace` cleans a commit message the
  way `git stripspace` does, and takes the subject as the *first paragraph*
  rather than the first line — which is what `git log --format=%s` prints and
  what a tool taking the first line disagrees with. 74 cases against
  `ssh-keygen -y`, `ssh-keygen -l`, `git stripspace` and `git log --format`;
  `signingdiff` is a gate.

- **A stable name for a change, and a way to check who signed something.**
  `patchid` reproduces `git patch-id`: the same fix cherry-picked onto three
  branches has three commit hashes and one patch id, because the line numbers
  in each hunk header and all whitespace are stripped before hashing. That is
  what `git cherry` and every "already upstream?" check are built on, and it
  is why a patch that survived a mail client's reflowing still matches. The
  consequence worth knowing is written in the source: two changes differing
  only in whitespace share an id, so it is not a checksum. `sshsig` verifies
  `ssh-keygen -Y sign` signatures — the format git uses to sign commits with
  an SSH key, which needs no keyring beyond the `authorized_keys` people
  already keep. What is signed is not the message but a structure naming the
  namespace and holding a hash of it, which is what stops a signature made
  for one purpose being replayed as another. ed25519 signatures are checked
  cryptographically through the tree's own implementation; an RSA or ECDSA
  one is parsed and reported rather than claimed, because a verifier that
  returns true for what it did not examine is worse than none. 105 cases
  against `git patch-id` and `ssh-keygen`; `gitsigndiff` is a gate.

- **The parameter encoding behind every non-ASCII attachment name.**
  `rfc2231` reads `filename*=UTF-8''%D0%BE...` and the numbered
  continuations that split a long value across several parameters, joining
  them in order and decoding the two charsets that account for almost all of
  it. A charset it does not know is reported as undecoded rather than
  guessed at, because a wrong guess produces a filename that looks plausible
  and is not the one that was sent. `base64mime` wraps base64 at 76
  characters the way a MIME body needs and reads back whatever somebody
  else's encoder produced — broken at 64, at 72, with CRLF, with the padding
  missing — while still refusing a character outside the alphabet, which
  means corruption rather than re-wrapping. `dbmdumb` reads the two-file
  format `dbm.dumb` writes and `shelve` is built on; its index is
  ISO-8859-1 rather than UTF-8, so it is taken as bytes, which is what a
  key holding 0xFF needs. 74 cases against `email`, `base64` and `dbm.dumb`;
  `paramdiff` is a gate.

- **What happens to a password before it is hashed.** `saslprep` is RFC 4013,
  which SCRAM requires and `postgres` therefore needs: two people typing what
  they believe is the same password on two keyboards can produce different
  bytes — a non-breaking space, a precomposed `é`, a fullwidth letter from an
  IME — and without preparation one of those logins fails and nobody can say
  why. It is nameprep with two differences: no case folding, because a
  password is case-sensitive, and ASCII control characters prohibited where
  nameprep leaves them alone. A *stored* string may not contain an unassigned
  code point and a *query* string may, so that a password stored today still
  verifies after a Unicode update; `prepare` takes which it is rather than
  guessing. `unicodeescape` reads and writes the backslash escapes in a
  source literal — where an unknown escape keeps both characters and a
  truncated one is refused rather than guessed at. `passwdfile` reads
  `/etc/passwd`, `/etc/group` and `/etc/shadow`, keeping apart the three
  things a shadow field can mean: a hash, `*` or `!` for an account no
  password will open, and *empty*, which means no password is needed at all.
  82 cases against `stringprep`, `unicode_escape`, and `pwd` and `grp`
  reading the same machine's files; `prepdiff` is a gate.

- **Three files git keeps beside a repository.** `gitindex` reads
  `.git/index` — the staging area, which is a complete snapshot of the next
  commit rather than a list of changes, and where a merge conflict lives as
  three entries for one path instead of one. All three index versions,
  including version 4, which reuses part of the previous path and does not
  pad, so a reader that assumes padding reads rubbish from the second entry
  onwards. The trailing SHA-1 is verified, because a truncated index is
  otherwise indistinguishable from a valid one until the names stop making
  sense. `gitattributes` reads what a repository says about each path,
  expands macros, and keeps *unset* and *unspecified* apart — the second
  means "fall back to git's own guess", and collapsing them gives the
  opposite behaviour on the files where it matters. `mailmap` applies the
  file that says two commits were by the same person, including the form
  that tells apart two people who once shared a machine account. 72 cases
  against `git ls-files --stage`, `git check-attr` and `git check-mailmap`;
  `gitfilediff` is a gate.

- **Four more encodings, three of them seven-bit.** `utf7` is what non-ASCII
  looked like before 8BITMIME and is the basis of the encoding IMAP uses for
  mailbox names, so a client without it cannot show a folder by the name its
  owner gave it. Its awkward parts are all in the encoder: the base64 is of
  UTF-16 rather than UTF-8, the run has no padding, and the closing `-` is
  written only when what follows could be mistaken for more of the run — a
  `+` already inside a run is just another character, while one interrupting
  plain text is its own escape. `utf32` is the fixed-width encoding without
  surrogate pairs, which refuses surrogates and anything past 0x10FFFF
  rather than passing them through. `iso2022kr` and `hzgb` are the Korean
  and Chinese escape encodings; both go through the tables `euckr` and
  `gb18030` already carry rather than keeping a second copy, and both have
  one rule that catches implementations which assume they resemble
  ISO-2022-JP: Korean's designator appears once, before the first shifted
  run and never again, and HZ's switches are `~{` and `~}` with `~~` for a
  literal tilde. 700 cases against Python's codecs; `encoding5diff` is a
  gate.

- **The shortest decimal that reads back as the same double.**
  `shortestfloat` prints `0.1 + 0.2` as `0.30000000000000004` — not `0.3`,
  which is a different number, and not `0.29999999999999999`, which is the
  same number and does not look like it. It is done with exact integer
  arithmetic over `bigint` rather than in floating point: a double is
  `m × 2^e`, so rounding it to k digits is a question about integers. Slow
  and right, where the fast algorithms are several hundred lines of tables
  and proofs. `formatspec` and `picklefile` each had to write down a float
  caveat for want of this. 22,653 cases against `repr` and `%.*f`, including
  eighteen thousand random bit patterns.

- **Internationalised domain names.** `idna` is IDNA2003 — nameprep's four
  steps, with the tables generated from Python's own `stringprep` — so that
  `MÜNCHEN.de` and `münchen.de` become the same bytes. `punycode` gained a
  decoder, and lost a bug: RFC 3492 appends the delimiter after the basic
  code points whether or not anything follows, so an all-ASCII label encodes
  to `example-`; without it a decoder reads the whole label as encoded digits
  and produces three characters nobody asked for. `encode_domain` never hit
  the case, which is why it went unnoticed. `idnadiff` and `floatreprdiff`
  are gates.

### Fixed

- **`punycode.encode` left off the delimiter for an all-ASCII label**, which
  made its output undecodable by every other implementation. `encode_domain`
  is unaffected — it leaves ASCII labels alone — and the demo that asserted
  the old behaviour now asserts the RFC's.

- **Two formats the other end still speaks.** `xmlrpcwire` marshals XML-RPC
  request and response bodies — supervisord, Odoo, a good deal of WordPress
  and a long tail of internal services — where the type of every value is
  written down in the document, so a reader never has to guess whether `1`
  was a number or a string. It gets the three rules implementations disagree
  about right: an untyped `<value>` is a string, `<i4>` is `<int>` under the
  original implementation's name, and a fault travels with HTTP 200, so a
  client that only reads the status sees success. `difftext` writes unified
  and context diffs byte-identical to `difflib`'s, which matters exactly
  when the output is fed to something else: `patch` rejects a diff whose
  `@@` line is off by one, and both of the ways to get it wrong — a
  one-line range written `-5,1`, an empty range written `-1,0` — look
  reasonable. It builds on `closematch`, which gained `matching_blocks`.
  404 cases against `xmlrpc.client` and `difflib`: `xmlrpcdiff` and
  `difftextdiff` are gates.

- **Two packages for reading and writing what Python does.** `picklefile`
  reads pickles — protocols 0 to 5, the memo, contexts, every integer
  encoding — and refuses the four opcodes that name and call a Python
  callable, which is what makes a reader safe where a full unpickler is a
  remote-code-execution primitive with a friendly name. The handful of pure
  constructors Python uses to store `bytes`, `set` and `frozenset` at older
  protocols are allowed by name and nothing else is. A structure that
  contains itself is refused rather than returned one level deep.
  `formatspec` is the mini-language after the colon in `{n:>8,d}`, for
  integers and strings: fill, alignment, sign, `#`, the leading zero that is
  not a width digit, grouping every three digits or every four in hex,
  precision, and the presentation types. Float presentations are refused
  rather than approximated — correctly-rounded decimal conversion is its own
  piece of work. 15,868 cases against `pickle` and `format()`:
  `picklediff` and `formatspecdiff` are gates.

- **Two packages whose whole purpose is agreeing with somebody else.**
  `mersenne` is MT19937 — a worse generator than the xoshiro256** `random`
  already has, and the only right answer when a paper says "seeded with
  12345" or a fixture came out of a Python script, because "random" in those
  sentences means this exact algorithm. Seeding is what differs between
  languages, not the generator, so `python_seeded` reproduces what
  `random.seed(n)` does. `gettextmo` reads compiled `.mo` catalogues, which
  is what `msgfmt` produces and what every translation that exists is stored
  in: both byte orders, contexts and plurals hidden inside the key, and the
  `Plural-Forms` rule evaluated as the C expression it is — Japanese has one
  form, English two, Polish three, Arabic six, and a reader that hardcodes
  "one or many" is wrong for most of the world. 1,356 cases against Python's
  `random` and `gettext`: `mersennediff` and `gettextdiff` are gates.

- **Two more ways to write Japanese, and the country table.** `eucjp` is the
  Unix encoding of JIS X 0208 — every byte of a multi-byte character has its
  high bit set, which is the property Shift-JIS gives up and the argument
  that produced UTF-8. `iso2022jp` is the seven-bit, stateful one that
  Japanese email is still in, where escape sequences switch character set and
  a stream that ends in kanji is a stream the next reader cannot start. Both
  go through the 6,879-entry table `shiftjis` already carried rather than
  copying it; the conversion between the two byte layouts is arithmetic.
  `iso3166` is the country codes, their names and the time zones each uses,
  taken from the tz database's `iso3166.tab` and `zone.tab` — which is also
  what the gate re-reads, so a tzdata update fails the gate instead of
  leaving the table quietly stale. The alpha-3 and numeric codes are
  deliberately absent: no table on a normal machine carries them, and
  writing 249 of them from memory is the thing rule 6 exists to prevent.
  27,789 cases: `japanesediff` and `iso3166diff` are gates.

- **Three packages about who a machine lets in.** `unixcrypt` computes the
  password hashes in `/etc/shadow` and `.htpasswd` — md5crypt, Apache's
  `$apr1$`, sha256crypt and sha512crypt — which is what checking an existing
  account database needs; none of them should be chosen for something new,
  and `argon2`, `scrypt` and `bcrypt` were already here for that. `sshkey`
  reads an `authorized_keys` line, its length-prefixed blob, and both
  fingerprint formats `ssh-keygen -l` prints, and splits the options on the
  commas that are not inside a quoted value — the difference between one
  `command="a,b"` restriction and two. `crc64` is the ECMA-182 checksum `.xz`
  uses by default, which `xzfile` could previously read past and not verify;
  it now verifies it, and eight more gate cases flip each of those bytes in
  turn. Checked against `openssl passwd`, the sha-crypt specification's own
  vectors and `ssh-keygen`: `unixcryptdiff` and `sshkeydiff` are gates.

- **Compression that compresses, and a reader for `.xz`.** `zlib` could read a
  deflate stream and could write exactly one kind — stored blocks, deflate's
  word for "no compression" — so `gzip`, `zip` and `png` could all emit files
  that other tools accept and that nothing made smaller. `deflate` is the
  encoder: LZ77 matching through hash chains, fixed-Huffman blocks, and a
  stored block whenever that is smaller, so incompressible input costs five
  bytes rather than growing. `gzip` is RFC 1952 in both directions, with the
  optional header fields, multiple members, and the trailer actually
  verified. `xzfile` reads `.xz` down to the range decoder — container,
  LZMA2 chunking with its state resets, LZMA itself, and the index and footer
  checked, which is what catches a file whose blocks each decoded cleanly and
  which is still not the file that was written. Checked against Python's
  `zlib`, `gzip` and `lzma` over 194 cases including corrupted ones;
  `compressdiff` is a gate. Dynamic Huffman blocks are not implemented and
  the roadmap says so rather than the package implying otherwise.

- **Six packages for things that were decided a long time ago.** `md5`,
  because a `MD5SUMS` file beside a download cannot be checked without it and
  version 3 UUIDs name it in their definition; `uuidnamed`, the v3 and v5
  UUIDs built on MD5 and SHA-1; `base32hex`, RFC 4648's other alphabet, whose
  whole point is that encodings sort the way the bytes do; `filemode`, the ten
  characters `ls -l` prints, including the three that replace an execute
  column rather than adding one; `htmlentities`, all 2231 of HTML's named
  references, where `htmlparse` knew a dozen; and `closematch`, the
  `difflib` ratio behind every "did you mean", auto-junk cutoff included.
  Checked against `hashlib`, `uuid`, `base64`, `stat`, `html` and `difflib`
  over 590 cases: `md5diff`, `enc4diff` and `closematchdiff` are gates now.
  `docs/packages-to-600.md` is the tracker from here; the 200 one is closed.

### Changed

- **Fifty-three clauses promoted from `provisional` to `stable`**, taking the
  stable set to 167 of 239 — forty-nine on 2026-09-10 and four more on
  2026-09-13, when their clause texts passed a month unchanged and the same
  audit cleared them. Nothing about the language changed; what changed is
  what 1.0 promises not to break. Each was checked against the five-part test in
  `docs/stability.md` on its own evidence — a conformance program passing under
  both engines, a program history with no expectation ever reversed, an age past
  a month, a clause text that admits nothing unfinished, and no membership of an
  area with open design work. Thirty provisional clauses did not pass and are
  listed with their reasons in the commit; `E-RECURSION-DEPTH` passed and was
  held back anyway, because the stack check its text describes is being looked
  at.

- **The allocation path resolves one thread-local instead of four.** On macOS a
  thread-local is a call, and `sample` put that call at the top of the profile
  of the allocation benchmark with about a third of the samples: the allocator
  asked for three separate ones and releasing a value asked for two. They are
  one structure now, and the "is a region open" test reads a counter rather than
  borrowing a `RefCell`. Paired measurements, best of nine alternating runs:
  `tree` 1.19x, `wordfreq` 1.10x, `sort` 1.03x, and no change to the programs
  that do not allocate. The machine was not idle, so `benchmarks/README.md`
  keeps its own table and is due a re-measure.

### Added

- **The package set passed 500** — 513 directories that are not group demos, of
  which 31 are gate harnesses. The eight batches that took it there are in
  `docs/ecosystem-roadmap.md`, each checked against the implementation that owns
  its format, and thirteen of those checks became gates.

## 1.0.0 — 2026-08-31

The release bar in `docs/release-scope.md` is met: **S1 zero, S2 zero**, every
gate green, and every known S3/S4 in `docs/known-issues.md` with a reproducer.

What 1.0 commits to is the 113 clauses listed `stable`, not the whole language:
79 clauses are `provisional` and 46 `experimental`, and those tiers mean today
exactly what they meant yesterday. A version number is not a promotion.

### Added

- **`grasm fmt` formats**, rather than only indenting. Each line is reprinted
  from its tokens, so the output depends on the program and not on the
  whitespace it arrived with. `crates/grasm_syntax/tests/fmt_preserves_tokens.rs`
  requires the token stream to be identical before and after across every `.gr`
  file in the tree, and idempotence in the same pass; `scripts/fmttest.sh`
  requires the tree to be in that form. `spec/conformance/L-WHITESPACE.gr` is
  exempt by name — its odd layout is the fixture.

- **`grasm lint`**, over the syntax tree: unused bindings, unused parameters,
  unused imports, shadowing, and over-long functions — the five rules
  `apps/grlint` names as needing a tree rather than lines. A name counts as
  used only when it is *read*, so `var n = 0` followed by `n = 1` is an unused
  binding. `scripts/linttest.sh` requires it to report on every program it can
  parse without falling over.

- **A monotonic clock and a sleep**: `monotonic_millis()` and
  `sleep_millis(n)`, `[L-LIB-CLOCK]`, in all three engines. The wall clock
  moves backwards; a duration measured with it goes negative twice a year. On
  wasm32 the embedder supplies both, as it already did the wall clock. A replay
  does not sleep.

- **Certificates, in Grasm**: `packages/asn1` (DER), `packages/p256`
  (verify-only P-256) and `packages/x509`, checked by `scripts/cert_diff.sh`
  against OpenSSL over 30 certificates — signature verdicts, DER structure node
  for node against `openssl asn1parse`, every certificate also presented with a
  byte changed inside the signed region, and an RSA certificate that must be
  refused rather than accepted. It checks one signature against one key: no
  chain building, no trust store, no dates, no hostname matching.

- **`packages/termios`**, raw terminal mode with no new builtin. `extern fn`
  reaches libc, `bytes` crosses as a pointer, and what C writes through it is
  visible coming back — which is all `tcgetattr` needs. Verified under a real
  pty, not reasoned about.

### Fixed

- **F92**: a value produced by the last expression of a loop body was handed to
  nobody and never released. A loop body is a block in statement position, and
  neither `while` nor `for` released its tail. One helper, called from every
  loop form, so a fourth cannot be added that forgets.

- **A gate reported the opposite of what it found.** `set -o pipefail` with
  `… | grep -q …`: `grep -q` exits at the first match, SIGPIPE kills the
  upstream, and the pipeline's status comes from *that* — so it failed exactly
  when the pattern matched, and only on inputs long enough that the upstream
  was still writing. Two runs of the same gate on the same tree disagreed.
  Eight sites across five scripts; `gatecheck` now refuses the combination.

- **A wasm import no embedder supplied.** Adding `monotonic_millis` added
  `env.grasm_monotonic_millis` to the module's imports, and a missing import is
  an instantiation failure — all 256 programs failed at once with nothing
  pointing at the cause. `wasmtest` now reads the import section out of the
  module and fails if any embedder does not supply every entry.

- **`\s` in a gate's `grep -E`** is not POSIX, so the gate's answer depended on
  which grep was installed.

- `spec/stability.md`'s own tally had drifted by eleven clauses. It is now
  checked by `crates/grasm_spec`.


- **GrasmWire**, a VPN protocol written entirely in Grasm. 2,580 lines from
  the field arithmetic up, against a budget of five thousand:
  `packages/x25519`, `chacha20`, `poly1305`, `chachapoly`, `blake2s`, `hkdf`,
  `noise` and `grasmwire`, with `apps/grasmwire` as the endpoint. Nothing
  underneath but the compiler — no C, no `.rs` in the protocol, nothing
  shelling out.

  Every primitive that has published vectors is checked against them: RFC 8439
  for the cipher, the authenticator and the AEAD, RFC 7693 for the hash, RFC
  7748 for the curve — both scalar multiplications and the whole §6.1 exchange
  — and all three RFC 5869 cases for the key schedule. The handshake has no
  published vectors, because the construction is Noise_IKpsk2 and the
  identifier string is this protocol's own, so it is checked as nineteen
  properties plus three pinned values. The protocol layer is forty-five more.

  **Every package below the endpoint declares no capabilities at all.** The
  curve, the hash, the AEAD, the handshake and the protocol cannot open a
  socket or read a clock, and `grasm deps` says so. A handshake needs
  randomness and a clock but not the *capability* to reach either: both arrive
  as arguments. That is also what makes three minutes of session lifetime
  testable in no time at all, and what let the handshake be pinned.

  It is **not wire-compatible with WireGuard** and the reason is one line — a
  different identifier string. Claiming compatibility would mean testing
  against a peer this project does not have. `docs/grasmwire.md` has the rest,
  including the one piece that is missing and why it is not being added blind.

- GrasmWire's four post-handshake behaviours, each demonstrated between two
  running processes rather than only asserted. It **rekeys**: a new ephemeral,
  a newer timestamp, both sides landing on the same replacement session, and
  traffic carried under keys that differ from the old ones. It **follows a peer
  that moves**, and only on a datagram that authenticates — moving on an
  unauthenticated one would let anyone redirect a live session by spoofing a
  single packet. It **refuses a recording**, on the timestamp. And **under load
  it stops allocating**: it answers an initiation with a cookie keyed to the
  sender's own address, remembers nothing, and requires `mac2` on the retry.

- `chacha20.hchacha` and `chachapoly.xseal`/`xopen` — HChaCha20 and
  XChaCha20-Poly1305, which the cookie needs and which nothing else here did.
  A 12-byte nonce is safe when a counter guarantees it never repeats; a cookie
  has no counter, goes to whoever just wrote in, and is sealed under a key that
  does not change, so the nonce must be random — and 96 random bits collide
  around 2^48. HChaCha20 is the same twenty rounds with the final addition left
  out, which is the whole construction rather than an optimisation, and it
  matches the XChaCha20 draft's vector.

- `udp_recv_from(h) -> (bytes, str, i64) ! IoError`, in all three engines. A
  datagram socket has no fixed peer, so `udp_recv` dropping the sender's
  address meant a receiver could only answer peers it was configured with in
  advance — which rules out roaming, one of the things a VPN is *for*. Found
  by writing one. `agreetest` holds the three engines to one answer, and
  `udp_recv` stays for callers that already know where to reply.

- `scripts/grasmwire_e2e.sh`, the thirty-first gate. Two real processes over
  real UDP with fresh ephemeral keys: both derive the same session hash without
  either sending it, three IP packets go through the tunnel and come back
  unchanged, a recorded first message is refused on its timestamp, a valid
  handshake from an unlisted key is refused, and two runs never produce the
  same session.

### Changed

- `[L-LIB-ARGS]`: `args()` was described as "arguments after `--`", which is
  true under `grasm run` and not true of a compiled binary, where there is no
  separator to strip. The behaviour was already right and already checked by
  `agreetest` in all three modes; only the one-line summary was wrong.

- `packages/http` and `packages/http_server`: the first two libraries of a
  first release. `http` parses requests, renders responses and routes with
  `:param` and `*` segments, and **declares no capability** — it cannot open
  a socket, and the compiler checks that rather than the README claiming it.
  `http_server` owns the accept loop and declares `io` and `net`. Both are in
  the corpus, so three engines are held to one answer on every run; the socket
  round trip is opt-in behind `-- serve`.
- `site/`: sixteen pages, generated by `scripts/site.py` from one template.
  Every figure it states is computed — from the tree, or from
  `site/facts.json`, which `scripts/allgates.sh` writes after every gate
  passes. `scripts/site.py --check` fails when the pages, the figures or the
  run disagree.
- `packages/bytes_codec`, `packages/time`, `packages/random`,
  `packages/uuid`, `packages/path`, `packages/url`, `packages/log`,
  `packages/csv`, `packages/cli`, `packages/testing`, `packages/toml`,
  `packages/resilience`, `packages/cache`, `packages/crypto_hash`,
  `packages/regex`, `packages/fs`, `packages/jwt`, `packages/cors`,
  `packages/dotenv`, `packages/sanitize`, `packages/metrics`,
  `packages/errors`, `packages/trace`, `packages/redis`,
  `packages/websocket`, `packages/password`, `packages/protobuf`,
  `packages/postgres`, `packages/xml`, `packages/oauth2`: the first
  wave of the release library, planned in `docs/packages-plan.md`. All but
  `log` declare **no capability at all** — a calendar, a seeded generator, path
  arithmetic and URL text are computation, and the reach each one is usually
  bundled with (the clock, the entropy pool, the disk, the socket) stays with
  the consumer. `log` is the other lever: logging is an *effect*, so a library
  that logs declares `Log` and not `io`, and what logging means is decided
  where the handler is installed.
- `packages/http` now takes its percent coding and query parsing from
  `packages/url` rather than carrying a second copy.
- `packages/xml`: parse and render, with **no DTD and no external entities**.
  That is not a gap: an external entity is a parser that fetches a URL
  somebody else wrote, which is XXE, and every parser that has one has had the
  vulnerability. A DOCTYPE is skipped without being read and an unknown entity
  is then refused, so there are two independent reasons the file is never
  opened — and it is why this package can declare no capabilities and mean it.
- `packages/oauth2`: the authorisation-code flow with PKCE and client
  credentials. PKCE is checked against RFC 7636's own vector, and is not
  optional — `authorize_url` takes a challenge and offers no version without
  one. Neither is `state`, and `callback_matches` compares it in constant
  time. Against a real provider this needs TLS, which the plan lists as
  blocked.
- `packages/protobuf`: the wire format — varints, zigzag, packed repeated
  fields, nested messages. A message without its schema still reads, which is
  what lets one be forwarded or inspected. **Not gRPC**: that is protobuf over
  HTTP/2 over TLS, and this language has neither.
- `packages/postgres`: the version 3 wire protocol and **SCRAM-SHA-256**,
  which is what PostgreSQL 14 and later default to — checked against RFC
  7677's own vector, so the proof and the server signature are values anybody
  can look up. `md5` authentication is deliberately absent. No TLS, so this
  reaches a database on a trusted network or a local socket.
- `packages/websocket`: the upgrade handshake and the frame format, `ws://`
  only. The accept value is checked against RFC 6455's own example, which is
  the difference between computing something and computing the right thing.
  Masking is checked in both directions — a server that accepts an unmasked
  frame sits behind a proxy that can be poisoned — and a control frame is
  refused if it is fragmented or over 125 bytes.
- `packages/password`: PBKDF2-HMAC-SHA256, with a stored form that carries its
  own algorithm and cost, so a file written today survives both changing.
  `needs_rehash` asks the question at the one moment the plaintext is in hand.
- `packages/redis`: RESP2 and RESP3, encode and decode, and a client. Both
  versions, because a RESP3 map and a RESP2 array of pairs mean the same thing
  and a client that reads one refuses half the servers it meets. Command
  lengths are byte counts; a bulk string's terminator is checked in full,
  which is what stops a client reading one byte short and taking the next
  reply from the middle of this one.
- `packages/errors`: a failure that carries its cause, its place and its
  fields, with `one_line`, an indented `full`, and JSON written without the
  `json` package — a reporter that cannot serialise an error when the JSON
  library is what failed has a hole exactly where it is needed.
- `packages/trace`: spans and W3C `traceparent`, with the collector as an
  effect. An all-zero trace or span id is refused: a receiver that accepts one
  joins every unrelated request into a single trace.
- `packages/sanitize`: escaping named by *destination* — HTML text, an
  attribute, a script block, a URL, a shell argument, a header value, a file
  name. There is no general `escape`, because what is safe in one is not safe
  in another; `html_url` refuses `javascript:` by scheme, since no character
  escaping stops it.
- `packages/metrics`: counters, gauges, histograms and the Prometheus text
  format, with the registry as an **effect** — a library that counts something
  declares `Metric` and not `io`, and where the numbers go is the program's
  decision.
- `packages/jwt`: HS256, and the algorithm is an argument to `verify` rather
  than something read from the token — so `alg: none` is refused because it
  disagrees with what the caller required, not by a special case. `exp`,
  `nbf`, `iat`, `iss` and `aud` are checked against a clock the caller passes.
- `packages/cors`: origin matching, preflight, credentials, exposed headers
  and max-age, as a function from a request to the headers a response needs.
- `packages/dotenv`: `.env` files, answering a map and never touching the
  process environment — which is global, and which a library has no business
  writing to.
- `packages/regex`: a Thompson machine — the pattern becomes an instruction
  list and matching runs every live thread in lockstep, so it is linear in the
  input for every pattern. `(a+)+b` against thirty `a`s, which takes a
  backtracking engine longer than the universe has lasted, returns at once.
  Backreferences and lookaround are refused by name: both need the machine to
  remember where it has been, which is the thing that makes the other engine
  exponential.
- `packages/fs`: walking, globbing, copying, temporary paths — the half of the
  filesystem that reaches the disk, declaring `io`. The other half is `path`,
  which declares nothing, so a program that only manipulates paths depends on
  that instead and the compiler holds it to the difference.
- `packages/crypto_hash`: `ProjectGold/lib-sha256` moved here and given the
  two things everything else wants — HMAC-SHA-256 and PBKDF2. Every digest is
  checked against a published vector (FIPS 180-4, RFC 4231, RFC 6070), because
  a hash tested only against itself agrees with nothing. `ProjectGold/37`
  points here, so there is one copy. **Not AES or RSA**: a cipher written here
  would have no constant-time guarantee from the compiler, and a block cipher
  whose timing depends on its key is a key that leaks.
- `packages/json`: `ProjectGold/lib-json` moved here, renamed (`parse`,
  `render`, `escape`) and given what a library needs that a program does not —
  `get`, `at`, `path`, `as_str`, `as_int`, `as_num`, `as_bool`, `is_null`,
  `kind`, `count`, `keys_of`, `items_of` to reach into a document, and
  `object`, `list`, `text`, `number`, `boolean`, `null`, `with_key`,
  `with_item` to build one. Declares no capability. The six ProjectGold
  programs that used it point here, so there is one copy rather than two.
- `packages/http_client`: dials, sends what `http` composed, and reads until
  the declared frame has arrived. Declares `net` and nothing else, so a
  program that only builds requests never acquires the ability to send them.
  Its demo talks to `http_server` over a real socket — a Grasm client and a
  Grasm server, both halves on one thread.
- `scripts/fuzz_modules.py`: the first generator that builds a program of more
  than one file. Every position a name from another module can stand in —
  twenty of them — crossed with how far away the declaring module is, each a
  whole package tree built and run three ways against an exact answer, plus a
  round that checks the capability ceiling in both directions. It found F71 on
  its first complete run.
- `scripts/fuzz_exits.py` grew a sixth way out: `try`. It is the one early exit
  the reader does not see — the branch that leaves is written nowhere — and a
  block that owns cleanup has to release on a path no line names. 71 programs
  to 85.
- `scripts/fuzz_modules.py` also covers a generic function called across a
  module boundary, an effect performed in one package and handled in another,
  and the capability ceiling for a user-declared effect — [T-CAPABILITY]
  charges a package for what it leaves in a `pub` row for its consumer to
  handle, not only for what the runtime discharges. 58 programs to 69, two
  ceiling cases to four.
- `scripts/lib/corpus.sh`: one definition of what the corpus is. Five gates
  were each carrying their own copy of the same four directory names.
  `scripts/gatecheck.sh` now fails any gate that writes the list itself — a
  rule that found a fifth copy in `northstar.sh` the first time it ran.

### Fixed
- A boxed place assigned to itself is freed. `h.a = h.a` on a list field and
  `xs[0] = xs[0]` on a list element each leaked one allocation: the store saw
  the value was already there and dropped neither the slot's reference nor the
  caller's, and the caller's was the surplus one (F79).
- A type name declared in two packages is refused with both modules named and
  a help that says what to do. It said "duplicate type name `Bad`" and pointed
  at one line in a dependency, with nothing to say there was another (F78).
- A generic container may be built empty. `let a: Box[str] = empty_box()` was
  refused for `V` "not determined by the arguments" — the binding determines
  it, the literal inside determines it, and an enclosing instantiation
  determines it, and none of the three was consulted (F77).
- `scripts/difftest.sh` and `scripts/leakcheck.sh` fail when the native
  backend declines a program, and say which. Both counted the skips and
  passed — one printed them after the summary that `allgates.sh` reads, the
  other not at all — so a program that stopped compiling stayed green (F76).
- A call into another module can be typed where the type has to be worked out
  — as a list element, say. The lowering resolved `utils.helper(x)`; the probe
  did not, and refused it naming a *module* as a receiver (F75).
- A handler a call answers may be passed as an argument.
  `with Cfg handled_by make()` compiled and `install(make())` did not, because
  the argument position accepted only a literal or a name (F73).
- A `match` whose live arms all yield nothing is typed as `()` rather than
  refused. The probe skipped unit arms looking for one that said more — right
  for a mixed match — and then refused when every arm was unit (F74).
- Hex and binary literals name a **bit pattern**, so all 64 bits are writable
  and `0xffffffffffffffff` is -1. They went through `i64::from_str_radix`, so
  the whole top half of the range was unwritable — which is where every
  published hash constant lives. Decimal still names a magnitude and is still
  range-checked. The diagnostic now says which fault: no digits, a digit not
  in the base, or more than 64 bits (F72).
- A type alias may name a type from another module, and a type an `effect`
  declares. `type Cb = fn(Thing) -> i64` was "unknown type `Thing`" while
  `fn f(t: Thing)` on the next line resolved, because aliases were resolved in
  the pass that collected their own module. Alias resolution is now its own
  pass over the whole program (F68).
- A generic type declared in one module may be compiled in another. It was
  registered under a module-qualified key, which is what a *function* gets;
  types share one namespace across the program, and an ordinary struct two
  lines away was already keyed by its bare name (F71).
- `ProjectGold/21-http-server` decoded percent escapes one byte at a time, so
  `caf%C3%A9` came out as `cafÃ©` — and `expected.txt` had recorded that as
  the right answer since the program was written (F69).
- The same program declared Content-Length in characters rather than bytes,
  and its own framing check measured the body the same wrong way, so it agreed
  with the bug it exists to catch (F70).

- `scripts/fuzz_laws.py` checks ninety-five laws against arithmetic rather
  than against a second engine — the first gate that can see a wrong answer
  the interpreter and a bundle share. It covers every method
  `spec/library.md` names — failing by name when one has no law — and the
  language's own semantics — one law for every behavioural clause the
  specification states, with both coverage checks failing by name when a
  clause or a method has none (F66).
- A closure may read `.value` through a `ref` it captured. The probe knew
  the struct case and not this one, so the same read outside a closure
  compiled and inside it did not (F67).
- A list, map, tuple or struct literal is released when an element or
  field leaves the function: `[1, if c { return 1 } else { 2 }]` kept the
  partly built value (F65).
- A `parallel` block's task list is released when a `return` or a `fail`
  leaves it, and `parallel { return … }` compiles — the statement never
  said its body had left (F64).
- `break` out of a `region`, `return` out of a plain block, `fail` out of
  an `unsafe` block: each left the following statement emitted after the
  terminator, which Cranelift refused (F63).
- A handler's captures are released when a `fail`, `return`, `break` or
  `continue` leaves its `with` block. Each kept one allocation per boxed
  capture (F62).
- `break` and `continue` pop the handlers of the `with` blocks they leave.
  The third exit from a block, and the same fault as F59 (F61).
- A handler is popped when a `fail` or `return` leaves its `with` block.
  It stayed installed, so the next operation found the wrong one — and
  after a few rounds, a stack slot in a frame that had gone (F59).
- `return` out of a `with` block compiles. The statement after it was
  emitted after the return, which Cranelift refused (F60).
- `GRASM_LEAK_CHECK` counts the NUL-terminated copies handed to C. They
  are `CString` allocations rather than the runtime's, so one leaked was
  invisible to every gate (F57).
- A `with` block's discarded value is released. Every `with` whose body
  ended in a boxed value kept one allocation for the life of the process;
  the output was right either way (F57).
- A write *inside* a referent compiles: `r.value[0] = v`, and the same
  through a list or a struct field. Reading it always worked, and so did
  replacing the whole referent (F56).
- [T-COMPARE] states which types compare. `==` and `!=` on any two values
  of one type; ordering on `i64`, `f64`, `str` and tuples of ordered
  components. It was true before and written down nowhere (F55).
- `fold`, `unwrap_or`, `expect` and `parse_int_base` compile where the
  backend has to name a call's result type — a list literal's element.
  Each worked in every other position (F54).
- A range's type is written `range`, so one can be a parameter, a result
  or a field; and a range reaches its methods from any shape that produces
  one, not only from a name (F53).
- An empty `map()` may be an element of a list literal. Every other
  position accepted one; only that one asked the probe, which had no
  answer for `map` (F52).
- A closure may be declared inside another closure. Every nesting was
  refused natively — with a capture and without, annotated and not (F51).
- A handler operation may capture a unit. It crashed the compiler — the
  two halves that pack a capture into an environment listed every scalar
  but that one (F50).
- `r.value` reads and writes a reference to a sum, `Option` included.
  It was refused because a sum is a named type just as a struct is, which
  left a `ref` to a sum readable through `match` and never writable (F48).
- `return` of an `if` whose branches all fail compiles. It was rejected
  by Cranelift's verifier; the same body without the `return` always
  worked (F46).
- Interpolating a unit compiles natively. It worked under a type
  annotation and was refused without one; both spellings now render `()`
  (F45).
- **`--native` compiles a generic struct declaration.**
  `type Pair[A, B] = { fst: A, snd: B }`, nested, in lists, through generic
  functions, compared, mutated and printed. A generic type is kept whole until a
  use site fixes its parameters — the same treatment a generic function gets —
  and each member is registered under a name built from its arguments. A literal
  reads those arguments off the fields it gives, the same unification a generic
  call does on its arguments.

  The mangled name is how two members are told apart and is **not** what the
  type is called: it stops at the value's edge, so `print` shows `Pair{..}`
  rather than `Pair[i64,str]{..}`.

- **`--native` compiles a generic sum type.** `type Opt[T] = | Has(v: T) |
  Nothing`, including two instantiations in one program. A constructor fixes the
  sum's parameters from its arguments, the way a struct literal fixes a
  struct's from its fields.

  A type parameter no argument mentions still needs an annotation: `Good(1)` for
  `Res[T, E]` says what `T` is and nothing about `E`, so one compiled copy
  cannot be chosen. The refusal names the parameter.
- **`--native` compiles a UFCS call of a generic free function.**
  `[1, 2].describe()` for `fn describe[T](xs: [T])`. A generic function has no
  symbol until a call site fixes its type parameters, and the lookup this path
  used does not carry generics — so the call was refused while
  `describe([1, 2])` compiled. The receiver is the first parameter ([L-UFCS]),
  which is exactly where the type parameters are read from.

  This also removes a refusal that had been *correct*: a generic free function
  with the same name as a function-typed field. The checker resolves it to the
  function; the backend could not see it, and calling the field printed a heap
  address and multiplied it by two. It resolves the same way as the checker now,
  and both directions of that crossing are gated.

  Three refusals remain in §11, down from seventeen.
- **`--native` compiles a handler that outlives the expression that wrote it.**
  Returned from a function, stored in a struct field, held in a list, walked by
  a `for`, printed, instantiated generically, installed inside `parallel` — six
  shapes, one change. A handler value is reference-counted and its operations'
  captured environments are on the heap, so the question *is this environment
  still alive?* has the same answer as for every other value. A handler written
  directly at a `with` never builds one: its operations are lifted at compile
  time and its captures live in the frame that ran the `with`, and that path is
  untouched.

  Four refusals remain in §11, down from seventeen this morning.
- **`--native` compiles an effect handler passed as a parameter.**
  `fn run(h: LogHandler) { with Log handled_by h { … } }` — the shape
  [E-HANDLER-OWN-ROW] documents. Smaller than it looked: effect *operation*
  calls were already fully dynamic, a lookup by effect and operation name
  followed by an indirect call, so nothing about dispatch changed. What was
  missing was a way to *carry* a handler, because operations are lifted from a
  literal at compile time and a `with` on a parameter has no literal to lift. A
  handler value is now a block of `(code, env)` pairs built in the frame that
  wrote the literal — which is exactly why one may be passed down, and still may
  not be returned or stored in a struct: those outlive the frame, and stay
  refused by name.
- **`--native` compiles a type parameter that appears only in a function-typed
  parameter's result.** `fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K]`.
  Monomorphization binds a type parameter by matching each declared parameter
  against the argument's actual type, and that matcher had no case for a
  *function* type — so `T` was bound from `xs` and `K` was determined by
  nothing, though the closure passed for `f` had always known its own type.
  `ref T` and `T ! E` were missing for the same reason and are handled now too.
- **`--native` compiles a closure whose parameter types nothing local fixes.**
  `let f = |p| p.0 + p.1`, called as `f((i, v))` in a later statement. Such a
  closure learns its parameters from its call site and inference reaches both;
  the backend was reconstructing that from the shapes visible *at the closure*,
  which cannot reach a call site in another statement. The checker now exports
  what it settled (`closure_param_types`), and the backend asks before
  reconstructing. The reconstruction stays as the fallback, for closures
  inference genuinely leaves unresolved — and those are refused by the checker
  itself, so every engine refuses them with one message.
- **`--native` compiles a `fold` seeded with an unannotated `map()`.** The
  cause was asymmetry rather than difficulty: an empty *list* asked the checker
  what it had resolved the literal to and an empty *map* did not, though the
  checker records both ([T-EMPTY-LIT]). So the seed arrived as a map with
  unknown key and value, and the refusal said the seed and the function differed
  when they were written identically.
- **`--native` compiles a call through a callee that is not a name.**
  `TABLE[0](21)`, `fs[0](3)`, `b.f(6)` where `f` is a struct field of function
  type. The callee is evaluated **once**, bound to a name the source cannot
  spell, and called by the path a named function value already used — bound
  rather than called in place because that path reads the value twice, once for
  the code pointer and once for the environment, and evaluating the callee twice
  would run its side effects twice.
- **`--native` compiles `==` on two bare `ref`s.** A `ref` is a slot and a
  generation packed into one word, and two are equal when they name the same
  slot at the same generation — identity, which is what the interpreter answers
  and the only thing a reference can usefully be asked. So the comparison is the
  word. It compiled for a *list* of refs already, because the runtime's deep
  equality reaches the same answer element by element; only two bare ones had
  nowhere to go. Slot reuse after `release` agrees too, which is what the
  generation is for.
- **`--native` compiles `map` with a function that returns nothing.** The
  refusal was stale in one half and advice in the other. `RtPayload::Unit`
  exists precisely so a list of nothings prints as `[(), ()]` rather than
  `[0, 0]`, so there was a representation; the rest of the message — *write a
  `for` loop, which is what a list of nothings was standing in for* — is good
  advice, and advice that makes two engines disagree about which programs exist
  is worse than no advice. Deleting the refusal was the whole fix; three
  engines agree, including on printing the list.
- **`--native` compiles a range held as a value.** `let r = 0..n`, passed,
  printed, compared, walked. `for i in 0..n` written literally is compiled into
  a counted loop and never builds a range at all — which is why it costs
  nothing, and it is unchanged — so there had been no range value for a binding
  to hold. One exists now: three scalars, so `0..2_000_000` costs what `0..2`
  does. It prints as its bounds rather than its elements, and two compare equal
  when bounds and inclusiveness match, so `0..0` is not equal to `5..5` though
  both are empty. `len` is answered from the bounds without building anything.
- **`--native` compiles a match on `IoError`'s variants** (F38). `IoError` is in
  the prelude — nobody declares it in source — and the backend built its type
  table by walking each module's declarations, so there was never anything to
  find. Telling a missing file from a broken one was a build-time refusal, in a
  language where every `io` call hands one of these back. The variant list is
  now read from the checker's own table rather than copied, so a third variant
  cannot make the two disagree again. Two gaps for one fix: `let r: str !
  IoError = read_file(p)` compiles too.
- **A recorded `()` came back as `0` on another engine** (F37). The compiled
  runtime answered `write_file`, `write_bytes`, `make_dir`, `remove_file` and
  both socket writes with `Ok(0)` where the interpreter answers `Ok(())`. No
  direct run could tell them apart — the static type is `()`, and a compiled
  build prints unit from the type rather than from the value — but a journal is
  exactly a value leaving the engine that made it, and a cross-engine replay
  printed `0` and `[0, 0]`. `RtPayload::Unit` already existed for this; six call
  sites had not been converted.
- **Every gate now builds what it tests** (F36). `scripts/wasmtest.sh` read a
  prebuilt wasm module it never built, so when the wasm target stopped
  compiling it went on reporting `identical: 182` against a module from the
  previous week — for two full runs. The audit that followed found the same
  shape in all nine gates: each opened `./target/release/grasm` without
  building it. A gate reporting a number is making a claim about the code as it
  stands, and reading an artefact it did not build is the one way to make that
  claim without checking it.

- **`grasm check` no longer takes exponential time on a chain of method calls**
  either. The same defect as the field chain, one path over: a method call
  infers its receiver to resolve the method, and `check_fn_call` then checks
  that same receiver against the parameter it fills. Twenty `.bump(1)`s took
  0.7 seconds and forty never finished. Inference is now memoized for the
  shapes that cannot hold a closure — narrow deliberately, since a closure's
  type depends on the position it fills and its body is checked against the
  enclosing function's error type, so for those a second visit is not a repeat.
  Found by auditing for siblings of the field-chain defect rather than by an
  evaluator.
- **`grasm check` no longer takes exponential time on a chain of field reads.**
  `infer_field` inferred its receiver twice — once for the handler-operation
  check, once for the read — so cost doubled per link: twenty-two `.next`s took
  1.3 seconds, twenty-six took twenty-one, and forty never finished, at one
  core and 13 MB. Not a stack overflow and not the nesting cap, which
  [G-NEST-LIMIT] says must *report* rather than hang. A thousand links now
  check instantly.
- **A `par_map` inside a closure emits its worker.** The worklists are drained
  to a fixpoint because compiling one body discovers another; the parallel
  workers were drained once, before that loop, so a `par_map` written inside a
  closure or a handler operation had its lifted worker declared and never
  defined — and the *linker* reported it as a missing C function `gp_main_1`,
  for a program containing no `extern fn`. The identical defect the loop
  already existed to prevent for closures.
- **A struct field of function type is reachable again.** [L-UFCS] takes a free
  function in method position only when "its first parameter accepts the
  receiver", and only the name was compared: declaring `fn len(xs: [i64])`
  anywhere in the module made every `n.len()` on a struct with a `len` field a
  type error naming `[i64]`. Worse, the engines resolve by name and had no way
  to know better, so once the checker was fixed they still called the free
  function: the checker now records which calls it resolved to a field, and
  both engines read that.
- **`--native` compiles two more shapes**, and §11 loses two rows: a parallel
  operation given a function *value* (a `fn`-typed parameter, or `let f = sq`),
  which refused with a message about a closure in programs containing none; and
  a closure literal in a `match` or `if` arm whose binding is annotated, which
  refused with "no type says what its parameters are" about a binding that says
  exactly that. The annotation now reaches the arms.
- **A parallel operation may no longer reach a `ref` from outside** — the race
  the previous entry created. [T-PAR-MAP-PURE] had named the shape and said it
  was "not a race today: the native backend refuses a write through a `ref`
  outright". It stopped refusing, and a program `check` accepted then died with
  SIGTRAP, or panicked inside the runtime with `field access on non-struct`, or
  printed a different wrong answer on each run at exit 0 — several workers
  reaching one slot touch a reference count that is not atomic. Reading was as
  unsafe as writing, because a read takes a count on what it finds. Now refused
  three ways: an element type that reaches a reference, any mention of an
  outside reference inside the function, and a generic carrying one in
  (recorded on the generic's parameter and settled at its call sites, the way
  [T-MAP-KEY-GENERIC] settles map keys). Found independently by both evaluators
  of round 42.
- **`--native` compiles writes through a `ref`.** The backend refused every one
  of them, so §11 listed the shape and two conformance programs ran under the
  interpreter alone. The write now lands in the slab rather than rebuilding the
  referent and storing it back — which a cycle proves is not the same thing:
  `a.out[0].out[0].name` arrives back at `a`, and the outer copy, stored
  afterwards, erased the inner write. `grasm_ref_make_unique` gives the slab
  sole ownership so the existing mutators write in place, and the walk
  short-circuits above the reference, since a chain of no-op write-backs leaked
  one reference per level (the setters skip their release when handed back the
  pointer they already hold). 203 programs identical across all three engines,
  203 leak-free.
- **`--native` compiles returning a closure.** The refusal named the return,
  while what was actually missing was the closure's *parameter* types — which
  the declared return type had already written down. Taking them from there
  compiles the shape, capturing values of any type and capturing a function
  value to call inside. With this and the `ref` writes, **every program in the
  repository compiles natively: 204 identical across all three engines, 0
  unsupported, 0 leaking.**
- **The UFCS help no longer promises a call the compiler will not make.**
  `"hi".print()` was refused with "a free function `fn print(str, ...)` would
  also work here (UFCS)" — exactly the call that had just been refused.
  [L-UFCS] now says the language's own free functions are not methods of any
  type, and the help names the spelling that works: `"hi" |> print`.

### Fixed — verification of the day's changes

- **The backend resolved a method call differently from the checker, and
  printed a heap address.** The checker records which method calls it resolved
  to a struct field; the backend read that table only to *suppress* its
  field-call path, which is half a rule. It resolves a free function with a
  lookup that does not carry generics, so a *generic* free function sharing a
  name with a function-typed field left the lookup empty and the call fell
  through to the field — while the checker had resolved it to the function.
  `grasm run` printed `77`; the compiled binary printed a string and then
  multiplied its heap address by two. Exit 0, nothing on stderr, a different
  number every run. The table is authoritative in both directions now: it
  suppresses and it permits, and a call the backend cannot resolve the checker's
  way is refused by name, as §11 promises.
- **A field reached through a `ref` was refused** when a same-named free
  function could not accept the receiver. Reading through a reference is
  [T-REF-READ]; being *passed* as one is not, and `fn step(b: Box, …)` cannot
  take a `ref Box`. Treating them as interchangeable made that function a UFCS
  candidate which then failed its own argument check, so the call reported
  "expected `Box`, found `ref Box`" instead of falling back to the field.

### Fixed — the round-44 list

- **An incremental `--native` rebuild shipped a stale imported constant.** A
  module's cached object was keyed on its own source and on the program's
  *signature* surface — and a signature omits a constant's value, correctly, since
  a constant changing value is not an API break. A cache key is wrong to omit it:
  [E-CONST] makes a module-level `let` one value for the whole program, the
  importer's object materialises it, and changing `pub let K = 10` to `999` left
  the importer's entry valid. Bump a dependency, rebuild, ship the old value —
  across a package boundary too. Now keyed on a fingerprint of everything the
  object read, which is the rule rustc's incremental engine uses.
- **An effect operation's result was not row-checked.** `check_expr` returned
  early for a closure once its *own* row was checked, so rows nested in its
  result type never were — every other expression got that check one line
  further down. `MkHandler{ make: || shout }` for `fn make() -> fn(i64) -> i64`
  handed a printing function through an empty-row result, so a function
  declaring `/ {Mk}` printed and a `capabilities = []` package reached `io`,
  while the same signature written as an ordinary `fn` was refused.
- **The registry was found only from one directory.** [T-MODULE-DEP] says "the
  nearest enclosing `.grasm/registry`" and the walk was written to do that — but
  it walked by `parent()` from a relative path, which has no parents. The same
  package built or did not build depending on where the shell was, and the error
  told the reader to put the registry beside the package, where it already was.
- **A deeply nested tuple literal cost O(n³) to check** — 1000 levels took 7.8
  seconds, now 31 ms. `record_value_row` runs on every expression and copied the
  whole type to look at its head.
- **Two specification statements were false.** [L-LIB-SORT-BY] said a sort key
  "must be `i64`, `f64` or `str`" while tuple keys worked in all three engines
  and [L-LIB-TUPLE] said so; [T-REF-WRITE] said the compiled backend refuses
  writes through a reference, one day after it stopped.

- **A compiled binary is 25% smaller** — 437 KB to 328 KB, against [G5]'s 300 KB
  budget. The symbol table is a quarter of a binary and nothing reads it at run
  time. Two other levers were tried and reverted with their numbers recorded in
  `Cargo.toml`, so the next person does not spend the afternoon rediscovering
  them: `panic = "abort"` saves 1 KB (the unwinding tables come from
  precompiled `std`), and `opt-level = "z"` on the runtime makes the binary
  *bigger* by blocking the inlining link-time optimisation needs.

### Added

- **Record a run, and replay it.** `GRASM_RECORD=j ./prog` writes a journal of
  everything the program read from outside itself; `GRASM_REPLAY=j` makes a
  later run take its answers from that journal and reach outside itself for
  **nothing** — not the reads and not the writes. Record a program that writes
  a file, delete the file, replay: identical output, and the file stays
  deleted. `print` and `eprint` are the deliberate exception, because a replay
  whose output you cannot see is not worth running.

  All **28** journaled builtins on **all four engines** — the whole `io`, `net`
  and `proc` surface. A recording made by the native build replays
  byte-identically under `grasm run` *and inside a wasm module that has no
  sockets, no processes and no filesystem*, TCP session and child process
  included. That is the case that matters: the failure happens in the fast
  engine and the debugger is in the other one.

  The effect system is what makes it cheap. Knowing which calls are
  non-deterministic is normally the hard part of building such a tool; here it
  was already written down, and `crates/grasm_sema/src/builtins.rs` says for
  each builtin what a replay owes it. A builtin with an effect row that says
  nothing about replay fails `cargo test`.

  Divergence is refused rather than guessed: a run that takes a different path
  from the recording stops with exit 7 and names the call that disagreed.

- **`grasm journal <file>`** prints a recording back out — every call in order,
  what it answered, and a tally of which builtins were reached. A journal is a
  file of framed bytes, and evidence nobody can examine is not evidence.
  `--full` stops truncating long payloads.

- **A range answers the list combinators** — `to_list`, `map`, `filter`, `sum`,
  `len` ([L-LIB-RANGE-OPS]). Building a list from a count needed
  `var xs: [i64] = []`, a `for`, a `push` and two closing braces; `(0..n).map(f)`
  says the same thing. It costs no syntactic form — a call is a call — so the
  kernel count in `spec/core.md` does not move. Most of the reason [G11] went
  from 1.41× Python to 1.22×, and 0.74× Go to 0.65×.

- **`spec/core.md` — the Grasm-Core / Grasm-Full split**, the commitment
  MASTERPLAN §19.2 makes binding and that four places in the repository asserted
  while none was true. One rule (a form is Full if and only if a
  meaning-preserving rewrite into other forms is written down), applied: 37 Core,
  18 Full, a rewrite beside every Full form. Thirty-seven is not thirty, and the
  document says so and names the seven forms that would close the gap.
  `cargo test -p grasm_spec --test core` reads the AST rather than a list, so a
  new construct that is neither kernel nor sugar fails the build; it also prints
  all three §19.2 budgets on every run.

## 0.9.0 — 2026-08-16

The first release with a scope. Everything below `1.0` is still pre-1.0 by the
rules in `docs/stability.md`; what changes here is that a version now has a
defined surface and a defined bar, written down in `docs/release-scope.md`.

### The 1.0 surface

**In:** types and inference, functions, pattern matching, collections and the
standard library, modules, fallible values `T ! E`, ownership and `inout`, the
interpreter, the bundle format and the native backend.

**Out, and documented as out:** effects and their rows, the capability ceiling,
parallel purity, regions, Tier-2 `ref`, and FFI. Those clauses are
`experimental` — usable, specified, tested, and not covered by the
compatibility promise.

### What blocks a release

S1 (a crash, a wrong answer, a disagreement between the three engines that
[E-PANIC-TRACE] does not license, or a build that leaks state) and S2 (a
`stable` clause proved unsound) block. S3 and S4 — unsoundness in surface the
specification already labels experimental, over-strictness anywhere, and
diagnostics — are changelog content, and every open one is listed with a
reproducer in `docs/known-issues.md`.

At this release: **no S1, no S2 and no S3 are open.** One S4 (the UFCS help text
promises a call the compiler will not make).

### Effect rows come from the type, not from a guess

The effect pass used to reconstruct, syntactically, a fact the type checker
already had: which function value a name holds, and therefore what calling it
performs. It did that with a name-to-sources map, a pool of every function
value a body mentions, and a special case per binder — `let`, `for`, a `match`
payload, a tuple destructuring, an index, a field, `??`, a block, a closure
parameter. Every missing cell of that grid was a defect, and ten rounds of
cold-model evaluation spent most of their findings there.

A function's row is part of its type and travels through every carrier by
unification, so the checker now publishes the row of each function-valued
expression and of each call whose callee is not an expression of its own, and
the effect pass reads it. **511 lines of guessing deleted**, and the class of
defect they produced no longer has a mechanism.

The deletion was made on evidence: the whole gate set was run first with the
syntactic path switched off, and only what it caught was removed.

### Gates

201 conformance and example programs identical across `grasm run`, the bundle
and the native binary; 16 three-engine agreement programs; 201 leak-free
builds; the §11 gap list proved to match the compiler in both directions; and
five generated-program gates — effect rows (204 programs, both directions,
including across a package boundary), FFI spellings (36), effect handlers,
differential, fallible values and generic instantiation. All of it now runs in
CI rather than by hand.

### Also in this release

- **`scripts/fuzz_extern.py`** — every spelling by which a C function can be
  named or called, each checked against a row missing `io` (must fail) and
  against the complete row (must compile and run).
- **[T-EFFECT-OP-RESULT]** — an effect operation's declared result carries its
  row. An operation answering `-> fn(i64) -> i64 / {io}` handed out a printing
  function that nothing charged.
- Scope decides for a **module alias** and an **effect name**, as it already
  did for a function name.
- Five clauses demoted from `stable` to `experimental`
  (`T-FN-TYPE-ROW`, `T-FN-TYPE-ROW-PARAM`, `T-FN-TYPE-ROW-CARRIED`,
  `T-FN-VALUE-ROW`, `T-UNSAFE-UFCS`): their implementation changed in nearly
  every evaluation round, which is what the promotion criteria forbid.

## Unreleased

### Added

- **Property tests and benchmarks.** `test "name" (xs: [i64]) { ... }` generates
  its inputs; parameters are the only difference from a unit test, so fuzzing is
  the same machinery pointed at `str`. Failures report a shrunk counterexample
  and the seed that found it. `bench "name" { ... }` reports ns/op via
  `grasm bench`. `grasm test --coverage` reports statement-line coverage.
- **Documentation.** `///` comments document the declaration that follows;
  `grasm doc` renders Markdown with signatures taken from the AST. Fenced blocks
  in doc comments are executed by `grasm test`. `docs/tutorial.md` is a guided
  tour whose code the test suite compiles and runs.
- **Debugging.** Panics print a stack trace. `grasm dap` is a Debug Adapter
  Protocol server: breakpoints, stepping, frames, locals, variable lookup.
- **Standard library.** `any` / `all` / `find` / `min` / `max` / `is_empty` on
  lists, `lines` / `pad_start` / `pad_end` / `is_empty` on strings, `is_empty`
  on maps, and `now_millis` / `env` / `exit`.
- **Benchmarks.** Five programs in Grasm, C, Rust, Go, and Python, with a
  harness that verifies answers before reporting timings.
- **Project documents.** LICENSE (Apache-2.0), NOTICE, CONTRIBUTING,
  GOVERNANCE, SECURITY, CODE_OF_CONDUCT, and this file.
- **`grasm deps --locked`** fails when anything differs from `grasm.lock`,
  including a widened capability ceiling, which is reported separately from a
  content change.

- **The native backend compiles structured concurrency and generic functions.**
  `parallel`/`spawn` lower to a recorded list of spawn-site indices dispatched
  after the block, so ordering and joining match the interpreter without
  closures or environment capture. Generic functions are monomorphized per
  instantiation, including across module boundaries.
- **The native backend compiles function values.** A named function used as
  data, a closure bound to a name, closures capturing their environment, and
  lists of either — all dispatched through an indirect call. Native coverage
  goes from 103 programs to **all of them: nothing in the repository is left
  to the interpreter.**

- **Unboxed scalar collections.** `[i64]`, `[f64]`, and `[bool]` store
  elements inline — one word each, no per-element heap cell, no refcount
  traffic. sieve 89→58ms, matmul 187→92ms, sort 97→49ms (now faster than C
  and Go). Representation only; `[E-COW]` says it is never observable, and
  difftest holds every program byte-identical across both engines.

- **Native binaries dead-strip the runtime.** `-Wl,-dead_strip` (macOS) /
  `--gc-sections` (Linux) at link time: 1,503 KB → ~442 KB mean, matching
  Rust. The rest is `std` machinery; going lower needs `no_std` or
  `build-std`.

- **`par_filter` — the other half of effect-proven parallelism.** The predicate
  runs across cores into a mask; compaction then walks the mask in order, so
  the output is the same whatever the core count is ([E-PAR-FILTER-ORDER]).

- **Version requirements on dependencies.** `version = "^0.3.1"` accepts that
  version or any later one that does not change the leading non-zero number;
  `=0.3.1` accepts exactly it; bare `0.3.1` means `^0.3.1`. Before 1.0 the
  minor number carries the weight of a major, the same rule the stability
  policy states for the language.

- **`fold`, `entries`, `expect`.** The three gaps cold-model runs kept hitting.
  `xs.fold(init, f)` is the one combinator whose result is not a function of
  the element type — `[i64].fold("", ..)` is a `str`. `m.entries()` returns
  `[(K, V)]` in key order, so walking a map no longer means iterating `keys()`
  and looking each one back up. `o.expect(msg)` is the escape hatch for
  absence being a bug rather than a case; the message is required, because a
  panic nobody can read is not an improvement on the `??` it replaces.
- **`panic` accepts a computed message.** Natively it took a literal only, so
  `panic("value {n} is out of range")` did not compile.
- **Function types.** `fn(A, B) -> R` may be written wherever a type may be,
  so a program can declare its own higher-order function — `fn apply(f: fn(i64)
  -> i64, x: i64)` — instead of being limited to the built-in combinators. A
  closure passed to such a parameter takes its parameter types from it. The
  type carries an effect row and **no row means the empty row**, so
  `fn(i64) -> i64` accepts only functions that perform nothing and an effectful
  one is rejected where it is passed. That check is what stops a function value
  from being a way to carry an effect past a declared row.
- **`grasm publish` and `grasm add`.** `publish` copies a package into a
  registry after checking it against its own ceiling, and **refuses to
  overwrite a version that already exists** — a published version is
  immutable, which is what makes the lockfile's checksum mean something.
  `add` appends a dependency to `grasm.toml` without reformatting the file.
- **[L-DISPLAY] and [L-UFCS].** How every type renders, because a program's
  output is what its correctness is judged on; and that a free function whose
  first parameter accepts the receiver may be called as `x.f(y)`.
- **A registry, and minimal version selection.** A dependency with a `version`
  and no `path` is resolved from a directory of `<name>/<version>/` packages
  (`GRASM_REGISTRY`, or the nearest `.grasm/registry`). Each package gets the
  highest floor any requirement stated, resolved to the lowest version present
  at or above it, then checked against every requirement. No search and no
  backtracking, so the answer does not depend on walk order and adding a
  dependency cannot silently upgrade one you already had; requirements that
  cannot all hold are an error naming each requirer. The capability ceiling
  now covers code you did not write: a registry package exceeding its declared
  effects fails every consumer's build, and contents changed under a fixed
  version number fail `grasm deps --locked`.

- **`par_map` — multi-core, deterministic.** `xs.par_map(f)` spreads elements
  across cores: **5.9× on 8 cores**, byte-identical output. Safety comes from
  the effect system — `f` must be effect-free and the compiler proves it
  ([T-PAR-MAP-PURE]) — and results are returned in input order regardless of
  core count ([E-PAR-MAP-ORDER]). `parallel`/`spawn` stays single-core by
  design; its tasks share scope and would race.

### Fixed

- `exit(n)` called `std::process::exit` from inside the interpreter, killing any
  embedder — the test harness died with status 3 instead of reporting a failure.
- The native map scanned its sorted entries linearly, making every map operation
  O(n) and whole programs quadratic. `wordfreq` was slower natively than in the
  interpreter (1430ms vs 460ms); binary search took it to 183ms.
- Boxing a `bool` or small `i64` allocated, so a sieve allocated once per flag
  write. Those values are now interned. `sieve`: 538ms → 84ms.
- `bench` with no name hung the parser: declaration recovery broke on the same
  token `parse_decl` had rejected. Recovery now always consumes something.
- `pad_start` / `pad_end` silently used only a multi-character fill's first
  character, and substituted a space for an empty fill.
- The lockfile reader collapsed `[[package]]` into a single table, so
  verification compared one package and ignored the rest.
- A task body could reference a binding declared inside its `parallel` block —
  a loop variable, say. The checker accepted it and the interpreter panicked at
  run time with "unknown name", because tasks run after the block's scope is
  gone. It is now a compile error ([T-TASK-CAPTURE]).

- Element access in a hot loop went through the runtime on every read and
  write. A list a loop body only ever indexes is now pinned for the loop and
  reached through its data pointer. The first round left a refcount round trip
  on each read — an increment and a matching decrement around a value the load
  never used, four indirect calls per iteration of `matmul`'s inner loop.
  `matmul` 99ms → 61ms → **30ms**, against C's 25ms.
- Maps scanned a sorted vector; they are hash tables now, and iteration sorts,
  so `[E-MAP-ORDER]` is paid once per iteration rather than on every lookup.
  A lookup also no longer copies its key onto the heap to look at it.
- `RtPayload::Variant` held its names as `String`, so every `Some(x)` — every
  map lookup builds one — allocated a private copy of the word "Some". Names
  are static data and are borrowed now.
- Scalar list iteration returned a boxed trait object, so summing a list paid
  a dynamic dispatch per element. Now 0.1ns/element.
- Two directories claiming the same package name resolved to whichever the
  walk reached first, silently, and the other dependent linked something it
  never asked for. It is an error naming both directories, both versions, and
  who required each.
- **The parallel-purity proof only checked closures written at the call site.**
  `xs.par_map(announce)` where `announce` prints compiled and ran across
  threads — including with `/ {io}` declared on it. The check walked the
  argument as an expression, and naming a value performs nothing; what counts
  is the row of the function about to run on several cores. Named functions,
  named closures, and effects reached transitively are all caught now.
- **`grasm deps` exited 0 for a package it could not analyze**, omitting it
  from the capability table and writing the lockfile. A dependency that cannot
  be checked has not passed; it is an error and no lockfile is written.
- Tuples printed as `[1, one, true]` natively and `(1, one, true)` under the
  interpreter: the runtime had no tuple, so the backend built one as a list.
  Found by writing [L-DISPLAY], which specifies what `print` produces for
  every type.
- A failed import named one of the two places it looked, and named the one it
  did not want. It lists both now.
- The native backend's static type inference — used where no annotation
  exists, most sharply for a list literal's element type — knew four builtin
  methods. Every other one failed there, so `[xs.min() ?? 0]` ran under the
  interpreter and would not compile natively while `xs.min() ?? 0` alone
  compiled fine. Found by `scripts/fuzz_differential.py`, which generates
  well-typed programs and compares both engines; the 130-program differential
  gate had missed it because none of those puts a reduction inside a list
  literal.
- `scripts/crosstest.sh` counted a link failure as "unsupported", so a runtime
  archive older than its own source read as a backend limitation and cost
  seven programs of coverage without a word. It now refuses a stale archive up
  front and names every skip with the step that gave up.

### Changed

- `scripts/difftest.sh` covers `spec/conformance/` and `benchmarks/` as well as
  `examples/` and `projects/` — 19 programs to 128. It compares stdout and exit
  codes strictly; stderr is compared with the interpreter-only panic trace block
  excluded, a gap recorded in the README rather than hidden.
