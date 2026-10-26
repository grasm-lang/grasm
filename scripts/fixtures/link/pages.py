"""Writes `pages.c` for scripts/linktest.sh and scripts/linkdiff.sh: 4,000
functions of seven shapes -- leaf, frame with an array, calls, a static
helper, floating point, many locals -- all reached from `main` through a
table of their addresses.

The corpus's largest program fits its unwind information in one
second-level page; this needs four, so the multi-page layout of
`__unwind_info` is held to ld's. The table is 4,000 pointers, so the
chained fixups run across several pages of `__DATA_CONST` too.

    python3 pages.py > pages.c
"""

N = 4000


def main():
    out = ["#include <stdio.h>", "volatile long sink = 1;"]
    for i in range(N):
        k = i % 7
        if k == 0:
            out.append(f"long f{i}(long x) {{ return x * {i} + 1; }}")
        elif k == 1:
            n = i % 13 + 2
            out.append(f"long f{i}(long x) {{ long a[{n}]; for (int j = 0; j < {n}; j++) a[j] = x + j; sink = a[{n - 1}]; return a[0]; }}")
        elif k == 2:
            out.append(f"long f{i}(long x) {{ return x > 1000000 ? f{i - 1}(x) + printf(\"%ld\\n\", x) : x + {i}; }}")
        elif k == 3:
            out.append(f"long f{i}(long x) {{ long y = f{i - 2}(x) + f{i - 1}(x + 1); sink = y; return y; }}")
        elif k == 4:
            out.append(f"static long g{i}(long x) {{ return x ^ {i}; }} long f{i}(long x) {{ return g{i}(x) + f{i - 3}(x); }}")
        elif k == 5:
            out.append(f"long f{i}(long x) {{ double d = x * 1.5; for (int j = 0; j < {i % 5}; j++) d = d * 0.5 + j; return (long)d; }}")
        else:
            out.append(f"long f{i}(long x) {{ long a = x, b = x + 1, c = x + 2, d = x + 3; sink = a; sink = b; sink = c; sink = d; return f{i - 5}(a + b + c + d) & 0xffff; }}")
    out.append("long (*const table[])(long) = {")
    out.append(", ".join(f"f{i}" for i in range(N)))
    out.append("};")
    out.append("int main(void) {")
    out.append("    long t = 0;")
    out.append(f"    for (int i = 0; i < {N}; i++) t += table[i](i % 3) & 0xff;")
    out.append("    printf(\"%ld\\n\", t);")
    out.append("    return 0;")
    out.append("}")
    print("\n".join(out))


main()
