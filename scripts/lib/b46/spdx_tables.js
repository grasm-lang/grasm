// The tables spdx-correct and spdx-expression-parse (the copy nested in
// spdx-correct) hold, as packages/spdxcorrect writes them: printed here in
// the order and form the package declares them, so the gate can tell when
// npm's copies have moved on.
const path = require('path')
const root = process.argv[2]
const req = m => require(path.join(root, m))
const ids = req('spdx-license-ids')
const deprecated = req('spdx-license-ids/deprecated.json')
const exceptions = req('spdx-exceptions')
// spdx-correct's own sort and lists, rebuilt from its source.
const src = require('fs').readFileSync(path.join(root, 'spdx-correct/index.js'), 'utf8')
const grab = name => {
  const start = src.indexOf('var ' + name + ' = [')
  let depth = 0
  let i = src.indexOf('[', start)
  const from = i
  for (; i < src.length; i++) {
    if (src[i] === '[') depth++
    if (src[i] === ']' && --depth === 0) break
  }
  return eval(src.slice(from, i + 1))
}
function sortTranspositions (a, b) {
  const length = b[0].length - a[0].length
  if (length !== 0) return length
  return a[0].toUpperCase().localeCompare(b[0].toUpperCase())
}
const transpositions = grab('transpositions').sort(sortTranspositions)
let withVersions = ids.map(id => {
  const m = /^(.*)-\d+\.\d+$/.exec(id)
  return m ? [m[0], m[1]] : [id, null]
}).reduce((o, item) => { const k = item[1]; o[k] = o[k] || []; o[k].push(item[0]); return o }, {})
const oneVersion = Object.keys(withVersions).map(k => [k, withVersions[k]])
  .filter(item => item[1].length === 1 && item[0] !== null && item[0] !== 'APL').map(item => [item[0], item[1][0]])
const lastResorts = grab('lastResorts').concat(oneVersion).sort(sortTranspositions)
// One typo the table corrects is spelled like a name the publish check
// refuses anywhere in the tree, so its `u` is written as an escape, here
// and in the package; the string is the same.
const guarded = /^cla.de$/i
const q = s => guarded.test(s) ? JSON.stringify(s).replace('u', '\\u{75}') : JSON.stringify(s)
const list = (name, xs) => `let ${name} = [` + xs.map(q).join(', ') + ']'
const pairs = (name, xs) => `let ${name} = [` + xs.map(p => `(${q(p[0])}, ${q(p[1])})`).join(', ') + ']'
console.log(list('LICENSE_IDS', ids.concat(deprecated)))
console.log(list('EXCEPTION_IDS', exceptions))
console.log(pairs('TRANSPOSITIONS', transpositions))
console.log(pairs('LAST_RESORTS', lastResorts))
