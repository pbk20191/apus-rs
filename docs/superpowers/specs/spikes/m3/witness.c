// Witness functions for a protocol implemented in Rust: Swift calls these through a hand-built witness table.
// Witness method convention: formal arguments, then Self's metadata and the witness table; self by address in
// the context register.
#include <stdint.h>
#define SWIFTCALL __attribute__((swiftcall))
#define SCTX __attribute__((swift_context))
typedef struct { uint64_t w0, w1; } SwiftString;

void rs_title(void *state, uint64_t out[2]);
void rs_greet(void *state, uint64_t name0, uint64_t name1, uint64_t out[2]);

static SWIFTCALL SwiftString title_witness(const void *Self, const void *witness_table, SCTX void **self) {
  uint64_t out[2];
  rs_title(*self, out);
  return (SwiftString){out[0], out[1]};
}
static SWIFTCALL SwiftString greet_witness(uint64_t name0, uint64_t name1, const void *Self, const void *witness_table,
                                           SCTX void **self) {
  uint64_t out[2];
  rs_greet(*self, name0, name1, out);
  return (SwiftString){out[0], out[1]};
}
const void *m3_title_witness(void) { return (const void *)title_witness; }
const void *m3_greet_witness(void) { return (const void *)greet_witness; }
