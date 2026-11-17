// Code the Swift runtime calls into: task entry, poll step, continuation resume point, Swift-call return,
// child-task entry, cancellation handler. Rust calls the runtime's sync functions itself (sys/include/apus/abi.h);
// only what needs swiftasynccall / swift_context lives here.
//
// Tail calls (checked with objdump -d on arm64): the task completion call, swift_continuation_await,
// swift_task_switch, swift_taskGroup_waitAll and the generic Swift call are emitted as `b`/`br`, never `bl`.
#include "apus/abi.h"

// Continuation functions are cast between their (context) and (context, error) shapes, as the runtime itself
// does with function_cast: the extra argument rides in swift_context and is ignored or supplied.
#pragma clang diagnostic ignored "-Wcast-function-type-mismatch"

// Our initial task context. The runtime allocates it (initialContextSize) and fills `base`
// (Parent = NULL, ResumeParent = completeTaskAndRelease); FutureAsyncContext is just AsyncContext.
typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  ContinuationAsyncContext cont;      // reused at every suspension
  void *rust;                         // Arc<Task> raw pointer (closure context, consumed)
  void *cancel_record;
  SerialExecutorRef executor;         // generic, or the main executor for MainActor tasks
  uint64_t unused_result;
  void *current_call;                 // call record of the in-flight same-Task Swift call
  void *call_ret;                     // RetRegs of that call
  bool group_initialized;
  HopCall hop;                        // the in-flight same-Task call, when it hops to another executor
  CallRegs *hop_regs;
  TaskFutureWaitAsyncContext wait;
  void *group[NUM_WORDS_TASK_GROUP] __attribute__((aligned(16)));   // TaskGroup storage: NumWords_TaskGroup = 32 (ABI/MetadataValues.h)
} RsTaskContext;

typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  void *record;
  void *ret;                          // RetRegs of the child's call
  HopCall hop;
  CallRegs *hop_regs;
} ChildContext;

// Metadata for (). Not `$sytN`: that symbol is the full metadata (prefix included) and IRGen adds the
// address-point offset itself; passing it directly crashes swift_task_create_common.
static const void *empty_tuple_metadata(void) { return swift_getTupleTypeMetadata(0, 0, NULL, NULL, NULL).metadata; }


static APUS_SWIFTCALL void on_cancel(APUS_SWIFT_CONTEXT void *rust) { rs_cancelled(rust); }

static APUS_SWIFTASYNCCALL void step(APUS_ASYNC_CONTEXT AsyncContext *ctx);

static APUS_SWIFTASYNCCALL void resume_point(APUS_ASYNC_CONTEXT AsyncContext *cont) {
  return step(cont->parent);                 // cont->parent is our RsTaskContext
}

static APUS_SWIFTASYNCCALL void finish(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  swift_task_removeCancellationHandler(c->cancel_record);
  rs_finish(c->rust);
  return ((ThrowingContinuation *)c->base.resume_parent)(&c->base, NULL);   // completeTaskAndRelease(ctx, error: nil)
}

static APUS_SWIFTASYNCCALL void after_wait_all(APUS_ASYNC_CONTEXT AsyncContext *ctx, APUS_SWIFT_CONTEXT void *error) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  swift_taskGroup_destroy(c->group);
  c->group_initialized = false;
  // The last child resumes us on the generic executor (TaskGroup.cpp): a MainActor task hops back so that it
  // completes, and its output is handed over or dropped, on the main thread.
  if (c->executor.identity) return swift_task_switch(ctx, finish, c->executor);
  return finish(ctx);
}

// MainActor.shared: a singleton, looked up once (a race only computes the same object twice).
static void *main_actor(void) {
  static void *shared;
  void *known = __atomic_load_n(&shared, __ATOMIC_ACQUIRE);
  if (known) return known;
  known = main_actor_shared(main_actor_metadata(0).metadata);   // +1, kept for good
  __atomic_store_n(&shared, known, __ATOMIC_RELEASE);
  return known;
}

void apus_fill_isolation(CallRegs *regs, bool main_isolated) {
  if (regs->isolation < 0) return;
  regs->gpr[regs->isolation] = main_isolated ? (uint64_t)main_actor() : 0;
  regs->gpr[regs->isolation + 1] = main_isolated ? (uint64_t)&MAIN_ACTOR_ACTOR : 0;
}

static GenericAsyncFn *code_of(const AsyncFunctionPointer *fn) {
  return (GenericAsyncFn *)((const char *)&fn->function + fn->function);
}

static void store_results(RetRegs *ret, uint64_t r0, uint64_t r1, uint64_t r2, uint64_t r3,
                          double f0, double f1, double f2, double f3, void *error) {
  ret->gpr[0] = r0; ret->gpr[1] = r1; ret->gpr[2] = r2; ret->gpr[3] = r3;
  ret->fpr[0] = f0; ret->fpr[1] = f1; ret->fpr[2] = f2; ret->fpr[3] = f3;
  ret->error = error;
}

// A same-Task call returns here: ResumeParent(direct results..., callee context, error).
static APUS_SWIFTASYNCCALL void after_call(uint64_t r0, uint64_t r1, uint64_t r2, uint64_t r3,
                                      double f0, double f1, double f2, double f3,
                                      APUS_ASYNC_CONTEXT AsyncContext *callee, APUS_SWIFT_CONTEXT void *error) {
  AsyncContext *ctx = callee->parent;
  swift_task_dealloc(callee);
  RsTaskContext *c = (RsTaskContext *)ctx;
  store_results(c->call_ret, r0, r1, r2, r3, f0, f1, f2, f3, error);
  void *record = c->current_call;
  c->current_call = NULL;
  rs_call_done(record);
  // The callee is nonisolated and may have resumed anywhere: hop back, as compiled Swift does.
  if (c->executor.identity) return swift_task_switch(ctx, step, c->executor);
  return step(ctx);
}

// A hopping same-Task call, back on the Task's own executor: only now may the call record go (it can hold the
// actor's last reference, which must not be released while the Task is still on that actor).
static APUS_SWIFTASYNCCALL void after_hop(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  void *record = c->current_call;
  c->current_call = NULL;
  rs_call_done(record);
  return step(ctx);
}

// A hopping same-Task call: now on the target executor, call, then hop back to the Task's own executor.
static APUS_SWIFTASYNCCALL void hop_run(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  rs_call_sync(c->hop.function, c->hop_regs, c->hop.indirect, c->call_ret, c->hop.kind);
  return swift_task_switch(ctx, after_hop, c->executor);   // the main executor, or the generic one (0, 0)
}

// ---- child Tasks: a discarding TaskGroup, like `async let` / `withDiscardingTaskGroup` ----
static APUS_SWIFTASYNCCALL void child_after(uint64_t r0, uint64_t r1, uint64_t r2, uint64_t r3,
                                       double f0, double f1, double f2, double f3,
                                       APUS_ASYNC_CONTEXT AsyncContext *callee, APUS_SWIFT_CONTEXT void *error) {
  ChildContext *k = (ChildContext *)callee->parent;
  swift_task_dealloc(callee);
  store_results(k->ret, r0, r1, r2, r3, f0, f1, f2, f3, error);
  rs_call_done(k->record);
  return ((ThrowingContinuation *)k->base.resume_parent)(&k->base, NULL);
}

// Back on the generic executor: the record may go now (see after_hop), then the child completes.
static APUS_SWIFTASYNCCALL void child_finish(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  ChildContext *k = (ChildContext *)ctx;
  rs_call_done(k->record);
  return ((ThrowingContinuation *)k->base.resume_parent)(&k->base, NULL);
}

// A hopping call in a child Task: call on the target executor, then back to the generic executor to finish.
static APUS_SWIFTASYNCCALL void child_hop_run(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  ChildContext *k = (ChildContext *)ctx;
  rs_call_sync(k->hop.function, k->hop_regs, k->hop.indirect, k->ret, k->hop.kind);
  return swift_task_switch(ctx, child_finish, (SerialExecutorRef){0, 0});
}

// AsyncVoidClosureEntryPoint: discarding children start through the runtime's non_future_adapter.
static APUS_SWIFTASYNCCALL void child_entry(APUS_ASYNC_CONTEXT AsyncContext *ctx, APUS_SWIFT_CONTEXT void *record) {
  ChildContext *k = (ChildContext *)ctx;
  k->record = record;
  const AsyncFunctionPointer *fn;
  CallRegs *regs;
  RetRegs *ret;
  HopCall hop = {0};
  rs_call_target(record, &fn, &regs, &ret, &hop);
  apus_fill_isolation(regs, false);   // a child Task runs nonisolated, like `async let`
  k->ret = ret;
  if (hop.function) {
    apus_fill_isolation(regs, hop.executor.identity == swift_task_getMainExecutor().identity);
    k->hop = hop;
    k->hop_regs = regs;
    return swift_task_switch(ctx, child_hop_run, hop.executor);
  }
  AsyncContext *callee = swift_task_alloc(fn->expected_context_size);
  callee->parent = ctx;
  callee->resume_parent = (TaskContinuationFunction *)child_after;
  return CALL_GENERIC(code_of(fn), regs, callee);
}

static void spawn_child(RsTaskContext *c, void *record) {
  if (!c->group_initialized) {
    swift_taskGroup_initializeWithFlags(GROUP_DISCARD_RESULTS, c->group, empty_tuple_metadata());
    c->group_initialized = true;
  }
  TaskGroupOption option = { OPTION_KIND_TASK_GROUP, NULL, c->group };
  size_t flags = CREATE_IS_CHILD_TASK | CREATE_ENQUEUE_JOB | CREATE_ADD_PENDING_GROUP_TASK_UNCONDITIONALLY
               | CREATE_IS_DISCARDING_TASK | CREATE_IS_TASK_FUNCTION_CONSUMED;
  AsyncTaskAndContext child = swift_task_create_common(flags, &option, empty_tuple_metadata(), (void *)child_entry,
                                                       record, sizeof(ChildContext));
  rs_set_child(record, child.task);
}

static APUS_SWIFTASYNCCALL void step(APUS_ASYNC_CONTEXT AsyncContext *ctx) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  if (rs_poll(c->rust)) {
    if (c->group_initialized)   // structured concurrency: the parent never completes before its children
      return swift_taskGroup_waitAll(&c->unused_result, ctx, c->group, NULL, after_wait_all, &c->wait.base);
    return finish(ctx);
  }
  const AsyncFunctionPointer *fn;
  CallRegs *regs;
  RetRegs *ret;
  void *record;
  HopCall hop = {0};
  if (rs_take_call(c->rust, &fn, &regs, &ret, &record, &hop)) {
    if (hop.function) {
      // It runs on the target executor: isolated to the main actor only when that is where it hops.
      apus_fill_isolation(regs, hop.executor.identity == swift_task_getMainExecutor().identity);
      c->current_call = record;
      c->call_ret = ret;
      c->hop = hop;
      c->hop_regs = regs;
      return swift_task_switch(ctx, hop_run, hop.executor);
    }
    AsyncContext *callee = swift_task_alloc(fn->expected_context_size);
    callee->parent = ctx;
    callee->resume_parent = (TaskContinuationFunction *)after_call;
    c->current_call = record;
    c->call_ret = ret;
    apus_fill_isolation(regs, c->executor.identity != NULL);   // the main task runs on the main actor
    return CALL_GENERIC(code_of(fn), regs, callee);
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
static APUS_SWIFTASYNCCALL void start(APUS_ASYNC_CONTEXT AsyncContext *ctx, void *rust) {
  RsTaskContext *c = (RsTaskContext *)ctx;
  c->rust = rust;
  c->current_call = NULL;
  c->group_initialized = false;
  c->executor = rs_is_main(rust) ? swift_task_getMainExecutor() : (SerialExecutorRef){0, 0};
  c->cancel_record = swift_task_addCancellationHandler(on_cancel, rust);
  return step(ctx);
}

static APUS_SWIFTASYNCCALL void entry(void *indirect_result, APUS_ASYNC_CONTEXT AsyncContext *ctx, APUS_SWIFT_CONTEXT void *rust) {
  return start(ctx, rust);
}

const void *rs_entry_fn(void) { return (const void *)entry; }

// ---- async main: a Rust task as the `() async throws -> ()` closure `_runAsyncMain` awaits ----
// The caller allocates our context (expected_context_size) and sets its parent and resume function; `finish`
// returns through them with no error, after which `_runAsyncMain` exits the process. It runs once per process, so
// the task rides in a global and the closure has no context object.
static void *async_main_rust;
static AsyncFunctionPointer async_main_function;

static APUS_SWIFTASYNCCALL void async_main_entry(APUS_ASYNC_CONTEXT AsyncContext *ctx, APUS_SWIFT_CONTEXT void *closure_context) {
  return start(ctx, async_main_rust);
}

void apus_run_async_main(void *rust) {
  async_main_rust = rust;
  async_main_function.function = (int32_t)((intptr_t)async_main_entry - (intptr_t)&async_main_function.function);
  async_main_function.expected_context_size = sizeof(RsTaskContext);
  run_async_main(&async_main_function, NULL);
  __builtin_unreachable();
}
size_t rs_context_size(void) { return sizeof(RsTaskContext); }

// ---- plain Jobs (timers): the runtime hands them to dispatch, which calls the metadata's VTableInvoke ----

static APUS_SWIFTASYNCCALL void run_job(void *job) { rs_job_run(job); }
static void invoke_job(void *job, void *unused, uint32_t flags) {
  swift_job_run(job, (SerialExecutorRef){0, 0});   // SerialExecutorRef::generic()
}
static void destroy_job(void *job) {}
static const JobMetadata job_metadata = { 0, destroy_job, 0, METADATA_KIND_JOB, 0, {0, 0, 0}, DISPATCH_SWIFT_OBJECT_TYPE, invoke_job };

const void *rs_job_metadata(void) { return &job_metadata.kind; }
const void *rs_run_job_fn(void) { return (const void *)run_job; }

// ---- synchronous Swift calls through one generic trampoline (src/swift/sync.rs) ----
// Sync functions take an indirect result address in the swift_indirect_result register (x8 / rax), unlike
// async ones (first integer argument). Direct results: up to four integer or four floating-point registers.
// The error register is always passed: a non-throwing callee leaves it alone.
typedef struct { uint64_t r[4]; } Ints4;
typedef struct { double f[4]; } Floats4;
#if defined(__aarch64__)
#define SYNC_PARAMS uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, \
                    double, double, double, double, double, double, double, double
#define SYNC_ARGS(r) (r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], (r)->gpr[6], (r)->gpr[7], \
                     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7]
#else
#define SYNC_PARAMS uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, \
                    double, double, double, double, double, double, double, double
#define SYNC_ARGS(r) (r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], \
                     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7]
#endif
typedef APUS_SWIFTCALL Ints4 SyncFnInts(SYNC_PARAMS, APUS_SWIFT_CONTEXT void *, APUS_SWIFT_ERROR void **);
typedef APUS_SWIFTCALL Floats4 SyncFnFloats(SYNC_PARAMS, APUS_SWIFT_CONTEXT void *, APUS_SWIFT_ERROR void **);
typedef APUS_SWIFTCALL void SyncFnIndirect(APUS_SWIFT_INDIRECT_RESULT void *, SYNC_PARAMS, APUS_SWIFT_CONTEXT void *, APUS_SWIFT_ERROR void **);

// kind: 0 = integer results, 1 = floating-point results, 2 = indirect result at `indirect`
void rs_call_sync(const void *fn, const CallRegs *r, void *indirect, RetRegs *ret, int kind) {
  void *error = NULL;
  if (kind == 2) {
    ((SyncFnIndirect *)fn)(indirect, SYNC_ARGS(r), r->self, &error);
  } else if (kind == 1) {
    Floats4 f = ((SyncFnFloats *)fn)(SYNC_ARGS(r), r->self, &error);
    for (int i = 0; i < 4; i++) ret->fpr[i] = f.f[i];
  } else {
    Ints4 v = ((SyncFnInts *)fn)(SYNC_ARGS(r), r->self, &error);
    for (int i = 0; i < 4; i++) ret->gpr[i] = v.r[i];
  }
  ret->error = error;
}

// `String._fromUTF8Repairing(_: UnsafeBufferPointer<UInt8>) -> (result: String, repairsMade: Bool)`: copies the
// bytes (any length, NUL bytes included). Three result registers, which only a swiftcall declaration can receive.
void rs_string_from_utf8(const uint8_t *start, intptr_t count, SwiftString *out) {
  StringAndFlag s = swift_string_from_utf8_repairing(start, count);
  out->words[0] = s.words[0];
  out->words[1] = s.words[1];
}
