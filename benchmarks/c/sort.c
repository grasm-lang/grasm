#include <stdio.h>
#include <stdlib.h>
static int cmp(const void *a, const void *b) {
    long x = *(const long *)a, y = *(const long *)b;
    return (x > y) - (x < y);
}
int main(void) {
    long n = 400000;
    long *xs = malloc(n * sizeof(long));
    long seed = 12345;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        xs[i] = seed % 1000000;
    }
    qsort(xs, n, sizeof(long), cmp);
    long checksum = 0;
    for (long k = 0; k < n; k++) checksum = (checksum + xs[k] * (k % 7 + 1)) % 1000000007L;
    printf("%ld %ld\n", n, checksum);
    free(xs);
    return 0;
}
