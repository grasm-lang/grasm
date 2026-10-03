# Mining the corpus for what to optimise

Two scripts, and the rule that makes them worth having: **static frequency is
not cost**. The corpus says which calls the compiler emits most; a profile
says which ones the machine spends time in. They disagree sharply, and acting
on the first without the second wastes days.

- `scripts/mine_calls.sh` builds every corpus program that has a `main`
  (256 of them) with symbols kept, disassembles each, and counts every
  `bl _grasm_*` by callee and by containing function. One pass, binaries
  thrown away as it goes.
- `scripts/mine_profile.sh` samples several real programs — the ETL decoder,
  `dirtyheap`, a long `tree`, a long `wordfreq` — and aggregates the samples
  by symbol.

## What they said, 2026-10-03

Static, 572,894 call sites across 256 programs:

| sites | callee |
|---|---|
| 210,661 | `grasm_val_dec` |
| 63,695 | `grasm_str_lit_immortal` |
| 35,989 | `grasm_val_inc` |
| 19,081 | `grasm_list_push` |
| 14,907 | `grasm_list_len` |

Reference counting is **43%** of every runtime call the compiler emits. The
obvious conclusion — inline the release fast path — is wrong.

Dynamic, the same runtime over four real programs:

| samples | symbol |
|---|---|
| 190 | generated code (`dirtyheap`'s BFS) |
| 60 | `StructTy::release_words` |
| 58 | `free` |
| 48 | `_tlv_get_addr` |
| 45 | `grasm_map_add_i64_bytes` |
| 11 | `desc_ty` |

`grasm_val_dec` does not appear at all. Those 210,661 sites are overwhelmingly
cold: a release that frees nothing is a load, a compare and a return, and the
corpus is mostly code that runs once. **Allocation and thread-local lookups
are the runtime's real cost**, which is the same answer every measurement this
week has given — `dirtyheap` was won by removing allocations, not by making
any operation cheaper.

## What it found that was actionable

`release_words` at number two is the inline-field release added the day
before. For a type whose inline fields are plain buffers the backend already
knows the layout, so the descriptor walk, its `desc_ty` lookup and its *two*
reaches for the thread's state are all avoidable: the frees are emitted
directly. Worth about 5% of `dirtyheap`'s drop and under 1% of its run —
recorded here because "the second-hottest symbol was worth under one percent"
is itself the finding.

A second pattern the static pass exposed and nothing has acted on: a string
literal is immortal, so `grasm_val_dec` on one can never do anything, and
`len()` on one is a compile-time constant. A loop over a literal emits both
calls per iteration. Whether that is worth fixing is unmeasured — the dynamic
pass did not rank it, which by the rule above means it should not be built
until a profile puts it somewhere.
