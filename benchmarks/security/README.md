# The supply-chain benchmark

One row no other language in `../` can fill.

```
./scripts/security_bench.sh
```

```
language   toolchain says     why
--------   ----------------   ---
Grasm      COMPILE ERROR      dependency performs [io], permits []
Rust       builds clean       std::fs is ambient; no capability recorded
Go         builds clean       os.ReadFile importable by any package
Python     compiles clean     open() needs no import or declaration
```

## The attack

A consumer depends on `mathlib`, a library it trusts to only do arithmetic.
`mathlib` v1.0.0 is pure. v1.0.1 keeps the **same signature and the same
return value** but now reads `/etc/passwd` and writes it out on the way past:

```
// v1.0.0                          // v1.0.1 — same API, same result
pub fn total(xs: [i64]) -> i64 {   pub fn total(xs: [i64]) -> i64 {
    xs.sum()                           let s = read_file("/etc/passwd") ?? ""
}                                      write_file("/tmp/exfil.txt", s)
                                       xs.sum()
                                   }
```

This is the real supply-chain shape: not malware you'd spot in review, but a
benign dependency that turns hostile a version later, updated automatically,
three levels deep in a graph nobody reads.

## What each toolchain does

The same attack is written in `grasm/`, `rust/`, `go/`, and `python/`. The
benchmark swaps each language's clean library for its tampered twin, builds
the *consumer*, and asks one question: **before the tampered code runs, does
the toolchain reveal that the dependency now touches the filesystem?**

- **Grasm** — `mathlib` declares `capabilities = []`. Effects are inferred
  from source, so its `io` is discovered and checked against that ceiling. The
  consumer's build **fails to compile**. Not a lint, not a warning — an error,
  in every consumer, automatically.
- **Rust** — `std::fs` is reachable from any dependency. Nothing in the type
  system or `Cargo.toml` records that a function reads a file. Builds clean.
- **Go** — `os.ReadFile` is importable by any package. Builds clean.
- **Python** — `open` needs no import at all. Compiles clean.

The others are not *insecure* — they simply put this on a different layer
(runtime sandboxing, review, `cargo-vet`, capabilities in the OS). The point
is narrow and measured: **Grasm's toolchain catches it at compile time, and
theirs do not.** The guarantee is attacked, not asserted — `crates/grasm_pkg`
hides the same `io` behind call chains, closures, UFCS, tests, handlers, and
spawned tasks, and requires every one to be caught.

Nothing here reads a real `/etc/passwd`: the benchmark measures what each
toolchain *reports*, then restores every file it touched.
