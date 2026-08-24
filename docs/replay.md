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
