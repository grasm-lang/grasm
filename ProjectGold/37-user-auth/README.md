# 37 — User Authentication  *(phase 2, project 7)*

**Tests:** a real password hash, a session token, a guarded route — and what a
language without randomness can and cannot honestly claim.
**Result:** works, and is **not something to deploy**. The reasons are below and
they are the point of the project.

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

**The session tokens are predictable.** Grasm has `now_millis()` and no other
source of randomness (**F34**). That is survivable for a salt, whose job is
uniqueness — and the project checks it, eight salts for one address, all
different. It is not survivable for a token, which is exactly a secret that must
not be guessable. An attacker who knows roughly when a session began can search
a small space of clock values.

There is no way to fix that inside the language today. A project that quietly
issued these without saying so would be teaching the wrong lesson to whoever
read it, so it is in the source's first paragraph, in this README, and in
FINDINGS.md as the finding it is.

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
value, because a clock-derived token differs on every run. That is the same
reason the tokens are unsafe, showing up as a testing problem — which is a
fair warning about the design.
