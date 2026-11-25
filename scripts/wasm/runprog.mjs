// Runs a Grasm program compiled to a wasm module by the Grasm backend
// (`s3 --target wasm32`, docs/self-hosting.md §6.24).
//
//     node scripts/wasm/runprog.mjs <program.wasm> [argument ...]
//
// The module imports what its C library (`compiler/rtwasm`) asks of the
// host, and nothing else: the standard streams, entropy for the maps' seed,
// and the three clocks the Rust playground's module imports. It exports
// `memory`; `_args`, the address of the area this script writes the
// argument vector into (eight-byte pointers, then the NUL-terminated strings,
// the program's name first); and `_start(argc)`, which runs the program and
// answers its exit status -- `exit` and a panic included, which unwind to
// `_start` inside the module. Standard input is empty.
//
// A recursion deeper than one engine stack hops to a fresh one
// (docs/self-hosting.md §6.26): the module calls `grasm_deeper(record)`, and
// this calls the module's export `_deep(record)` through
// `WebAssembly.promising`, which runs it on a stack of its own to its end
// before returning -- so the run stays synchronous -- and answers 1; the
// module checks that the stack finished. With no JS promise integration
// (`GRASM_WASM_STACKS=0` stands in for an engine without it, as in
// `scripts/wasm/run.mjs`) it answers 0, and the recursion ends in the
// clause's panic.
import { readFileSync, writeSync } from "node:fs";

const [, , wasmPath, ...progArgs] = process.argv;
let memory = null;
let deep = null;
// What ended a deeper stack otherwise than by returning or by `exit` -- a
// trap -- reaches this only as a rejected promise, after the module has
// already trapped on finding the stack unfinished: said at the end.
let lost = null;
process.on("exit", () => {
  if (lost) process.stderr.write(`the deeper stack: ${lost}\n`);
});
const bytes = (p, n) => new Uint8Array(memory.buffer, p, n);

const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {
  env: {
    // `write(2)` on standard output or error: every byte, as the C library
    // asks, so a short write never reaches it.
    grasm_write: (fd, p, n) => {
      const b = bytes(p >>> 0, n >>> 0);
      let off = 0;
      while (off < b.length) off += writeSync(fd, b, off, b.length - off);
      return n;
    },
    grasm_read: () => 0,
    // The maps' seed ([L-LIB-MAP], hash flooding): the platform's own
    // generator, never a clock or a constant.
    grasm_entropy: (p, n) => {
      crypto.getRandomValues(bytes(p >>> 0, n >>> 0));
      return 0;
    },
    grasm_now_millis: () => Date.now(),
    grasm_monotonic_millis: () => performance.now(),
    grasm_deeper: (rec) => {
      if (!deep) return 0;
      // The deepest stack's rejection comes first: it is the cause.
      deep(rec).catch((e) => {
        lost ??= e;
      });
      return 1;
    },
    // A call from wasm into the host cannot yield, so a sleep is a spin, as
    // in `scripts/wasm/run.mjs`.
    grasm_sleep_millis: (ms) => {
      const until = performance.now() + ms;
      while (performance.now() < until) {}
    },
  },
});
memory = instance.exports.memory;
if ("promising" in WebAssembly && process.env.GRASM_WASM_STACKS !== "0" && instance.exports._deep) {
  deep = WebAssembly.promising(instance.exports._deep);
}
const argv = ["main", ...progArgs].map((a) => new TextEncoder().encode(a + "\0"));
const area = instance.exports._args() >>> 0;
const view = new DataView(memory.buffer);
let at = area + 8 * (argv.length + 1);
argv.forEach((a, k) => {
  view.setBigUint64(area + 8 * k, BigInt(at), true);
  bytes(at, a.length).set(a);
  at += a.length;
});
if (at - area > 65536) throw new Error("the arguments do not fit the module's area");
process.exitCode = instance.exports._start(argv.length) & 0xff;
