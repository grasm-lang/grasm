// The oracle for packages/nodeutilformat: Node's own util.format and
// util.inspect. Each argument is s<units>, n<bits>, b<digits>, t, f, l
// (null) or u (undefined); strings are UTF-16 units in hex joined by '.'.
const util = require('util');
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const str = h => h === '-' ? '' : String.fromCharCode(...h.split('.').map(x => parseInt(x, 16)));
const us = s => s.length ? Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16)).join('.') : '-';
const buf = new DataView(new ArrayBuffer(8));
const arg = t => {
  switch (t[0]) {
    case 's': return str(t.slice(1));
    case 'n': buf.setBigUint64(0, BigInt('0x' + t.slice(1))); return buf.getFloat64(0);
    case 'b': return BigInt(t.slice(1));
    case 't': return true;
    case 'f': return false;
    case 'l': return null;
    default: return undefined;
  }
};
const out = [];
for (const l of lines) {
  const f = l.split(' ');
  try {
    const args = f[1] === '~' ? [] : f[1].split(',').map(arg);
    out.push(us(f[0] === 'fm' ? util.format(...args) : util.inspect(args[0])));
  } catch (e) {
    out.push('E' + us(e.message));
  }
}
process.stdout.write(out.join('\n') + '\n');
