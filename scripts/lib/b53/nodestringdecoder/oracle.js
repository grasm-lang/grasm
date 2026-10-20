// The oracle for packages/nodestringdecoder: Node's own StringDecoder.
// A line is an encoding name (hex of UTF-8, '~' for undefined) and a run
// of steps: 'w:<hex>' writes a chunk, 'e' and 'e:<hex>' end. Each step's
// string is printed as UTF-16 code units, four hex digits each.
const { StringDecoder } = require('string_decoder');
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const unhex = h => h === '-' ? Buffer.alloc(0) : Buffer.from(h, 'hex');
const units = s => s.length === 0 ? '-' : Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16).padStart(4, '0')).join('');
const out = [];
for (const l of lines) {
  const [name, ...steps] = l.split(' ');
  let d;
  try {
    d = new StringDecoder(name === '~' ? undefined : unhex(name).toString('utf8'));
  } catch (e) {
    out.push('E:' + Buffer.from(e.message, 'utf8').toString('hex'));
    continue;
  }
  const got = [];
  for (const s of steps) {
    if (s.startsWith('w:')) got.push(units(d.write(unhex(s.slice(2)))));
    else if (s === 'e') got.push(units(d.end()));
    else got.push(units(d.end(unhex(s.slice(2)))));
  }
  out.push(got.join('|'));
}
process.stdout.write(out.join('\n') + '\n');
