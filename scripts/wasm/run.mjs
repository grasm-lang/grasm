// Runs a Grasm program through the wasm build of the reference interpreter.
//
// Used by scripts/wasmtest.sh, and the same twenty lines a web page needs — so
// the glue the gate exercises is the glue that ships, rather than a second
// copy that can drift from it.
import { readFileSync } from "node:fs";

const [, , wasmPath, srcPath] = process.argv;
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {
  // The module imports one symbol it never calls: `dlsym`, reached through
  // Rust's `std` on this target. Answering 0 is "no such symbol", which is
  // what a caller of `dlsym` is required to handle — and nothing in a Grasm
  // program can reach it, since `extern fn` needs a native linker.
  env: {
    // The host's clock. `std::time` panics on this target, and a panic here is
    // an abort — so `now_millis()` is asked for rather than faked, and a
    // browser answers with `Date.now()`.
    grasm_now_millis: () => Date.now(),
    dlsym: () => 0,
  },
});

const ex = instance.exports;
const bytes = () => new Uint8Array(ex.memory.buffer);

export function run(source, stdin = "") {
  const enc = new TextEncoder();
  const encoded = enc.encode(source);
  const src = ex.grasm_alloc(encoded.length);
  bytes().set(encoded, src);
  // `input()` has no stdin on wasm32, so the lines are handed in.
  const inBytes = enc.encode(stdin);
  const inPtr = inBytes.length ? ex.grasm_alloc(inBytes.length) : 0;
  if (inBytes.length) bytes().set(inBytes, inPtr);
  const res = ex.grasm_run_with_stdin(src, encoded.length, inPtr, inBytes.length);
  ex.grasm_free(src, encoded.length);
  if (inBytes.length) ex.grasm_free(inPtr, inBytes.length);

  // Read the frame the ABI documents: total, exit code, stdout, diagnostics.
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

const stdinPath = process.argv[4];
const stdinText = stdinPath ? readFileSync(stdinPath, "utf8") : "";
const { code, out, err } = run(readFileSync(srcPath, "utf8"), stdinText);
process.stdout.write(out);
if (err) process.stderr.write(err.endsWith("\n") ? err : err + "\n");
process.exit(code & 0xff);
