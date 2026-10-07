# A value struct in a local loses data, and sometimes the process

> **Fixed 2026-10-07.** The cause and the one-line fix are at the end; the
> narrowing is kept because it is how the next one of these gets found.

Found 2026-10-07 by `allgates`' `gold` gate, which failed on
`ProjectGold/30-async-chat`: the interpreter prints 21 lines, the native build
stops after three and exits on a signal that differs between runs -- SIGSEGV,
SIGBUS, SIGTRAP, and once `panic: expected a list`. A signal that changes on
the same binary is memory corruption.

This is a **G4 violation**: safe Grasm, default settings, no `unsafe`
anywhere.

## What it is not

- Not a regression from the compile-speed fix of the same day. The compiler
  built from the commit before it fails identically, three runs of three.
- Not inline fields. `GRASM_INLINE_REPORT=1` shows this program inlines
  nothing, and `GRASM_NO_INLINE=1` still crashes.
- Not the interpreter's problem. **`GRASM_VALUES=0` runs it correctly to
  completion**, which places it squarely in the value-struct path.

## The smallest case, and a quieter one beside it

```grasm
type Room = { conns: [i64] }

fn joined(r: Room, h: i64) -> Room {
    Room{ conns: r.conns + [h] }
}

fn step(r: Room, h: i64) -> Room {
    var room = r
    room = joined(room, h)
    room
}

fn main() / {io} {
    var room = Room{ conns: [] }
    for i in 0..8 {
        room = step(room, i)
    }
    print("holds {room.conns.len()}")
}
```

`grasm run` prints `holds 8`. The native build dies with SIGTRAP and prints
nothing.

Worse, because a crash at least announces itself:

```grasm
fn step(r: Room, h: i64) -> Room {
    let b = joined(r, h)
    b
}
```

`grasm run` prints `holds 8`. The native build prints **`holds 2`** and exits
0. A silent wrong answer in a program with no unsafe construct in it.

## What was narrowed, and what was not

Holding the shape fixed and varying one thing at a time:

| | native |
|---|---|
| `joined(r, h)` returned directly, no local | correct |
| `let a = r`, return `a` | correct |
| `var room = Room{...}` built locally, returned | correct |
| `var tmp = room` from a **local**, reassigned | correct |
| `var room = r` from a **parameter**, reassigned | **crash** |
| `let b = joined(r, h)`, return `b` | **wrong answer** |

So it needs a value struct with a boxed field, a local bound from a call or a
parameter, and a second value of the same type alive in the same function.

One fix was tried and reverted: giving a `let` binding a slot of its own
rather than letting `own_val` hand back the call's result slot. The comment at
the assignment path says a call's result slot belongs to the call site and is
reused, which looked like the same fault one layer down. It is not -- the
change turned the silent wrong answer into a crash, which is a different bug
being uncovered rather than this one being fixed, and it was reverted.

## The instruction sequence, which names the fault

`GRASM_DUMP_IR=1` on `step`, with `v0` the parameter, `v1` the integer and
`v2` the caller's result slot. `fn0`/`fn4` are both `u0:14`, the increment;
`fn2`/`fn3` are both `u0:15`, the decrement:

```
    v3 = stack_addr.i64 ss0      ; room's own slot
    v4 = load.i64 v0
    store v4, v3                 ; room = r, words copied
    v5 = load.i64 v3
    call fn0(v5)                 ; INC  -- the retain for `var room = r`   ok

    v6 = stack_addr.i64 ss1      ; joined()'s result slot
    call fn1(v3, v1, v6)         ; room = joined(room, h)
    v7 = load.i64 v3
    call fn2(v7)                 ; DEC  -- release the old room            ok
    v8 = load.i64 v6
    store v8, v3                 ; the new words move in                   ok

    v9 = load.i64 v3
    call fn3(v9)                 ; DEC  -- releases what was just stored   WRONG

    v10 = load.i64 v3
    store v10, v2                ; copy to the caller's slot
    v11 = load.i64 v2
    call fn4(v11)                ; INC  -- on a list already freed
    return
```

Two increments and **three** decrements across the pair of values, and the
third lands between the store and the return. `emit_val_return` is not the
culprit: it copies to the result slot and only then releases the scope, which
is the right order and is what `v10`-`v11` are.

The reading that fits is that the assignment **moved** the temporary's words
into `room` without marking the temporary moved, so the temporary is released
afterwards -- and because `room` now holds the same pointer, releasing the
temporary releases `room`. `move_or_copy_val` decides move-against-copy by
`fresh_vals`, and the assignment path at the `inc_val_fields` call says counts
are taken on a *copied* value before the old one is released. A moved one
takes no count, so nothing may release it either.

## Where to look next

`move_or_copy_val` decides move-against-copy by whether the source is in
`fresh_vals`, and `own_val` returns a temporary unchanged. The interaction
between that, the slot a call writes its result into, and the function
epilogue's drop of each live value-struct local is where the accounting goes
wrong. Both programs above belong in the corpus as conformance cases once it
is fixed, because `difftest` has 511 programs and not one of them has this
shape -- which is why it took a gate on a hand-written app to find it.


## Fixed

`VType::Val(_)` is not `boxed()` -- a value struct's words live in a slot
rather than behind a pointer -- and `tail_moves` skipped everything that is
not `boxed()`:

```rust
if !vt.boxed() || self.borrowed_vars.contains(&var.as_u32()) {
    continue;
}
```

So a value-struct local never entered the tail-move analysis, never reached
`self.moved`, and `pop_scope_with_decs` released it even when it was the value
being returned. The release landed between the store and the copy into the
caller's slot, which is the `fn3` in the trace above.

A value struct is not boxed, but **its fields are counted**, so it has to take
part in that analysis exactly as a boxed local does:

```rust
let counted = vt.boxed() || matches!(vt, VType::Val(_));
if !counted || self.borrowed_vars.contains(&var.as_u32()) {
    continue;
}
```

`pop_scope_with_decs` already knew what to do with a moved value struct; it
was simply never told one had moved.

After it: `ProjectGold/30-async-chat` is byte-identical across engines, both
reproducers answer correctly, `difftest` is 511/511 and `leakcheck` reports no
leaks -- the risk being that marking more locals moved would leak instead of
double-free, and it does not.
