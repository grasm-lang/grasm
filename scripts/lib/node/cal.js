// The reference side of scripts/cal_diff.py: ICU's calendars through
// Intl.DateTimeFormat. Reads [[calendar, [day numbers]], ...] as JSON on
// stdin and writes, for each, the dates as "era|year|month|day".
'use strict'
const DAY = 86400000
const formats = {}
function fmt (cal) {
  if (!formats[cal]) {
    const opts = { timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric' }
    if (!['chinese', 'dangi'].includes(cal)) opts.era = cal === 'japanese' ? 'long' : 'short'
    formats[cal] = new Intl.DateTimeFormat('en-u-ca-' + cal, opts)
  }
  return formats[cal]
}
function one (cal, d) {
  const p = {}
  for (const x of fmt(cal).formatToParts(new Date(d * DAY))) p[x.type] = x.value
  if (cal === 'chinese' || cal === 'dangi') return `|${p.relatedYear}|${p.month}|${p.day}`
  return `${p.era || ''}|${p.year}|${p.month}|${p.day}`
}
let input = ''
process.stdin.on('data', c => { input += c })
process.stdin.on('end', () => {
  process.stdout.write(JSON.stringify(JSON.parse(input).map(([cal, days]) => days.map(d => one(cal, d)).join(','))))
})
