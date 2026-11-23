// Compares `mathfuzz`'s lines (`<fn> <x bits> <y bits> <answer bits>`, read
// from standard input) with the Rust playground's libm (the probe module
// `scripts/wasm/libmprobe.rs` builds), bit for bit; a NaN is any NaN, as
// the two are allowed to carry different payloads. Prints the count per
// function and every disagreement (at most 20), and exits 1 if there is one.
//
//     node scripts/wasm/mathcmp.mjs <probe.wasm> < lines
import { readFileSync } from "node:fs";

const probe = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(process.argv[2]))).exports;
const two = new Set(["pow", "atan2", "hypot", "fmod", "fmin", "fmax"]);
const buf = new DataView(new ArrayBuffer(8));
const toF = (s) => { buf.setBigInt64(0, BigInt(s), true); return buf.getFloat64(0, true); };
const toB = (f) => { buf.setFloat64(0, f, true); return buf.getBigInt64(0, true); };
const lines = readFileSync(0, "utf8").split("\n").filter(Boolean);
const count = new Map();
let bad = 0;
for (const line of lines) {
  const [name, xs, ys, rs] = line.split(" ");
  const f = probe["r_" + name];
  if (!f) { console.log(`no function ${name} in the probe`); process.exit(2); }
  const x = toF(xs), y = toF(ys);
  const want = two.has(name) ? f(x, y) : f(x);
  const got = toF(rs);
  count.set(name, (count.get(name) || 0) + 1);
  const same = Number.isNaN(want) ? Number.isNaN(got) : toB(want) === BigInt(rs);
  if (!same) {
    bad += 1;
    if (bad <= 20) console.log(`DIFFERS ${name}(${x}${two.has(name) ? ", " + y : ""}): ours ${got} (${rs}), the playground's ${want} (${toB(want)})`);
  }
}
const total = [...count.values()].reduce((a, b) => a + b, 0);
console.log(`calls: ${total}   functions: ${count.size}   disagreeing: ${bad}`);
process.exit(bad ? 1 : 0);
