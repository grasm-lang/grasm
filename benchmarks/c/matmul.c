#include <stdio.h>
#include <stdlib.h>
int main(void) {
    long n = 200;
    long *a = malloc(n * n * sizeof(long));
    long *b = malloc(n * n * sizeof(long));
    long *c = calloc(n * n, sizeof(long));
    for (long i = 0; i < n * n; i++) { a[i] = i % 7; b[i] = i % 5; }
    for (long i = 0; i < n; i++)
        for (long k = 0; k < n; k++) {
            long aik = a[i * n + k];
            if (aik != 0)
                for (long j = 0; j < n; j++) c[i * n + j] += aik * b[k * n + j];
        }
    long total = 0;
    for (long i = 0; i < n * n; i++) total += c[i];
    printf("%ld\n", total);
    free(a); free(b); free(c);
    return 0;
}
