# 17 — Simple Shell Environment — **blocked**

**Tests:** process spawning, system-level execution, background forks.

**Result:** cannot be written in Grasm today.

## Why

A shell's one irreducible operation is starting another program. Grasm has no
builtin for it, which is fine — that is what `extern fn` is for. But the C
boundary carries scalars only:

```grasm
extern fn system(cmd: str) -> i32
```
```
error: `str` has no C representation
  help: an `extern fn` takes and returns `i64` or `f64`
```

So `system`, `execv`, `fork`, `posix_spawn` and `popen` are all unreachable —
every one of them takes a pointer. What `extern fn` can call today is `abs`,
`sqrt`, `pow` and their kind.

This is FINDINGS.md **F8**, and it is the same root cause as **F9**: there is no
type in the language that denotes a contiguous run of bytes with a length, which
is what a C pointer parameter needs and what a binary file needs.

## What would unblock it

Either a byte type (`[u8]` or `bytes`) with a defined C representation, which
also unblocks 18, 25 and the binary half of 21 and 30 — or a `spawn`/`exec`
builtin charged to a new `proc` capability, which would fit `[T-CAPABILITY]`
neatly and is a much smaller change. The capability shape is already proven:
`net` was added the same way.

The interactive half of the shell — a prompt loop, tokenising a command line,
handling `cd` and `exit` internally, history — is all writable today, and
project 20 is that program without the fork.
