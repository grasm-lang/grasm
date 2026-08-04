words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]
counts = {}
for i in range(400000):
    w = words[i % 8] + str(i % 97)
    counts[w] = counts.get(w, 0) + 1
print(len(counts), sum(counts.values()))
