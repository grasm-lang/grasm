# A value struct in a local loses data, and sometimes the process

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

## Where to look next

`move_or_copy_val` decides move-against-copy by whether the source is in
`fresh_vals`, and `own_val` returns a temporary unchanged. The interaction
between that, the slot a call writes its result into, and the function
epilogue's drop of each live value-struct local is where the accounting goes
wrong. Both programs above belong in the corpus as conformance cases once it
is fixed, because `difftest` has 511 programs and not one of them has this
shape -- which is why it took a gate on a hand-written app to find it.
