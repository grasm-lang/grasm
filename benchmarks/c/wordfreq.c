// A small open-addressing table: the point is to measure hashing and string
// work, so using a hand-rolled map keeps this comparable to the others rather
// than measuring a library nobody else here has.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CAP 4096
typedef struct { char *key; long count; } Entry;
static Entry table[CAP];
static long distinct = 0;

static unsigned long hash(const char *s) {
    unsigned long h = 1469598103934665603UL;
    while (*s) { h ^= (unsigned char)*s++; h *= 1099511628211UL; }
    return h;
}

static void bump(const char *key) {
    unsigned long i = hash(key) % CAP;
    while (table[i].key) {
        if (strcmp(table[i].key, key) == 0) { table[i].count++; return; }
        i = (i + 1) % CAP;
    }
    table[i].key = strdup(key);
    table[i].count = 1;
    distinct++;
}

int main(void) {
    const char *words[8] = {"alpha","beta","gamma","delta","epsilon","zeta","eta","theta"};
    char buf[64];
    for (long i = 0; i < 400000; i++) {
        snprintf(buf, sizeof buf, "%s%ld", words[i % 8], i % 97);
        bump(buf);
    }
    long total = 0;
    for (long i = 0; i < CAP; i++) if (table[i].key) total += table[i].count;
    printf("%ld %ld\n", distinct, total);
    return 0;
}
