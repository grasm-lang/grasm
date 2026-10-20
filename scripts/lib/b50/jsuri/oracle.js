// The oracle for packages/jsuri: V8's URI functions. Input text is hex
// UTF-8; answers are UTF-16 code units in hex, joined by `.`.
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const unhex = h => h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8');
const units = s => s.length ? Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16)).join('.') : '-';
const fns = { ec: encodeURIComponent, eu: encodeURI, dc: decodeURIComponent, du: decodeURI, es: escape, un: unescape };
const out = [];
for (const l of lines) {
  const [op, a] = l.split(' ');
  try { out.push(units(fns[op](unhex(a)))); } catch (e) { out.push('E' + e.constructor.name); }
}
process.stdout.write(out.join('\n') + '\n');
