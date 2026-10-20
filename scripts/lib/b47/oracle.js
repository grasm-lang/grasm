// The oracle for packages/httpcachesem: npm's own http-cache-semantics.
// Reads case files named on stdin (one path a line) and prints one hex line
// per case. `now()` is the clock the case gives, through a subclass, so the
// library itself is unmodified.
'use strict';
const NPM = process.argv[2];
const CachePolicy = require(NPM + '/http-cache-semantics');
const fs = require('fs');
let NOW = 0;
class Policy extends CachePolicy { now() { return NOW; } }

const unhex = h => h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8');
const hex = s => Buffer.from(s, 'utf8').toString('hex') || '-';
const bits = x => { const b = Buffer.alloc(8); b.writeDoubleBE(x); return b.toString('hex'); };
const enc = h => Object.keys(h).filter(k => h[k] !== undefined).map(k => hex(k) + '=' + hex(String(h[k]))).join(',') || '-';

function parse(text) {
  const c = { reqh: {}, resh: {}, qh: {}, r2h: {} };
  for (const line of text.split('\n')) {
    const f = line.split(' ');
    switch (f[0]) {
      case 'OPT': c.opts = { shared: f[1] === '1', cacheHeuristic: parseFloat(f[2]), ignoreCargoCult: f[3] === '1' };
        if (f[4] !== '-') c.opts.immutableMinTimeToLive = parseFloat(f[4]); break;
      case 'REQ': c.req = {}; if (f[1] !== '_') c.req.method = unhex(f[1]); if (f[2] !== '_') c.req.url = unhex(f[2]); break;
      case 'RQH': c.reqh[unhex(f[1])] = unhex(f[2]); break;
      case 'RES': c.res = {}; if (f[1] !== '_') c.res.status = parseInt(f[1], 10); break;
      case 'RSH': c.resh[unhex(f[1])] = unhex(f[2]); break;
      case 'Q': c.q = {}; if (f[1] !== '_') c.q.method = unhex(f[1]); if (f[2] !== '_') c.q.url = unhex(f[2]); break;
      case 'QH': c.qh[unhex(f[1])] = unhex(f[2]); break;
      case 'R2': c.r2 = f[1] === 'none' ? undefined : {}; if (f[1] !== 'none' && f[1] !== '_') c.r2.status = parseInt(f[1], 10); break;
      case 'R2H': c.r2h[unhex(f[1])] = unhex(f[2]); break;
      case 'T': c.t = f.slice(1).map(Number); break;
    }
  }
  c.req.headers = c.reqh; c.res.headers = c.resh; c.q.headers = c.qh;
  if (c.r2) c.r2.headers = c.r2h;
  return c;
}

function run(c) {
  const out = [];
  NOW = c.t[0];
  const p = new Policy(c.req, c.res, c.opts);
  NOW = c.t[1];
  out.push('S ' + (p.storable() ? 1 : 0));
  out.push('M ' + bits(p.maxAge()));
  out.push('A ' + bits(p.age()));
  out.push('L ' + String(p.timeToLive()));
  out.push('X ' + (p.stale() ? 1 : 0));
  const ev = p.evaluateRequest(c.q);
  out.push('E ' + (ev.response ? enc(ev.response.headers) : '_') + ' ' +
    (ev.revalidation ? (ev.revalidation.synchronous ? 1 : 0) + ' ' + enc(ev.revalidation.headers) : '_'));
  out.push('V ' + enc(p.revalidationHeaders(c.q)));
  NOW = c.t[2];
  try {
    const r = p.revalidatedPolicy(c.q, c.r2);
    out.push('P ' + (r.modified ? 1 : 0) + ' ' + (r.matches ? 1 : 0) + ' ' + (r.policy.storable() ? 1 : 0) + ' ' +
      String(r.policy.timeToLive()) + ' ' + enc(r.policy.responseHeaders()));
  } catch (e) {
    out.push('P error');
  }
  return out.join('\n');
}

const paths = fs.readFileSync(0, 'utf8').split('\n').filter(Boolean);
for (const path of paths) {
  let line;
  try { line = hex(run(parse(fs.readFileSync(path, 'utf8')))); } catch (e) { line = 'ERR ' + hex(String(e.message)); }
  console.log(line);
}
