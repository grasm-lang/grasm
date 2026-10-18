// The reference side of scripts/b18_diff.py for npm's own modules: ssri and
// validate-npm-package-name, loaded from the npm that ships with Node. Reads
// a JSON list of requests on stdin, writes a JSON list of answers.
'use strict'
const path = require('path')
const { execSync } = require('child_process')
const npmRoot = path.join(execSync('npm root -g').toString().trim(), 'npm', 'node_modules')
const ssri = require(path.join(npmRoot, 'ssri'))
const validate = require(path.join(npmRoot, 'validate-npm-package-name'))
const US = '\u001f'

const text = (h) => Buffer.from(h, 'hex').toString('utf8')

function summary (sri) {
  if (!sri) return 'null'
  const groups = Object.keys(sri).map(algo =>
    algo + '=' + sri[algo].map(h => h.digest + '?' + h.options.join('?')).join(','))
  return [sri.toString(), sri.toString({ strict: true }), sri.toString({ strict: true, sep: ',x\t' }),
    sri.pickAlgorithm() || '-', groups.join(';')].join(US)
}

function hashText (h) {
  return h ? h.toString() : 'false'
}

function answer (r) {
  switch (r.op) {
    case 'sri.parse':
      return summary(ssri.parse(text(r.a), { strict: r.b === 'strict' }))
    case 'sri.match': {
      const a = ssri.parse(text(r.a))
      const b = ssri.parse(text(r.b))
      return a && b ? hashText(a.match(b)) : 'false'
    }
    case 'sri.check':
      return hashText(ssri.checkData(Buffer.from(r.a, 'hex'), text(r.b)))
    case 'sri.from': {
      const [algos, options] = r.b.split(US)
      const got = ssri.fromData(Buffer.from(r.a, 'hex'), {
        algorithms: algos ? algos.split(',') : [],
        options: options ? options.split(',') : [],
      })
      return Object.keys(got).length ? got.toString() : 'null'
    }
    case 'npm.name': {
      const v = validate(text(r.a))
      return [v.validForNewPackages, v.validForOldPackages, (v.warnings || []).join(';'), (v.errors || []).join(';')].join(US)
    }
  }
  return 'UNKNOWN'
}

let input = ''
process.stdin.on('data', d => { input += d })
process.stdin.on('end', () => {
  process.stdout.write(JSON.stringify(JSON.parse(input).map(answer)))
})
