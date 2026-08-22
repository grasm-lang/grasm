# Standard Library

Every operation the language provides, by receiver type. There are no imports:
these are always available.

Each `L-` clause below has a conformance program under `spec/conformance/`
that exercises what it claims. That is per clause, not per row: a table entry
whose clause is covered elsewhere can still drift, which is how `read_file`
came to be documented as returning `Option[str]` when it is fallible.

## 1. Free functions

**[L-LIB-FREE]** Available without a receiver.

| call | type | notes |
|------|------|-------|
| `print(x)` | `(T)` | writes `x` then a newline; performs `io` |
| `input()` | `() -> str` | one line from stdin, without the newline; `io` |
| `args()` | `() -> [str]` | arguments after `--`; performs `io` |
| `panic(msg)` | `(str)` | prints `panic: msg`, exits 101 |
| `assert(c)` | `(bool)` | panics when `c` is false |
| `assert_eq(a, b)` | `(T, T)` | panics when `a != b` |
| `fail(e)` | `(E)` | raises the error branch of a `T ! E` |
| `map()` | `() -> Map[K, V]` | a new empty map |
| `now_millis()` | `() -> i64` | milliseconds since the epoch; `io` |
| `env(name)` | `(str) -> Option[str]` | environment variable; `io` |
| `exit(code)` | `(i64)` | ends the program with that status — its low eight bits ([L-LIB-EXIT]) |
| `read_file(p)` | `(str) -> str ! IoError` | fallible, not `Option`; `io` ([L-LIB-IO]) |
| `write_file(p, s)` | `(str, str) -> () ! IoError` | fallible; `io` ([L-LIB-IO]) |
| `file_exists(p)` | `(str) -> bool` | `io` |

**[L-LIB-EXIT]** `exit(code)` ends the program with the **low eight bits** of
`code`, which is what an operating system reports and what C's `exit` does with
the same number: `exit(-1)` is 255 and `exit(256)` is 0. Clamping to 0..=255
instead was worse in the direction that matters — it made `exit(-1)`, the most
common spelling of "fail" in a script, report *success*.

`exit` performs no effect, so a dependency declaring `capabilities = []` can
end the host process. That is deliberate and worth stating: `panic` and an
endless loop already do the same, and a ceiling has never claimed to bound how
long a dependency runs or whether it returns — only what it can *reach*.
Nothing observable outside the process changes, so charging `io` for it would
buy no guarantee while making every program that reports a status ask for the
filesystem.

**[L-LIB-IO]** The three ways a library call reports "no": `read_file` is
**fallible** (`str ! IoError`) because a failure has a reason worth reading —
`NotFound(path)`, and so on. `env` returns `Option[str]`, because an unset
variable has no reason to give. `file_exists` returns `bool`, because the
question was already yes-or-no.

## 1a. How values are written

**[L-DISPLAY]** `print(x)` and string interpolation render every type the same
way, and this is normative because a program's output is what its correctness
is judged on.

| type | rendering | example |
|------|-----------|---------|
| `i64` | decimal | `-7` |
| `f64` | the fewest **digits** that round-trip, written out positionally — never exponent notation, and **always with a point** at every magnitude | `1.5`, `1000000.0`, `1000000000000000.0`, `-0.0`; `1e300` is 303 characters |
| `f64`, not finite | `inf`, `-inf`, `NaN` — `NaN` is the one mixed-case rendering here | `inf` |
| `()` | `()` | |
| `bool` | `true` / `false` | |
| `str` | its characters, **unquoted and unescaped** | `b c` |
| `[T]` | elements in order, `, `-separated, in brackets | `[1, 2, 3]`, `[a, b c]` |
| `(A, B)` | components in order, `, `-separated, in parentheses | `(1, one, true)` |
| `Map[K, V]` | `key: value` pairs in sorted key order, in braces | `{k: 1}` |
| struct | `Name{field: value, …}` in declaration order | `P{x: 3, y: hi}` |
| variant with fields | `Name(v, …)` | `B(7)` |
| variant without | `Name` | `A` |
| `Option[T]` | `Some(v)` or `None` | `Some(1)` |
| `ref T` | the slot it names and that slot's generation, never what it points at — the value may already be gone, and reading it to render would be the use-after-free the tier prevents | `ref#0.0` |
| a named function | `<fn module::name>` | `<fn main::dbl>` |
| a closure | `<closure>` — it has no name to show | |
| a handler | a struct with the operation names as fields | `CHandler{now: <closure>}` |
| a range | `start..end`, as written | `1..5` |

Unquoted means a one-element list holding the empty string renders exactly as
an empty list does: `[""]` and `[]` are both `[]`, and `"".split(",")` — one
field, per the table above — looks like no fields at all. That is the price of
a rendering meant to be read rather than parsed, and it is stated because the
alternative is a reader concluding the library is wrong. `len()` tells them
apart.

Strings nested inside a collection are **not** quoted, so `[a, b c]` is a
two-element list whose second element contains a space. Rendering is for people
reading output; it is not a serialisation format and does not round-trip.

## 1b. Calling a free function as a method

**[L-UFCS]** A free function whose first parameter accepts the receiver may be
called in method position: `f(x, y)` and `x.f(y)` are the same call. This
applies to functions a program declares, not only to the library.

```
fn double(n: i64) -> i64 { n * 2 }

fn main() {
    print("{double(5)}")     // 10
    print("{5.double()}")    // 10 — the same call
}
```

Method syntax is resolved against the built-in methods for the receiver's type
first, and a free function is considered only when no built-in matches. So
declaring a function named `len` does not change what `xs.len()` means.

The language's *own* free functions — `print`, `read_file`, `write_file`,
`file_exists`, `input`, `args`, `now_millis`, `env`, `exit`, `panic`, `assert`,
`assert_eq` — are not reached this way. They are not methods of any type, and a
second spelling for each of them buys nothing: `x |> print` already applies a
free function to a value ([G-PIPE]), and it is charged the same row a call is.
`"hi".print()` is refused, and the refusal says so and points at the pipeline —
it used to offer UFCS for `print`, which is the call it had just refused.

## 2. `i64` and `f64`

**[L-LIB-FIXED]** `x.to_fixed(places)` is `f64` as text with exactly that
many digits after the point, 0 to 17; anything else panics. Both engines
format through the same code, so they agree digit for digit — which a
formatter built by hand out of `round` and `to_str` cannot promise, and which
is what every report with an average needs.

A value that is not finite answers with `inf`, `-inf` or `NaN` regardless of
the requested precision, matching [L-DISPLAY].

Ties go **away from zero**, the same rule `round` uses ([L-LIB-ROUND]), so
`(2.5).to_fixed(0)` is `"3"` and `(0.125).to_fixed(2)` is `"0.13"`. A tie
means the *stored* value is exactly halfway, which is rarer than it looks:
`2.675` is really `2.67499…`, so `(2.675).to_fixed(2)` is `"2.67"`. Nothing is
scaled to decide this — scaling `2.675` by 100 lands on `267.5` and would
round it the wrong way.

**[L-LIB-REMOVE]** `xs.remove(i)` takes the element at `i` out of the list
and answers with it, shifting the rest down. Out of range panics, the way
indexing does — `pop` is the one that answers `Option`, because "the list was
empty" is a thing a caller can reasonably not know and "index 9 of 2" is not.

This is what draining a queue from the front *reads* like. Without it a ready
queue cost `ready = ready.slice(1, ready.len())` a step, which reads like the
workaround it was — but the cost is the same: `remove(0)` shifts the rest down,
so both are O(n) per step and a full drain is quadratic either way. Measured at
80 000 elements, 17.2 s for `remove(0)` against 12.8 s for the slice. If a
queue is what you have, reverse it once and `pop` from the end, which is O(1);
this method is for taking an element out of the middle.

**[L-LIB-SUM]** `xs.sum()` of an empty list is the zero of the element type:
`0` for `[i64]`, `0.0` for `[f64]`.

This held only in the compiled backend for a long time. The interpreter decides
int-versus-float from the elements, and an empty list has none, so an empty
float sum was `0` under `grasm run` and `0.0` from a native binary — and a
float method on the result panicked with `no method to_fixed on 0`, in the
engine this specification calls normative, on the empty-average case every
reporting program has. It was recorded here as a known deviation on the
grounds that fixing it needed the interpreter to carry element types. It did
not: the checker knows the sum is an `f64` and can say so, which is the same
answer it now gives for a promoted literal ([T-INT-TO-FLOAT-CARRIED]).

**[L-LIB-CMP]** `a.cmp(b)` answers `-1`, `0` or `1` in exactly the order
`sort` uses — [L-LIB-SORT-NAN]'s total order for floats included. It exists
because that order was specified, used internally, and callable by nothing: a
program needing it reimplemented it from `<` and `>`, got NaN wrong, and
scrambled a column in precisely the way that clause warns about. Naming a
hazard without handing over the fix is how a specification becomes a trap.

It is also what makes a *comparator* sort writable — several keys with
directions decided at run time, which a single `sort_by` key expression cannot
express.

**[L-LIB-SORT-NAN]** Floats sort by a **total** order: `-NaN` before every
number, `NaN` after every number, and `-0.0` before `0.0`. This is not IEEE
comparison, where a NaN is unordered with everything — `<` keeps that. A sort
needs an order, and calling every NaN comparison "equal" is not one: given a
non-order a sort returns a list that is not sorted *at all*, so one `nan` cell
in a file scrambled a whole column rather than misplacing one value.

**[L-LIB-ROUND]** `round` breaks ties **away from zero**: `2.5` and `3.5`
round to `3.0` and `4.0`, `-2.5` to `-3.0`, `-0.5` to `-1.0`. It is not
banker's rounding, and a report's figures depend on which it is.
`(i64_min).abs()` is `i64_min`: there is no positive `i64` that far out, and
the wrap is the same one `/` and `%` take ([E-ARITH-INT]).

**[L-LIB-NUM]**

| method | type | notes |
|--------|------|-------|
| `n.abs()` | `-> i64` / `-> f64` | |
| `n.min(m)`, `n.max(m)` | `(T) -> T` | |
| `x.floor()`, `x.ceil()`, `x.round()` | `f64 -> f64` | |
| `x.sqrt()` | `f64 -> f64` | |
| `n.to_str()` | `-> str` | the same text `print` gives, [L-DISPLAY] included ([L-LIB-TO-STR]) |
| `n.as_f64()` | `i64 -> f64` | widening |
| `x.as_i64()` | `f64 -> i64` | truncates toward zero, saturating ([L-LIB-AS-I64]) |

Conversion is by method, not by a cast or a `int(...)`-style function. A `str`
becomes a number with `parse_int` / `parse_float`, which return `Option`
because the text may not be a number.

**[L-LIB-MATH]** Trigonometry, exponentials and logarithms, on `f64`:

| method | type | notes |
|--------|------|-------|
| `x.sin()`, `x.cos()`, `x.tan()` | `f64 -> f64` | radians |
| `x.asin()`, `x.acos()`, `x.atan()` | `f64 -> f64` | radians |
| `x.atan2(y)` | `(f64) -> f64` | `x.atan2(y)` is `atan(x/y)` with the quadrant of `(y, x)` |
| `x.sinh()`, `x.cosh()`, `x.tanh()` | `f64 -> f64` | |
| `x.exp()`, `x.ln()` | `f64 -> f64` | base *e* |
| `x.log2()`, `x.log10()` | `f64 -> f64` | |
| `x.log(b)` | `(f64) -> f64` | base `b` |
| `x.cbrt()`, `x.hypot(y)` | `f64 -> f64` | |
| `x.to_radians()`, `x.to_degrees()` | `f64 -> f64` | |
| `pi()`, `e()` | `-> f64` | |

Every one is pure: no allocation, no error case, and **no capability**. That
last part is the reason they are here rather than left to `extern fn cos` — a
declared C function performs `io` ([T-EXTERN-VALUE]), so reaching libm for a
cosine put a trigonometry helper over the ceiling of any package declaring
`capabilities = []`. A pure function should not cost a capability.

`pi` and `e` are the only two builtins that yield to a function you declare
yourself: a program with its own `fn e()` keeps it. `e` is a common enough
identifier that taking it outright would break working programs.

**[L-LIB-TO-STR]** `to_str` produces exactly what `print` would. In
particular an `f64` keeps its point at every magnitude: `(1.0).to_str()` is
`"1.0"`. The interpreter used to hand the job to its host language, which drops
the point on a whole value, so it answered `"1"` where the compiled backend
answered `"1.0"` — and the documented idiom `xs.map(|v| v.to_str()).join(", ")`
printed a `[f64]` as a list of integers, which is the one thing [L-DISPLAY]
exists to prevent.

**[L-LIB-AS-I64]** `as_i64` is total: it has an answer for every `f64`, so it
never panics and never returns `Option`. A value too large for `i64` gives
`i64`'s maximum, one too small gives its minimum, and `NaN` gives `0`. That is
saturation, not wrapping — `1.0e30.as_i64()` is `9223372036854775807`, and it
being a *clamp* rather than a truncated bit pattern is the part worth knowing,
because the two disagree and only one of them stays ordered.

## 3. `str`

**[L-LIB-STR]** Lengths and offsets are in **characters, not bytes**
([E-STR-CHARS]). `s.byte_len()` is the one exception and is the only way to ask
the other question: `"🙂".len()` is 1 and `"🙂".byte_len()` is 4. Characters are
the right unit for a high-level language and the wrong one the moment a program
writes a string to a socket or a file and has to say how long it is — a
`Content-Length` counts bytes, and computing it from `len()` is wrong for every
string that is not ASCII.

**[L-LIB-TRIM-SIDES]** `trim_start` and `trim_end` remove whitespace from one
end only; `trim` removes it from both. None of the three takes an argument —
whitespace is what they trim. The one-sided pair exists because a line's
leading indentation usually carries meaning and its trailing newline does not,
and `trim` cannot tell you that.

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
| `s.byte_len()` | `-> i64` | bytes, for a length that goes on the wire ([L-LIB-STR]) |
| `s.is_empty()` | `-> bool` | |
| `s.substr(start, len)` | `(i64, i64) -> str` | **start and length**, not start and end; clamps past the end rather than panicking, unlike indexing |
| `s.chars()` | `-> [str]` | one single-character string per element |
| `s.lines()` | `-> [str]` | splits on newlines; no trailing empty line |
| `s.split(sep)` | `(str) -> [str]` | on an exact separator; adjacent separators give empty elements |
| `s.words()` | `-> [str]` | on runs of whitespace, with no empties ([L-LIB-WORDS]) |
| `s.contains(t)`, `s.starts_with(t)`, `s.ends_with(t)` | `(str) -> bool` | |
| `s.index_of(t)` | `(str) -> Option[i64]` | `None` when absent |
| `s.replace(from, to)` | `(str, str) -> str` | |
| `s.trim()` | `-> str` | |
| `s.trim_start()`, `s.trim_end()` | `-> str` | one side only ([L-LIB-TRIM-SIDES]) |
| `s.to_upper()`, `s.to_lower()` | `-> str` | |
| `s.repeat(n)` | `(i64) -> str` | |
| `s.pad_start(w, fill)`, `s.pad_end(w, fill)` | `(i64, str) -> str` | fill repeats and is cut to width ([E-PAD-CYCLE]) |
| `s.parse_int()` | `-> Option[i64]` | surrounding space and a leading `+` are accepted; `1_000` and `0xFF` are **not** ([L-LIB-PARSE]) |
| `n.to_base(b)` | `(i64) -> str` | digits `0-9a-z`; a base outside `2..36` panics ([L-LIB-BASE]) |
| `s.parse_int_base(b)` | `(i64) -> Option[i64]` | the other direction; no prefix accepted ([L-LIB-BASE]) |
| `s.parse_float()` | `-> Option[f64]` | accepts `inf`, `-inf`, `nan`; overflow gives an infinity, not `None` ([L-LIB-PARSE]) |
| `x.is_nan()`, `x.is_finite()` | `-> bool` | what `parse_float` makes necessary ([L-LIB-PARSE]) |
| `s.code_at(i)` | `(i64) -> i64` | Unicode code point of character `i`; panics out of range ([L-LIB-CODES]) |
| `s.codes()` | `-> [i64]` | every code point, in order |
| `n.to_char()` | `i64 -> str` | a code point back to a one-character string; panics if `n` is not one |

**[L-LIB-CODES]** `s.code_at(i)` and `s.codes()` give Unicode **code points**
(`char` scalar values), counting characters not bytes, and `n.to_char()` turns
one back into text. `code_at` panics out of range, like indexing; `to_char`
panics on a value that is not a code point, surrogates included. Together they
are what lets a program compute *on* text — hash it, encode it, parse a number
by hand — rather than only cut and compare it.

`len`, `code_at` and `substr` all answer in constant time (`substr` in time
proportional to the slice it returns), whatever the text. They
count *characters*, so each used to decode the string from the start:
`while i < s.len()` was quadratic in two ways at once, and 400 000 characters
took 6.7 seconds where the identical loop over a list took 0.21. A string
never changes once built, so its character count is computed once and kept.
When that count equals the byte count the text is ASCII and an index is an
index; when it does not, a character index is built once, the first time one
is asked for by position. Fixing only the ASCII half left the same 6.3 seconds
waiting behind a single two-byte character, and building the index for
`code_at` while leaving `substr` to walk from the start left a `substr(i, 1)`
scan quadratic — 39 seconds over 262 144 characters — next to a `code_at` scan
of the same string costing 0.11.

**[L-LIB-WORDS]** `s.words()` splits on runs of whitespace and yields no
empty elements, which is what reading a line of aligned columns needs.
`s.split(" ")` is the other thing — an exact separator, where two spaces in a
row mean an empty element between them — and both are worth having, because a
CSV wants the second and a log line wants the first.

**[L-LIB-PARSE]** `parse_int` and `parse_float` answer `None` for text that
is not a number, and are worth stating exactly because untrusted input reaches
them:

- `parse_int` accepts surrounding whitespace and a leading `+` or `-`. It does
  **not** accept the `1_000` and `0xFF` forms that [L-INT] allows in *source*:
  a literal in a program and a number in a data file are different things.
- `parse_float` accepts `inf`, `-inf` and `nan`, and a value too large to
  represent gives an infinity rather than `None`. So `"1e400"` parses, and the
  `inf` it produces then flows into arithmetic, `as_i64` ([L-LIB-NUM]) and
  sorting ([L-LIB-SORT-NAN]) — which is how those three came to be specified.

## 4. `[T]`

**[L-LIB-LIST]** Indexing out of bounds panics; `get` returns `Option`
([E-INDEX-BOUNDS]).

| method | type | notes |
|--------|------|-------|
| `for x in xs { … }` | | iterates elements, not indices ([E-LOOP-LIST]) |
| `xs.len()`, `xs.is_empty()` | | |
| `xs[i]` | `-> T` | panics when out of range |
| `xs.get(i)` | `(i64) -> Option[T]` | |
| `xs.first()`, `xs.last()` | `-> Option[T]` | |
| `xs.push(v)` | `(T)` | mutates; receiver must be `var` |
| `xs.pop()` | `-> Option[T]` | mutates |
| `xs.remove(i)` | `(i64) -> T` | mutates; panics out of range, like indexing ([L-LIB-REMOVE]) |
| `a.cmp(b)` | `(T) -> i64` | `-1`, `0` or `1` in the order `sort` uses; `i64`, `f64`, `str` and tuples ([L-LIB-CMP]) |
| `xs.sort()` | | mutates; ascending. Ordered means `i64`, `f64`, `str`, and a tuple of those ([L-LIB-TUPLE]) — sorting anything else is a compile error, the same one `<` gives |
| `xs.rev()` | `-> [T]` | |
| `xs.slice(a, b)` | `(i64, i64) -> [T]` | `a` up to but excluding `b`; clamps past either end rather than panicking, like `substr` |
| `xs.contains(v)` | `(T) -> bool` | |
| `xs.join(sep)` | `(str) -> str` | `[str]` only — map to `str` first |
| `xs.sum()` | `-> T` | numeric elements; `0` of the element type when empty ([L-LIB-SUM]) |
| `xs.min()`, `xs.max()` | `-> Option[T]` | `None` when empty; not available for `bool` |
| `xs.fold(init, f)` | `(B, (B, T) -> B) -> B` | left fold; the accumulator's type comes from `init`, not from `T` |
| `xs.map(f)` | `(T -> U) -> [U]` | |
| `xs.par_map(f)` | `(T -> U) -> [U]` | multi-core; `f` must be effect-free ([T-PAR-MAP-PURE]) |
| `xs.filter(f)` | `(T -> bool) -> [T]` | |
| `xs.par_filter(f)` | `(T -> bool) -> [T]` | multi-core; `f` must be effect-free; input order kept ([E-PAR-FILTER-ORDER]) |
| `xs.any(f)`, `xs.all(f)` | `(T -> bool) -> bool` | short-circuit ([E-SEARCH-SHORTCIRCUIT]) |
| `xs.find(f)` | `(T -> bool) -> Option[T]` | first match |
| `xs + ys` | `([T], [T]) -> [T]` | concatenation, the way `+` joins strings ([E-LIST-CONCAT]) |
| `xs.sort_by(key)` | `(T -> K)` | mutates; sorts by an extracted `i64`/`f64`/`str` key, stably ([L-LIB-SORT-BY]) |
| `xs.enumerate()` | `-> [(i64, T)]` | pair each element with its index |
| `xs.zip(ys)` | `([U]) -> [(T, U)]` | pairs up to the shorter of the two |
| `pair.0`, `pair.1`, … | `-> T` | a tuple's components are named by position ([L-LIB-TUPLE]) |

**[L-LIB-SORT-BY]** `xs.sort_by(key)` sorts by what `key` returns for each
element, which is where a list of structs gets its order from — `sort` needs
the *elements* ordered, and a struct is not. The key is `i64`, `f64`, `str`, or
a **tuple** of those, which is what makes
`xs.sort_by(|p| (p.dept, p.age, p.name))` a multi-key sort rather than an error
([L-LIB-TUPLE]). This clause said "must be `i64`, `f64` or `str`" while tuple
keys worked in all three engines — two clauses of one document disagreeing, and
the one that was wrong was the one a reader would find first. The sort is stable: elements with equal keys stay in the order they
were written. It sorts **ascending**, always; descending is
`xs.sort_by(k)` followed by `xs = xs.rev()`. Negating the key works for a
number and has no counterpart for a `str`, so the reversal is the general
answer and the one to reach for. The assignment is the whole point: `sort_by`
mutates and `rev` answers with a new list, so the clause's earlier wording —
`xs.rev()` on its own line — left the list ascending and said nothing.

**[L-LIB-PAIRS]** `xs.enumerate()` pairs each element with its index and
`xs.zip(ys)` pairs the two lists positionally, stopping at the shorter — there
is no element to invent past the end of one. Both give `[(A, B)]`, which
`for (a, b) in …` binds by name ([G-LET-TUPLE]).

**[L-LIB-TUPLE]** A tuple's components are read by position — `pair.0`,
`pair.1` — and a tuple of ordered components is itself ordered, compared
component by component. That is what `<` on tuples means, what `sort` on a
list of them does, `min`/`max` included, and what makes
`xs.sort_by(|p| (p.dept, p.age, p.name))` a multi-key sort rather than an
error. `min` and `max` used to spell out the types they accepted instead of
asking whether the element was ordered, so `xs.enumerate().min()` was refused
on a list `sort` orders happily.

That example is worth spelling out, because it does *not* mean "index of the
smallest": components compare left to right, so `enumerate().min()` compares
indices first and always answers `(0, xs[0])`. To find the smallest and where
it was, put the value first — `xs.enumerate().map(|p| (p.1, p.0)).min()`.

## 5. `Map[K, V]`

**[L-LIB-MAP]** Keys are `i64`, `str`, or `bool`. Iteration is in sorted key
order ([E-MAP-ORDER]).

| method | type | notes |
|--------|------|-------|
| `m.len()`, `m.is_empty()` | | |
| `m.get(k)` | `(K) -> Option[V]` | |
| `m.set(k, v)` | `(K, V)` | mutates; receiver must be `var` |
| `m.has(k)` | `(K) -> bool` | |
| `m.delete(k)` | `(K) -> bool` | mutates; whether it was present |
| `m.keys()` | `-> [K]` | sorted |
| `m.values()` | `-> [V]` | in key order |
| `m.entries()` | `-> [(K, V)]` | pairs, in key order — the only way to reach both at once |

## 6. `Option[T]`

**[L-LIB-OPTION]**

| form | notes |
|------|-------|
| `Some(v)`, `None` | constructors |
| `o ?? fallback` | the value, or `fallback`; the right side is evaluated only when needed ([E-COALESCE-LAZY]) |
| `o.is_none()`, `o.is_some()` | |
| `o.unwrap_or(d)` | same as `??` |
| `o.expect(msg)` | the payload, or panics with `msg` — for when absence is a bug, not a case |
| `match o { Some(v) -> … None -> … }` | exhaustive |

**[L-LIB-RANGE-OPS]** A range answers `to_list`, `map`, `filter`, `sum` and
`len`, producing a list of the same elements `for i in a..b` walks. There is one
definition of what a range contains and not two: the interpreter turns the range
into that list and reuses the list method, and the compiled backend rewrites the
call into the loop it already compiles, since it has no range *value* (§11).

The clause exists because of what it replaces. Building a list from a count
needed `var xs: [i64] = []`, a `for`, a `push` and two closing braces — four
lines where `(0..n).map(f)` says the same thing, in every program that builds a
list from a count. [G11] budgets Grasm at or below Python's token count and it
measured 1.41×; this is most of the difference, and adding it moved the figure
to 1.22× without adding a syntactic form — a call is a call, so the kernel's
count in [`core.md`](core.md) does not move.

## Network — the `net` capability

**[L-LIB-NET]** Eight builtins reach the network, and each performs `net`:

| | type | |
|---|---|---|
| `tcp_listen(port)` | `(i64) -> i64 ! IoError` | listens on `127.0.0.1:port`; `0` asks the system to choose |
| `tcp_connect(host, port)` | `(str, i64) -> i64 ! IoError` | dials `host:port`; a name is resolved |
| `tcp_accept(h)` | `(i64) -> i64 ! IoError` | waits for a connection, answering with a handle for it |
| `tcp_read(h)` | `(i64) -> str ! IoError` | what has arrived, as text |
| `tcp_read_bytes(h)` | `(i64) -> bytes ! IoError` | the same read, unconverted |
| `tcp_write(h, text)` | `(i64, str) -> () ! IoError` | writes and flushes |
| `tcp_write_bytes(h, b)` | `(i64, bytes) -> () ! IoError` | writes and flushes |
| `net_close(h)` | `(i64) -> ()` | closes a listener or a connection |
| `net_wait(hs, ms)` | `([i64], i64) -> [i64] ! IoError` | the handles that can be read without waiting |
| `net_port(h)` | `(i64) -> i64 ! IoError` | the local port a handle is bound to |

`tcp_connect` is the half that makes a program a *client*. Without it a Grasm
program could listen and accept and never dial — no fetching a URL, no calling
an API, no talking to a database — which is a gap invisible until somebody tries
to write an ordinary program. Resolution failure and connection failure are the
same `IoFailed` to a caller, because both mean "there is nothing to talk to",
and the message carries the difference.

A handle is an integer and a payload is text, so reaching the network needed no
pointer type and no byte buffer. That is the reason the surface looks like this:
adding either to the language would have been a far larger change than adding
the network, and `extern fn` could already call `socket()` and `listen()` —
what it could not do was pass a `sockaddr` or a buffer.

`tcp_read` performs **one** read rather than looping to end-of-stream, because
an HTTP client holds the connection open after its request and reading to the
end would wait forever. A caller that needs more reads again. Bytes that are not
valid UTF-8 are replaced rather than refused: a socket carries bytes, a `str`
carries text, and a client's mistake should not end a server.

That replacement is right for a protocol made of text and wrong for one that is
not. `multipart/form-data` carries a JPEG, and turning every invalid sequence
into U+FFFD corrupts the file in a way nothing downstream can detect — the
upload arrives, the length looks plausible, and the image is broken.
`tcp_read_bytes` and `tcp_write_bytes` are the same two calls without the
conversion. They needed no new type: `bytes` already existed ([L-LIB-BYTES]),
and the socket simply had no way to speak it.

Both pairs stay, rather than one replacing the other, because an HTTP header is
text and a body may not be: a server reads the request as bytes and decodes the
part it knows is text, and a program serving only text should not have to.

`net_port` is what makes `tcp_listen(0)` usable. Asking the system to choose a
free port is the only way to write a program that runs on a machine where
something else already holds the port it wanted — and with no way to read the
choice back, a program could bind a port it was then unable to name, dial, or
print.

`net_wait` is what lets one thread hold many conversations. Every other call
here blocks: without it a server serves one client at a time, and serving two
would need a thread per connection — which a Grasm program cannot spawn, because
`spawn` does not carry `net`. So the language had a network a program could not
usefully *serve* on, and the missing piece was readiness, not threads.

It answers with the subset of `hs` that will not block, **in the order they were
asked in** rather than whichever order the kernel reports, so a program that
walks the result serves its connections in a fixed order ([L-DETERMINISM]). A
timeout of `0` asks without waiting, a negative timeout waits indefinitely, and
an empty wait set is answered immediately with an empty list rather than an
error — a server that has just closed its last connection asks exactly that on
its next turn. A handle that names nothing is an `IoFailed`, the same as using a
connection after closing it.

A hung-up peer is reported as **ready**. The read that follows returns nothing,
which is how a caller learns the conversation ended; reporting it as not-ready
would leave a dead connection in the wait set forever, and the loop would never
notice it should be dropped.

Readiness needed no new type: the wait set is an ordinary `[i64]` and the
timeout an ordinary `i64`, which is the same reason the rest of this surface
looks the way it does.

**[L-LIB-UDP]** Three more reach the network as **datagrams**, and each
performs `net`:

| | type | |
|---|---|---|
| `udp_bind(port)` | `(i64) -> i64 ! IoError` | binds a datagram socket; `0` asks the system to choose |
| `udp_send(h, host, port, b)` | `(i64, str, i64, bytes) -> i64 ! IoError` | sends one datagram, answering with the bytes sent |
| `udp_recv(h)` | `(i64) -> bytes ! IoError` | receives **one** datagram, whole |

UDP is not TCP with the reliability removed; it is a different shape, and the
signatures have to say so. A stream has no message boundaries and both ends are
fixed at connect time, so `tcp_write(h, text)` is complete. A datagram is a
*whole message* sent to an address named at send time, so `udp_send` takes the
destination on every call. Giving the two the same shape would have lost exactly
the information that makes UDP worth having.

Payloads are `bytes`, not `str`. The protocols that use UDP are binary — a DNS
query is a packed structure with bit-level flags — and routing one through the
lossy UTF-8 conversion the stream operations use, correctly, for text would
corrupt it ([L-LIB-BYTES]).

`udp_recv` receives one datagram whole or fails. A read that returned half a
message, or two joined together, would have destroyed the only thing UDP
guarantees. Nothing past 65535 bytes can arrive over IPv4 in one piece, so the
buffer cannot truncate a datagram that was deliverable.

Datagram handles share the table with stream handles, so `net_wait`, `net_port`
and `net_close` work on all three kinds unchanged. A program that speaks two
protocols multiplexes them in one loop, which is the ordinary case rather than
an advanced one. Using a stream operation on a datagram handle is an `IoFailed`
naming the operation to use instead.

Closing a handle twice is not an error, for the same reason releasing a `ref`
twice is not ([T-REF-GENERATIONAL]) — cleanup order is not something a caller
should have to track. Using a closed handle *is* an error, and an ordinary one:
`tcp_read` on it answers `Err`, never a panic and never a wrong answer.

**`net` is a capability of its own, not part of `io`.** That distinction is the
whole point. [T-CAPABILITY]'s headline example is a package permitted to read a
file and forbidden to open a socket, and until there was a network to gate, that
example could not be shown to anybody:

```
error: package `lib` performs `net` but its manifest only permits `io`
  help: either add the capability to that package's manifest, or stop it from
        performing the effect
```

**[L-LIB-BASE]** `n.to_base(b)` writes an integer in base `b` and
`s.parse_int_base(b)` reads one back. Digits are `0-9a-z`, lower case, and a
negative number gets a leading `-`.

Base ten is not the only base a systems program needs. Every protocol, hash,
colour and permission mask is read and written in hex or binary — the same
programs that needed [L-LIB-BITS] — and without these, printing a packet ID the
way every other tool prints it means hand-writing a digit table.

A base outside `2..36` **panics** rather than answering. There is no right
answer to return and a wrong one would be silently wrong, which is the same
reasoning as `code_at` panicking out of range.

Neither side writes or accepts a prefix. The base is the argument, not something
to be guessed from the text: `"ff".parse_int_base(16)` is `Some(255)` and
`"0xff".parse_int_base(16)` is `None`. That keeps it consistent with `parse_int`,
which refuses `0xFF` ([L-LIB-PARSE]) — the difference is now that a program can
say which base it means instead of being stuck with one.

`to_base(10)` and `to_str` agree, including at `i64::MIN`, which is converted
through its unsigned magnitude because negating it overflows.

## Processes — [L-LIB-PROC]

**[L-LIB-PROC]** `proc_run(program, args)` runs another program to completion and
answers with `(exit code, stdout, stderr)`:

| | type | |
|---|---|---|
| `proc_run(prog, args)` | `(str, [str]) -> (i64, str, str) ! IoError` | performs `proc` |
| `proc_pipeline(stages, input)` | `([(str, [str])], str) -> (i64, str, str) ! IoError` | each stage's output feeds the next |

**`proc` is a capability of its own, and it is strictly larger than `io` or
`net`.** A program that can run `curl` has the network whether or not it
declared `net`; one that can run `rm` has the filesystem. Folding process
execution into `io` would make every capability claim in the language weaker
than it reads — a package's manifest could say `io` and mean everything.

The result is a **tuple, not a new type**. Three values with obvious positions
did not need a name, and inventing one would have put a type in the prelude that
nothing else uses.

**Output is captured, not inherited.** A builtin that wrote to the parent's
stdout would make the three engines disagree about ordering, because the child's
writes and the parent's go through different buffers. A caller that wants the
output shown prints it, which also means it can be examined first.

`args` are passed as a list and never through a shell, so a filename containing
a space or a `;` is one argument and not an injection. A program that genuinely
wants shell semantics asks for them: `proc_run("sh", ["-c", line])`.

`proc_pipeline` exists because a pipeline **cannot be built out of
`proc_run`**. That waits for a program to finish and hands back its output,
and a pipeline needs one child's stdout to *be* another's stdin. Copying the
bytes through Grasm instead would need concurrent reads and writes on pipe
handles — a much larger surface for a smaller result, and the kernel already
does this.

`input` feeds the first stage's stdin, which is the other thing `proc_run`
could not do: a filter with nothing to filter is not a filter. It is written on
a thread, because a large input and a chatty child deadlock if the parent writes
everything before it reads anything. An empty input closes stdin rather than
leaving it open, so a stage reading to end-of-input does not wait for input that
is never coming.

The result is the **last stage's** exit code, stdout and stderr, which is what
every shell means by a pipeline's status. An earlier stage's stderr goes to the
program's own stderr rather than being discarded: a diagnostic nobody sees is
worse than one in the wrong place. If any stage cannot be started, everything
already running is killed and the whole call is an `IoFailed` — a pipeline that
failed halfway leaves nothing behind. An empty pipeline is an error, not a
silent success.

An exit code of **`-1`** means a signal killed the process. No process can exit
with `-1`, so it cannot be confused for a status the program chose. A program
that could not be started at all is an `IoFailed` naming it, which is a
different thing from a program that ran and failed.

## Bit manipulation — [L-LIB-BITS]

**[L-LIB-BITS]** Seven methods on `i64`:

| | | |
|---|---|---|
| `a.bit_and(b)` | `(i64, i64) -> i64` | bitwise and |
| `a.bit_or(b)` | `(i64, i64) -> i64` | bitwise or |
| `a.bit_xor(b)` | `(i64, i64) -> i64` | bitwise exclusive or |
| `a.bit_not()` | `(i64) -> i64` | ones' complement |
| `a.shl(n)` | `(i64, i64) -> i64` | shift left |
| `a.shr(n)` | `(i64, i64) -> i64` | shift right, **sign-preserving** |
| `a.ushr(n)` | `(i64, i64) -> i64` | shift right, **zero-filling** |

**Methods rather than operators**, and the reason is that every symbol is
already taken or ambiguous. `^` is exponentiation. `|` begins a closure and
separates sum-type variants, so `a | b` cannot be told from the start of one.
`&&` is already reported as "written `and`". And words would collide with the
logical operators, which in this language are `and`, `or` and `not` — so
`a and b` would be boolean and `a band b` bitwise, one letter apart. Methods are
what `.abs()` and `.cmp()` already are, and they carry their own precedence.

**The shift count is masked to `0..63`.** `1.shl(64)` is `1`, not `0` and not a
trap, and `256.shr(-1)` is `256.shr(63)`, which is `0`. This is what the
hardware does, and it is stated because the alternatives are worse: Rust's `<<`
panics in a debug build and wraps in a release one, which is the class of
difference-between-builds this project refuses to ship, and Cranelift leaves an
out-of-range count undefined, which would make the two engines disagree.

`shr` preserves the sign and `ushr` does not, which is the only reason both
exist: `(-16).shr(2)` is `-4` and `(-16).ushr(60)` is `15`.

There is no unsigned integer type, so a program working in 32 bits masks with
`0xFFFFFFFF` itself. `spec/conformance/L-LIB-BITS.gr` shows the rotation that
SHA-256 needs, written that way.

## The other stream — [L-LIB-EPRINT]

**[L-LIB-EPRINT]** `eprint(v)` writes to standard error. It is `print` in every
other respect: same single argument, same interpolation, same refusal to take a
fallible value, same `io` effect.

It exists because a program that cannot separate its diagnostics from its output
cannot be used in a pipeline, and being usable in a pipeline is most of what a
command-line tool is for. A benchmark's timings, a compiler's warnings and a
tool's progress are all diagnostics; sending them to stdout puts them in the
middle of the data, which is the one place they cannot be noticed and the one
place they do damage. It is the same reason a panic does not go there
([E-PANIC-EXIT]).

Both engines write the two streams independently and identically. The compiled
build brackets the write with a flag the runtime reads, rather than carrying a
second copy of the formatting code — one code path, so the two streams cannot
format differently.

## Files and directories — [L-LIB-FS]

**[L-LIB-FS]** Two additions to the file surface:

| | type | |
|---|---|---|
| `make_dir(path)` | `(str) -> () ! IoError` | creates the whole path; already existing is success |
| `remove_file(path)` | `(str) -> () ! IoError` | deletes; a missing path is `NotFound` |
| `list_dir(path)` | `(str) -> [str] ! IoError` | entry names, **sorted** |

`make_dir` creates every missing component and treats an existing directory as
success, so a program can prepare its own working directory without first asking
whether one is there.

Without these a file could be created and truncated but never removed, a
directory could not be read at all, and a program could not make one — so a program that made a temporary file
could only empty it, and the next run saw a leftover.

**`list_dir` sorts.** A filesystem's own order differs between machines and
between runs, and principle 6 makes determinism a feature: an unsorted listing
would make any program that printed one untestable, and would break three-engine
agreement on the first run. Both engines sort, and identically.

Names are returned, not paths — the caller already knows the directory it asked
about, and joining is one concatenation.

## Bytes — [L-LIB-BYTES]

**[L-LIB-BYTES]** `bytes` is a contiguous run of bytes, and it is the type `str`
deliberately is not.

A `str` is *text*: it holds UTF-8, and anything above 127 is re-encoded on the
way out. That is correct for text and fatal for anything else. Asked to write the
seven bytes of `mov rax, 1` — `48 c7 c0 01 00 00 00` — a `str` put **nine** bytes
on disk, because `0xC7` became `c3 87`. A language whose only sequence type does
that cannot read an image, build a network packet, or emit an object file.

| | type | |
|---|---|---|
| `bytes_of(list)` | `([i64]) -> bytes` | every element must be in `0..255` |
| `s.to_bytes()` | `(str) -> bytes` | the UTF-8 encoding of the text |
| `b.to_str()` | `(bytes) -> str` | lossy, as `tcp_read` is |
| `b.len()` | `(bytes) -> i64` | count of bytes |
| `b.is_empty()` | `(bytes) -> bool` | |
| `b.at(i)` | `(bytes, i64) -> i64` | one byte, `0..255`; out of range panics |
| `b.slice(from, count)` | `(bytes, i64, i64) -> bytes` | out of range panics |
| `b.concat(other)` | `(bytes, bytes) -> bytes` | |
| `b.to_list()` | `(bytes) -> [i64]` | |
| `read_bytes(path)` | `(str) -> bytes ! IoError` | |
| `write_bytes(path, b)` | `(str, bytes) -> () ! IoError` | |

**`bytes` is distinct from `[i64]` on purpose.** A list of integers cannot
promise its elements are bytes, stores eight bytes per element, and cannot be
handed to anything that wants a buffer. The promise is the type.

**A byte is an `i64`.** `at` gives a number in `0..255` rather than a one-byte
value type, because the language has one integer type and adding a second to
describe an element would be a worse trade than the range check.

**A `bytes` prints as hex**, `0x` then two digits per byte, and not as the text
it might decode to. Showing it as text would undo the distinction the type
exists to make.

`to_str` is lossy for the same reason `tcp_read` is ([L-LIB-NET]): a buffer
carries bytes and a `str` carries text, and a malformed byte should not end a
program that is inspecting one.
