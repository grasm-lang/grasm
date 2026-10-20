// The oracle for nodelegacyurl: Node 25's legacy url.parse, url.format
// and url.resolve. Strings are 4 hex digits per UTF-16 unit ('-' when
// empty); 'n' is null, 'u' undefined.
const url = require('url');
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const str = (h) => h === '-' ? '' : String.fromCharCode(...h.match(/.{4}/g).map((x) => parseInt(x, 16)));
const hex = (s) => s.length === 0 ? '-' : Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16).padStart(4, '0')).join('');
const field = (v) => v === null ? 'n' : v === undefined ? 'u' : v === true ? 't' : v === false ? 'f' : typeof v === 'string' ? hex(v) :
  'o:' + Object.keys(v).map((k) => hex(k) + '=' + (Array.isArray(v[k]) ? 'a' + v[k].map(hex).join('.') : 's' + hex(v[k]))).join(',');
const arg = (t) => t === 'n' ? null : t === 'u' ? undefined : t === 't' ? true : t === 'f' ? false : t.startsWith('o:') ?
  Object.fromEntries(t.slice(2).split(',').filter((x) => x).map((kv) => kv.split('=')).map(([k, v]) => [str(k), str(v)])) : str(t);
const keys = ['protocol', 'slashes', 'auth', 'host', 'port', 'hostname', 'hash', 'search', 'query', 'pathname', 'path', 'href'];
const out = [];
for (const l of lines) {
  if (!l) continue;
  const f = l.split(' ');
  try {
    if (f[0] === 'pa') {
      const u = url.parse(str(f[1]), f[2] === '1', f[3] === '1');
      out.push(keys.map((k) => field(u[k])).join('|'));
    } else if (f[0] === 'rs') {
      out.push(hex(url.resolve(str(f[1]), str(f[2]))));
    } else {
      const o = {};
      ['protocol', 'slashes', 'auth', 'host', 'port', 'hostname', 'hash', 'search', 'query', 'pathname'].forEach((k, i) => {
        if (f[i + 1] !== 'u') o[k] = arg(f[i + 1]);
      });
      out.push(hex(url.format(o)));
    }
  } catch (e) {
    out.push('E:' + e.constructor.name + ':' + e.code + ':' + hex(e.message));
  }
}
process.stdout.write(out.map((o) => o + '\n').join(''));
