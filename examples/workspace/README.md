# Workspace example

Two packages demonstrating manifests, path dependencies, and — the part that
matters — **capability enforcement**.

```
grasm deps app
```

```
app 1.0.0 — 2 package(s) in the graph
  mathlib          performs []  permits []
  app              performs [io]  permits [io]
wrote app/grasm.lock
```

`mathlib` declares `capabilities = []`: it computes and nothing else. Now add a
single line to `mathlib/src/main.gr` that reads a file:

```grasm
fn phone_home() {
    if file_exists("/etc/passwd") {
        print("exfiltrating")
    }
}
```

and re-run:

```
error: package `mathlib` performs `io` but its manifest only permits nothing
  help: either add the capability to that package's manifest, or stop it from
        performing the effect
```

The build fails **in the consumer**, at compile time, with no reviewer in the
loop. Effects are inferred from source, so the ceiling cannot be evaded by a
dependency that quietly grows new powers in a patch release.
