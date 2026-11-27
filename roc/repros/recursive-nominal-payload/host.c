#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void *roc_alloc(size_t len, size_t align) { return malloc(len); }
void *roc_realloc(void *p, size_t len, size_t align) { return realloc(p, len); }
void roc_dealloc(void *p, size_t align) { free(p); }
void roc_dbg(const char *s, size_t n) {}
void roc_expect_failed(const char *s, size_t n) {}
void roc_crashed(const char *s, size_t n) { exit(1); }

/* The layout `roc glue` (RustGlue.roc) gives these types. */
struct Node;
struct Child { struct Node *node; float size; };              /* 16 bytes */
struct RocList { void *ptr; size_t len, cap; };
struct Axis { struct Child *first; struct RocList rest; _Bool horizontal; }; /* 40 */
struct Leaf { uint64_t id; };
struct Node { union { struct Axis ax; struct Leaf win; } payload; uint8_t tag; }; /* 48; Ax = 0, Win = 1 */

/* A box whose payload holds refcounted values: a two-word header, refcount 1. */
static void *box(const void *value, size_t size) {
	char *base = malloc(16 + size);
	*(intptr_t *)(base + 8) = 1;
	memcpy(base + 16, value, size);
	return base + 16;
}

uint64_t roc_sum(struct Node node);

int main(void) {
	struct Node leaf = { .payload.win = { 7 }, .tag = 1 };
	struct Child child = { box(&leaf, sizeof leaf), 1.0f };
	struct Node root = { .payload.ax = { box(&child, sizeof child), { 0, 0, 0 }, 1 }, .tag = 0 };
	printf("%llu\n", (unsigned long long)roc_sum(root)); /* 7 */
}
