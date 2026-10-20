// The oracle for packages/nodeparseargs: Node's own util.parseArgs.
// A line is: args (hex of UTF-8, comma separated, '~' none), options
// (name:type:short:multiple:default, '~' parts unset), strict,
// allowPositionals ('~' unset) and allowNegative. Strings print as UTF-16
// code units, four hex digits each.
const { parseArgs } = require('util');
const lines = require('fs').readFileSync(0, 'utf8').split('\n').filter(l => l.length);
const unhex = h => h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8');
const units = s => s.length === 0 ? '-' : Array.from({ length: s.length }, (_, i) => s.charCodeAt(i).toString(16).padStart(4, '0')).join('');
const val = v => typeof v === 'string' ? 's' + units(v) : typeof v === 'boolean' ? (v ? 'b1' : 'b0') : 'l[' + v.map(val).join(';') + ']';
const out = [];
for (const l of lines) {
  const [a, o, strict, ap, an] = l.split(' ');
  const args = a === '~' ? [] : a.split(',').map(unhex);
  const options = {};
  if (o !== '~') {
    for (const spec of o.split(',')) {
      const [name, type, short, multiple, def] = spec.split(':');
      const c = { type: type === 's' ? 'string' : 'boolean' };
      if (short !== '~') c.short = unhex(short);
      if (multiple === '1') c.multiple = true;
      if (def !== '~') {
        const one = x => c.type === 'string' ? unhex(x) : x === '1';
        c.default = c.multiple ? def.split('+').filter(x => x.length).map(one) : one(def);
      }
      options[unhex(name)] = c;
    }
  }
  const config = { args, options, strict: strict === '1', allowNegative: an === '1', tokens: true };
  if (ap !== '~') config.allowPositionals = ap === '1';
  try {
    const r = parseArgs(config);
    const values = Object.keys(r.values).map(k => units(k) + '=' + val(r.values[k])).join(',') || '~';
    const pos = r.positionals.map(units).join(',') || '~';
    const toks = r.tokens.map(t => [t.kind, t.index, t.name === undefined ? '~' : units(t.name), t.rawName === undefined ? '~' : units(t.rawName),
      t.value === undefined ? '~' : units(t.value), t.inlineValue === undefined ? '~' : (t.inlineValue ? '1' : '0')].join('|')).join(',') || '~';
    out.push(values + ' ' + pos + ' ' + toks);
  } catch (e) {
    out.push('E:' + e.code + ':' + units(e.message));
  }
}
process.stdout.write(out.join('\n') + '\n');
