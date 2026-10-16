#!/usr/bin/env python3
"""Compares `packages/xxhash` against the reference library.

    python3 scripts/lib/xxhashcheck.py < lines

Each line is `length seed xxh32 xxh64`. The input bytes are rebuilt from the
length by the same rule the demo uses, so only the digests are carried
across and there is no fixture to drift.

A u64 above 2^63 comes back from Grasm as a negative `i64` holding the same
bits, which is the convention `packages/siphash` set. Converted here rather
than in Grasm, because printing an unsigned decimal would mean implementing
unsigned division to check a hash.
"""

import sys

import xxhash


def signed64(v):
    return v - (1 << 64) if v >= (1 << 63) else v


def main():
    bad = 0
    checked = 0
    for line in sys.stdin:
        parts = line.split()
        if len(parts) != 4:
            continue
        n, seed, got32, got64 = (int(x) for x in parts)
        data = bytes((i * 37 + 11) % 256 for i in range(n))
        want32 = xxhash.xxh32(data, seed=seed).intdigest()
        want64 = signed64(xxhash.xxh64(data, seed=seed).intdigest())
        if got32 != want32:
            bad += 1
            if bad <= 5:
                print(f"    XXH32 len {n} seed {seed}: {got32}, wanted {want32}")
        if got64 != want64:
            bad += 1
            if bad <= 5:
                print(f"    XXH64 len {n} seed {seed}: {got64}, wanted {want64}")
        checked += 1
    if checked < 100:
        print(f"    only {checked} digests were checked; the demo printed too few")
        return 1
    print(f"    digests: {checked * 2} across {checked} length-and-seed pairs, "
          f"exact against the reference")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
