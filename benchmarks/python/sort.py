n = 400000
xs = []
seed = 12345
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    xs.append(seed % 1000000)
xs.sort()
checksum = 0
for k, v in enumerate(xs):
    checksum = (checksum + v * (k % 7 + 1)) % 1000000007
print(n, checksum)
