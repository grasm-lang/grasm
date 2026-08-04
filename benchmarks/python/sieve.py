limit = 2000000
flags = [True] * (limit + 1)
count = 0
for i in range(2, limit + 1):
    if flags[i]:
        count += 1
        for j in range(i * i, limit + 1, i):
            flags[j] = False
print(count)
