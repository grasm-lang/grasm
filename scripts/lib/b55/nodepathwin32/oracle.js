// The oracle for packages/nodepathwin32: Node 25's path.win32, with
// process.cwd() answering the working directory each line gives. Text
// travels as hex of UTF-8 ("-" when empty); answers likewise (a lone
// surrogate goes out as U+FFFD, which is what Buffer does with one).
const w = require('path').win32;
const unhex = (h) => (h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8'));
const hex = (s) => Buffer.from(s, 'utf8').toString('hex') || '-';
let cwd = '/';
process.cwd = () => cwd;
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter((l) => l.length > 0);
const out = [];
for (const l of lines) {
  const f = l.split(' ');
  const a = f.slice(1).map(unhex);
  let r;
  switch (f[0]) {
    case 'norm': r = hex(w.normalize(a[0])); break;
    case 'abs': r = String(w.isAbsolute(a[0])); break;
    case 'join': r = hex(w.join(...a)); break;
    case 'res': cwd = a[0]; r = hex(w.resolve(...a.slice(1))); break;
    case 'rel': cwd = a[0]; r = hex(w.relative(a[1], a[2])); break;
    case 'ns': cwd = a[0]; r = hex(w.toNamespacedPath(a[1])); break;
    case 'dir': r = hex(w.dirname(a[0])); break;
    case 'base': r = hex(f.length > 2 ? w.basename(a[0], a[1]) : w.basename(a[0])); break;
    case 'ext': r = hex(w.extname(a[0])); break;
    case 'parse': { const p = w.parse(a[0]); r = [p.root, p.dir, p.base, p.ext, p.name].map(hex).join('.'); break; }
    default: { const o = {}; ['root', 'dir', 'base', 'ext', 'name'].forEach((k, i) => { if (a[i] !== '') o[k] = a[i]; }); r = hex(w.format(o)); }
  }
  out.push(r);
}
process.stdout.write(out.map((x) => x + '\n').join(''));
