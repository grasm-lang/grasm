const n = 200
var a = newSeq[int](n * n)
var b = newSeq[int](n * n)
var c = newSeq[int](n * n)
for i in 0 ..< n * n:
  a[i] = i mod 7
  b[i] = i mod 5
for i in 0 ..< n:
  for k in 0 ..< n:
    let aik = a[i * n + k]
    if aik != 0:
      for j in 0 ..< n:
        c[i * n + j] += aik * b[k * n + j]
var total = 0
for v in c: total += v
echo total
