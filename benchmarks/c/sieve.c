#include <stdio.h>
#include <stdlib.h>
int main(void) {
    long limit = 2000000;
    char *flags = malloc(limit + 1);
    for (long i = 0; i <= limit; i++) flags[i] = 1;
    long count = 0;
    for (long i = 2; i <= limit; i++) {
        if (flags[i]) {
            count++;
            for (long j = i * i; j <= limit; j += i) flags[j] = 0;
        }
    }
    printf("%ld\n", count);
    free(flags);
    return 0;
}
