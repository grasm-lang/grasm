# 17 — Simple Shell Environment — **works**

**Tests:** process spawning, system-level execution, exit statuses, built-ins.
**Result:** a working shell, with POSIX-style quoting.

## What it does

```
--- parsing ---
  double quotes: 2 -> [echo][hello world]
  escaped quote: 2 -> [echo][say "hi"]
  unclosed double: error — unclosed " quote
--- running programs ---
hello from a child process
7
no-such-program-anywhere: not found
127
```

`grasm run main.gr -- repl` is an interactive shell.

## Why it was blocked, and what changed

A shell's one irreducible operation is starting another program, and Grasm had
no way to. `extern fn` carries scalars only, so `system`, `execv`, `fork`,
`posix_spawn` and `popen` — every one of which takes a pointer — were all
unreachable (**F8**).

The fix was *not* to widen the C boundary. It was `run_program(program, args)` under a
capability of its own ([L-LIB-PROC]).

**`proc` is strictly larger than `io` or `net`.** A program that can start
`curl` has the network whether or not it declared `net`; one that can start `rm`
has the filesystem. Folding process execution into `io` would have made every
capability claim in the language weaker than it reads — a manifest could say
`io` and mean everything. So this file declares `/ {io, proc}`, and a package
that never asked for `proc` could not contain it.

F8 itself is **not** closed by this. `extern fn` still carries only `i64` and
`f64`, and every C function taking a pointer is still unreachable. What changed
is that the one thing this project needed no longer has to go through C.

## What the parsing is for

A shell that splits on spaces cannot open a file whose name has one in it, which
is most files a person actually has. So quoting is in the parser: double quotes,
single quotes, backslash escapes, quotes starting mid-word, and a backslash that
is literal inside single quotes as POSIX requires. The failure cases — unclosed
quote, trailing backslash — are named rather than silently producing a different
command.

That half is pure, so it runs on every gate across all three engines. The
process half runs too, against `echo` and `sh`, which POSIX guarantees exist.

## The built-ins, and why they have to be built in

`cd` cannot be a program: a child process changes its own directory and exits,
which is not what anyone means. `exit` and `status` are the same shape of thing.
`cd` here is *tracked rather than performed* — `run_program` starts each program in this
process's directory and nothing in the language can change that, so the shell
reports what it was told rather than pretending. A shell that silently ignored
`cd` would be worse than one that says what it does.

## What it does not have

No pipes, no redirection, no job control, no background `&`. Each needs the
child's file descriptors, and `run_program` captures both streams and returns them —
which is the right default for a builtin and the wrong primitive for a pipeline.
That is a real gap and it is recorded as a finding.
