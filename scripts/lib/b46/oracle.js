// The npm half of scripts/b46_diff.py: node oracle.js <npm's node_modules>
// <op> <case file>...; one line per case file, each input's answer
// hex-encoded and joined by ';', as packages/b46test answers.
const root = process.argv[2]
const op = process.argv[3]
const fs = require('fs')
const un = h => h === '-' ? '' : Buffer.from(h, 'hex').toString()
const opt = h => h === '_' ? undefined : un(h)
const hx = s => Buffer.from(s).toString('hex')
const brace = require(root + '/brace-expansion')
const ini = require(root + '/ini')
const correct = require(root + '/spdx-correct')
const parse = require(root + '/spdx-correct/node_modules/spdx-expression-parse')
const checks = require(root + '/npm-install-checks')
const pick = require(root + '/npm-pick-manifest')

function answer (f) {
  if (op === 'brace') return brace(un(f[0])).map(hx).join(',')
  if (op === 'ini') {
    const t = un(f[1])
    try {
      if (f[0] === 'dec') return JSON.stringify(ini.decode(t))
      if (f[0] === 'decd') return JSON.stringify(ini.decode(t, { bracketedArray: false }))
      if (f[0] === 'safe') return ini.safe(t)
      if (f[0] === 'unsafe') return JSON.stringify(ini.unsafe(t))
      const o = f[2].split(',')
      const opts = { align: o[1] === '1', newline: o[2] === '1', sort: o[3] === '1', whitespace: o[4] === '1', platform: o[5] === '1' ? 'win32' : 'linux', bracketedArray: o[6] === '1' }
      if (o[0] !== '-') opts.section = un(o[0])
      return ini.encode(JSON.parse(t), opts)
    } catch (e) { return 'ERR' }
  }
  if (op === 'spdx') {
    const s = un(f[1])
    if (f[0] === 'valid') { try { parse(s); return 'T' } catch (e) { return 'F' } }
    try { const r = correct(s, { upgrade: f[0] === 'fix' }); return r === null ? 'NULL' : r } catch (e) { return 'ERR' }
  }
  if (op === 'install') {
    try {
      const v = JSON.parse(un(f[1]))
      if (f[0] === 'engine') { checks.checkEngine(v, opt(f[2]), opt(f[3]), f[4] === '1'); return 'OK' }
      if (f[0] === 'platform') {
        const env = { os: un(f[3]), cpu: un(f[4]) }
        const lc = opt(f[5])
        if (lc !== undefined) env.libc = lc
        else if (env.os !== 'linux') env.libc = undefined
        checks.checkPlatform(v, f[2] === '1', env)
        return 'OK'
      }
      const errs = checks.checkDevEngines(v, JSON.parse(un(f[2])), { semver: { loose: f[3] === '1', includePrerelease: f[4] === '1' } })
      return '[' + errs.map(e => e.engine + '|' + (e.isWarn ? 'w' : '-') + (e.isError ? 'e' : '-') + '|' + e.errors.map(x => x.message).join(';')).join('#') + ']'
    } catch (e) { return e.code === 'EBADENGINE' || e.code === 'EBADPLATFORM' ? e.code : 'THROW:' + e.message }
  }
  if (op === 'pick') {
    try {
      const o = { nodeVersion: un(f[5]), includeStaged: f[7] === '1', avoidStrict: f[9] === '1' }
      if (opt(f[3]) !== undefined) o.defaultTag = opt(f[3])
      if (opt(f[4]) !== undefined) o.before = opt(f[4])
      if (opt(f[6]) !== undefined) o.npmVersion = opt(f[6])
      if (opt(f[8]) !== undefined) o.avoid = opt(f[8])
      const m = pick(JSON.parse(un(f[1])), opt(f[2]), o)
      return (typeof m.version === 'string' ? m.version : '-') + '|' + (m._shouldAvoid ? 'A' : '-') + (m._outsideDependencyRange ? 'O' : '-') +
        (m._isSemVerMajor === true ? 'M' : m._isSemVerMajor === false ? 'm' : '-') + '|' + JSON.stringify(m.bin ?? null)
    } catch (e) {
      return e.code ? 'THROW:' + e.code + (/No avoidable/.test(e.message) ? ':avoid' : '') : (/Only tag/.test(e.message) ? 'THROW:unsupported' : 'THROW:?' + e.message)
    }
  }
  throw new Error('unknown op ' + op)
}

for (const file of process.argv.slice(4)) {
  const lines = fs.readFileSync(file, 'utf8').split('\n').filter(l => l)
  console.log(lines.map(l => hx(answer(l.split(' ')))).join(';'))
}
