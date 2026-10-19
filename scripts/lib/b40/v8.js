// Oracle for scripts/b40_diff.py: random values serialized by v8.serialize,
// each file deserialized and printed as b40test prints its own reading.
const v8 = require('v8');
const fs = require('fs');
const [seed, dir, n] = process.argv.slice(2);
let s = Number(seed) >>> 0;
const rand = () => { s = (s * 1103515245 + 12345) >>> 0; return s / 4294967296; };
const pick = k => Math.floor(rand() * k);
const kinds = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array', 'Int32Array', 'Uint32Array',
  'Float32Array', 'Float64Array', 'DataView', 'Buffer', 'BigInt64Array', 'BigUint64Array'];

function str() {
  let t = '';
  for (let k = pick(8); k > 0; k--) {
    const c = pick(6);
    t += c === 0 ? 'é' : c === 1 ? '☺' : c === 2 ? '😀' : c === 3 ? String.fromCharCode(0xd800 + pick(2)) : String.fromCharCode(97 + pick(26));
  }
  return t;
}

function value(d) {
  switch (pick(d > 3 ? 10 : 24)) {
    case 0: return undefined;
    case 1: return null;
    case 2: return rand() < 0.5;
    case 3: return pick(2 ** 31) * (rand() < 0.5 ? -1 : 1);
    case 4: return (rand() - 0.5) * 10 ** pick(30);
    case 5: return [-0, NaN, Infinity, 2 ** 31, -(2 ** 31) - 1][pick(5)];
    case 6: return BigInt(pick(1e9)) ** BigInt(pick(5)) * (rand() < 0.5 ? -1n : 1n);
    case 7: case 8: case 9: return str();
    case 10: { const o = {}; for (let k = pick(4); k > 0; k--) o[rand() < 0.3 ? pick(100) : str()] = value(d + 1); return o; }
    case 11: { const a = []; for (let k = pick(4); k > 0; k--) a.push(value(d + 1)); if (rand() < 0.2) a.extra = 1; return a; }
    case 12: { const a = []; a[pick(30) + 2] = value(d + 1); return a; }
    case 13: return new Date(pick(2e12));
    case 14: return new RegExp('a' + str().replace(/[\\^$.*+?()[\]{}|/]/g, ''), ['', 'g', 'gi', 'msuy', 'dgimsuy'][pick(5)]);
    case 15: { const m = new Map(); for (let k = pick(3); k > 0; k--) m.set(value(d + 1), value(d + 1)); return m; }
    case 16: { const t = new Set(); for (let k = pick(3); k > 0; k--) t.add(value(d + 1)); return t; }
    case 17: return new Uint8Array([pick(256), pick(256)]).buffer;
    case 18: {
      const name = kinds[pick(kinds.length)];
      const buf = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
      if (name === 'Buffer') return Buffer.from(buf.subarray(0, pick(8)));
      if (name === 'DataView') return new DataView(buf.buffer, buf.byteOffset, 8);
      const C = globalThis[name];
      return new C(buf.buffer.slice(buf.byteOffset, buf.byteOffset + 16));
    }
    case 19: return [Object(pick(9)), Object(str()), Object(rand() < 0.5), Object(5n)][pick(4)];
    case 20: { const e = new [Error, RangeError, TypeError, SyntaxError, URIError, EvalError, ReferenceError][pick(7)](str()); if (rand() < 0.3) e.cause = value(d + 1); return e; }
    case 21: { const o = { a: 1 }; o.self = o; return o; }
    case 22: { const x = { k: str() }; return [x, x]; }
    default: return [1, , 3];
  }
}

const units = t => Array.from({ length: t.length }, (_, k) => t.charCodeAt(k).toString(16).padStart(4, '0')).join('');
const bits = f => { const b = Buffer.alloc(8); b.writeDoubleLE(f); return b.readBigInt64LE().toString(); };
const hex = u8 => Buffer.from(u8.buffer, u8.byteOffset, u8.byteLength).toString('hex');
let seen;
function key(k) { return /^(0|[1-9][0-9]*)$/.test(k) && Number(k) < 4294967295 ? 'i' + k : 's' + units(k); }
function props(o, skipIndex, len) {
  return Object.keys(o).filter(k => !(skipIndex && /^(0|[1-9][0-9]*)$/.test(k) && Number(k) < len)).map(k => key(k) + ':' + dump(o[k])).join(',');
}
function dump(v) {
  if (v === undefined) return 'u';
  if (v === null) return 'n';
  if (typeof v === 'boolean') return v ? 't' : 'f';
  if (typeof v === 'number') return 'd' + bits(v);
  if (typeof v === 'bigint') return 'b' + (v < 0n ? '-' : '+') + (v < 0n ? -v : v).toString(16);
  if (typeof v === 'string') return 's' + units(v);
  if (seen.has(v)) return '^' + seen.get(v);
  const me = seen.size;
  seen.set(v, me);
  if (Array.isArray(v)) {
    const items = [];
    for (let k = 0; k < v.length; k++) items.push(k in v ? dump(v[k]) : 'h');
    return 'A' + me + '[' + v.length + '](' + items.join(',') + '){' + props(v, true, v.length) + '}';
  }
  if (v instanceof Date) return 'D' + bits(v.getTime());
  if (v instanceof RegExp) {
    const f = { g: 1, i: 2, m: 4, y: 8, u: 16, s: 32, d: 128, v: 256 };
    return 'R' + units(v.source) + '/' + [...v.flags].reduce((a, c) => a + (f[c] || 0), 0);
  }
  if (v instanceof Map) return 'M' + me + '(' + [...v].map(([a, b]) => dump(a) + '=>' + dump(b)).join(',') + ')';
  if (v instanceof Set) return 'S' + me + '(' + [...v].map(dump).join(',') + ')';
  if (v instanceof ArrayBuffer) return 'B' + me + ':' + Buffer.from(v).toString('hex');
  if (ArrayBuffer.isView(v)) {
    const name = Buffer.isBuffer(v) ? 'Buffer' : v.constructor.name;
    return 'T' + me + ':' + kinds.indexOf(name) + ':' + hex(v);
  }
  if (v instanceof Error) {
    const has = (k) => Object.prototype.hasOwnProperty.call(v, k);
    return 'E' + me + ':' + v.name + ',' + (has('message') ? units(v.message) : '-') + ',' + (typeof v.stack === 'string' ? units(v.stack) : '-') + ',' + ('cause' in v ? dump(v.cause) : '-');
  }
  if (v instanceof Number || v instanceof String || v instanceof Boolean || v instanceof BigInt) return 'X' + me + ':' + dump(v.valueOf());
  return 'o' + me + '(' + props(v, false, 0) + ')';
}

for (let i = 0; i < Number(n); i++) {
  const f = `${dir}/v${i}.v8`;
  fs.writeFileSync(f, v8.serialize(value(0)));
  seen = new Map();
  console.log(f + ' ' + dump(v8.deserialize(fs.readFileSync(f))));
}
