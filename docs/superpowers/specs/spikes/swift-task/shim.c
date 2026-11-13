// Rust futures as genuine Swift Concurrency Tasks — no Swift source.
// ABI transcribed from swiftlang/swift @ swift-6.4.2-RELEASE:
//   include/swift/ABI/Task.h        AsyncContext, ContinuationAsyncContext, AsyncVoidClosureEntryPoint
//   include/swift/ABI/TaskStatus.h  CancellationNotificationStatusRecord::FunctionType
//   include/swift/ABI/MetadataValues.h  TaskCreateFlags, AsyncContinuationFlags, TaskOptionRecordKind
//   stdlib/public/Concurrency/Task.cpp  swift_task_create_commonImpl, completeTaskAndRelease, continuations
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#define SWIFTCALL __attribute__((swiftcall))
#define SWIFTASYNCCALL __attribute__((swiftasynccall))
#define ACTX __attribute__((swift_async_context))
#define SCTX __attribute__((swift_context))

typedef struct { void *identity; uintptr_t implementation; } SerialExecutorRef;
typedef struct AsyncContext AsyncContext;
typedef SWIFTASYNCCALL void TaskContinuationFunction(ACTX AsyncContext *);
struct __attribute__((aligned(16))) AsyncContext { AsyncContext *parent; TaskContinuationFunction *resume_parent; };
typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  size_t flags; size_t await_synchronization; void *error_result; void *normal_result;
  SerialExecutorRef resume_to_executor;
} ContinuationAsyncContext;
_Static_assert(sizeof(ContinuationAsyncContext) == 64, "ContinuationAsyncContext");

// Our task's initial context: the runtime allocates `initialContextSize` bytes and fills `base`
// (Parent = NULL, ResumeParent = completeTaskAndRelease). The rest is ours.
typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  ContinuationAsyncContext cont;      // reused for every suspension
  void *rust;                         // Arc<RTask> (the closure context, consumed)
  void *cancel_record;
  SerialExecutorRef executor;         // generic, or the main executor for MainActor tasks
  uint64_t unused_result;
} RsTaskContext;

typedef struct { void *task; AsyncContext *initial_context; } AsyncTaskAndContext;
extern SWIFTCALL AsyncTaskAndContext swift_task_create_common(size_t flags, void *options, const void *result_type,
                                                              void *function, void *closure_context, size_t initial_context_size);
extern SWIFTCALL void *swift_continuation_init(ContinuationAsyncContext *ctx, size_t flags);
extern SWIFTASYNCCALL void swift_continuation_await(ContinuationAsyncContext *ctx);
extern SWIFTCALL void swift_continuation_resume(void *task);
typedef SWIFTCALL void CancelFn(SCTX void *);
extern SWIFTCALL void *swift_task_addCancellationHandler(CancelFn *fn, void *ctx);
extern SWIFTCALL void swift_task_removeCancellationHandler(void *record);
extern SWIFTCALL void *swift_task_getCurrent(void);
extern SWIFTCALL bool swift_task_isCancelled(void *task);
extern SWIFTCALL size_t swift_task_currentPriority(void *task);
extern SWIFTCALL size_t swift_task_escalate(void *task, size_t priority);
extern SWIFTCALL void swift_task_cancel(void *task);
extern SWIFTCALL SerialExecutorRef swift_task_getMainExecutor(void);

// Rust side (C ABI)
int  rs_poll(void *rust);                    // 1 = Ready
int  rs_park(void *rust, void *continuation);// 1 = parked; 0 = woken during poll, poll again
void rs_finish(void *rust);
void rs_cancelled(void *rust);
bool rs_is_main(void *rust);

static SWIFTCALL void on_cancel(SCTX void *rust) { rs_cancelled(rust); }

typedef SWIFTASYNCCALL void CompleteFn(ACTX AsyncContext *, SCTX void *error);
static SWIFTASYNCCALL void step(ACTX AsyncContext *ctx);

static SWIFTASYNCCALL void resume_point(ACTX AsyncContext *cont) {
  return step(cont->parent);                 // cont->parent is our RsTaskContext
}

static SWIFTASYNCCALL void step(ACTX AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  if (rs_poll(c->rust)) {
    swift_task_removeCancellationHandler(c->cancel_record);
    rs_finish(c->rust);
    return ((CompleteFn *)c->base.resume_parent)(&c->base, NULL);   // completeTaskAndRelease(ctx, error: nil)
  }
  ContinuationAsyncContext *k = &c->cont;
  k->base.parent = &c->base;
  k->base.resume_parent = resume_point;
  k->normal_result = &c->unused_result;
  bool pinned = c->executor.identity != NULL;
  void *task = swift_continuation_init(k, pinned ? 1u << 1 /*HasExecutorOverride*/ : 0);
  if (pinned) k->resume_to_executor = c->executor;
  if (!rs_park(c->rust, task)) swift_continuation_resume(task);    // woken mid-poll: resume ourselves
  return swift_continuation_await(k);
}

// AsyncVoidClosureEntryPoint: (async context, closure context)
static SWIFTASYNCCALL void entry(ACTX AsyncContext *ctx, SCTX void *rust) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  c->rust = rust;
  c->executor = rs_is_main(rust) ? swift_task_getMainExecutor() : (SerialExecutorRef){0, 0};
  c->cancel_record = swift_task_addCancellationHandler(on_cancel, rust);
  return step(ctx);
}

typedef struct { size_t flags; void *parent; SerialExecutorRef executor; } InitialSerialExecutorOption;

void *rs_task_create(void *rust, uint8_t priority, bool main, bool copy_task_locals) {
  InitialSerialExecutorOption opt = { 0 /*InitialSerialExecutor*/, NULL, {0, 0} };
  if (main) opt.executor = swift_task_getMainExecutor();
  size_t flags = priority | 1u << 12 /*EnqueueJob*/ | 1u << 15 /*TaskFunctionConsumed*/
               | (copy_task_locals ? 1u << 10 /*CopyTaskLocals*/ : 0);
  return swift_task_create_common(flags, main ? &opt : NULL, NULL /*non-future*/, (void *)entry, rust,
                                  sizeof(RsTaskContext)).task;   // +1
}
void  rs_continuation_resume(void *t) { swift_continuation_resume(t); }
void *rs_current_task(void) { return swift_task_getCurrent(); }
bool  rs_task_is_cancelled(void *t) { return swift_task_isCancelled(t); }
uint8_t rs_task_priority(void *t) { return (uint8_t)swift_task_currentPriority(t); }
void  rs_task_escalate(void *t, uint8_t p) { swift_task_escalate(t, p); }
void  rs_task_cancel(void *t) { swift_task_cancel(t); }

// ---- plain Jobs for timers (from the previous PoC) ----
extern SWIFTCALL void swift_job_run(void *job, SerialExecutorRef executor);
extern SWIFTCALL void swift_task_enqueueGlobalWithDelay(unsigned long long delay_ns, void *job);
typedef struct __attribute__((aligned(16))) {
  const void *metadata; uint64_t refcounts; void *scheduler_private[2];
  uint32_t flags; uint32_t id; void *voucher; void *reserved;
  SWIFTASYNCCALL void (*run_job)(void *job);
} SwiftJob;
void rs_job_run(void *job);
static SWIFTASYNCCALL void run_job(void *job) { rs_job_run(job); }
static void invoke(void *obj, void *unused, uint32_t flags) { swift_job_run(obj, (SerialExecutorRef){0, 0}); }
static void destroy(void *obj) {}
static const struct {
  void *type_layout; void (*destroy)(void *); void *vwt;
  uintptr_t kind; void *opaque; void *opaque_objc[3];
  unsigned long vtable_type; void (*vtable_invoke)(void *, void *, uint32_t);
} job_metadata = { 0, destroy, 0, 3 | 0x400 | 0x100, 0, {0, 0, 0}, 1, invoke };
void rs_job_init(SwiftJob *j, uint8_t priority) {
  *j = (SwiftJob){ .metadata = &job_metadata.kind, .flags = 1u | ((uint32_t)priority << 8), .run_job = run_job };
}
void rs_enqueue_after(SwiftJob *j, unsigned long long ns) { swift_task_enqueueGlobalWithDelay(ns, j); }
