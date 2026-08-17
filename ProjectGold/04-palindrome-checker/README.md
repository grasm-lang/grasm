# 04 — String Reverser & Palindrome Checker

**Tests:** text encoding, `.chars()` versus `.len()`, code points versus bytes.

**Result:** works. All three engines agree.

## What it actually exercises

This is the project on the list most likely to be quietly wrong in a language
that has not thought about text. Reversing by byte is correct for ASCII and
produces mojibake for anything else.

Grasm's `.chars()` yields code points, so `"añb"` reverses to `"bña"` and
`"🙂ab🙂ba🙂"` is correctly identified as a palindrome. A byte-wise
implementation gets both wrong, and the emoji case catastrophically so.

`normalize` keeps everything outside ASCII rather than dropping it, because
dropping it would silently change the answer for exactly the text this program
claims to handle.

## Findings

**F2** — `str.len()` counts code points, not bytes: `"🙂".len()` is 1 and
`"añb".len()` is 3. Consistent, and probably right for a high-level language, but
there is no way to ask how many bytes a string occupies — which matters the
moment one is written to a socket or a file with a length prefix.
