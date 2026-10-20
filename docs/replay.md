<!--
Not a spec clause. The spec's normative clauses each carry a conformance
program that one run of one file can check, and this property spans two runs
of a program with different environments — so it lives here beside `wasm.md`,
which is an engine-level guarantee gated the same way.

The gate is `scripts/replaytest.sh`.
-->

# Recording a run, and replaying it

A run may be *recorded*: every builtin whose answer came from
outside the program is written to a **journal**, in call order. A later run
given that journal takes its answers from it and does not reach outside the
program at all.

`GRASM_RECORD=<path>` records; `GRASM_REPLAY=<path>` replays. Both set means
recording, because a replay that rewrote the journal it was reading would
destroy the evidence it exists to preserve.

Which builtins are journaled is not a separate list. A builtin either declares
an effect row or it does not, and one that does not cannot read anything —
so the effect system already answers the question that is normally the hard part
of building a record-and-replay tool. Every builtin with a non-empty row is
either **recorded** or **repeated**, and `crates/grasm_sema/src/builtins.rs`
says which for each; a builtin that is neither is a compile-time error in the
implementation's own tests.

*Recorded* covers the whole `io`, `net` and `proc` surface bar two. A replay
does not perform the effect at all — not the reads and not the writes. Re-running
a `write_file` to reproduce a bug would damage a disk, and re-running a
`tcp_connect` would need the other end of the connection to still be there,
which is exactly what it will not be.

*Repeated* is `print` and `eprint` alone: those are the program speaking to the
person watching it, and a replay whose output you cannot see is not worth
running.

**A replay is byte-identical across engines.** The journal carries values in one
portable encoding that every engine reads, so a recording made by a compiled
build replays under `grasm run` — which is the case that matters, because the
failure happens in the fast engine and the debugger is in the other one.

**Divergence is refused, not guessed.** On replay the recorded call name is
checked against the call actually made. If they differ, or the journal runs out,
the run stops with exit status 7 and says which call disagreed. The alternative
is to hand back the next record anyway, which produces a run that looks like a
replay and is not — and a tool whose whole promise is *this is what happened*
must not be able to lie about it.

A replay that ends with records unread says so: it stopped earlier than the run
it was replaying, and silence would let that pass for success.

**A C function is not replayed; a replay stops at it.** Whatever an `extern fn`
reads or changes happens outside the journaled builtins, so a recording holds
none of it. Until 2026-10-19 a replay simply called the function again, for
real -- `getpid()` printed a new number under a run that said it was a replay.
Now the first C call of a replay stops it with exit status 7 and the
function's name, in both engines, however the function is reached. A program
that calls C can be recorded, and replayed up to its first C call.

## What it costs

A run with neither variable set reads one thread-local per journaled call and
does nothing else — no wrapper allocation, no serialisation, no branch on a
global. None of the benchmark programs calls a journaled builtin inside a loop,
so this is not expected to be measurable there. That is a prediction, not a
measurement; `scripts/bench.sh` is where it would show up if it were wrong.

## In a browser tab

The browser engine cannot *write* a journal — wasm32 has no filesystem — but it
can be handed one:

```js
import { replay } from "./run.mjs";
replay(source, new Uint8Array(journalBytes));
```

which reaches the module's `grasm_run_with_journal` export. A recording of a
program that opened a TCP socket, ran a child process and listed a directory
replays there byte-identically, in a module that has none of those three. That
is the clearest statement of what a journal is: everything the program read, and
nothing of the machine it read it from.

`scripts/replaytest.sh` runs exactly that comparison on every invocation.

## Reading a recording

```
$ grasm journal j
j — 13 record(s), 598 bytes

   1  now_millis    1787562277933
   2  random_bytes  Ok(3 byte(s) 2a 5e 25)
   3  proc_run      Ok((0, "hi from a chil" … 2 more char(s), ""))
   4  tcp_listen    Ok(0)
   5  net_port      Ok(59989)
   6  tcp_connect   Ok(1)
   7  tcp_accept    Ok(2)
   8  tcp_write     Ok(())
   9  tcp_read      Ok("ping\n")
  10  net_close     ()
  ...
  13  list_dir      Ok(["a.txt", "bun", "bundle.txt", … 6 more])

  13 call(s) across 11 builtin(s): list_dir 1, net_close 3, net_port 1, ...
```

A file of framed bytes is evidence nobody can examine. This is the only way to
answer *what did the program actually see?* without running it again — which is
the one thing a recording exists so you do not have to do.

Long payloads are cut and say how much was cut; `--full` prints all of it. The
tally at the end is worth reading on its own: a recording with four hundred
`now_millis` calls is telling you something about the program before you read a
single value.

## In a debugger

`launch.json`:

```json
{
  "type": "grasm",
  "request": "launch",
  "program": "${workspaceFolder}/src/main.gr",
  "journal": "${workspaceFolder}/production.jrnl"
}
```

Breakpoints, stepping and variables work as they always did — but the values
are the recorded ones. Stop on the line after `now_millis()` and the variable
holds the time the failure happened, not the time you looked.

A journal that cannot be read **fails the launch** rather than starting a live
run with a warning. A session that quietly ran against the real world while you
believed you were watching a replay would be worse than an error, because you
would trust what you saw.

## Writing a journal by hand

A journal does not have to have been recorded. Written by hand it is a *script
of what the world will say*:

```rust
grasm_journal::framed(&[
    ("read_file".into(), Wire::err(Wire::io_error("IoFailed", "disk quota exceeded"))),
])
```

The program cannot tell the difference, because there is none — the effect
system says those are the only places the world reaches it. So the failure
paths become reachable without a broken machine to reach them from, which is
otherwise the hardest kind of test to write. The disk full, the socket refusing,
the clock jumping backwards, `read_file` returning something no file contains.

This is the same property that makes replay work, used in the other direction.

## What is not covered

**Recording in the browser.** Only replaying. A journal is written to a file
and wasm32 has none; a page that wanted to record would need somewhere to put
the bytes, and no page here does.

**Threads.** The journal is per-thread, so a program whose parallel workers
each reach outside themselves records one journal on the main thread and
nothing from the workers. `par_map` and `par_filter` refuse an effectful
function, so no program in the corpus can hit this — but a future one could,
and the honest statement is that the guarantee is single-threaded today.

**The order of *writes* is not reproduced against the world.** A replay does
not write, so a bug whose symptom is a corrupted file on disk is reproduced as
the *sequence of calls that would have corrupted it*, not as the corruption.
That is usually what you want and occasionally is not.

## Trying it

```
$ cat > /tmp/w.gr <<'EOF'
fn main() / {io} {
  match write_file("/tmp/evidence.txt", "written for real") {
    Ok(u) -> print("wrote it")
    Err(e) -> print("write failed")
  }
  print("clock " + now_millis().to_str())
}
EOF
$ grasm build --native /tmp/w.gr -o /tmp/w
$ GRASM_RECORD=/tmp/j /tmp/w
wrote it
clock 1787561313367
$ rm /tmp/evidence.txt
$ GRASM_REPLAY=/tmp/j grasm run /tmp/w.gr     # a different engine
wrote it
clock 1787561313367
$ ls /tmp/evidence.txt
ls: /tmp/evidence.txt: No such file or directory
```

The clock is the recorded one, and nothing was written.
