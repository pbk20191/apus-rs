// Code the Swift runtime calls into: task entry, poll step, continuation resume point, Swift-call return,
// child-task entry, cancellation handler. Rust calls the runtime's sync functions itself (src/swift/rt.rs);
// only what needs swiftasynccall / swift_context lives here.
//
// Tail calls (checked with objdump -d on arm64): the task completion call, swift_continuation_await,
// swift_task_switch, swift_taskGroup_waitAll and the closure call are emitted as `b`/`br`, never `bl`.
#include "swift_abi.h"

// Continuation functions are cast between their (context) and (context, error) shapes, as the runtime itself
// does with function_cast: the extra argument rides in swift_context and is ignored or supplied.
#pragma clang diagnostic ignored "-Wcast-function-type-mismatch"

// ---- runtime (include/swift/Runtime/Concurrency.h, Metadata.h) ----
typedef struct { void *task; AsyncContext *initial_context; } AsyncTaskAndContext;
typedef struct { const void *metadata; size_t state; } MetadataResponse;
extern SWIFTCALL AsyncTaskAndContext swift_task_create_common(size_t flags, void *options, const void *result_type,
                                                              void *function, void *closure_context, size_t size);
extern SWIFTCALL MetadataResponse swift_getTupleTypeMetadata(size_t request, size_t flags, const void *const *elements,
                                                             const char *labels, const void *proposed_witnesses);
extern SWIFTCALL void *swift_continuation_init(ContinuationAsyncContext *ctx, size_t flags);
extern SWIFTASYNCCALL void swift_continuation_await(ContinuationAsyncContext *ctx);
extern SWIFTCALL void swift_continuation_resume(void *task);
extern SWIFTCALL SerialExecutorRef swift_task_getMainExecutor(void);
extern SWIFTASYNCCALL void swift_task_switch(ACTX AsyncContext *resume_ctx, TaskContinuationFunction *resume,
                                             SerialExecutorRef executor);
extern SWIFTCALL void *swift_task_alloc(size_t size);
extern SWIFTCALL void swift_task_dealloc(void *ptr);
typedef SWIFTCALL void CancelFn(SCTX void *);
extern SWIFTCALL void *swift_task_addCancellationHandler(CancelFn *fn, void *ctx);
extern SWIFTCALL void swift_task_removeCancellationHandler(void *record);
extern SWIFTCALL void swift_taskGroup_initializeWithFlags(size_t flags, void *group, const void *T);
extern SWIFTCALL void swift_taskGroup_destroy(void *group);
extern SWIFTASYNCCALL void swift_taskGroup_waitAll(void *result, ACTX AsyncContext *caller, void *group, void *body_error,
                                                   ThrowingContinuation *resume, AsyncContext *call_context);

// ---- Rust (src/swift/task.rs, call.rs) ----
int  rs_poll(void *rust);                                                       // 1 = Ready
int  rs_park(void *rust, void *continuation);                                   // 1 = parked, 0 = woken mid-poll
void rs_finish(void *rust);
void rs_cancelled(void *rust);
bool rs_is_main(void *rust);
int  rs_take_call(void *rust, ClosureBox **box, void **result, void **record);  // 1 = run it in this Task
int  rs_take_child(void *rust, void **record);                                  // 1 = run it as a child Task
void rs_call_closure(void *record, ClosureBox **box, void **result);
void rs_set_child(void *record, void *child_task);                              // +1 child task
void rs_call_done(void *record, void *error);                                   // error: owned `any Error` or NULL

// Flags (include/swift/ABI/MetadataValues.h).
enum {
  CONT_HAS_EXECUTOR_OVERRIDE = 1u << 1,     // AsyncContinuationFlags
  GROUP_DISCARD_RESULTS = 1u << 8,          // TaskGroupFlags::TaskGroup_DiscardResults
  CREATE_IS_CHILD_TASK = 1u << 8,           // TaskCreateFlags
  CREATE_ENQUEUE_JOB = 1u << 12,
  CREATE_ADD_PENDING_GROUP_TASK_UNCONDITIONALLY = 1u << 13,
  CREATE_IS_DISCARDING_TASK = 1u << 14,
  CREATE_IS_TASK_FUNCTION_CONSUMED = 1u << 15,
  OPTION_KIND_TASK_GROUP = 1,               // TaskOptionRecordKind::TaskGroup
};

// Metadata for (). Not `$sytN`: that symbol is the full metadata (prefix included) and IRGen adds the
// address-point offset itself; passing it directly crashes swift_task_create_common.
static const void *empty_tuple_metadata(void) { return swift_getTupleTypeMetadata(0, 0, NULL, NULL, NULL).metadata; }

typedef SWIFTASYNCCALL void CompleteFn(ACTX AsyncContext *, SCTX void *error);

static SWIFTCALL void on_cancel(SCTX void *rust) { rs_cancelled(rust); }

static SWIFTASYNCCALL void step(ACTX AsyncContext *ctx);

static SWIFTASYNCCALL void resume_point(ACTX AsyncContext *cont) {
  return step(cont->parent);                 // cont->parent is our RsTaskContext
}

static SWIFTASYNCCALL void finish(ACTX AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  swift_task_removeCancellationHandler(c->cancel_record);
  rs_finish(c->rust);
  return ((CompleteFn *)c->base.resume_parent)(&c->base, NULL);   // completeTaskAndRelease(ctx, error: nil)
}

static SWIFTASYNCCALL void after_wait_all(ACTX AsyncContext *ctx, SCTX void *error) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  swift_taskGroup_destroy(c->group);
  c->group_initialized = false;
  return finish(ctx);
}

static AnyClosureFn *closure_code(const ClosureBox *box) {
  const AsyncFunctionPointer *afp = box->fn;
  return (AnyClosureFn *)((const char *)&afp->function + afp->function);
}

// A same-Task call returns here: ResumeParent(callee context, error).
static SWIFTASYNCCALL void after_call(ACTX AsyncContext *callee, SCTX void *error) {
  AsyncContext *ctx = callee->parent;
  swift_task_dealloc(callee);
  RsTaskContext *c = (RsTaskContext *)ctx;
  void *record = c->current_call;
  c->current_call = NULL;
  rs_call_done(record, error);
  // The callee is nonisolated and may have resumed anywhere: hop back, as compiled Swift does.
  if (c->executor.identity) return swift_task_switch(ctx, step, c->executor);
  return step(ctx);
}

// ---- child Tasks: a discarding TaskGroup, like `async let` / `withDiscardingTaskGroup` ----
static SWIFTASYNCCALL void child_after(ACTX AsyncContext *callee, SCTX void *error) {
  ChildContext *k = (ChildContext *)callee->parent;
  swift_task_dealloc(callee);
  rs_call_done(k->record, error);
  return ((CompleteFn *)k->base.resume_parent)(&k->base, NULL);
}

// AsyncVoidClosureEntryPoint: discarding children start through the runtime's non_future_adapter.
static SWIFTASYNCCALL void child_entry(ACTX AsyncContext *ctx, SCTX void *record) {
  ChildContext *k = (ChildContext *)ctx;
  k->record = record;
  ClosureBox *box;
  void *result;
  rs_call_closure(record, &box, &result);
  AsyncContext *callee = swift_task_alloc(box->fn->expected_context_size);
  callee->parent = ctx;
  callee->resume_parent = (TaskContinuationFunction *)child_after;
  return closure_code(box)(result, callee, box->context);
}

static void spawn_child(RsTaskContext *c, void *record) {
  if (!c->group_initialized) {
    swift_taskGroup_initializeWithFlags(GROUP_DISCARD_RESULTS, c->group, empty_tuple_metadata());
    c->group_initialized = true;
  }
  struct { size_t flags; void *parent; void *group; } option = { OPTION_KIND_TASK_GROUP, NULL, c->group };
  size_t flags = CREATE_IS_CHILD_TASK | CREATE_ENQUEUE_JOB | CREATE_ADD_PENDING_GROUP_TASK_UNCONDITIONALLY
               | CREATE_IS_DISCARDING_TASK | CREATE_IS_TASK_FUNCTION_CONSUMED;
  AsyncTaskAndContext child = swift_task_create_common(flags, &option, empty_tuple_metadata(), (void *)child_entry,
                                                       record, sizeof(ChildContext));
  rs_set_child(record, child.task);
}

static SWIFTASYNCCALL void step(ACTX AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  if (rs_poll(c->rust)) {
    if (c->group_initialized)   // structured concurrency: the parent never completes before its children
      return swift_taskGroup_waitAll(&c->unused_result, ctx, c->group, NULL, after_wait_all, &c->wait.base);
    return finish(ctx);
  }
  ClosureBox *box;
  void *result;
  void *record;
  if (rs_take_call(c->rust, &box, &result, &record)) {
    AsyncContext *callee = swift_task_alloc(box->fn->expected_context_size);
    callee->parent = ctx;
    callee->resume_parent = (TaskContinuationFunction *)after_call;
    c->current_call = record;
    return closure_code(box)(result, callee, box->context);
  }
  while (rs_take_child(c->rust, &record)) spawn_child(c, record);
  ContinuationAsyncContext *k = &c->cont;
  k->base.parent = &c->base;
  k->base.resume_parent = resume_point;
  k->normal_result = &c->unused_result;
  bool pinned = c->executor.identity != NULL;
  void *task = swift_continuation_init(k, pinned ? CONT_HAS_EXECUTOR_OVERRIDE : 0);
  if (pinned) k->resume_to_executor = c->executor;
  if (!rs_park(c->rust, task)) swift_continuation_resume(task);   // woken mid-poll: resume ourselves
  return swift_continuation_await(k);
}

// AsyncGenericClosureEntryPoint: (indirect result, context, closure context). The result type is (): nothing to write.
static SWIFTASYNCCALL void entry(void *indirect_result, ACTX AsyncContext *ctx, SCTX void *rust) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  c->rust = rust;
  c->current_call = NULL;
  c->group_initialized = false;
  c->executor = rs_is_main(rust) ? swift_task_getMainExecutor() : (SerialExecutorRef){0, 0};
  c->cancel_record = swift_task_addCancellationHandler(on_cancel, rust);
  return step(ctx);
}

const void *rs_entry_fn(void) { return (const void *)entry; }
size_t rs_context_size(void) { return sizeof(RsTaskContext); }

// ---- plain Jobs (timers): the runtime hands them to dispatch, which calls the metadata's VTableInvoke ----
// Job header: include/swift/ABI/Task.h (class Job). Metadata shape: FullMetadata<DispatchClassMetadata> as in
// stdlib/public/Concurrency/Task.cpp (jobHeapMetadata, which is not exported): kind Job = 3 | NonType |
// RuntimePrivate (ABI/MetadataKind.def), dispatchSwiftObjectType = 1, OpaqueObjC[3] on ObjC platforms.
extern SWIFTCALL void swift_job_run(void *job, SerialExecutorRef executor);
void rs_job_run(void *job);

static SWIFTASYNCCALL void run_job(void *job) { rs_job_run(job); }
static void invoke_job(void *job, void *unused, uint32_t flags) {
  swift_job_run(job, (SerialExecutorRef){0, 0});   // SerialExecutorRef::generic()
}
static void destroy_job(void *job) {}
static const struct {
  void *type_layout; void (*destroy)(void *); void *value_witnesses;            // FullMetadata prefix
  uintptr_t kind; void *opaque; void *opaque_objc[3];                           // HeapMetadata + DispatchClassMetadata
  unsigned long vtable_type; void (*vtable_invoke)(void *, void *, uint32_t);
} job_metadata = { 0, destroy_job, 0, 3 | 0x400 | 0x100, 0, {0, 0, 0}, 1, invoke_job };

const void *rs_job_metadata(void) { return &job_metadata.kind; }
const void *rs_run_job_fn(void) { return (const void *)run_job; }
