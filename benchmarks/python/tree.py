# Allocation-shaped work: build a tree, walk it, drop it, repeat.
import sys


class Node:
    __slots__ = ("value", "kids")

    def __init__(self, value, kids):
        self.value = value
        self.kids = kids


def build(depth, branch, seed):
    if depth <= 0:
        return Node(seed % 1000, [])
    kids = [build(depth - 1, branch, seed * 31 + i + 1) for i in range(branch)]
    return Node(seed % 1000, kids)


def total(n):
    return n.value + sum(total(k) for k in n.kids)


def main():
    sys.setrecursionlimit(10000)
    acc = 0
    for r in range(20):
        t = build(9, 3, r + 1)
        acc = (acc + total(t)) % 1000000007
    print(acc)


main()
