# Binding a C library

Twenty of the forty-six packages `docs/ecosystem.md` found absent are wrappers
around a C library that already exists and that nobody should rewrite:
SQLite, DuckDB, FFmpeg, ONNX, libusb, Vulkan, Raylib. Each is a day's work
once the declarations exist and a month if they are typed by hand.

Two things were needed, and the second was not the one that looked obvious.

## What the FFI can carry

An `extern fn` takes `i64`, `f64`, `str` and `bytes`, and answers `i64` or
`f64` ([T-EXTERN], [T-EXTERN-POINTER]). That is less than C has and more than
it looks, because a pointer is an address and an address is an `i64`:

- a handle C hands back (`sqlite3*`, `FILE*`) is an `i64` held and passed back;
- an out-parameter is a `bytes` the caller sizes and C fills -- how
  `packages/termios` reads a `struct termios` out of `tcgetattr`, and how a
  `sqlite3**` yields its handle;
- text C hands back is read with `strlen` and `memcpy` declared over an `i64`
  address.

So handle-based C APIs need no new language feature. Verified by using one:
`getenv` returning a `char*`, read through `strlen` and `memcpy`, and
`malloc`/`free` as an opaque handle.

What it cannot carry is a struct passed or returned *by value*, and varargs.
`packages/bindgen` reports those rather than guessing, because a wrong FFI
signature is not a compile error -- it is garbage three calls later.

## `packages/bindgen`

Preprocessed C in, `extern fn` declarations out. It declares no capabilities:
it opens no file and runs no preprocessor, so the consumer does the IO and
decides whether to trust what comes back.

    cc -E -P /usr/include/sqlite3.h > sqlite3.i
    grasm run packages/bindgen-demo/src/main.gr -- sqlite3.i /usr/include/sqlite3.h

The second path is the *raw* header, read only for its integer `#define`s --
`SQLITE_OK`, `SQLITE_ROW`, the vocabulary a handle-based API answers in.
`cc -E` deletes those before the declarations become readable, so they are
read separately.

On `sqlite3.h`: **275 functions, 198 constants, 8 refused.** On `string.h`:
58 functions, nothing refused.

Three things it had to learn, each from a real header:

- `__attribute__((...))` is not a declaration. Before dropping it and its
  parenthesised group, the output carried `extern fn __attribute__(...)` and
  110 real declarations were lost behind it.
- `sqlite3_close(sqlite3*)` names no parameter, so its only identifier is the
  *type*. Taking it for the name produced `p_sqlite3` where `p_a0` belongs.
- C's leading underscores (`__s`, `__n`) are a header keeping out of a
  consumer's macro namespace, and read as noise in Grasm.

## `--link`, which was the actual blocker

bindgen generates correct declarations and they did not work. An `extern fn`
is resolved by the system linker, and nothing could put a library on the link
line -- the error even said so. `termios` works because libc is always there;
SQLite is not.

`grasm build --native prog.gr --link sqlite3` adds `-lsqlite3`. A bare name
becomes `-lname`; a value starting with `-` or `/` is passed through as
written, which is how a search path or an archive by path gets in. The values
are collected once from the command line into a `OnceLock` and read once at
the link.

## End to end

`scratchpad/sqltest.gr`, built with `--link sqlite3`, opens an in-memory
database, creates a table, inserts two rows, prepares a statement, steps it,
reads an integer and a text column, finalises and closes:

    opened: true
    create 0, insert 0
    prepare 0
    row: 7 seven
    row: 42 answer
    rows 2, closed

Every declaration in it came out of `bindgen`. What a `grasm-sqlite` package
now needs is the safe layer over these: a handle type, `Result` instead of an
`i64` return code, and a statement that finalises itself. None of that is FFI
work.

## What is still missing

A library whose API passes structs by value cannot be bound, and that is a
real limit for some graphics APIs -- `Vector3` by value is everywhere in
Raylib. The workaround is a C shim, which is the thing this set out to avoid;
the fix is `extern` support for a struct of scalars, and it is unmeasured.
