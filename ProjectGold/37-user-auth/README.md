# 37 — User Authentication  *(phase 2, project 7)*

**Tests:** a real password hash, an unguessable session token, a guarded route —
and what a language can honestly claim about each.
**Result:** works. Still **not something to deploy** — one reason closed since
it was written, one remains, and both are below.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then talk to it with curl
```

## Where project 27 stops being an exercise

`lib_sha256` is project 27's from-scratch SHA-256, extracted as a package. Here
it stands between a stolen file and every account in it, so the first thing this
project does is check it against the two canonical vectors:

```
sha256("")    = e3b0c442...7852b855
sha256("abc") = ba7816bf...f20015ad
```

A crypto implementation that is not checked against known answers is a random
number generator with extra steps.

## Two things this gets right

**Salted.** Without a salt, two accounts with the same password have the same
hash — which tells an attacker who to try first and turns one cracked password
into two.

**Iterated.** 600 rounds rather than one, so a stolen file is not a rainbow
table lookup away from the passwords.

And two smaller ones, both of which cost something:

* **Login says "no such account, or wrong password" for both.** Telling them
  apart tells an attacker which addresses are worth attacking. The cost is a
  worse message for an honest user, and that is the right way round.
* **Digests are compared character by character**, not with `==`, which returns
  as soon as two differ and so leaks how much of a guess was right. "-ish",
  because a language with no `black_box` cannot promise the backend keeps the
  loop — written that way anyway, since the intent is checkable where the
  guarantee is not.

## Two things it gets wrong, on purpose, and says so

**SHA-256 is not a password hashing function.** It is fast, and fast is the
wrong property: it is what lets an attacker try billions of guesses a second.
A real system uses argon2 or scrypt, which are deliberately slow and
memory-hard. Iterating is the only defence available here and is strictly
weaker.

**The session tokens *were* predictable, and are not any more.** When this was
written Grasm had `now_millis()` and no other source of randomness (**F34**), so
tokens were derived from the clock and guessable by anyone who knew roughly when
a session began. That was in the source's first paragraph rather than hidden.

`random_bytes` closed it. Tokens are 256 bits of kernel entropy, and if entropy
is unavailable `login` answers **500 rather than issuing one** — a token that
might be guessable is worse than an error, because it looks exactly like a good
one.

The salt still uses the clock, and that is correct rather than left over: a
salt's job is uniqueness, and the project checks it — eight salts for one
address, all different.

**Registration reveals which addresses have accounts** — 409 rather than the
201 a new address gets. Fixing it means not telling a user why their form
failed. Stated rather than fixed, because that trade is the reader's to make.

## Verified live

```
$ curl -XPOST localhost:8978/register -d 'name=Ada&email=ada@x.org&password=hunter2x'
{"registered": "ada@x.org"}                                      201
$ TOK=$(curl -XPOST localhost:8978/login -d '...' | ...)          64 characters
$ curl localhost:8978/me -H "Authorization: Bearer $TOK"
{"email": "ada@x.org", "name": "Ada"}                            200
$ curl localhost:8978/me                                          401 no bearer token
$ curl -XPOST localhost:8978/login -d '...password=wrongpass'     401
$ curl -XPOST localhost:8978/logout -H "Authorization: Bearer $TOK"  200 {"ended": "1"}
$ curl localhost:8978/me -H "Authorization: Bearer $TOK"          401 not a valid session
```

And what is actually on disk — a salt and a verifier, never the password:

```
ada@x.org	Ada	217aa270e8e9991367dfd3616fc95420	f4f15b09e0d2...cc9f18a9
```

## The harness and the secret

The comparison across three engines reports a token's **length** and not its
value, because a token differs on every run.

That line used to be a warning: the tokens differed because they were derived
from the clock, and the same fact that made them awkward to test made them
guessable. It is now the opposite — they differ because they are 256 bits of
kernel entropy, and a test that could predict one would mean the entropy was
broken.

The test did not change. What it was evidence *of* did.
