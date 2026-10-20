// The oracle for jsstringmethods: Node 25's String.prototype methods on
// UTF-16 code units. Strings are 4 hex digits per code unit ('-' when
// empty, 'u' for undefined); numbers are decimal text, 'nan', 'inf',
// '-inf', or 'u' for undefined.
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const str = (h) => h === 'u' ? undefined : h === '-' ? '' : String.fromCharCode(...h.match(/.{4}/g).map((x) => parseInt(x, 16)));
const num = (t) => t === 'u' ? undefined : t === 'nan' ? NaN : t === 'inf' ? Infinity : t === '-inf' ? -Infinity : Number(t);
const hex = (s) => s.length === 0 ? '-' : Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16).padStart(4, '0')).join('');
const out = [];
for (const l of lines) {
  if (!l) continue;
  const f = l.split(' ');
  const s = str(f[1]);
  try {
    switch (f[0]) {
      case 'sp': { const r = s.split(str(f[2]), num(f[3])); out.push(r.length === 0 ? '~' : r.map(hex).join(',')); break; }
      case 'ps': out.push(hex(s.padStart(num(f[2]), str(f[3])))); break;
      case 'pe': out.push(hex(s.padEnd(num(f[2]), str(f[3])))); break;
      case 'at': { const r = s.at(num(f[2])); out.push(r === undefined ? 'u' : hex(r)); break; }
      case 'cp': { const r = s.codePointAt(num(f[2])); out.push(r === undefined ? 'u' : String(r)); break; }
      case 'wf': out.push(s.isWellFormed() + '|' + hex(s.toWellFormed())); break;
      case 'tm': out.push([s.trim(), s.trimStart(), s.trimEnd()].map(hex).join(',')); break;
      case 'sb': out.push(hex(s.substring(num(f[2]), num(f[3])))); break;
      case 'sr': out.push(hex(s.substr(num(f[2]), num(f[3])))); break;
      case 'sl': out.push(hex(s.slice(num(f[2]), num(f[3])))); break;
      case 'io': out.push(String(s.indexOf(str(f[2]), num(f[3])))); break;
      case 'li': out.push(String(s.lastIndexOf(str(f[2]), num(f[3])))); break;
      case 'ic': out.push(String(s.includes(str(f[2]), num(f[3])))); break;
      case 'sw': out.push(String(s.startsWith(str(f[2]), num(f[3])))); break;
      case 'ew': out.push(String(s.endsWith(str(f[2]), num(f[3])))); break;
      case 'rp': out.push(hex(s.repeat(num(f[2])))); break;
      case 're': out.push(hex(s.replace(str(f[2]), str(f[3])))); break;
      case 'ra': out.push(hex(s.replaceAll(str(f[2]), str(f[3])))); break;
      default: out.push('?');
    }
  } catch (e) {
    out.push('E:' + e.constructor.name + ':' + Buffer.from(e.message, 'utf8').toString('hex'));
  }
}
process.stdout.write(out.map((o) => o + '\n').join(''));
