// The oracle for jsdate: Node 25's Date with TZ=UTC. Lines are
// "pa <string>" (Date.parse), "ut <args...>" (Date.UTC) or "fm <number>"
// (new Date(number) and its formats and UTC getters). Strings are 4 hex
// digits per UTF-16 unit ('-' when empty); numbers are decimal text,
// 'nan', 'inf' or '-inf'.
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const str = (h) => h === '-' ? '' : String.fromCharCode(...h.match(/.{4}/g).map((x) => parseInt(x, 16)));
const num = (t) => t === 'nan' ? NaN : t === 'inf' ? Infinity : t === '-inf' ? -Infinity : Number(t);
const show = (x) => Number.isNaN(x) ? 'NaN' : String(x);
const out = [];
for (const l of lines) {
  if (!l) continue;
  const f = l.split(' ');
  if (f[0] === 'pa') {
    out.push(show(Date.parse(str(f[1]))));
  } else if (f[0] === 'ut') {
    out.push(show(Date.UTC(...f.slice(1).map(num))));
  } else {
    const d = new Date(num(f[1]));
    let iso;
    try { iso = d.toISOString(); } catch (e) { iso = 'E:' + e.constructor.name + ':' + e.message; }
    const getters = ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes', 'Seconds', 'Milliseconds'].map((g) => show(d['getUTC' + g]()));
    out.push([show(d.getTime()), iso, d.toUTCString(), d.toString(), d.toDateString(), d.toTimeString(), getters.join(',')].join('|'));
  }
}
process.stdout.write(out.map((o) => o + '\n').join(''));
