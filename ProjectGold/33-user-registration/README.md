# 33 — User Registration Form  *(phase 2, project 2)*

**Tests:** form decoding, field-level validation, a store that rejects
duplicates, and **one copy of the rules across both sides**.
**Result:** works.

## The point of this one

A form is normally validated twice — once in JavaScript for the message, once
on the server for the truth — and the two copies drift. Here the browser fetches
**`main.gr` itself** and runs it through the wasm build of the reference
interpreter, with the form body on stdin. The verdict the page shows and the
verdict the server enforces come from the same function in the same file.

```
grasm run main.gr < /dev/null                 # the comparison the harness makes
echo 'name=Ada&email=ada%40x.org&password=hunter2x' | grasm run main.gr
grasm run main.gr -- serve                    # then serve index.html over http
```

A line on stdin is a form body; no stdin runs the demonstration. That is what
lets one file be the server, the checker and the page's validator without a flag
the browser would have to know about — and it is why the demo needs
`< /dev/null` when you run it from a terminal, because otherwise `input()` waits
for you.

Verified: the same body through the CLI and through wasm gives the same JSON,
byte for byte.

```
$ echo 'name=&email=nope&password=x' | grasm run main.gr
{"ok": false, "errors": ["name: must not be empty", "email: needs exactly one @, got 0", ...]}
# and the identical string out of grasm_run_with_stdin in the browser
```

## What it validates

Every reason at once, never the first — a form that reveals one problem per
submission makes the user submit four times to learn four things.

* **name** — not empty, at most 64 characters
* **email** — exactly one `@`, something before it, a domain with a dot that is
  not at either end, no spaces
* **password** — at least 8 characters, with a letter and a digit

Two addresses differing only in case are one account. The local part is
technically case-sensitive and no mail provider treats it that way, and a
registration form that lets `Ada@x.org` and `ada@x.org` both exist has created a
support ticket rather than a feature.

## What it deliberately does not do

**It does not store passwords.** It validates one and throws it away. Storing it
would mean either plaintext — a bad example somebody copies — or a hash, and
hashing is project 7's subject, where the from-scratch SHA-256 of project 27
stops being an exercise. The store holds a name and an email.

**The exchange is sequential, not concurrent.** Order is the point here: the
second attempt at `ada@example.org` is refused *because* the first succeeded.
Project 32 is the one that demonstrates concurrency; this one would have had to
suppress its own result to do the same.

**An address already taken is a refusal, not a validation error** — 422 with a
different message, because the form was filled in correctly and the answer is
still no.

## Found while writing this

The committed `grasm.wasm` was four months behind the language. The browser half
refused `eprint` and `byte_len` — builtins every command-line engine had
accepted for weeks — because nothing compared the checked-in module against a
fresh build. `scripts/wasmtest.sh` now does, and its list of what the target
cannot do has been brought up to date too: `write_bytes` and `proc_pipeline`
arrived after it was written, so two conformance programs were being reported as
mismatches rather than skips.
