// C-convention entry points for the header's APUS_THUNK functions: clang lowers each Swift-convention call.
#include "apus/abi.h"

uint8_t apus_swift_task_isCancelled(void *task) { return swift_task_isCancelled(task) & 1; }

SerialExecutorRef apus_actor_unowned_executor(void *actor, const void *self_type, const void *actor_witness) {
  return actor_unowned_executor(actor, self_type, actor_witness);
}

SwiftString apus_error_domain(const void *value, const void *self_type, const void *error_witness) {
  return error_domain(value, self_type, error_witness);
}

intptr_t apus_error_code(const void *value, const void *self_type, const void *error_witness) {
  return error_code(value, self_type, error_witness);
}
