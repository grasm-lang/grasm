# GrasmWire

A VPN protocol written entirely in Grasm. No C, no `.rs` in the protocol, no
shelling out to `wg`. The field arithmetic, the cipher, the hash, the key
schedule, the handshake and the protocol are all Grasm source in `packages/`,
and the only thing under them is the Grasm compiler.

## What it is

| layer | package | lines | what it does |
|---|---|---|---|
| curve | `x25519` | 425 | Diffie-Hellman over Curve25519 |
| cipher | `chacha20` | 175 | the stream cipher |
| authenticator | `poly1305` | 185 | the one-time authenticator |
| AEAD | `chachapoly` | 145 | the two, joined as RFC 8439 says |
| hash | `blake2s` | 220 | hash, keyed hash, HMAC |
| key schedule | `hkdf` | 90 | RFC 5869, over any HMAC |
| handshake | `noise` | 471 | Noise_IKpsk2 |
| protocol | `grasmwire` | 524 | counters, replay, timers, padding |
| endpoint | `apps/grasmwire` | 345 | UDP, keys, the loop |

**2,580 lines**, against a budget of five thousand.

## What is checked, and against what

Everything with a published vector is checked against it. Everything without
one is checked as a property.

| | |
|---|---|
| ChaCha20 | every RFC 8439 vector |
| Poly1305 | every RFC 8439 vector, including A.3 #2 |
| ChaCha20-Poly1305 | RFC 8439 §2.8.2 |
| HChaCha20 | the XChaCha20 draft's vector |
| BLAKE2s | RFC 7693 |
| X25519 | every RFC 7748 vector — both scalar multiplications and the whole §6.1 exchange |
| HKDF | all three RFC 5869 SHA-256 cases, both halves |
| the handshake | 19 properties, 3 pinned values |
| the protocol | 45 properties |
| end to end | two processes, real UDP, with and without a cookie |
| everything, again | 753 random cases against an implementation sharing no code |

The handshake has no published vectors, because the construction is Noise's
but the identifier string is this protocol's own. So it is checked two ways:
as properties that any correct handshake must satisfy, and as three pinned
values — the session hash and both transport keys, from fixed keys — so that
a refactor which moves a label or a mix step is caught by the test file rather
than by a peer six months later.

### The end-to-end run

Two processes, two real UDP sockets, ephemeral keys from the operating system:

    [connect] sent a 148-byte first message
    [listen]  the caller is e3712d851a0e5d79, which is on the list
    [listen]  sent a 92-byte reply
    [connect] handshake complete, session hash 49a50e7e0efd328c...
    [listen]  handshake complete, session hash 49a50e7e0efd328c...
    [connect] sent packet 0: 40 bytes became 80 on the wire
    [connect] packet 0 came back byte for byte
    [listen]  a replayed handshake, refused on its timestamp
    [listen]  rekeyed, new session hash 13c83192aced27f1...
    [connect] a packet under the new key came back byte for byte

and, when the responder is told it is under load:

    [listen]  under load: answered with a cookie and stored nothing
    [connect] the far side is under load and sent a cookie
    [connect] sent the same message again with mac2 on it
    [listen]  the second attempt carries a valid mac2, proceeding

Both sides derive the same session hash without either sending it. Three IP
packets go through the tunnel and come back unchanged. A recorded first
message, replayed with every tag in it genuine, is refused on its timestamp.
A cryptographically valid handshake from a key that is not on the list is
refused after the identity is decrypted and before anything else happens.

## The four things a session does after the handshake

**It rekeys.** A session is replaced after two minutes and refused after three,
and the gap between those is deliberate: rekeying costs a round trip, and a
peer that only started when the old key was already invalid would drop traffic
every time. The end-to-end run does a real one — a new ephemeral, a newer
timestamp, both sides landing on the same new session hash, and a packet
carried under the new keys.

**It follows a peer that moves.** The endpoint is learned from the datagram,
not configured, and it moves **only when a packet that authenticates comes from
somewhere else**. Moving it on an unauthenticated packet would let anyone
redirect a live session by spoofing one datagram, which is a denial of service
that costs the attacker nothing.

**It refuses a recording.** A first message replayed later carries the old
timestamp, and the responder keeps only the greatest it has seen per peer —
which refuses the recording without storing a list of everything ever seen.

**And under load it stops allocating.** `mac1` proves the sender knew a public
key, and public keys are public, so it filters noise rather than an attacker.
Under real load the responder answers an initiation with a **cookie**: a keyed
hash of the sender's own address, sealed to whoever is at that address. Only
somebody actually there can read it, the next message must carry `mac2`
computed under it, and the responder recomputes the cookie from the address in
front of it — having stored nothing in between. An attacker who can make a
responder allocate per unanswered packet has already won, so it does not.

The cookie is sealed with **XChaCha20-Poly1305**, and that is not decoration.
A 12-byte nonce is safe when a counter guarantees it never repeats, and there
is no counter here: the key does not change and the cookie goes to whoever just
wrote in, so the nonce has to be random — and 96 random bits collide around
2^48. 192 do not. The construction needed only HChaCha20, which is the same
twenty rounds with the final addition left out.

## Decisions worth defending

**The construction is Noise's, the implementation is not.** Inventing a key
exchange is how people ship protocols that stay broken for years. Noise_IK has
had the analysis a new design would not have. What is new here is that it runs
on Grasm from the field arithmetic up.

It is **WireGuard's variant** of Noise_IKpsk2 rather than literal Noise, and
the difference is worth naming: in the Noise specification the `e` token calls
`MixHash` on the ephemeral and nothing else, while here — as in WireGuard —
each ephemeral is also mixed into the chaining key. Both sides do it, so the
handshake is self-consistent; the point of writing it down is that a reader
checking this against a specification needs to know which one.

**It is not wire-compatible with WireGuard**, and the reason is one line:
`identifier` differs, so the initial hash differs, so every byte after it
differs. Claiming compatibility would mean testing against a peer this project
does not have, and an untested compatibility claim is worse than an honest
incompatibility.

**Every package below the endpoint declares no capabilities at all.** Not one
of them can open a socket, read a file or see the clock — `grasm deps` says so
and the compiler enforces it. A handshake needs randomness and a clock but does
not need the *capability* to reach either: both arrive as arguments from the
caller who already has them. That is also what makes the handshake
reproducible enough to pin, and what makes forty-five protocol properties
testable with no socket and no waiting. Three minutes of session lifetime is
tested in no time at all.

**Ten limbs, not five.** Grasm has one integer type, `i64`. A field element in
five 51-bit limbs needs a 102-bit product, which does not fit. Ten limbs of
26 and 25 bits keeps every product under 52 bits. This is a real cost the
language imposed and the arithmetic absorbed it.

**Nothing branches on a secret.** The conditional subtraction in the curve, the
point swap in the ladder, the tag comparison and the mac comparison are all
mask-based. A peer is never told *which* check failed, because that would be an
oracle.

## Two orderings that are load-bearing

The send counter advances **before** the datagram exists, so a send that fails
cannot roll it back and later encrypt two different payloads under one nonce.

The replay window advances **only after** the tag verifies, so a forged packet
cannot burn a counter the real peer is about to use and get its genuine packet
dropped as a replay.

Both are written down at the line where they happen, because both are the kind
of thing a later reader would helpfully "clean up".

## What building it found

**In the language, a missing builtin.** `udp_recv` answered the datagram and
dropped the address it came from, so a responder could only reply to peers it
had been configured with in advance — which rules out every protocol where a
client's address is unknown until it calls, and rules out roaming, which is one
of the things a VPN is *for*. `udp_recv_from(h) -> (bytes, str, i64) ! IoError`
now exists, in all three engines, checked by `agreetest`. The responder here
uses it, and follows a peer that moves — but only on a datagram that
*authenticates*, since following an unauthenticated one would let anyone
redirect a session by spoofing a single packet.

**In the language, a wrong sentence.** One spec line was imprecise: `args()` was described as
"arguments after `--`", which is true under `grasm run` and not true of a
compiled binary, where there is no separator to strip. The behaviour was
already correct and already checked by `agreetest` in all three modes; only the
one-line summary was wrong, and it is now `[L-LIB-ARGS]`.

**In my own code, by building on it.** The `noise` package documented that
every handshake message is exactly one of two lengths and did not enforce the
timestamp width, so a thirteen-byte timestamp silently produced a 149-byte
message. A length is the one field an observer reads for free. And a test
vector here was forty-two bytes claiming to be forty; `unpad` returned forty,
which is what a length field is for — the code caught the test.

## What is missing, and what it needs

**There is no TUN device**, and this one is not going to be added blind.
Without it GrasmWire tunnels packets a program hands it rather than packets the
operating system routes. Everything above the device works and is tested: the
end-to-end check sends real IPv4 packets, and the padding is undone by reading
the length field out of their headers, so the data path is exercised with the
thing it would actually carry.

What is absent is the kernel interface, and attaching to one **requires root**.
Measured here rather than assumed: opening `com.apple.net.utun_control` gets as
far as the control id and then fails with `Operation not permitted` for an
ordinary user. Linux's `/dev/net/tun` wants `CAP_NET_ADMIN`.

That makes it the one piece that cannot be held to this project's own standard.
A builtin here would be platform-specific unsafe FFI that no gate could run,
in a language whose entire discipline is that four engines agree and every gate
is green. Writing it untested would buy a checkbox and spend the thing that
makes the rest of this trustworthy. It needs a machine with root and a gate
that can use one.

**`now_millis()` is the wall clock.** A clock adjustment moves a session's
apparent age, which is the input to every timer here. A monotonic clock is the
correct source for a duration and Grasm does not have one.

None of the three is a defect in what is written. All three are places where
the protocol is finished and the language is not, and they are listed here
rather than worked around quietly.
