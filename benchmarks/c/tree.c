// Allocation-shaped work: build a tree, walk it, free it, repeat.
#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    long value;
    long n;
    struct Node **kids;
} Node;

static Node *build(long depth, long branch, long seed) {
    Node *node = malloc(sizeof(Node));
    node->value = seed % 1000;
    if (depth <= 0) { node->n = 0; node->kids = NULL; return node; }
    node->n = branch;
    node->kids = malloc(sizeof(Node *) * branch);
    for (long i = 0; i < branch; i++)
        node->kids[i] = build(depth - 1, branch, seed * 31 + i + 1);
    return node;
}

static long total(Node *n) {
    long sum = n->value;
    for (long i = 0; i < n->n; i++) sum += total(n->kids[i]);
    return sum;
}

static void release(Node *n) {
    for (long i = 0; i < n->n; i++) release(n->kids[i]);
    free(n->kids);
    free(n);
}

int main(void) {
    long acc = 0;
    for (long r = 0; r < 20; r++) {
        Node *t = build(9, 3, r + 1);
        acc = (acc + total(t)) % 1000000007;
        release(t);
    }
    printf("%ld\n", acc);
    return 0;
}
