/* Thread-local variables for scripts/linktest.sh and scripts/linkdiff.sh.
 *
 * Neither native backend nor the runtime emits a thread-local variable
 * (they keep per-thread cells through pthread keys), so this is what puts
 * TLVP relocations, `__thread_vars` descriptors bound to `__tlv_bootstrap`
 * and a thread-local template (`__thread_data` and `__thread_bss`) through
 * compiler/link. Built with `cc -c`; the gates hold the link to ld's. */
_Thread_local long counter = 5;
_Thread_local long zeroed;
_Thread_local char buf[64];
static _Thread_local int hidden = 7;
extern _Thread_local long other;

long bump(void)
{
    counter += 1;
    zeroed += 2;
    hidden += 3;
    buf[3] += 9;
    other += 10;
    return counter + zeroed + hidden + buf[3] + other;
}
