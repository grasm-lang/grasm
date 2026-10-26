/* An exported function nothing in the program calls, for scripts/linktest.sh
 * and scripts/linkdiff.sh: linked with -dead_strip and
 * -exported_symbol _only_by_name, as `grasm build --native` links an
 * `export fn`, it must survive dead-stripping and stay in the export trie,
 * because the only way to it is its name. */
#include <dlfcn.h>
#include <stdio.h>

int only_by_name(int x)
{
    return x * 7;
}

int main(void)
{
    int (*f)(int) = (int (*)(int))dlsym(RTLD_DEFAULT, "only_by_name");
    if (!f) {
        printf("only_by_name is not exported\n");
        return 1;
    }
    printf("only_by_name(6) = %d\n", f(6));
    return 0;
}
