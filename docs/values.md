# Value structs in compiled code

Why, measured first. The `tree` benchmark builds and walks trees of
`type Node = { value: i64, kids: [Node] }`. On 2026-10-01, CPU time:

| program | time |
|---|---|
| Rust, idiomatic (`Vec<Node>`, nodes stored inline) | 9.0 ms |
| Grasm, compiled | 20.5 ms |
| Rust written the way Grasm represents it (`Rc` per node and value, `Vec<Rc<Node>>`) | 50.1 ms |

Grasm's runtime is 2.4x faster than Rust *on the same representation*, so the
gap to idiomatic Rust is the representation and nothing else. Five cheaper
fixes were each measured at about zero before this was written (regions, a
bump arena, removing per-element calls, inline list reads, a unique-owner
release fast path): the cost is a box per node and pointer-chasing between
them, not any particular operation on the boxes.

The fix is the one Rust gets for free: a struct that is a *value* is stored
where it is used -- in a register or a stack slot, or contiguously inside the
list that holds it -- rather than in a box of its own.

## Which structs

Not every struct, at first. A struct type is **value-eligible** when every use
of every value of that type, anywhere in the program, is one of:

- built by a literal, or returned by a function;
- a field read (`n.value`, `n.kids`);
- passed as a parameter, or returned;
- stored into a field of another value-eligible struct;
- pushed onto, indexed in, measured (`len`), or iterated over (`for`) in a list
  of that type -- and the list itself used only in those ways.

Anything else -- printing it, `==`, a map key or value, capture by a closure,
an effect operation, a generic function, any other list method (`map`,
`sort_by`, `contains`, ...) -- and the type stays boxed, exactly as today. The
analysis is whole-program and conservative: an eligible type is one about which
nothing generic ever has to be told. That is the set the runtime never sees
except to free, which is what keeps this change out of the hundreds of runtime
functions that take a boxed value.

`tree`'s `Node` is eligible. So are the record types of most numeric code.

## Representation

- **A value in a function** is a stack slot holding its field words -- a
  scalar's bits or a boxed field's pointer, in declaration order, exactly the
  words a struct box holds today. The backend handles it as one SSA value, the
  slot's address, so the rest of codegen keeps its one-value-per-expression
  shape.
- **Calls** pass the address of the caller's slot; the callee reads through it
  (parameters are immutable unless `inout`, which writes back as it does now).
  A function returning one takes a hidden pointer to the caller's slot and
  writes the words there.
- **A list of them** stores the words contiguously: `nfields` words per
  element. `Node` is two words, sixteen bytes. The list is a new payload,
  `ListVals { ty, words }`, whose `ty` carries the field kinds.
- **Ownership** is per boxed field. Copying a value (assignment, storing into
  a list or a field) increments its boxed fields; a value going out of scope
  decrements them. Freeing a `ListVals` walks its words with the type's kinds,
  so `dec_walk` frees a whole tree without a box per node.

## Stages, each measured and gated

1. Eligibility analysis, behind `GRASM_VALUES=1` at compile time, reporting
   which types it would treat as values. No code change yet.
2. Runtime: `ListVals` -- new, push words, length, element address, the release
   walk, and the leak and poison checks knowing about it.
3. Codegen for values in functions: literals, field reads, parameters, returns,
   copies and drops.
4. Codegen for lists of values: `[]`, `push`, `len`, index, `for`.
5. *Done, 2026-10-01.* On by default: difftest, leakcheck and usecheck agree
   with it on the whole corpus (507 programs each). `GRASM_VALUES=0` keeps
   every struct boxed, and the gates run that build too.

Then widen eligibility one generic operation at a time, each by boxing a value
at the boundary where it meets the runtime.

## Next: list fields stored inline (planned 2026-10-02)

A value struct's list field (`kids: [Node]`) is one word today: a pointer to a
list box whose payload holds the `Vec`. The box is what `tree` still pays for
per internal node -- the first push copies the shared empty list into a new
box, and the release walk takes one more step per list -- and, with the URL's
box, what `dirtyheap`'s BFS misses on.

**Ceiling, estimated from the profile before building:** in `tree`, the
first-push slow path (`grasm_vals_push`, `grasm_vals_new`, `alloc_in`,
`desc_ty`) and the release walk's extra step per list are about a quarter of
the samples; the buffer's `malloc`/`free` stays either way. So roughly
16.6 -> 12.5 ms against Rust's 9.0: a gain, not parity. `dirtyheap` needs the
URL inline as well -- Rust measured with only one of the two inline gained
nothing, because the two misses overlap -- so list fields alone do not move it.
Stage 1 measures this ceiling for real before the rest is built.

**Measured, 2026-10-02, before building:** a Rust model of `tree` written both
ways with everything else equal -- kids behind a pooled, refcounted box with a
shared immortal empty list (Grasm's shape), and kids as an inline `Vec` with
the same capacity policy -- ran 14.5 ms and 13.7 ms: the box is worth about
6%, not the quarter the profile suggested. That is under the 15% bar stage 1
sets, so stage 1 stops here for `tree`. For `dirtyheap`, Rust with both the
URL and the edges boxed ran 1.44x its arena form, so inline lists *and* strings
could buy at most about that; the decision to build it is open.

**Representation.** An inline list field takes three words, the `Vec`'s
address, length and capacity (in `vec_layout()` order), so a field is no longer
one word: each field gets a *width*, and the offset of field `i` is the sum of
the widths before it. `StructTy.kinds` gains a kind, `FK_LIST_VALS` (and later
one per scalar list kind), whose width is 3; boxed structs never use it, so the
table they share is unchanged for them.

**Where it is allowed.** As with value structs, by whole-program analysis: a
list field is inline only if every use of it is a `for` over it, `len`, an
index read, a push through the struct's own variable, or a move into it in a
struct literal from a local list that is used only the same ways. Anything
else -- passing `n.kids` to a function, a list method, printing -- keeps the
field a box, exactly as today.

**Stages, each measured and gated:**

1. Prototype on `tree` alone, behind `GRASM_INLINE_LISTS=1`: inline `kids`,
   local `kids` unboxed until the literal moves it in, `for` and drop over the
   inline words. Measure. Stop here if the gain is under 15%.
2. Field widths throughout codegen (`val_fields`, slots, offsets, copies,
   drops, pushes, versioned loops) and the runtime (`for_each_box`,
   `dec_walk`, `vals_push`/`unshare`/`reserve`, display); layout check for the
   new kind.
3. The eligibility analysis, reporting before it acts; then on by default with
   difftest, leakcheck and usecheck agreeing across the corpus, and
   `GRASM_INLINE_LISTS=0` gated too.
4. Strings: the same for `str` fields, which is what `dirtyheap` needs.

**Decided 2026-10-02: build it, lists and strings, for `dirtyheap`.** Scope of
the first version, chosen so that what is allowed is a property of the
program and not of one benchmark:

- Kinds: an inline `[i64]`/`[f64]` field, an inline list of value structs,
  and an inline `str` field -- three words each (address, length, capacity).
- A field is inline when every use of it in the program is a `for` over it,
  `len`/`byte_len`, an index read, interpolation into a string, `==` with a
  string, or the struct literal that fills it. The literal *absorbs* any list
  or string: it takes the buffer when it holds the only reference and copies
  otherwise. Every other use keeps the field a box, as today.
- Copying a value with an inline field copies the buffer (value semantics);
  dropping it frees the buffer and whatever its elements own.
- The interpreter is unchanged: it is the reference these are diffed against.

### Measured, and what shipped (2026-10-02)

Built behind a switch and measured before the analysis was written:

| program | boxed fields | inline fields |
|---|---|---|
| `dirtyheap`, 5,000,000 nodes | 3067 ms, 1128 MB | **1788 ms, 753 MB** |
| `dirtyheap`, 2,000,000 nodes, interleaved | 1137-1230 ms | **713-716 ms** |
| `tree` (`kids: [Node]` inline) | 20.6 ms | 27.4 ms |

So a string and a list of scalars pay for themselves, and a list of *values*
does not: a node's release becomes a runtime call where it was one inlined
decrement, which costs more than the list box it removes. The analysis
therefore offers a field inline when it is a `str` or a list of scalars and
every use of it is a read -- `len`, `byte_len`, `is_empty`, a `for`, an index,
interpolation, a comparison -- with the struct literal the only thing that
fills it. `GRASM_NO_INLINE=1` keeps every field a pointer; `GRASM_INLINE_REPORT=1`
prints what was chosen.

`dirtyheap` now uses **less memory than Rust's arena** (753 MB against 789)
and is 1.67x its time, from 2.6x. The remaining gap is the build: the URL is
made as a string value and then copied into the field, where Rust formats it
straight into place.

### After it landed (2026-10-02)

Two more measurements on `dirtyheap`'s build, which is what is left of its
gap (2.1x Rust's, where its BFS and drop are now at or ahead of parity):

- Building an interpolated string **into** the field, so no string value is
  made, copied out of and freed, is worth about 4% of the whole program.
- The `%` in its LCG costs nothing worth chasing: a variant using
  `bit_and(2147483647)`, which is the one instruction a compiler that could
  prove the value non-negative would emit, is **no faster**. So the interval
  analysis that would let Grasm emit it on its own was not written; the build
  is bound by allocation and memory, not by that arithmetic.
