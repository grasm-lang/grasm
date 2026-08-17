# ProjectGold — phase 1 results

Thirty programs written to a list this project did not write. **26 work and
agree byte for byte across all three engines** — `grasm run`, `grasm build`, and
`grasm build --native`. **4 are blocked**, each with the missing language
feature named and a note on what would unblock it.

Re-run with `./run.sh`.

| # | Project | Status | What it found |
|---|---|---|---|
| 01 | Calculator CLI | ✅ | — |
| 02 | Guess the Number | ✅ | confirmed `[E-ARITH-INT]` wrapping on 3 engines |
| 03 | FizzBuzz Variant | ✅ | — |
| 04 | Palindrome Checker | ✅ | **F2** `str.len()` counts code points; byte length unaskable |
| 05 | Matrix Math | ✅ | — |
| 06 | Contact Book | ✅ | **F1** wrapped conditions need a trailing operator |
| 07 | Unit Converter | ✅ | — |
| 08 | Fibonacci | ✅ | **F4** `_` was a `match` pattern only |
| 09 | Password Generator | ✅ | — |
| 10 | Grep Utility | ✅ | **F5** no multi-line or raw strings |
| 11 | Todo + File Storage | ✅ | **F6** files cannot be deleted; directories cannot be listed |
| 12 | Markdown → HTML | ✅ | — |
| 13 | JSON Parser | ✅ | — |
| 14 | Game of Life | ✅ | — |
| 15 | Sorting Benchmark | ✅ | **F7** no stderr |
| 16 | Key-Value Database | ✅ | — |
| 17 | Simple Shell | ⛔ | **F8** `extern fn` carries only `i64`/`f64` |
| 18 | Image Converter | ⛔ | **F9** no byte type |
| 19 | Event Emitter | ✅ | **F10** a `ref` to a scalar can never be read |
| 20 | Expression REPL | ✅ | — |
| 21 | HTTP Server | ✅ | — (verified live with `curl`) |
| 22 | Multithreaded Scraper | ✅ | consumed **F11**; found **F14** no concurrent I/O |
| 23 | Turing Machine | ✅ | — |
| 24 | Micro VM | ✅ | — |
| 25 | DNS Resolver | ⛔ | **F9**, **F12** no UDP |
| 26 | Garbage Collector | ✅ | — |
| 27 | SHA-256 | ✅ | consumed **F3**; 12/12 vectors |
| 28 | SQL Engine | ✅ | — |
| 29 | Self-Hosting Lexer | ✅ | **25/25 files match the reference lexer** |
| 30 | Async Chat | ⛔ | **F13** every socket operation blocks |

## Fixed while writing these

Three findings were closed rather than recorded, because the programs kept
hitting them:

* **F3 → [L-LIB-BITS]** — seven bit-manipulation methods on `i64`. Unblocked
  project 27.
* **F4 → [G-WILDCARD-BINDING]** — `_` accepted wherever a name is bound.
* **F1** — the diagnostic now names the operator that begins the next line and
  says to move it up. (Hit four times while writing these; the fix is a
  diagnostic, and the fact that it kept happening is itself a data point.)
* **F11 → `tcp_connect`** — a Grasm program can now be a client. Verified end to
  end: project 22 fetches from project 21, both Grasm.

## Still open, in the order they block the most

1. **F14 — no concurrent I/O.** `par_map` is the only multicore path and refuses
   every effect; `spawn` is deliberately single-core. A program can compute in
   parallel, and can perform I/O, and cannot do both. This is the largest gap
   found, it sits under the north-star goal that claims the most (G6), and it is
   the only one whose fix is interesting rather than mechanical — `async` is
   already in the effect vocabulary and nothing performs it.
2. **F9 — no byte type.** Blocks 18 and 25, is the root cause of F8, and is why
   `server.gr` cannot serve `grasm.wasm`. One type, and it fixes the FFI too.
3. **F13 — blocking sockets.** The other half of F14: even single-threaded
   multiplexing is impossible, because nothing can ask whether a handle is
   ready.
4. **F7 — no stderr.** A tool cannot separate diagnostics from output.
5. **F5 — no multi-line strings.** Every embedded fixture is one long line.
6. **F2, F6, F10, F12** — smaller.

## What did not go wrong

No wrong answers. Across 26 programs and three engines there was **not one case**
of an engine disagreement, a miscompile, a crash on valid input, or a wrong
result — the failures were all "this cannot be written", never "this was written
and lied".

Every blocked project is blocked on a *missing feature*, and every finding is a
gap rather than a defect. That is a different and much better failure mode than
the alternative, and it is the strongest single statement this directory makes.
