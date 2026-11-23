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
import { readFileSync, writeSync } from "node:fs";

const [, , wasmPath, ...progArgs] = process.argv;
let memory = null;
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
    // A call from wasm into the host cannot yield, so a sleep is a spin, as
    // in `scripts/wasm/run.mjs`.
    grasm_sleep_millis: (ms) => {
      const until = performance.now() + ms;
      while (performance.now() < until) {}
    },
  },
});
memory = instance.exports.memory;
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
