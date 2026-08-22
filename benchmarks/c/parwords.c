// pthreads: the portable baseline. (OpenMP is not universally available.)
//
// The string handling is by hand, which is the honest comparison: this is what
// parallel text costs in C, including the allocation and the freeing.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <unistd.h>

#define N 20000

static char *words[N];
static char *out[N];

static char *heavy(const char *w) {
    size_t wl = strlen(w);
    size_t cap = wl + 200 + 1;
    char *acc = malloc(cap);
    memcpy(acc, w, wl);
    size_t len = wl;
    for (int i = 0; i < 200; i++) acc[len++] = (char)('0' + (i % 7));
    acc[len] = '\0';
    char *res = malloc(32 + wl);
    sprintf(res, "%zu-%s", len, w);
    free(acc);
    return res;
}

typedef struct { long lo, hi; } Range;

static void *work(void *arg) {
    Range *r = arg;
    for (long i = r->lo; i < r->hi; i++) out[i] = heavy(words[i]);
    return NULL;
}

int main(void) {
    for (long i = 0; i < N; i++) {
        words[i] = malloc(16);
        sprintf(words[i], "w%ld", i);
    }
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
    long h = 0;
    for (long i = 0; i < N; i++) {
        for (const unsigned char *p = (const unsigned char *)out[i]; *p; p++) {
            h = (h * 131 + *p) % 1000000007L;
        }
    }
    printf("%ld\n", h);
    for (long i = 0; i < N; i++) { free(words[i]); free(out[i]); }
    free(th); free(rs);
    return 0;
}
