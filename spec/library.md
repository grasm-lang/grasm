# Standard Library

Every operation the language provides, by receiver type. There are no imports:
these are always available.

`spec/conformance/L-LIBRARY.gr` exercises this whole surface, so an entry here
that stopped being true would fail the test suite.

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
| `read_file(p)` | `(str) -> Option[str]` | `io` |
| `write_file(p, s)` | `(str, str)` | `io` |
| `file_exists(p)` | `(str) -> bool` | `io` |

## 1a. How values are written

**[L-DISPLAY]** `print(x)` and string interpolation render every type the same
way, and this is normative because a program's output is what its correctness
is judged on.

| type | rendering | example |
|------|-----------|---------|
| `i64` | decimal | `-7` |
| `f64` | shortest form that round-trips, always with a point | `1.5`, `1000000.0`, `-0.0` |
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

**[L-LIB-NUM]**

| method | type | notes |
|--------|------|-------|
| `n.abs()` | `-> i64` / `-> f64` | |
| `n.min(m)`, `n.max(m)` | `(T) -> T` | |
| `x.floor()`, `x.ceil()`, `x.round()` | `f64 -> f64` | |
| `x.sqrt()` | `f64 -> f64` | |
| `n.to_str()` | `-> str` | |
| `n.as_f64()` | `i64 -> f64` | widening |
| `x.as_i64()` | `f64 -> i64` | truncates toward zero |

Conversion is by method, not by a cast or a `int(...)`-style function. A `str`
becomes a number with `parse_int` / `parse_float`, which return `Option`
because the text may not be a number.

## 3. `str`

**[L-LIB-STR]** Lengths and offsets are in **characters, not bytes**
([E-STR-CHARS]).

| method | type | notes |
|--------|------|-------|
| `s.len()` | `-> i64` | characters |
| `s.is_empty()` | `-> bool` | |
| `s.substr(start, len)` | `(i64, i64) -> str` | **start and length**, not start and end |
| `s.chars()` | `-> [str]` | one single-character string per element |
| `s.lines()` | `-> [str]` | splits on newlines; no trailing empty line |
| `s.split(sep)` | `(str) -> [str]` | |
| `s.contains(t)`, `s.starts_with(t)`, `s.ends_with(t)` | `(str) -> bool` | |
| `s.index_of(t)` | `(str) -> Option[i64]` | `None` when absent |
| `s.replace(from, to)` | `(str, str) -> str` | |
| `s.trim()` | `-> str` | |
| `s.to_upper()`, `s.to_lower()` | `-> str` | |
| `s.repeat(n)` | `(i64) -> str` | |
| `s.pad_start(w, fill)`, `s.pad_end(w, fill)` | `(i64, str) -> str` | fill repeats and is cut to width ([E-PAD-CYCLE]) |
| `s.parse_int()` | `-> Option[i64]` | |
| `s.parse_float()` | `-> Option[f64]` | |

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
| `xs.sort()` | | mutates; ascending. Bools have no order, so sorting them does nothing |
| `xs.rev()` | `-> [T]` | |
| `xs.slice(a, b)` | `(i64, i64) -> [T]` | `a` up to but excluding `b` |
| `xs.contains(v)` | `(T) -> bool` | |
| `xs.join(sep)` | `(str) -> str` | `[str]` only — map to `str` first |
| `xs.sum()` | `-> T` | numeric elements |
| `xs.min()`, `xs.max()` | `-> Option[T]` | `None` when empty; not available for `bool` |
| `xs.fold(init, f)` | `(B, (B, T) -> B) -> B` | left fold; the accumulator's type comes from `init`, not from `T` |
| `xs.map(f)` | `(T -> U) -> [U]` | |
| `xs.par_map(f)` | `(T -> U) -> [U]` | multi-core; `f` must be effect-free ([T-PAR-MAP-PURE]) |
| `xs.filter(f)` | `(T -> bool) -> [T]` | |
| `xs.par_filter(f)` | `(T -> bool) -> [T]` | multi-core; `f` must be effect-free; input order kept ([E-PAR-FILTER-ORDER]) |
| `xs.any(f)`, `xs.all(f)` | `(T -> bool) -> bool` | short-circuit ([E-SEARCH-SHORTCIRCUIT]) |
| `xs.find(f)` | `(T -> bool) -> Option[T]` | first match |

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
