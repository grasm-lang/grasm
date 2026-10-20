// The oracle for packages/nodebuffer: Node's own Buffer. Strings travel as
// UTF-16 code units in hex joined by '.', '-' when empty.
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const str = h => h === '-' ? '' : String.fromCharCode(...h.split('.').map(x => parseInt(x, 16)));
const us = s => s.length ? Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16)).join('.') : '-';
const out = [];
for (const l of lines) {
  const [op, a, enc] = l.split(' ');
  try {
    if (op === 'fs') {
      const s = str(a);
      out.push((Buffer.from(s, enc).toString('hex') || '-') + ' ' + Buffer.byteLength(s, enc));
    } else {
      out.push(us(Buffer.from(a === '-' ? '' : a, 'hex').toString(enc)));
    }
  } catch (e) {
    out.push('E' + Buffer.from(e.message).toString('hex'));
  }
}
process.stdout.write(out.join('\n') + '\n');
