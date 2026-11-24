// Runs a Grasm program through a playground module: the wasm build of the
// reference interpreter (`crates/grasm_wasm`), or the Grasm playground built
// by the Grasm backend (`tools/playground`, docs/self-hosting.md §6.25).
//
// Used by scripts/wasmtest.sh, scripts/replaytest.sh and scripts/playtest.sh,
// and the same glue a web page needs -- so the glue the gates exercise is the
// glue that ships, rather than a second copy that can drift from it. Both
// modules take the same imports: each declares the ones it uses and ignores
// the rest.
//
// `GRASM_WASM_STACKS=0` runs as an engine without JS promise integration
// does, for the gates that hold the Grasm module's fallback.
import { readFileSync } from "node:fs";

const [, , wasmPath, srcPath] = process.argv;

// Stack switching for the Grasm module (§6.25 decision (2)): a fiber runs on
// an engine stack of its own, which `WebAssembly.promising` makes for the
// module's export `_fiber`, and a switch parks the running stack until
// something switches back to it. Chosen by feature detection, and told to
// the module through `grasm_stacks`; without it the module runs with no
// stack switched.
const stacks = "Suspending" in WebAssembly && process.env.GRASM_WASM_STACKS !== "0";
// Each parked stack, by the context it waits at: how to resume it, how to
// end it with an exception, and which stack it is (0 the run's own, else
// the context its fiber was started at).
const parked = new Map();
let running = 0;
let startFiber = null;
// An exception that ends a fiber's stack -- an `exit`, a panic -- ends the
// run: it is thrown into the run's own stack, where the export catches it.
function abort(e) {
  for (const [ctx, w] of parked) {
    if (w.stack === 0) {
      parked.clear();
      w.rej(e);
      return;
    }
  }
}
const grasmSwitch = (save, load) =>
  new Promise((res, rej) => {
    parked.set(save, { res, rej, stack: running });
    const w = parked.get(load);
    if (w) {
      parked.delete(load);
      running = w.stack;
      w.res();
    } else {
      running = load;
      queueMicrotask(() => startFiber(load).catch(abort));
    }
  });

let memory = null;
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {
  // The Rust module imports none of the first four, and once imported a
  // `dlsym` it never calls: answering 0 is "no such symbol".
  env: {
    // The host's clock. `std::time` panics on this target, and a panic here is
    // an abort — so `now_millis()` is asked for rather than faked, and a
    // browser answers with `Date.now()`.
    grasm_now_millis: () => Date.now(),
    // Monotonic, so a duration measured with it cannot go backwards.
    // `performance.now()` is exactly that on both a browser and Node.
    grasm_monotonic_millis: () => performance.now(),
    // A call from wasm into the host is synchronous and neither a browser nor
    // Node can yield inside one, so the wait is a spin. That is the honest
    // shape of `sleep` here: the thread really is unavailable for that long,
    // which is what the caller asked for.
    grasm_sleep_millis: (ms) => {
      const until = performance.now() + ms;
      while (performance.now() < until) {}
    },
    dlsym: () => 0,
    // The maps' seed ([L-LIB-MAP], hash flooding): the platform's
    // cryptographic generator, never a clock or a constant. An engine
    // without one is refused (the runtime will not run without a seed).
    grasm_entropy: (p, n) => {
      if (!globalThis.crypto || !crypto.getRandomValues) return -1;
      crypto.getRandomValues(new Uint8Array(memory.buffer, p >>> 0, n >>> 0));
      return 0;
    },
    grasm_stacks: () => (stacks ? 1 : 0),
    grasm_switch: stacks
      ? new WebAssembly.Suspending(grasmSwitch)
      : () => {
          throw new Error("no stack switching here");
        },
    grasm_fiber: (ctx) => {
      parked.delete(ctx);
    },
  },
});

const ex = instance.exports;
memory = ex.memory;
const bytes = () => new Uint8Array(ex.memory.buffer);
if (stacks && ex._fiber) startFiber = WebAssembly.promising(ex._fiber);
// A run's export, called through `promising` where stacks are switched, so
// that the run may suspend; awaited either way.
const call = (f) => {
  running = 0;
  return stacks ? WebAssembly.promising(f) : f;
};

// Replays a journal recorded elsewhere. `journal` is the bytes `GRASM_RECORD`
// wrote — the same file a native build produces, handed over rather than read,
// because wasm32 has no filesystem.
export async function replay(source, journal) {
  const enc = new TextEncoder();
  const encoded = enc.encode(source);
  const src = ex.grasm_alloc(encoded.length) >>> 0;
  bytes().set(encoded, src);
  const jPtr = journal.length ? ex.grasm_alloc(journal.length) >>> 0 : 0;
  if (journal.length) bytes().set(journal, jPtr);
  const res = await call(ex.grasm_run_with_journal)(src, encoded.length, jPtr, journal.length);
  ex.grasm_free(src, encoded.length);
  if (journal.length) ex.grasm_free(jPtr, journal.length);
  return frame(res);
}

export async function run(source, stdin = "") {
  const enc = new TextEncoder();
  const encoded = enc.encode(source);
  const src = ex.grasm_alloc(encoded.length) >>> 0;
  bytes().set(encoded, src);
  // `input()` has no stdin on wasm32, so the lines are handed in.
  const inBytes = enc.encode(stdin);
  const inPtr = inBytes.length ? ex.grasm_alloc(inBytes.length) >>> 0 : 0;
  if (inBytes.length) bytes().set(inBytes, inPtr);
  const res = await call(ex.grasm_run_with_stdin)(src, encoded.length, inPtr, inBytes.length);
  ex.grasm_free(src, encoded.length);
  if (inBytes.length) ex.grasm_free(inPtr, inBytes.length);

  return frame(res);
}

// The frame the ABI documents: total, exit code, stdout, diagnostics.
function frame(answer) {
  const res = answer >>> 0;
  const view = new DataView(ex.memory.buffer);
  const total = view.getUint32(res, true);
  const code = view.getInt32(res + 4, true);
  const outLen = view.getUint32(res + 8, true);
  const dec = new TextDecoder();
  const out = dec.decode(bytes().slice(res + 12, res + 12 + outLen));
  const errAt = res + 12 + outLen;
  const errLen = view.getUint32(errAt, true);
  const err = dec.decode(bytes().slice(errAt + 4, errAt + 4 + errLen));
  ex.grasm_free(res, total);
  return { code, out, err };
}

// `--session <list>` runs every program the list names in this one instance,
// one after another, as a page does: each line `source<TAB>stdin<TAB>journal`,
// `-` for none; one JSON line out per run. A run must not see the one
// before it (the Grasm module's reset, docs/self-hosting.md §6.25).
if (srcPath === "--session") {
  for (const line of readFileSync(process.argv[4], "utf8").split("\n")) {
    if (!line) continue;
    const [src, stdin, jrnl] = line.split("\t");
    const source = readFileSync(src, "utf8");
    const r =
      jrnl && jrnl !== "-"
        ? await replay(source, new Uint8Array(readFileSync(jrnl)))
        : await run(source, stdin && stdin !== "-" ? readFileSync(stdin, "utf8") : "");
    process.stdout.write(JSON.stringify(r) + "\n");
  }
  process.exit(0);
}

// `--journal <path>` replays instead of running; anything else is stdin.
const fourth = process.argv[4];
const source = readFileSync(srcPath, "utf8");
const { code, out, err } =
  fourth === "--journal"
    ? await replay(source, new Uint8Array(readFileSync(process.argv[5])))
    : await run(source, fourth ? readFileSync(fourth, "utf8") : "");
process.stdout.write(out);
if (err) process.stderr.write(err.endsWith("\n") ? err : err + "\n");
process.exit(code & 0xff);
