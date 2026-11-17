// C-convention entry points for the header's APUS_THUNK functions: clang lowers each Swift-convention call.
#include "apus/abi.h"

uint8_t apus_swift_task_isCancelled(void *task) { return swift_task_isCancelled(task) & 1; }
