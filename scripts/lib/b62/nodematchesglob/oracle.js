// The oracle for nodematchesglob: Node 25's path.posix.matchesGlob and
// path.win32.matchesGlob as a macOS host runs them (nocase on), and the
// same minimatch call with nocase off, which is what Node passes on Linux
// (lib/internal/fs/glob.js matchGlobPattern). Run with --expose-internals.
// Lines: op path pattern, both hex of UTF-8 ('-' when empty).
'use strict';
const path = require('path');
const { minimatch } = require('internal/deps/minimatch/index');
const linux = (platform) => ({ nocase: false, windowsPathsNoEscape: true, nonegate: true, nocomment: true, optimizationLevel: 2, platform, nocaseMagicOnly: true });
const unhex = (h) => (h === '-' ? '' : Buffer.from(h, 'hex').toString('utf8'));
const hx = (s) => (s === '' ? '-' : Buffer.from(s, 'utf8').toString('hex'));
const out = [];
for (const l of require('fs').readFileSync(0, 'utf8').split('\n')) {
  if (!l) continue;
  const [op, ph, gh] = l.split(' ');
  const p = unhex(ph);
  const g = unhex(gh);
  try {
    const r = op === 'px' ? path.posix.matchesGlob(p, g) : op === 'wx' ? path.win32.matchesGlob(p, g)
      : minimatch(p, g, linux(op === 'pl' ? 'posix' : 'win32'));
    out.push(r ? '1' : '0');
  } catch (e) {
    out.push('E:' + e.name + ':' + hx(e.message));
  }
}
process.stdout.write(out.map((o) => o + '\n').join(''));
