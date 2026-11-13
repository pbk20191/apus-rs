// Swift Concurrency ABI pieces used by the funclets. Source: swiftlang/swift @ swift-6.4.2-RELEASE.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SWIFTCALL __attribute__((swiftcall))
#define SWIFTASYNCCALL __attribute__((swiftasynccall))
#define ACTX __attribute__((swift_async_context))
#define SCTX __attribute__((swift_context))

// include/swift/ABI/Executor.h
typedef struct { const void *identity; uintptr_t implementation; } SerialExecutorRef;

// include/swift/ABI/Task.h: AsyncContext, TaskContinuationFunction
typedef struct AsyncContext AsyncContext;
typedef SWIFTASYNCCALL void TaskContinuationFunction(ACTX AsyncContext *);
struct __attribute__((aligned(16))) AsyncContext {
  AsyncContext *parent;
  TaskContinuationFunction *resume_parent;
};

// include/swift/ABI/Task.h: ContinuationAsyncContext
typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  size_t flags;
  size_t await_synchronization;
  void *error_result;
  void *normal_result;
  SerialExecutorRef resume_to_executor;
} ContinuationAsyncContext;
_Static_assert(sizeof(ContinuationAsyncContext) == 64, "ContinuationAsyncContext layout");

// stdlib/public/Concurrency/TaskPrivate.h: TaskFutureWaitAsyncContext (base + 3 words)
typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  void *error_result;
  void *success_result;
  void *reserved;
} TaskFutureWaitAsyncContext;

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
  TaskFutureWaitAsyncContext wait;
  void *group[32] __attribute__((aligned(16)));   // TaskGroup storage: NumWords_TaskGroup = 32 (ABI/MetadataValues.h)
} RsTaskContext;

typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  void *record;
  void *ret;                          // RetRegs of the child's call
} ChildContext;

// lib/IRGen: AsyncFunctionPointer = { relative offset to the code, callee context size }.
typedef struct { int32_t function; uint32_t expected_context_size; } AsyncFunctionPointer;

// One call shape for every Swift async function: Rust fills the argument registers (src/swift/call.rs),
// the callee reads the ones its signature uses. Integer argument registers: 8 on arm64, 6 on x86_64.
typedef struct { uint64_t gpr[8]; double fpr[8]; void *self; } CallRegs;
// Direct results arrive as arguments of the return continuation; the error (if the callee throws) in swift_context.
typedef struct { uint64_t gpr[4]; double fpr[4]; void *error; } RetRegs;
#if defined(__aarch64__)
typedef SWIFTASYNCCALL void GenericAsyncFn(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t,
                                           double, double, double, double, double, double, double, double,
                                           ACTX AsyncContext *, SCTX void *);
#define CALL_GENERIC(fn, r, callee) \
  fn((r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], (r)->gpr[6], (r)->gpr[7], \
     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7], callee, (r)->self)
#else
typedef SWIFTASYNCCALL void GenericAsyncFn(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t,
                                           double, double, double, double, double, double, double, double,
                                           ACTX AsyncContext *, SCTX void *);
#define CALL_GENERIC(fn, r, callee) \
  fn((r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], \
     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7], callee, (r)->self)
#endif
// Return continuation of a throwing async function / task completion: (context, error).
typedef SWIFTASYNCCALL void ThrowingContinuation(ACTX AsyncContext *, SCTX void *error);
