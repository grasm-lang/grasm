# The natural parallel idiom in CPython. The GIL makes threads useless for
# CPU-bound work, so multiprocessing is what people actually reach for — and
# here every argument and every result has to be pickled across a pipe, which
# is what parallel *text* costs in this model.
from multiprocessing import Pool


def heavy(w):
    acc = w
    for i in range(200):
        acc += str(i % 7)
    return f"{len(acc)}-{w}"


if __name__ == "__main__":
    words = [f"w{i}" for i in range(20000)]
    with Pool() as p:
        out = p.map(heavy, words, chunksize=512)
    h = 0
    for s in out:
        for c in s:
            h = (h * 131 + ord(c)) % 1000000007
    print(h)
