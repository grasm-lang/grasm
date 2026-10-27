/* Drives the runtime dialect's fixture (src/main.gr, compiled by the Grasm
 * backend with `--runtime`) for scripts/rtdialect.sh. Each check prints one
 * line, "ok <what> <count>" or "FAIL <what>: <why>", and the gate requires
 * every line it expects to say ok with the counts it expects.
 *
 * C because the checks need what only C has to hand: threads that share
 * the fixture's process cells, stacks mapped with a guard page for its
 * fibers, and a child process to show static bytes are read-only. */
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

extern int64_t fx_cells_zero(void);
extern int64_t fx_cells_apart(void);
extern int64_t fx_hit(void);
extern int64_t fx_hits(void);
extern int64_t fx_swap_byte(int64_t v);
extern const char *fx_static(int64_t k);
extern int64_t fx_static_len(int64_t k);
extern int64_t fx_mine_add(int64_t n);
extern void *fx_mine_addr(void);
extern int64_t fx_small_add(int64_t n);
extern void fx_switch(void *from, void *to);
extern void fx_init(void *ctx, void *top, void *entry, void *arg);
extern void fx_fiber(void *rec);

static int failures = 0;

#define FAIL(...)                                                             \
    do {                                                                      \
        printf("FAIL " __VA_ARGS__);                                          \
        printf("\n");                                                         \
        failures++;                                                           \
    } while (0)

/* ----- process cells ----- */

static void check_cells(void)
{
    if (fx_cells_zero() != 1) {
        FAIL("cells: a process cell is not zero at start");
        return;
    }
    int distinct = (int)fx_cells_apart();
    if (distinct != 6) {
        FAIL("cells: %d of 6 cells are 8-aligned, apart from the others and fixed", distinct);
        return;
    }
    /* Writable, and what one writes the next read sees. */
    int64_t a = fx_swap_byte(200), b = fx_swap_byte(7), c = fx_swap_byte(0);
    if (a != 0 || b != 200 || c != 7) {
        FAIL("cells: the byte cell answered %lld %lld %lld, not 0 200 7", (long long)a, (long long)b,
             (long long)c);
        return;
    }
    printf("ok cells %d\n", distinct);
}

#define THREADS 6
#define HITS 100000

static void *hitter(void *arg)
{
    (void)arg;
    for (int i = 0; i < HITS; i++) {
        fx_hit();
    }
    return 0;
}

static void check_shared(void)
{
    pthread_t t[THREADS];
    for (int i = 0; i < THREADS; i++) {
        pthread_create(&t[i], 0, hitter, 0);
    }
    for (int i = 0; i < THREADS; i++) {
        pthread_join(t[i], 0);
    }
    int64_t n = fx_hits();
    if (n != (int64_t)THREADS * HITS) {
        FAIL("shared: %d threads of %d hits made %lld", THREADS, HITS, (long long)n);
        return;
    }
    printf("ok shared %lld\n", (long long)n);
}

/* ----- static bytes ----- */

static const char *const TEXTS[5] = {
    "out of memory",
    "",
    "tab\there \"quoted\" back\\slash\n",
    "\xc3\xa9 \xe2\x9c\x93 \xf0\x9d\x84\x9e",
    "nul\0inside",
};
static const int64_t LENS[5] = {13, 0, 29, 11, 10};

static void check_static(void)
{
    int same = 0;
    for (int k = 0; k < 5; k++) {
        const char *p = fx_static(k);
        int64_t n = fx_static_len(k);
        if (n != LENS[k]) {
            FAIL("static: literal %d is %lld bytes, not %lld", k, (long long)n, (long long)LENS[k]);
            return;
        }
        if (memcmp(p, TEXTS[k], (size_t)n) != 0) {
            FAIL("static: literal %d's bytes differ", k);
            return;
        }
        if (p[n] != 0) {
            FAIL("static: literal %d does not end in a NUL", k);
            return;
        }
        same++;
    }
    if (memcmp(fx_static(5), TEXTS[0], 14) != 0 || fx_static_len(5) != 13) {
        FAIL("static: the literal written twice differs");
        return;
    }
    /* Read-only: a write through one faults, in a child. */
    pid_t pid = fork();
    if (pid == 0) {
        volatile char *w = (volatile char *)fx_static(0);
        w[0] = 'X';
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    if (!WIFSIGNALED(st)) {
        FAIL("static: a write to static bytes did not fault (status %d)", st);
        return;
    }
    printf("ok static %d\n", same);
}

/* ----- the runtime's thread-local cells ----- */

struct mine {
    int64_t first, second, small;
    void *addr;
};

static void *mine_thread(void *arg)
{
    struct mine *m = arg;
    m->first = fx_mine_add(7);
    m->second = fx_mine_add(7);
    m->small = fx_small_add(65535);
    m->addr = fx_mine_addr();
    return 0;
}

static void check_tls(void)
{
    if (fx_mine_add(5) != 5 || fx_small_add(3) != 3) {
        FAIL("tls: the main thread's cells did not start at zero");
        return;
    }
    struct mine m[4];
    pthread_t t[4];
    for (int i = 0; i < 4; i++) {
        pthread_create(&t[i], 0, mine_thread, &m[i]);
    }
    for (int i = 0; i < 4; i++) {
        pthread_join(t[i], 0);
    }
    for (int i = 0; i < 4; i++) {
        if (m[i].first != 7 || m[i].second != 14 || m[i].small != 65535) {
            FAIL("tls: thread %d saw %lld %lld %lld, not 7 14 65535", i, (long long)m[i].first,
                 (long long)m[i].second, (long long)m[i].small);
            return;
        }
        if (m[i].addr == fx_mine_addr()) {
            FAIL("tls: thread %d's cell is the main thread's", i);
            return;
        }
    }
    if (fx_mine_add(1) != 6 || fx_small_add(65535) != 2) {
        FAIL("tls: another thread changed the main thread's cells");
        return;
    }
    printf("ok tls %d\n", 4);
}

/* ----- the context switch ----- */

#define CTX_BYTES 256
#define STACK (256 * 1024)
#define FIBERS 3
#define ROUNDS 200

struct rec {
    void *self, *back, *log, *len;
    int64_t id, rounds, total;
};

static int64_t model_total(int64_t id)
{
    int64_t acc = id;
    for (int64_t i = 0; i < ROUNDS; i++) {
        acc = (acc * 3 + i) % 1000003;
    }
    return acc;
}

static void check_switch(void)
{
    static _Alignas(16) unsigned char main_ctx[CTX_BYTES];
    static _Alignas(16) unsigned char ctx[FIBERS][CTX_BYTES];
    static int64_t log[FIBERS * (ROUNDS + 1) + 8];
    static int64_t len = 0;
    struct rec r[FIBERS];
    for (int f = 0; f < FIBERS; f++) {
        size_t page = (size_t)getpagesize();
        unsigned char *s = mmap(0, STACK + page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (s == MAP_FAILED || mprotect(s, page, PROT_NONE) != 0) {
            FAIL("switch: no stack");
            return;
        }
        r[f] = (struct rec){ctx[f], main_ctx, log, &len, f + 1, ROUNDS, -1};
        /* An odd top: the context rounds it down to 16. */
        fx_init(ctx[f], s + page + STACK - 8, (void *)fx_fiber, &r[f]);
    }
    /* Round robin: each fiber in turn until every one has logged its end. */
    int switches = 0;
    for (int round = 0; round <= ROUNDS; round++) {
        for (int f = 0; f < FIBERS; f++) {
            fx_switch(main_ctx, ctx[f]);
            switches++;
        }
    }
    /* The model: fiber f+1 logs (f+1)*1000+i in round i, then 999. */
    int64_t k = 0;
    for (int round = 0; round <= ROUNDS; round++) {
        for (int f = 0; f < FIBERS; f++) {
            int64_t want = (f + 1) * 1000 + (round < ROUNDS ? round : 999);
            if (k >= len || log[k] != want) {
                FAIL("switch: entry %lld of the log is %lld, the model says %lld", (long long)k,
                     (long long)(k < len ? log[k] : -1), (long long)want);
                return;
            }
            k++;
        }
    }
    if (len != k) {
        FAIL("switch: the log has %lld entries, the model %lld", (long long)len, (long long)k);
        return;
    }
    for (int f = 0; f < FIBERS; f++) {
        if (r[f].total != model_total(f + 1)) {
            FAIL("switch: fiber %d's total is %lld, the model's %lld", f + 1, (long long)r[f].total,
                 (long long)model_total(f + 1));
            return;
        }
    }
    printf("ok switch %d\n", switches);
}

/* Whether this process's return-address keys are on: in an arm64 (not
 * arm64e) process `pacibsp` may be a NOP. Said on stderr, for the gate to
 * report, since it is a fact about the machine and not a check. */
static void say_pac(void)
{
    uint64_t out;
    __asm__ volatile("mov x16, x30\n"
                     "mov x30, %1\n"
                     "pacibsp\n"
                     "mov %0, x30\n"
                     "mov x30, x16\n"
                     : "=r"(out)
                     : "r"((uint64_t)0x100004000)
                     : "x16", "x30");
    fprintf(stderr, "pointer authentication: %s\n",
            out == 0x100004000 ? "off (pacibsp leaves the address as it was)" : "on");
}

int main(void)
{
    say_pac();
    check_cells();
    check_shared();
    check_static();
    check_tls();
    check_switch();
    printf("checks failed: %d\n", failures);
    return failures == 0 ? 0 : 1;
}
