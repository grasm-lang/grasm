const limit = 2_000_000
var flags = newSeq[bool](limit + 1)
for i in 0 .. limit: flags[i] = true
var count = 0
for i in 2 .. limit:
  if flags[i]:
    inc count
    var j = i * i
    while j <= limit:
      flags[j] = false
      j += i
echo count
