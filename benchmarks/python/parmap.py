# The natural parallel idiom in CPython. The GIL makes threads useless for
# CPU-bound work, so multiprocessing is what people actually reach for.
from multiprocessing import Pool


def mix(seed):
    acc = seed
    for _ in range(800):
        acc = (acc * 1103515245 + 12345) % 2147483648
    return acc % 1000


if __name__ == "__main__":
    xs = list(range(200000))
    with Pool() as p:
        out = p.map(mix, xs, chunksize=8192)
    print(sum(out))
