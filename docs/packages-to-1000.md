# Getting the package set to 1000

`docs/packages-to-650.md` closed at 650; the set is 669 now. This picks the
count up there and asks a different question first: **what does a serious
programming language have to ship before nobody can call it a toy?**

## How the list was made

Three references, read on 2026-10-18 through the Oort research tool:

- **Go's standard library index** (`pkg.go.dev/std`, go1.27.2) — the clearest
  statement anywhere of what one team thinks every program is entitled to.
- **blessed.rs** — the Rust community's own list of the crates everyone is
  expected to know, by category.
- **NIST's ACVP server releases and RFC 7932** — to confirm that the two
  largest new pieces, post-quantum signatures and Brotli, have a published
  specification and somewhere to check an implementation against.

Each was compared against the 669 by name, and then by function: a list of
candidates was grepped for `fn` definitions across `packages/`, because rule 7
forbids writing a package that already exists under another name. That pass
removed ten: `hmac` and `pbkdf2` (in `crypto_hash`), `soundex` `metaphone`
`nysiis` (in `phonetic`), `jaro` `dice` `cosine` (in `similarity`),
`circuitbreaker` (in `middleware`), `bwt` (in `bzip2`). FNV, which exists only as a helper inside `bloom`, stays
a candidate, because Go ships it on its own and so should this.

## The rule that decides what is buildable

Rule 6 says a package is checked against an implementation that already
exists. So the binding question for each candidate is not "is it useful" but
**"is there a reference on this machine to hold it to"**. These are:

| oracle | what it can check |
|---|---|
| OpenSSL 3.6.3 | ML-KEM, ML-DSA, SLH-DSA, X25519MLKEM768, every legacy cipher, PKCS#12, CMS, KDFs |
| Go 1.25 | `crypto/mlkem` `ecdh` `ecdsa` `des` `rc4`, `hash/fnv`, `index/suffixarray`, `net/textproto` `smtp`, `debug/dwarf`, `text/tabwriter`, `encoding/gob` |
| `brotli` `xz` `lz4` `zstd` | the compression formats, both directions |
| Node (ICU) | `Intl.Collator` (UCA), plural rules, list/number/relative-time formatting |
| Java | `java.text.Bidi`, `BigDecimal`, `javap`, `keytool`, `Properties` |
| Ruby, Perl | YAML (Psych), `Storable` |
| `ffmpeg` | audio and video containers, G.711, ADPCM, FLAC |
| Pillow, `cwebp` | image formats |
| fontTools | CFF, GSUB/GPOS, variable fonts, SVG paths |
| scapy | thirty-odd packet formats |
| `gpg`, `ssh-agent` | OpenPGP packets, the agent protocol |
| `git` | commit-graph, multi-pack-index, merge-file, apply |
| `sqlite3` | a dataframe's joins and groupings |
| `otool` `objdump` `dwarfdump` `nm` | object files |

A candidate with no oracle on the list is written down below as **deferred**
and not built, however useful. Writing it anyway would mean checking it
against expectations typed beside the code, which is the thing rule 6 exists
to stop.

## Not needed, and why

Not built, and not counted as missing:

- **Language features already.** Channels, `select`, futures, MPMC/SPSC
  queues, work-stealing pools, actors, barriers and latches: `concurrent`,
  `go` and `chan_*` are in the language (`docs/concurrency.md`), and tasks
  share nothing mutable, so lock-free structures have nothing to protect.
  `rayon` is `parallel`. `anyhow`/`thiserror` are `!` error types.
- **No meaning here.** `smallvec`/`arrayvec`/`bytemuck`/`zerocopy` (layout
  control Grasm does not expose), `zeroize` (no guarantee a library can
  make), `parking_lot`/`arc-swap`/`dashmap` (no shared mutable state),
  `once_cell`/`lazy_static` (no globals to initialise).
- **Runtime, not package.** `memmap2`, `signal-hook`, `notify`, GPU, windows
  and GUI toolkits: each needs a runtime capability first, as in
  `docs/ecosystem-200.md`.
- **Measured not worth it.** `indexmap`: 24.5µs a lookup at 1000 keys, but
  linear search beats hashing at the 10–50 keys a JSON object usually has.
  The real fix is `json.get`, a representation change, not a container.

## The candidates, in batches

Each batch is its packages plus one harness package, as before. A batch
lands when every package in it agrees with its oracle. For the algorithmic
batches (B15, B26) the oracle is a brute-force implementation in Python —
written independently, in another language, slow and obviously right — which
is the same standard `fuzz_laws` holds the core to.

### Tier 1 — what a language is judged on

**B1. Go's standard library, the parts not here** (Go, OpenSSL) —
`fnv` `suffixarray` `tabwriter` `textproto` `smtpwire` `des` `rc4` `ecdsa`
`ecdh` `mlkem` `dwarf` `pprofproto` `gobwire` `buildinfo`
`dsa` `gotemplate` `gotimefmt` `goquote`

**B2. Post-quantum, the rest** (OpenSSL) —
`mldsa` `slhdsa` `hybridkem`

**B3. Compression** (`brotli` `xz` `lz4` `gzip`, Python `lzma`) —
`brotli` `brotlienc` `lzmaraw` `lzip` `lz4frame` `compressz` `packbits`
`zstdenc`

**B4. YAML and other languages' files** (Ruby Psych, Java, Python, Perl) —
`yaml` `yamlemit` `jsonlines` `javaprops` `javaserial` `perlstorable`
`classfile` `jarmanifest` `pycfile` `taptest`
`v8serial`

**B5. Unicode and locale** (Node ICU 78, Java, Python `unicodedata`) —
`bidi` `collate` `pluralrules` `listformat` `relativetime` `numberformat`
`datetimeformat` `displaynames` `unicodeblock` `hangul`

**B6. Numbers** (Python `decimal` `fractions` `math` `cmath` `struct`,
Java `BigDecimal`, Go `math/big`) —
`decimal` `bigfloat` `modarith` `numtheory` `contfrac` `specialfn` `float16`
`bfloat16` `complexfn` `ntt` `bitops`

**B7. Calendars** (Node ICU `Intl.DateTimeFormat`) —
`hijri` `hebrewcal` `persiancal` `chinesecal` `japaneseera` `buddhistcal`
`ethiopiccal` `indiancal` `copticcal` `roccal`

`isoweek` was a candidate here and came off by rule 7: `iso8601` has the
ISO week date. `roccal`, Taiwan's Minguo calendar, which ICU also has,
went on.

### Tier 2 — what serious programs reach for

**B8. Block ciphers, hashes and modes** (OpenSSL) —
`camellia` `aria` `sm4` `sm3` `blowfish` `cast5` `idea` `seed` `rc2`
`cmac` `gmac` `kmac` `ccm` `ocb` `aessiv` `keywrap` `xts` `ripemd160`
`whirlpool`
`md4`

**B9. Curves** (OpenSSL) —
`ed448` `x448` `sm2` `ffdh`

P-384, P-521, secp256k1 and the Brainpool curves were four candidates
here; they landed as one package, `ecc`, with B1 -- see below.

**B10. Key and certificate files** (OpenSSL, `keytool`, `gpg`,
`ssh-agent`) —
`pkcs12` `pkcs1` `cms` `jks` `openpgp` `pgparmor` `sshagent` `kbkdf`
`x963kdf` `tls12prf` `sshkdf` `srpverifier`

**B11. Audio and video containers** (`ffmpeg`, `ffprobe`) —
`isobmff` `matroska` `oggpage` `vorbiscomment` `opushead` `flacdec` `adts`
`mp3frame` `aiff` `aufile` `caffile` `y4m` `webvtt` `assfile` `cuesheet`
`g711` `imaadpcm` `sdp`

**B12. Images** (Pillow, `cwebp`) —
`qoi` `pnm` `tga` `pcx` `xbm` `xpm` `sgiimage` `dds` `radiancehdr` `icns`
`apng` `gifenc` `webpenc` `jpegenc` `exif` `farbfeld`

**B13. Fonts** (fontTools) —
`cff` `gsub` `gpos` `kern` `otfname` `colrcpal` `ttcfile` `fvar` `gvar`
`svgpath` `hmtx` `os2table`

**B14. Packets, the Internet** (scapy) —
`vlan` `arp` `icmp` `icmpv6` `dhcp` `dhcpv6` `gre` `vxlan` `geneve` `sctp` `esp` `ikev2`
`stun` `rtp` `sixlowpan` `llmnr` `nbns`
`pcapng` `pflog` `sllcap`

Ethernet, IPv4, IPv6, TCP and UDP were candidates here and are already
`packets`; they came off the list by rule 7.

**B15. Packets, routing, links and LANs** (scapy) —
`igmp` `mpls` `lldp` `stp` `cdp` `rip` `ospf` `bgp` `eigrp` `isis` `pim`
`vrrp` `hsrp` `bfd` `nsh` `erspan` `l2tp` `pppoe` `eap` `dot11` `hci`
`dot15d4` `zigbee`
`btle` `canframe` `usbpcap` `radiotap`

**B16. Packets, the enterprise** (scapy) —
`snmp` `radius` `netflow` `kerberos` `ldapwire` `smb2` `dcerpc` `modbus`

**B17. Git, further down** (`git`) —
`commitgraph` `midx` `ewah` `mergefile` `gitblame` `gitlogfmt`
`gitstatus` `gitnotes` `ignorewalk` `reftable` `packrev` `packidx`

`gitapply` was a candidate here and came off by rule 7: `patchfile`
applies unified diffs already. `packidx`, the pack index itself, was
found missing and added.

**B18. Package ecosystems** (Go's x/mod, npm's own modules, Bundler, pip,
importlib.metadata) —
`gomod` `gosum` `cargolock` `packagejson` `npmlock` `yarnlock`
`gemfilelock` `pyproject` `requirements` `pkgmetadata` `cyclonedx`
`osvrecord` `ocimanifest` `sri` `npmname` `hostedgit` `npmspec`

`npmsemver` was a candidate here and came off by rule 7: `semverrange` is
npm's range semantics already. npm's `semver` becomes an oracle for it
instead. `sri`, `npmname`, `hostedgit` and `npmspec` were added on
2026-10-18, once npm's own `ssri`, `validate-npm-package-name`,
`hosted-git-info` and `npm-package-arg` were found in the npm that ships
with Node.

**B19. Archives** (`xar` `lsbom` `shar`, Python `zipfile`) —
`xar` `pkgbom` `debfile` `zipcrypto` `shar`

**B20. Web building blocks** (Node, Werkzeug, Python `urllib` `email`
`http.client`) —
`whatwgurl` `urlpattern` `dataurl` `digestauth` `contentdisposition`
`acceptheader` `cachecontrol` `httpchunked` `mimemessage` `imapwire`
`pop3wire`

### Tier 3 — the long tail a large ecosystem has

**B21. Data structures** (a brute-force Python model of each, fuzzed) —
`fenwick` `segtree` `intervaltree` `hamt` `lfu` `arccache` `jumphash`
`rendezvous` `rankselect` `wavelet` `sparseset` `xorfilter`
`quotientfilter` `minhash` `hnsw` `timerwheel` `slotmap`

`avlmap` and `treap` were candidates here and came off by rule 7, as
ordered maps `btree` and `skiplist` already are; so did `adaptiveradix`
(`trie`, `btree`) and `pairingheap` (`heap`).

**B22. Graphs** (Python `graphlib`, brute force) —
`scc` `mincostflow` `graphiso` `cliques` `graphcolor` `eulerpath`
`bridges` `lca`

`toposort` and `dominators` were candidates here and came off by rule 7:
`graphs.topological` and `ssaform.immediate_dominators` are them.

**B23. Text** (Python `re` and its parser, libc `regcomp` through `ctypes`) —
`pyregex` `regexsyntax`
`posixregex`

**B24. Signals** (gonum's `dsp/fourier`, `dsp/window` and `interp`,
brute-force definitions) —
`dct` `dwt` `spline` `windowfn`

**B25. Codecs Python has and the set does not** (Python `codecs`) —
`johab` `jisx0213` `cp950` `jisx0212`

**B26. Object files and the tools under them** (`otool` `objdump` `nm`
`codesign`, Go `go/parser`) —
`machofat` `codesign` `symtab` `buildid` `arm64dis` `x86len` `gosyntax`
`sqlitewal` `dataframe`
`sqlitejournal`

**B27. Documents** (`textutil`) —
`rtf` `docx` `odt`

**B28. What the Go module cache and the system can check** (`golang.org/x/net`
`x/tools`, `miekg/dns`, `paulmach/orb` and `osm`, `pires/go-proxyproto`,
`jackpal/go-nat-pmp`, `klauspost/compress`, Python `ntpath`, `xmllint`) —
`winpath` `xmlc14n` `txtar` `httpproxy` `bpf` `htmlcharset` `proxyproto`
`natpmp` `dnssec` `svcb` `wkb` `mvt` `maptile` `osmpbf` `s2compress`

Found on 2026-10-18 by listing the module cache: every one has its
reference on this machine. `langtag` and `ssdp` were looked at and are
`locale` and `netdiscovery` already.

## Deferred, for want of an oracle

`parquet` `arrowipc` `orc` `hdf5` `netcdf` (no pyarrow, h5py or netCDF4),
`mysqlwire` `mongowire` `cqlwire` `tdswire` (no server to hold them to),
`dbuswire` (no bus), `json5` `hjson` `edn` (no reference parser), `stl`
`ply` `obj` `gltf` (no mesh library), `proj4` `s2cell` `h3cell` (no
projection library), `flexlayout` (no layout engine outside a browser),
`ndarray` (no NumPy), `quic` `http3` (no peer), `osmpbf` `mvt` (no
`paulmach/protoscan` in the module cache). Each becomes buildable the
day its reference is installed.

## Landed

- [x] **B38: OpenType tables** — B13's `kern`, with `otfname` `os2table`
  `posttable` `ttcfile` `fvar` `avar` `stat` `colrcpal` `gsub` (B13 named
  `otfname`, `os2table`, `colrcpal` and `ttcfile` too), and the harness
  `b38test`; the new gate `b38diff` runs 3,734 cases against fontTools
  4.63: every table of those ten in 200 of macOS's font files (each font
  of a collection on its own) and in 40 fonts built here with fontTools'
  FontBuilder to reach COLR versions 0 and 1, CPAL with palette types,
  avar, every STAT axis-value format and fvar instances with and without
  PostScript names; collection directories against fontTools' TTC reader;
  and axis normalisation and avar mapping against fontTools'
  `normalizeValue` and `piecewiseLinearMap`, rounded to F2Dot14 as
  `otRound` rounds. Real fonts found three things: an extension lookup's
  later subtables were read unwrapped (DecoType Nastaleeq), NULL list
  offsets in GSUB mean empty lists (Noto), and fontTools reads only as many
  `post` names as the indices need and ignores a lone Microsoft `kern`
  subtable's overflowing 16-bit length, which the packages now do too. One
  break per package turned it red.

- [x] **B37: image formats** — B12's `qoi` `pnm` `tga` `pcx` `xbm` `xpm`
  `sgiimage` `dds` `icns` `apng`, with `sunraster`, which was not on the
  list, and the harness `b37test`; the new gate `b37diff` runs 1,667 cases
  against Pillow 12: files Pillow writes in every mode and compression it
  offers decoded here to Pillow's pixels, files written here decoded by
  Pillow, variants Pillow reads but does not write (16-bit TGA in every
  orientation, Sun rasters of every depth and colour map, legacy RLE icons,
  XPM, PNM with comments) made by small Python writers, and QOI's encoder
  equal to Pillow's byte for byte. Where Pillow reads a file wrongly or
  refuses what the format allows, the package follows the format and those
  inputs are left out: odd-width 24-bit PCX (Pillow, even reading its own,
  unpacks the planes by width rather than bytes-per-line), Sun RLE rows
  that need padding, TGA run packets crossing rows, XPM colours other than
  #RRGGBB, APNG blending with partial alpha. Pillow's own conventions were
  matched where the format leaves them open: PNM scaling rounds halves to
  even; a 16-bit TGA's top bit means transparent. Writing these found that
  `out = out + [...]` copies the whole list each time, quadratic (100,000
  appends 11 s against 0.01 s for `push`); the new packages push. One break
  per package turned it red. The count reaches 900.

- [x] **B36: PKCS#12 and Java keystores** — B10's `pkcs12` and `jks`, with
  the harness `b36test`; the new gate `b36diff` runs 206 cases on fresh
  identities each run (a CA and leaf certificates for RSA, EC and Ed25519
  keys): files `openssl pkcs12 -export` wrote under eight key schemes,
  four certificate schemes and four MAC hashes, or no MAC, read here with
  the key, both certificates, the friendly name and the local key id as
  they went in; files built here read by OpenSSL; keystores keytool
  converted read here, and keystores built here listed and converted back
  by keytool. A wrong password on a file with no MAC and an RC4-shrouded
  key decrypts to garbage with nothing to say so; OpenSSL refuses it when
  the garbage is not a key, and `pkcs12` now checks the same. Java 17
  refuses non-ASCII passwords for PBES2 key protection, so the keytool
  cases use ASCII ones. One break per package turned it red.

- [x] **B35: key files** — B10's `pkcs1`, with `pbkdf2` `pbes` `encpkcs8`
  `sec1` `pemcrypt`, which were not on the list (B10 named the files these
  read; `pkcs8` had left encrypted keys, PKCS#1 and SEC1 to other
  packages), and the harness `b35test`; the new gate `b35diff` runs 1,781
  cases against OpenSSL 3.6 on a fresh key pool each run (RSA of five
  sizes, three-prime RSA, e = 3, EC on five curves, Ed25519, X25519 and
  Ed448): every PKCS#1 and SEC1 structure read and written back byte for
  byte and converted to and from PKCS#8 and SubjectPublicKeyInfo as
  OpenSSL converts it; keys OpenSSL encrypted with PBES2 (five PRFs,
  eleven ciphers, scrypt) and the ten PBES1 and PKCS#12 schemes opened
  here, and keys encrypted here with `pbes`'s builders opened there;
  traditional `DEK-Info` PEM both ways with eleven ciphers; PBKDF2 over
  six hashes; and wrong passwords refused by both. A wrong password
  leaves valid padding about once in 256 tries, so the first run caught
  `pemcrypt` returning the garbage where OpenSSL, parsing it, refuses;
  both `pemcrypt` and `encpkcs8` now require the result to be a key. One
  break per package turned it red.

- [x] **B34: hashes, HMAC and key derivation** — B8's `ripemd160`
  `whirlpool` `sm3`, B10's `kbkdf` `x963kdf` `tls12prf` `sshkdf`, and
  `mdc2` `hmac` `sskdf` `x942kdf` `pkcs12kdf` `krb5kdf` `pbkdf1`, which
  were not on the list, with the harness `b34test`; the new gate
  `b34diff` runs 5,020 cases against OpenSSL 3.6's libcrypto: the four
  hashes around every block and padding boundary, HMAC over thirteen
  hashes with keys either side of each block size, and every KDF with
  every option varied (OpenSSL ignores a parameter name it does not know,
  so an ignored one shows up as disagreement). RIPEMD-160's tables come
  from x/crypto; Whirlpool's S-box is found in libcrypto, where OpenSSL
  stores each round-table entry in all eight rotations 64 bytes apart,
  and kept only because a model built on it hashes as OpenSSL does. Four
  OpenSSL behaviours were checked rather than copied: a KBKDF feedback IV
  must be empty or one PRF output long (as SP 800-108 defines K(0)), and
  `kbkdf` refuses other lengths; Kerberos constants longer than the block
  are refused there and here, RFC 3961 defining only shorter ones; an
  empty TLS PRF seed and an empty PKCS#12 salt and password together are
  refused by OpenSSL alone and kept out of the corpus. One break per
  package turned it red.

- [x] **B33: block ciphers** — B8's `camellia` `aria` `sm4` `seed`
  `blowfish` `cast5` `idea` `rc2`, with `blockmodes` (ECB, CBC, CFB, CFB8,
  OFB, CTR and PKCS#7 over any block function), which was not on the
  list, and the harness `b33test`; the new gate `b33diff` runs 7,173
  cases against OpenSSL 3.6's libcrypto: every cipher at every key size
  it allows (Blowfish 1 to 72 bytes, CAST5 5 to 16, RC2 1 to 128 at
  effective lengths of 1 to 1,024 bits) in every mode, both directions,
  with CTR counters that carry and wrap. No table was typed from memory:
  `scripts/gen_cipher_tables.py` computes Blowfish's from pi and compares
  them with x/crypto's, reads CAST5's and RC2's from x/crypto's source,
  and finds SM4's, Camellia's, ARIA's and SEED's S-boxes in libcrypto by
  shape alone, keeping a candidate only if a Python model of the cipher
  built on it encrypts as OpenSSL does at every key size; the gate
  re-derives them all. OpenSSL ignores parameter names it does not know,
  so RC2's effective length was silently unset until the name was found
  to be `keybits`. Blowfish keys over 72 bytes, CAST5 keys under 40 bits
  and RC2 keys over 128 bytes are refused here and accepted there. One
  break per package turned it red.

- [x] **B32: AES modes and MACs** — B8's `cmac` `gmac` `kmac` `ccm` `ocb`
  `aessiv` `keywrap` `xts`, with `gcmsiv` and `cbccts`, which were not on
  the list, and the harness `b32test`; the new gate `b32diff` runs 7,604
  cases against OpenSSL 3.6's libcrypto called in-process through ctypes
  (`scripts/lib/evp.py`, which later cipher batches reuse): AES and
  Triple-DES CMAC, GMAC with IVs of 1 to 40 bytes, KMAC and KMACXOF
  across the sponge's rate, CCM at every nonce and tag length (and
  associated data long enough for the 6-byte length framing), OCB, SIV
  with vectors of associated data, GCM-SIV, KW and KWP, XTS with
  ciphertext stealing, and CBC-CS1/CS2/CS3. Sealing is compared byte for
  byte, and opening on good, tampered and truncated inputs; every open,
  unwrap and decrypt family must see both verdicts. `sha3` gained cSHAKE
  and SP 800-185's encodings for KMAC. Three OpenSSL behaviours were
  checked rather than assumed: XTS refuses equal key halves for
  encryption but not decryption, and `xts` does the same; EVP "wraps" an
  empty key to nothing, which RFC 3394 and 5649 refuse and so does
  `keywrap`; and KMAC keys under 4 bytes are refused there and accepted
  here, as SP 800-185 allows. One break per package turned it red.

- [x] **B31: Go's module tooling** — B18's `gomod`, with `signednote`,
  `tlog` and `coverprofile`, which were not on the list, and the harness
  `b31test`; the new gate `b31diff` runs 3,270 cases against
  golang.org/x/mod and x/tools from the module cache. go.mod files of
  every directive, quoted and bare, with comments before, beside and
  inside blocks, CRLF and broken ones: what Parse reads and what Format
  writes, byte for byte. Signed notes: keys from seeds, notes signed by
  several keys, opened with known, unknown, repeated and ambiguous keys,
  tampered and malformed. Transparency logs of up to 300 records: tree
  hashes, the stored-hash layout, record and tree proofs made there and
  checked here whole and tampered, tile paths and tree heads. Coverage
  profiles with repeated blocks in all three modes. Each family must see
  both answers and refusals, or the gate fails. Go's base64 requires
  padding and ignores the unused bits of the last group, the reverse of
  `bytes_codec.decode_base64` on both; `decode_base64_go` reads it Go's
  way for the formats Go defines. Retracted versions are kept as written
  rather than canonicalised, as x/mod does. One break per package turned
  it red.

- [x] **B30: post-quantum signatures** — B2's `mldsa` and `slhdsa`, with
  the harness `b30test`; the new gate `b30diff` runs 1,334 cases against
  NIST's ACVP vectors (as Cloudflare's circl ships them) and circl itself,
  from the module cache: ML-DSA-44/65/87 key generation, signing and
  verification, internal and external, with contexts and pre-hashing;
  SLH-DSA in all twelve SHA2 and SHAKE parameter sets, deterministic and
  randomised, likewise. Signatures are byte-identical, and damaged ones
  are refused alike. Pre-hashing with SHA-224 and SHA-512/224 is refused,
  circl having neither to check it by. `sha3` read its Keccak round
  constants and rotations from top-level `let`s inside the permutation,
  rebuilding them about 600 times a call; passed in, one SHAKE call is 2.8
  times faster and SLH-DSA signing nearly five. One break per package
  turned it red.

- [x] **B29: modern public-key crypto** — `x448` `ed448` `hpke` `age`, with
  the harness `b29test`; the new gate `b29diff` runs 2,765 cases against
  Cloudflare's circl and metacubex/age, from the module cache: X448 over
  random and non-canonical u-coordinates and small-order points; Ed448 and
  Ed448ph keys and signatures equal to circl's byte for byte, and circl's
  verifying here and refused once tampered; HPKE in every KEM, KDF, AEAD
  and mode, ciphertexts and exported secrets identical both ways; age
  files in both directions (keys, passphrases, armor, payloads around the
  64 KiB chunk) and damaged ones refused alike. A 12-byte nonce built by
  shifting 8i bits put the sequence number in the wrong byte, since shifts
  wrap at 64; and every field operation in Ed448 rebuilt R^2 mod p from a
  top-level `let` until the context was threaded through (43 s to 0.15 s).
  HPKE's export-only mode is refused, circl having none to check it by.
  One break per package turned it red.

- [x] **B28, third part** — `s2compress` `crc32c` `natpmp` `xmlc14n`, with
  the harness `b28test3`; the new gate `b28cdiff` runs 8,921 cases:
  klauspost/compress's four block encoders and its stream writer read
  here and what is written here read there, hand-built blocks for every
  repeat-length class, and damaged blocks and streams judged alike; Go's
  Castagnoli CRC; go-nat-pmp's client talking to a loopback responder
  that answers with each case's bytes; and `xmllint --c14n` and
  `--exc-c14n` on random documents, with the no-comment forms held to
  xmllint on the same documents written without comments. `crc32c` was
  found missing (only `sctp` computed it, privately) and added. Three
  oracle quirks were kept out of the corpus rather than copied: libxml2
  writes `&` bare in a namespace URI, go-nat-pmp accepts a long mapping
  answer only because a 16-byte read truncates it, and klauspost's writer
  given nothing and an index option writes a stream its own reader
  refuses. One break per package turned it red. `mvt` moved to deferred:
  orb's vector-tile code needs the same missing `paulmach/protoscan`.

- [x] **B28, second part** — `htmlcharset` `svcb` `dnssec` `wkb`, with the
  harness `b28test2`; the new gate `b28bdiff` runs 10,597 cases against
  modules in the Go cache, offline: x/net's `DetermineEncoding` over
  documents built from decoys (comments, raw-text elements, attributes
  hiding `>`, the 1,024-byte cut) and the Encoding Standard's 228 labels,
  generated into the package by `scripts/gen_htmlcharset.py`; miekg/dns
  for SVCB in both forms (and the text the package writes, which miekg
  must pack back the same), key tags, DS digests, NSEC3 hashes, type
  bitmaps, validity windows, signatures it makes with random RSA, ECDSA
  and Ed25519 keys over random record sets, and Ed25519 and P-256
  signatures made here; orb for WKB and EWKB. Where miekg is lenient
  (empty RDATA, a record cut before its target, RSA/MD5 key tags summed
  like the rest) the package keeps RFC 9460 and RFC 4034, and the
  generator stays clear. One break per package turned it red. `osmpbf`
  moved to deferred: osm's PBF reader needs `paulmach/protoscan`, whose
  source is not in the module cache.

- [x] **B14 and B15, the radio and capture leftovers** — `pcapng` `dot11`
  `dot15d4` `btle` `ikev2`, with the harness `packettest4`; the new gate
  `packet4diff` runs 11,787 cases. tshark reads random pcapng files (two
  sections, both byte orders, decimal and binary resolutions, offsets,
  Simple and obsolete packet blocks, name and statistics blocks) and
  reads what `write` makes of them the same; tshark also reads 802.15.4
  frames under 2015's PAN ID compression table and every BLE PDU type;
  scapy builds 802.11, 2006 802.15.4 and IKEv2 from random fields, and its
  CRC-24 holds BLE's. Scapy was wrong three times and the packages kept
  the standard: the 802.11 capability field is little-endian, BLE's
  CRCInit and access address go least significant byte first, and an
  IPv6 traffic selector is 40 bytes long, not 20. A 64-bit address with
  the top bit set made `x.ushr(0) % 256` negative and crashed the first
  run; byte extraction now masks. One break per package turned it red.

- [x] **B25: codecs** — `johab` `cp950` `jisx0212` `jisx0213`, with the
  harness `cjktest`; the new gate `cjkdiff` runs 312,023 cases against
  Python's codecs: every byte, every pair a lead byte can start (refusals
  included), every EUC triple, every character decoded and encoded back,
  every encode-only code point and a sample of refused ones, in seven
  forms: Johab, CP950, Shift_JIS-2004, EUC-JIS-2004, their 2000 editions,
  and EUC-JP with JIS X 0212. `scripts/gen_dbcs.py` learned a trail base
  (Johab's symbols start at 0x31) and still reproduces CP949 byte for byte;
  `scripts/gen_jis.py` writes the JIS tables with a `--check` the gate
  runs. One break per package turned it red; Johab's first, keep the last
  of each duplicate, was neutral because Python does exactly that, and was
  replaced by generating the table without its trail base.

- [x] **B24: signals** — `dct` `dwt` `spline` `windowfn`, with the
  harness `sigtest`; the new gate `sigdiff` runs 2,340 cases: DCT-I and
  DST-I, every window, and five kinds of cubic spline (values and
  derivatives, past both ends too) against gonum, run offline from the
  module cache; DCT-II to IV and the Kaiser window against their
  definitions summed in Python; Haar and Daubechies-4 against the
  orthogonal matrix built from the filter; LeGall 5/3 and CDF 9/7 against
  JPEG 2000's lifting steps and reconstruction. A single sample through
  the 5/3 transform could not be inverted until JPEG 2000's own rule --
  it passes unchanged -- was followed. One break per package turned it red.

- [x] **B7: calendars** — `hijri` `hebrewcal` `persiancal` `chinesecal`
  `japaneseera` `buddhistcal` `ethiopiccal` `indiancal` `copticcal`
  `roccal`, with the harness `caltest`; the new gate `caldiff` holds 69,090
  dates across three thousand years to ICU 78.1 through Node -- random
  days, runs around year ends, leap months, era changes, the Umm al-Qura
  table's edges, the 1582 changeover ICU's Gregorian-based calendars keep,
  and every Rosh Hashanah from 4200 to 6329 AM. Umm al-Qura's months, the
  237 Japanese eras and the Chinese and Korean months of 1900 to 2100 are
  tables generated from ICU (`scripts/gen_calendars.py`, with `--check`).
  The Hebrew calendar was wrong before it was tested: GaTaRaD and
  BeTUTaKPaT were re-applied after a Sunday molad moved to Monday,
  postponing 38 years in 7000 by a day, 5807 among them -- found when ICU
  disagreed on three dates and *Calendrical Calculations*' own formula,
  written out in Python, sided with ICU over 20,000 years. One break per
  package turned its calendar red, the Indian one after a first break that
  cancels itself was replaced.

- [x] **B17, first part** — `packidx` `packrev` `midx` `commitgraph`
  `gitlogfmt`, with the harness `gittest`; the new gate `gitdiff` runs 949
  cases on twelve repositories built with `git commit-tree` -- exact
  messages, time zones, octopus merges, children dated before their
  parents, several packs -- against `git show-index` (version 1, version
  2, and version 2 with every offset in the 64-bit table), the .rev files
  git wrote, each pack's own index for the multi-pack-index, `git log` for
  the commit-graph and `git log -1 --format` for the pretty formats.
  Found on the way, in git 2.48 itself: a corrected-date offset of 2^32 or
  more is written to the commit-graph's overflow chunk with its high word
  lost, and commits dated past 2^32 get corrected dates below their
  parents'; `git commit-graph verify` reports both, and the package's
  `verify` reports the same commits (checked against it). The gate
  compares those dates only where git writes them correctly. `%f`
  sanitises only the first line of a subject. One break per package turned
  its family red.

- [x] **B18, first part** — `sri` `npmname` `gosum` `gemfilelock`
  `pkgmetadata`, with the harness `b18test`; the new gate `b18diff` runs
  1,830 cases against npm's own `ssri` and `validate-npm-package-name`
  (scripts/lib/node), x/mod's dirhash (over file sets and Python-written
  zips), Bundler's LockfileParser (scripts/lib/ruby) and
  importlib.metadata. The quirks followed are the references' own:
  ssri orders algorithms whose names are all digits first, as JavaScript
  object keys; Bundler folds a repeated gem only while the earlier one has
  no dependencies yet, because its key includes a list that grows after
  hashing, and sorts the ruby platform as "\0"; RubyGems sorts a
  requirement's parts by version. One break per package turned its family
  red, npmname's after a first break that changed nothing was replaced.

- [x] **B6, most of it** — `numtheory` `contfrac` `specialfn` `float16`
  `bfloat16` `complexfn` `ntt` `bitops`, with the harness `numtest`; the
  new gate `numdiff` runs 11,453 cases against Python: brute force for
  number theory, `fractions` for continued fractions, `decimal` for
  square-root expansions, schoolbook products for the NTT, bit-by-bit
  definitions, `struct` for half and single precision, `math` and `cmath`
  within stated tolerances. Found on the way: negating with `0.0 - x`
  loses the sign of a zero, which put complex `asin(2 + 0i)` on the wrong
  side of its branch cut; baby-step giant-step kept the first baby step of
  each value and so could miss the smallest logarithm when the base has
  small order; and erfc lost 5e-14 near x = 27 to the rounding of x^2,
  until the square was split as libm splits it. Every control turned its
  family red, two only after the generator was given small-order bases
  and exact ties for `limit_denominator`. `decimal` and `bigfloat` remain.

- [x] **B28, first part** — `winpath` `txtar` `httpproxy` `bpf`
  `proxyproto` `maptile`, with the harness `b28test`; the new gate
  `b28diff` runs 3,900 cases against CPython's `ntpath` and, through
  `scripts/lib/gomods` built offline with the module cache's Go 1.27
  toolchain, against x/tools/txtar, x/net/http/httpproxy, x/net/bpf,
  pires/go-proxyproto and paulmach/orb/maptile. Found on the way: Go reads
  an IPv4-mapped CIDR in NO_PROXY (`::ffff:10.0.0.0/104`) as the IPv4 block
  10.0.0.0/8, and the package now does too; and x/net/bpf's VM decodes
  `neg` but cannot run it, where the kernel negates A -- the package
  follows the kernel, and the gate compares only the text of programs
  that reach a `neg`. One break in each package turned its family red,
  after two first attempts that changed nothing observable were replaced
  and the generators given UNC paths and programs that return A.

- [x] **B21, second part** — `lfu` `arccache` `jumphash` `rendezvous`
  `minhash` `xorfilter` `quotientfilter`, with the harness `dstest2`; the
  new gate `ds2diff` runs 1,190 cases: the caches against Python models,
  `jumphash` against the paper's loop, `rendezvous` against
  dgryski/go-rendezvous and cespare/xxhash built offline from the module
  cache (`scripts/lib/gomods`), `minhash` against its hash family and the
  true Jaccard similarity, `xorfilter` by its guarantees (no member
  missed; 189 of 45,000 strangers let in, against 1 in 256 expected), and
  `quotientfilter` against a multiset of fingerprints. One break in each
  package turned all seven families red. go-rendezvous's own `Remove`
  indexes one past the end of its slice, so only `New` and `Lookup` are
  used as the reference.

- [x] **B21, first part** — `fenwick` `segtree` `intervaltree`
  `rankselect` `wavelet` `sparseset` `slotmap` `timerwheel`, with the
  harness `dstest`; the new gate `dsdiff` runs 1,860 random scripts of
  operations against Python models. The wheel had a defect before its first
  run: a timer cascading down onto its own deadline was filed as overdue
  and fired one `advance` late. Reverting that fix turned 25 cases red;
  breaking interval pruning, the rank mask and the quantile step turned
  540 more.

- [x] **B22: graphs** — `scc` `bridges` `eulerpath` `lca` `graphcolor`
  `cliques` `graphiso` `mincostflow`, with the harness `graphtest`; the new
  gate `graphdiff` runs 1,806 cases against brute force written in Python:
  reachability from every vertex, a bridge as an edge whose removal splits
  a component, every subset, every permutation, every colouring, every
  integer flow. Where the answer is not unique -- an Euler walk, an
  isomorphism, a flow -- it is checked for being one, and the brute force
  decides whether one exists. Breaking the bridge test, the isomorphism
  check and the negative-cycle refusal turned 113 cases red.

- [x] **B16, first part** — `snmp` `radius` `netflow` `modbus`, with `md4`
  from B8, and the harness `packettest3`; the packet gate runs 26,608
  cases. SNMP v1/v2c against scapy, RADIUS's response authenticator against
  scapy's and its password hiding against Python's `hashlib`, NetFlow v5 and
  v9 templates, Modbus TCP and RTU with CRC-16, MD4 against OpenSSL's
  legacy provider. On the way, `asn1.integer_of` read an INTEGER as
  unsigned and by multiplication -- a negative one came back wrong, and a
  twenty-octet CRL number trapped a debug build; it is two's complement
  and shift-built now. `ntlm` is left out of this batch.

- [x] **B15, second part: interior routing and the air** — `ospf` `bgp`
  `eigrp` `pim` `isis` `eap` `radiotap` `canframe` `usbpcap` `hci`, checked
  through `packettest2`; the packet gate runs 22,678 cases against scapy.
  OSPF's LSA Fletcher checksum (which a control moved by one byte and the
  gate caught), BGP's UPDATE attributes with one- and two-byte lengths,
  radiotap's per-field alignment counted from the header's start, EAPOL-Key
  frames.

- [x] **B15, first part: routing, links and LANs** — `igmp` `mpls` `lldp`
  `stp` `cdp` `rip` `vrrp` `hsrp` `bfd` `nsh` `erspan` `l2tp` `pppoe`, and
  the harness `packettest2`; the packet gate now runs 17,502 cases against
  scapy, every one of these thirteen agreeing on first contact. VRRPv3's
  checksum over an IPv4 or IPv6 pseudo-header, and CDP's checksum with
  Cisco's rule for an odd final byte, which a control removed and the gate
  caught.

- [x] **B14, second part: transports and services** — `rtp` `stun` `sctp`
  `dhcp` `dhcpv6` `esp` `nbns` `pflog`, checked through `packettest`; the
  gate now runs 10,602 cases. ESP with the NULL cipher is held to scapy and
  with AES-GCM to Go's `crypto/cipher`, tampered packets refused. Found on
  the way: a `pf_addr` is sixteen bytes for every family, which the first
  `pflog` had as four for IPv4; STUN padding may be any byte, and scapy
  writes spaces, so `stun` keeps what it read.

- [x] **B14, first part: links and tunnels** — `icmp` `icmpv6` `arp` `vlan`
  `gre` `vxlan` `geneve` `sllcap`, and the harness `packettest`. 5,202 cases
  against scapy: every packet built by scapy from random fields, decoded to
  scapy's own reading of it, and re-encoded to scapy's bytes -- checksums,
  lengths and ICMPv6's pseudo-header included -- with damaged checksums
  reported as damaged.

- [x] **B1, Go's template language** — `gotemplate`, checked through
  `gostdtest2`. 12,000 templates against Go's `text/template` -- 9,000
  generated from every construct, half of them well-typed for one Helm-like
  document, and 3,000 of those damaged -- run on JSON decoded as Go decodes
  it: 5,320 render to identical output and 6,680 are refused by both. Found
  on the way: `''` as a character constant, which Go refuses and a quoting
  routine accepts as empty, and `{{$x := $x}}`, which parses and fails only
  when run.

- [x] **B1, Go's formatter** — `goprintf`, and the harness `gostdtest2`.
  58,000 cases against Go's `fmt.Sprintf`, `Sprint`, `Sprintln` and
  `strconv.FormatFloat`: random format strings over every flag, width,
  precision, `*` and `[n]` form and verb, on operands of every kind --
  including the `%!d(string=x)`, `%!(MISSING)`, `%!(EXTRA ...)` and
  `%!(BADINDEX)` forms. Not on the list; `gotemplate` needs it for `printf`
  and for printing any value at all.

- [x] **B1, second half: the public-key half** — `modarith` `ecc`
  `ecdh` `ecdsa` `rfc6979` `dsa` `mlkem`, and the harnesses `pubkeytest` and
  `pqctest`. 2,711 cases against OpenSSL 3.6 and Python, and 189 against
  OpenSSL's ML-KEM: public keys and shared secrets on P-256, P-384, P-521,
  secp256k1, three Brainpool curves and SM2; ECDSA and DSA signatures
  byte-identical to OpenSSL's RFC 6979 `nonce-type:1`; verdicts on damaged
  signatures and off-curve points; ML-KEM-512/768/1024 keys, ciphertexts and
  implicit-rejection keys byte for byte; `modarith` against Python's `pow`.
  One generic curve package replaced four per-curve candidates from B9: the
  parts that go wrong -- reduction, the special cases of addition, the
  scalar loop -- are better written once and checked on eight curves.
  `rfc6979` was not on the list and is now: `ecdsa` and `dsa` both need
  it, and Rust ships it as its own crate. Everything that touches a secret
  is constant-time in the source, with `x25519`'s caveat that the compiler
  does not promise to keep it.

- [x] **B1, first half: Go's library, the parts with no keys** — `fnv`
  `rc4` `des` `goquote` `gotimefmt` `tabwriter` `textproto` `suffixarray`,
  and the harness `gostdtest`. 126,265 cases against Go 1.25's `hash/fnv`,
  `crypto/rc4`, `crypto/des`, `strconv`, `time`, `text/tabwriter`,
  `net/textproto` and `index/suffixarray`; the suffix array itself against
  every suffix sorted. Go's printable-rune table is generated from Go by
  `scripts/gen_goprint.go` and the gate fails when it is stale. Found on the
  way: a mistyped limb of FNV-128's offset basis, and `canonical_key`
  half-canonicalising a key that Go returns untouched because of a byte
  later in it.

## Where it stands

| | |
|---|---|
| At the start | 669 |
| Now | 911 |
| Candidates above | 316 packages in 28 batches, plus `ecc`, `rfc6979` and `goprintf` |
| Harnesses, one a batch | 28 |
| If every candidate lands | 1016 |
| Target | 1000 |

The margin over 1000 is 16. It is there because some
candidate will turn out to exist already under a name the grep missed, or to
disagree with its oracle in a way that says the oracle is the better
package, and the plan should not need rewriting when that happens.
