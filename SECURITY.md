# Security

## Reporting

Report suspected vulnerabilities privately, through GitHub's "Report a
vulnerability" flow on this repository, rather than in a public issue. Include a
program that reproduces the problem if you can — this project's whole discipline
is that behaviour is demonstrated, not described.

Expect an acknowledgement within a week. Fixes ship in a PATCH release.

## What is in scope

Grasm makes one security claim, and it is the one worth attacking:

**A package cannot exceed its declared capability ceiling.** A package's
manifest declares the effects it may perform. Because effects are *inferred from
source* rather than declared by hand, a dependency that quietly starts reading
the filesystem or opening sockets exceeds its ceiling and fails to build **in
every consumer** — a compile error, not a runtime surprise, and not something a
reviewer has to happen to notice.

The claim is tested adversarially rather than asserted: `crates/grasm_pkg`
hides an `io` effect behind a call chain, inside a closure, behind UFCS, in a
`test` or `bench` body, inside an effect handler, and inside a spawned task,
and requires every one to be caught — alongside a genuinely pure package that
must report nothing, so the suite cannot pass by calling everything `io`.

In scope:

- Any way to perform an effect that the capability check does not see.
- Any way to make the effect inference under-report what code does.
- Crashes, memory unsafety, or wrong results in the native runtime
  (`crates/grasm_rt`), which is the only `unsafe` code in the toolchain.
- A program that reads or writes outside what its effects say it does.

## What is not in scope

- **A program doing what it declared.** A package that declares `io` and then
  reads a file is behaving correctly. Capabilities bound what a dependency *can*
  do, not what a user chooses to run.
- **Denial of service through resource use.** An infinite loop, a very large
  allocation, or an expensive compilation is not a vulnerability.
- **The interpreter's stack depth limit.** Deep recursion produces a diagnostic
  and exit 101 by design.
- **Anything requiring an attacker who already runs code on the machine.**

## Threat model

The threat this design takes seriously is the **supply chain**: a dependency
that is benign when reviewed and hostile three versions later. The capability
ceiling turns that from a review problem into a compile error, and
`examples/workspace/` demonstrates it end to end — a library declared pure is
made to read `/etc/passwd`, and its consumer's build fails.

Two things that model does *not* cover today, stated plainly rather than
implied:

- **Build scripts.** Grasm has none. There is no `build.rs` equivalent, so
  there is nothing to sandbox — but that also means the guarantee rests on
  there continuing to be none.
- **A malicious compiler.** Capability checking happens during compilation. A
  compromised toolchain can lie about anything. Lockfile checksums (SHA-256 over
  each package's sorted sources) detect changed dependencies, not a changed
  compiler.

`grasm deps --locked` fails when anything differs from `grasm.lock` — a changed
checksum, an added or removed package, or a **widened capability ceiling**,
which is reported separately from a content change because it is the
security-relevant one. Use it in CI; without it, `grasm deps` accepts the
changes and rewrites the lockfile.

## Unsafe code

`crates/grasm_rt` is the only crate with `unsafe`, and every function in it is
reachable from generated machine code. Changes there get more scrutiny than
anywhere else in the project, and every one needs a program in the differential
suite that exercises it.
