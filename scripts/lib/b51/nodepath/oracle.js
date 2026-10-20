// The oracle for packages/nodepath: Node's own path.posix. Arguments are
// hex UTF-8, '-' for empty, lists joined by ','; process.cwd() is replaced
// by the cwd each line gives.
const path = require('path').posix;
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const unhex = h => h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8');
const hx = s => Buffer.from(s, 'utf8').toString('hex') || '-';
const list = h => h === '~' ? [] : h.split(',').map(unhex);
const out = [];
for (const l of lines) {
  const f = l.split(' ');
  const cwd = unhex(f[1]);
  process.cwd = () => cwd;
  let r;
  switch (f[0]) {
    case 'no': r = hx(path.normalize(unhex(f[2]))); break;
    case 'jo': r = hx(path.join(...list(f[2]))); break;
    case 're': r = hx(path.resolve(...list(f[2]))); break;
    case 'rl': r = hx(path.relative(unhex(f[2]), unhex(f[3]))); break;
    case 'dn': r = hx(path.dirname(unhex(f[2]))); break;
    case 'bn': r = hx(f[3] === '~' ? path.basename(unhex(f[2])) : path.basename(unhex(f[2]), unhex(f[3]))); break;
    case 'ex': r = hx(path.extname(unhex(f[2]))); break;
    case 'ab': r = String(path.isAbsolute(unhex(f[2]))); break;
    case 'pa': { const p = path.parse(unhex(f[2])); r = [p.root, p.dir, p.base, p.ext, p.name].map(hx).join(','); break; }
    default: { const [root, dir, base, ext, name] = list(f[2]); r = hx(path.format({ root, dir, base, ext, name })); }
  }
  out.push(r);
}
process.stdout.write(out.join('\n') + '\n');
