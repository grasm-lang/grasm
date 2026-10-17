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

**B7. Calendars** (Node ICU `Intl.DateTimeFormat`, Python `datetime`) —
`hijri` `hebrewcal` `persiancal` `chinesecal` `japaneseera` `buddhistcal`
`ethiopiccal` `indiancal` `copticcal` `isoweek`

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
`ethernet` `vlan` `arp` `ipv4hdr` `ipv6hdr` `tcphdr` `udphdr` `icmp`
`icmpv6` `dhcp` `dhcpv6` `gre` `vxlan` `geneve` `sctp` `esp` `ikev2`
`stun` `rtp` `sixlowpan` `llmnr` `nbns`
`pcapng` `pflog` `sllcap`

**B15. Packets, routing, links and LANs** (scapy) —
`igmp` `mpls` `lldp` `stp` `cdp` `rip` `ospf` `bgp` `eigrp` `isis` `pim`
`vrrp` `hsrp` `bfd` `nsh` `erspan` `l2tp` `pppoe` `eap` `dot11` `hci`
`dot15d4` `zigbee`
`btle` `canframe` `usbpcap` `radiotap`

**B16. Packets, the enterprise** (scapy) —
`snmp` `radius` `netflow` `kerberos` `ldapwire` `smb2` `ntlm` `dcerpc`
`modbus`

**B17. Git, further down** (`git`) —
`commitgraph` `midx` `ewah` `mergefile` `gitapply` `gitblame` `gitlogfmt`
`gitstatus` `gitnotes` `ignorewalk`
`reftable` `packrev`

**B18. Package ecosystems** (Go, npm's own `semver`, Bundler, pip) —
`gomod` `gosum` `cargolock` `packagejson` `npmsemver` `npmlock` `yarnlock`
`gemfilelock` `pyproject` `requirements` `pkgmetadata` `cyclonedx`
`osvrecord` `ocimanifest`

**B19. Archives** (`xar` `lsbom` `shar`, Python `zipfile`) —
`xar` `pkgbom` `debfile` `zipcrypto` `shar`

**B20. Web building blocks** (Node, Werkzeug, Python `urllib` `email`
`http.client`) —
`whatwgurl` `urlpattern` `dataurl` `digestauth` `contentdisposition`
`acceptheader` `cachecontrol` `httpchunked` `mimemessage` `imapwire`
`pop3wire`

### Tier 3 — the long tail a large ecosystem has

**B21. Data structures** (a brute-force Python model of each, fuzzed) —
`fenwick` `segtree` `intervaltree` `hamt` `adaptiveradix` `treap` `avlmap`
`lfu` `arccache` `jumphash` `rendezvous` `rankselect` `wavelet` `sparseset`
`xorfilter` `quotientfilter` `minhash` `hnsw` `timerwheel` `slotmap`
`pairingheap`

**B22. Graphs** (Python `graphlib`, brute force) —
`toposort` `scc` `dominators` `mincostflow` `graphiso` `cliques`
`graphcolor` `eulerpath` `bridges` `lca`

**B23. Text** (Python `re` and its parser, libc `regcomp` through `ctypes`) —
`pyregex` `regexsyntax`
`posixregex`

**B24. Signals** (brute-force definitions, `ffmpeg` filters) —
`dct` `dwt` `spline` `windowfn`

**B25. Codecs Python has and the set does not** (Python `codecs`) —
`johab` `jisx0213` `cp950`

**B26. Object files and the tools under them** (`otool` `objdump` `nm`
`codesign`, Go `go/parser`) —
`machofat` `codesign` `symtab` `buildid` `arm64dis` `x86len` `gosyntax`
`sqlitewal` `dataframe`
`sqlitejournal`

**B27. Documents** (`textutil`) —
`rtf` `docx` `odt`

## Deferred, for want of an oracle

`parquet` `arrowipc` `orc` `hdf5` `netcdf` (no pyarrow, h5py or netCDF4),
`mysqlwire` `mongowire` `cqlwire` `tdswire` (no server to hold them to),
`dbuswire` (no bus), `json5` `hjson` `edn` (no reference parser), `stl`
`ply` `obj` `gltf` (no mesh library), `proj4` `s2cell` `h3cell` (no
projection library), `flexlayout` (no layout engine outside a browser),
`ndarray` (no NumPy), `quic` `http3` (no peer). Each becomes buildable the
day its reference is installed.

## Landed

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
| Now | 689 |
| Candidates above | 310 packages in 27 batches, plus `ecc` and `rfc6979` |
| Harnesses, one a batch | 27 |
| If every candidate lands | 1008 |
| Target | 1000 |

The margin over 1000 is 8. It is there because some
candidate will turn out to exist already under a name the grep missed, or to
disagree with its oracle in a way that says the oracle is the better
package, and the plan should not need rewriting when that happens.
