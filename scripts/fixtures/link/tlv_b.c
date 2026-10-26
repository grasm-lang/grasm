/* The other half of tlv_a.c: a thread-local variable another object uses,
 * and a second thread, whose copies must start from the template again. */
#include <pthread.h>
#include <stdio.h>

_Thread_local long other = 100;
long bump(void);

static void *worker(void *arg)
{
    long *out = arg;
    *out = bump();
    return 0;
}

int main(void)
{
    long x = bump();
    x = bump();
    long y = 0;
    pthread_t t;
    pthread_create(&t, 0, worker, &y);
    pthread_join(t, 0);
    printf("main %ld, thread %ld\n", x, y);
    return 0;
}
