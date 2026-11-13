// Swift Concurrency Job ABI, transcribed from swiftlang/swift:
//   include/swift/ABI/Task.h (class Job), ABI/Metadata.h (DispatchClassMetadata),
//   ABI/MetadataKind.def (Job kind), stdlib/public/Concurrency/Task.cpp (jobHeapMetadata)
#include <stdint.h>
#define SWIFTCALL __attribute__((swiftcall))
#define SWIFTASYNCCALL __attribute__((swiftasynccall))

typedef struct { void *identity; uintptr_t implementation; } SerialExecutorRef;   // ABI/Executor.h
extern SWIFTCALL void swift_job_run(void *job, SerialExecutorRef executor);
extern SWIFTCALL void swift_task_enqueueGlobal(void *job);
extern SWIFTCALL void swift_task_enqueueGlobalWithDelay(unsigned long long delay_ns, void *job);
extern SWIFTCALL void swift_task_enqueueMainExecutor(void *job);
extern SWIFTCALL SerialExecutorRef swift_task_getMainExecutor(void);
extern struct dispatch_queue_s _dispatch_main_q;

typedef struct __attribute__((aligned(16))) {
  const void *metadata; uint64_t refcounts;         // HeapObject
  void *scheduler_private[2];
  uint32_t flags; uint32_t id;                      // JobFlags: kind bits 0-7, priority bits 8-15
  void *voucher; void *reserved;
  SWIFTASYNCCALL void (*run_job)(void *job);        // JobInvokeFunction = SWIFT_CC(swiftasync) void(Job*)
} SwiftJob;
_Static_assert(sizeof(SwiftJob) == 64, "Job is 64 bytes");

void rs_job_run(void *job);                         // Rust, C ABI
static SWIFTASYNCCALL void run_job(void *job) { rs_job_run(job); }

// Dispatch calls VTableInvoke when the cooperative queue drains the job (dispatch_async_swift_job).
// Mirrors executorForEnqueuedJob() in Task.cpp: the runtime stores the target queue in
// SchedulerPrivate[DispatchQueueIndex] (= 1 on 64-bit) before handing the job to dispatch.
static void invoke(void *obj, void *unused, uint32_t flags) {
  void *queue = ((SwiftJob *)obj)->scheduler_private[1];
  SerialExecutorRef ex = queue == (void *)&_dispatch_main_q ? swift_task_getMainExecutor()
                                                            : (SerialExecutorRef){0, 0};  // generic()
  swift_job_run(obj, ex);
}
static void destroy(void *obj) {}
static const struct {
  void *type_layout; void (*destroy)(void *); void *vwt;            // FullMetadata prefix
  uintptr_t kind; void *opaque; void *opaque_objc[3];               // HeapMetadata + DispatchClassMetadata
  unsigned long vtable_type; void (*vtable_invoke)(void *, void *, uint32_t);
} job_metadata = { 0, destroy, 0, 3 | 0x400 | 0x100, 0, {0, 0, 0}, 1 /*dispatchSwiftObjectType*/, invoke };

void rs_job_init(SwiftJob *j, uint8_t priority) {
  *j = (SwiftJob){ .metadata = &job_metadata.kind, .flags = 1u | ((uint32_t)priority << 8), .run_job = run_job };
}
void rs_enqueue(SwiftJob *j) { swift_task_enqueueGlobal(j); }
void rs_enqueue_after(SwiftJob *j, unsigned long long ns) { swift_task_enqueueGlobalWithDelay(ns, j); }
void rs_enqueue_main(SwiftJob *j) { swift_task_enqueueMainExecutor(j); }
