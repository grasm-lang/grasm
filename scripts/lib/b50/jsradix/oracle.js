// The oracle for packages/jsradix: V8's Number toString(radix) and parseInt.
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const buf = new DataView(new ArrayBuffer(8));
const num = h => { buf.setBigUint64(0, BigInt('0x' + h)); return buf.getFloat64(0); };
const bits = x => { buf.setFloat64(0, x); return buf.getBigUint64(0).toString(16); };
const unhex = h => h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8');
const hx = s => Buffer.from(s, 'utf8').toString('hex') || '-';
const out = [];
for (const l of lines) {
  const [op, a, b] = l.split(' ');
  try {
    switch (op) {
      case 'ts': out.push(hx(String(num(a)))); break;
      case 'tsr': out.push(hx(num(a).toString(Number(b)))); break;
      case 'tf': out.push(hx(num(a).toFixed(Number(b)))); break;
      case 'te': out.push(hx(b === '~' ? num(a).toExponential() : num(a).toExponential(Number(b)))); break;
      case 'tp': out.push(hx(b === '~' ? num(a).toPrecision() : num(a).toPrecision(Number(b)))); break;
      case 'pf': out.push(bits(parseFloat(unhex(a)))); break;
      case 'pi': out.push(bits(b === '~' ? parseInt(unhex(a)) : parseInt(unhex(a), Number(b)))); break;
      default: out.push(bits(Number(unhex(a))));
    }
  } catch (e) {
    out.push('R' + hx(e.message));
  }
}
process.stdout.write(out.join('\n') + '\n');
