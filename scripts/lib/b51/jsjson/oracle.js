// The oracle for packages/jsjson: V8's own JSON. Text travels as UTF-16
// code units in hex joined by '.', '-' when empty.
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const str = h => h === '-' ? '' : String.fromCharCode(...h.split('.').map(x => parseInt(x, 16)));
const us = s => s.length ? Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16)).join('.') : '-';
const out = [];
for (const l of lines) {
  const [op, a, space] = l.split(' ');
  try {
    const v = JSON.parse(str(a));
    const gap = space === '~' ? undefined : space[0] === 'n' ? Number(space.slice(1)) : str(space.slice(1));
    out.push(op === 'pa' ? us(JSON.stringify(v)) : us(JSON.stringify(v, null, gap)));
  } catch (e) {
    out.push('E' + us(e.message));
  }
}
process.stdout.write(out.join('\n') + '\n');
