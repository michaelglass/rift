// Builds Axis({ horizontal: True, fill: Tiled, first: Child { size: 1, node: Window(leaf idx 7,
// Unconstrained) }, rest: [] }) as `roc glue` (RustGlue.roc) lays it out, and asks Roc for the
// sum of its windows' idx: expected 7.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void *roc_alloc(size_t len, size_t align) { return malloc(len); }
void *roc_realloc(void *p, size_t len, size_t align) { return realloc(p, len); }
void roc_dealloc(void *p, size_t align) { free(p); }
void roc_dbg(const char *s, size_t n) {}
void roc_expect_failed(const char *s, size_t n) {}
void roc_crashed(const char *s, size_t n) { fprintf(stderr, "crashed\n"); exit(1); }

struct RocList { void *elements; size_t length, capacity_or_alloc_ptr; };
struct Constraints { uint64_t payload[7]; uint8_t tag; };             // 64; Unconstrained = 1
struct Leaf { struct Constraints constraints; uint32_t idx; int32_t pid; uint8_t fill; }; // 80
struct Child;
struct Axis { struct Child *first; struct RocList rest; uint8_t fill; uint8_t horizontal; };
struct Node { union { struct Axis axis; struct Leaf window; } payload; uint8_t tag; }; // 88; Axis = 0, Stack = 1, Window = 2
struct Child { struct Node *node; float size; };                       // 16
struct Shape { struct Node root; double x, y, w, h; double stack_offset; };

_Static_assert(sizeof(struct Leaf) == 80, "Leaf");
_Static_assert(sizeof(struct Node) == 88, "Node");
_Static_assert(sizeof(struct Child) == 16, "Child");

// A Roc box whose payload holds refcounted values: a 16-byte header, refcount 1 before the data.
static void *box(size_t size) {
	char *base = calloc(1, 16 + size);
	*(intptr_t *)(base + 8) = 1;
	return base + 16;
}

uint64_t roc_count(struct Shape shape);

int main(void) {
	struct Node *leaf = box(sizeof(struct Node));
	leaf->payload.window.constraints.tag = 1; // Unconstrained
	leaf->payload.window.idx = 7;
	leaf->payload.window.pid = 200;
	leaf->payload.window.fill = 1; // Tiled
	leaf->tag = 2; // Window
	struct Child *first = box(sizeof(struct Child));
	first->node = leaf;
	first->size = 1.0f;
	struct Shape shape;
	memset(&shape, 0, sizeof shape);
	shape.root.payload.axis = (struct Axis){ first, { NULL, 0, 0 }, 1, 1 };
	shape.root.tag = 0; // Axis
	shape.w = 1000.0;
	shape.h = 800.0;
	printf("%llu\n", (unsigned long long)roc_count(shape));
}
