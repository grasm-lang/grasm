import std/tables, std/strutils
const words = ["alpha","beta","gamma","delta","epsilon","zeta","eta","theta"]
var counts = initTable[string, int]()
for i in 0 ..< 400_000:
  let key = words[i mod 8] & $(i mod 97)
  counts.mgetOrPut(key, 0) += 1
var total = 0
for v in counts.values: total += v
echo counts.len, " ", total
