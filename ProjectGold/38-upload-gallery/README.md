# 38 — File Upload Gallery  *(phase 2, project 8)*

**Tests:** `multipart/form-data` over a byte socket, binary storage, and serving
files back unchanged.
**Result:** works. **This project was listed as blocked** in the phase 2 plan.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then serve index.html over http
```

## Why it was blocked, and what unblocked it

`multipart/form-data` is binary. A socket carried `str`, and a `str` is text —
so bytes that are not valid UTF-8 were replaced with U+FFFD on the way in. The
upload arrived, its length looked plausible, and the image was broken in a way
nothing downstream could detect.

`tcp_read_bytes` and `tcp_write_bytes` closed it. They needed no new type:
`bytes` already existed and the socket simply had no way to speak it.

The first thing this program prints is the difference, because a fix nobody can
see the need for is a fix nobody keeps:

```
  original:      10 bytes [255, 254, 65, 66, 0, 67, 137, 80, 78, 71]
  through a str: 16 bytes [239, 191, 189, 239, 191, 189, 65, 66, 0, 67, 239, 191, 189, 80, 78, 71]
  survived: false
  through bytes: 10 bytes, survived: true
```

Ten bytes became sixteen. `137` — the first byte of every PNG after the magic
`\x89` — is one of the ones destroyed.

## Only the head is decoded

The head of an HTTP request is text by specification and a body may be anything,
so `parse_req` decodes the head and slices the body. The same rule holds one
level down: a multipart part's *headers* are text and its *payload* is not, so
boundaries are searched for as bytes and payloads are sliced as bytes.

`bytes` has no `index_of` — a byte buffer is not text and the answer would be in
the wrong units — so `find_bytes` is written once and multipart parsing is
almost entirely that operation.

One detail worth its comment: the CRLF before a boundary belongs to the
delimiter, not to the payload. Off by two there and every uploaded file silently
gains two bytes.

## Two things an upload endpoint must get right

**A filename is attacker-controlled.** `../../etc/passwd` is the oldest trick
there is, so only `[A-Za-z0-9._-]` survives and everything else becomes `_`. A
leading dot would hide the file and an empty name is not a name, so both get a
prefix. The harness prints eight attempts and what each becomes.

```
$ curl 'localhost:8979/file/../../etc/passwd'   ->  404
```

**A `Content-Type` is attacker-controlled too.** What the browser is told comes
from the *extension*, not from what the upload claimed, and anything
unrecognised is served as `application/octet-stream`. A file claiming
`text/html` and served as such is a stored cross-site-scripting hole.

## Verified live, with a real PNG

```
$ curl -XPOST localhost:8979/upload -F file=@tiny.png    {"stored": ["tiny.png"]}  201
$ curl localhost:8979/files                              {"files": ["tiny.png"]}
$ curl localhost:8979/file/tiny.png -o back.png          73 bytes
$ cmp tiny.png back.png                                  identical
$ curl -D- localhost:8979/file/tiny.png | grep -i type   Content-Type: image/png
```

The PNG was generated with real IHDR/IDAT/IEND chunks and a valid CRC, so
"identical" means a browser will render it.

## The honest limits

* **One read.** `tcp_read_bytes` performs one read, so an upload larger than
  what arrives in the first packet is truncated. A real server loops until
  `Content-Length` is satisfied; this one is a demonstration and says so.
* **Everything is held in memory** — request, parts, and file. A gallery is a
  fine place to be reminded that a real one streams.
* **One thread**, as in every project of this phase.
