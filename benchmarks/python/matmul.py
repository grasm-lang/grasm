n = 200
a = [i % 7 for i in range(n * n)]
b = [i % 5 for i in range(n * n)]
c = [0] * (n * n)
for i in range(n):
    for k in range(n):
        aik = a[i * n + k]
        if aik:
            base_c = i * n
            base_b = k * n
            for j in range(n):
                c[base_c + j] += aik * b[base_b + j]
print(sum(c))
