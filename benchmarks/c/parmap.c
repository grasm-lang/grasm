// pthreads: the portable baseline. (OpenMP is not universally available.)
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <unistd.h>

#define N 200000
static long xs[N], out[N];

static long mix(long seed) {
    long acc = seed;
    for (int i = 0; i < 800; i++) acc = (acc * 1103515245L + 12345L) % 2147483648L;
    return acc % 1000;
}

typedef struct { long lo, hi; } Range;

static void *work(void *arg) {
    Range *r = arg;
    for (long i = r->lo; i < r->hi; i++) out[i] = mix(xs[i]);
    return NULL;
}

int main(void) {
    for (long i = 0; i < N; i++) xs[i] = i;
    long workers = sysconf(_SC_NPROCESSORS_ONLN);
    if (workers < 1) workers = 1;
    pthread_t *th = malloc(workers * sizeof(pthread_t));
    Range *rs = malloc(workers * sizeof(Range));
    long chunk = (N + workers - 1) / workers;
    for (long w = 0; w < workers; w++) {
        rs[w].lo = w * chunk;
        rs[w].hi = rs[w].lo + chunk > N ? N : rs[w].lo + chunk;
        if (rs[w].lo > N) rs[w].lo = rs[w].hi = N;
        pthread_create(&th[w], NULL, work, &rs[w]);
    }
    for (long w = 0; w < workers; w++) pthread_join(th[w], NULL);
    long total = 0;
    for (long i = 0; i < N; i++) total += out[i];
    printf("%ld\n", total);
    free(th); free(rs);
    return 0;
}
