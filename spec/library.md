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
| `args()` | `() -> [str]` | arguments after `--` |
| `panic(msg)` | `(str)` | prints `panic: msg`, exits 101 |
| `assert(c)` | `(bool)` | panics when `c` is false |
| `assert_eq(a, b)` | `(T, T)` | panics when `a != b` |
| `fail(e)` | `(E)` | raises the error branch of a `T ! E` |
| `map()` | `() -> Map[K, V]` | a new empty map |
| `now_millis()` | `() -> i64` | milliseconds since the epoch; `io` |
| `env(name)` | `(str) -> Option[str]` | environment variable; `io` |
| `exit(code)` | `(i64)` | ends the program with that status |
| `read_file(p)` | `(str) -> str ! IoError` | fallible, not `Option`; `io` ([L-LIB-IO]) |
| `write_file(p, s)` | `(str, str)` | `io` |
| `file_exists(p)` | `(str) -> bool` | `io` |

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

This is what draining a queue from the front needs. Without it a ready queue
cost `ready = ready.slice(1, ready.len())` a step, which is O(n) and reads
like the workaround it was.

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
([E-STR-CHARS]).

**[L-LIB-TRIM-SIDES]** `trim_start` and `trim_end` remove whitespace from one
end only; `trim` removes it from both. None of the three takes an argument —
whitespace is what they trim. The one-sided pair exists because a line's
leading indentation usually carries meaning and its trailing newline does not,
and `trim` cannot tell you that.

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
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

Both `len` and `code_at` answer in constant time. They count *characters*, so
each used to decode the string from the start: `while i < s.len()` was
quadratic in two ways at once, and 400 000 characters took 6.7 seconds where
the identical loop over a list took 0.21. A string never changes once built,
so its character count is computed once and kept; when that count equals the
byte count the text is ASCII and an index is an index.

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
| `xs.sort()` | | mutates; ascending. Only `i64`, `f64` and `str` are ordered — sorting anything else is a compile error, the same one `<` gives |
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
the *elements* ordered, and a struct is not. The key must be `i64`, `f64` or
`str`. The sort is stable: elements with equal keys stay in the order they
were written. It sorts **ascending**, always; descending is
`xs.sort_by(k)` followed by `xs.rev()`. Negating the key works for a number
and has no counterpart for a `str`, so the reversal is the general answer and
the one to reach for.

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
asking whether the element was ordered, so `xs.enumerate().min()` — the obvious
"index of the smallest" — was refused on a list `sort` orders happily.

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
