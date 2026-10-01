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
5. On by default once difftest, leakcheck and usecheck agree with it on the
   whole corpus; the switch kept to build either way.

Then widen eligibility one generic operation at a time, each by boxing a value
at the boundary where it meets the runtime.
