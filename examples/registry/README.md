# A registry, and which version you get

Three packages and one application, arranged so that the interesting case is
the default one: two packages want the same dependency at different floors.

```
.grasm/registry/
  textutil/1.0.0/     shout() ends the sentence with "!"
  textutil/1.2.0/     shout() ends it with "!!"
  greeter/1.0.0/      requires textutil ^1.2.0
app/                  requires textutil ^1.0.0 and greeter ^1.0.0
```

Run it:

```
grasm deps examples/registry/app
grasm run  examples/registry/app/src/main.gr
```

It prints `HELLO WORLD!!`, and the second exclamation mark is the whole point:
**textutil 1.2.0 was selected**, even though the application asked only for
`^1.0.0`.

## Why 1.2.0 and not 1.0.0, and why not the newest

Versions are chosen by **minimal version selection**:

1. Every requirement names a lowest acceptable version. Here that is 1.0.0
   from `app` and 1.2.0 from `greeter`.
2. Each package gets the **highest floor anyone stated** — 1.2.0.
3. That resolves to the lowest version the registry actually holds at or above
   the floor, and the choice is then checked against every requirement,
   including the upper bound `^` implies.

So the answer is not "newest wins" and not "first one found wins". Two
properties follow, and they are the reason for choosing selection over search:

- **Order does not matter.** Declaring the two dependencies the other way
  round selects the same versions, so the answer is a function of the
  manifests alone. A lockfile records the result; it is not what holds the
  build still.
- **Adding a dependency cannot silently upgrade one you already had.** Nothing
  moves unless some requirement says it must. Publish textutil 1.9.0 into the
  registry and this application keeps building against 1.2.0 until something
  asks for more.

When requirements genuinely cannot all hold, that is an error listing every
requirer and what each asked for, rather than a version picked on your behalf.

## What the ceiling is for

`capabilities = []` on `textutil` means it performs no effects, and that is
checked against what the code does rather than taken on trust. Try it:

```
# add a read_file(...) call to .grasm/registry/textutil/1.2.0/src/main.gr
grasm deps examples/registry/app
```

The build fails — in the *application*, not in textutil — because the
dependency exceeded its declared ceiling. This is the part that only matters
once packages come from a registry, since a path dependency is code you
already control.

Changing a published version's contents without changing its version number is
caught separately, by the checksum in `grasm.lock`:

```
grasm deps examples/registry/app --locked
```
