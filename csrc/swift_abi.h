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
  bool group_initialized;
  TaskFutureWaitAsyncContext wait;
  void *group[32] __attribute__((aligned(16)));   // TaskGroup storage: NumWords_TaskGroup = 32 (ABI/MetadataValues.h)
} RsTaskContext;

typedef struct __attribute__((aligned(16))) {
  AsyncContext base;
  void *record;
} ChildContext;

// lib/IRGen: AsyncFunctionPointer = { relative offset to the code, callee context size }.
typedef struct { int32_t function; uint32_t expected_context_size; } AsyncFunctionPointer;

// The user-side box `final class AsyncSwiftClosure { let body: () async throws -> Any }` (see README):
// HeapObject header, then the thick function (function word → AsyncFunctionPointer, context word).
typedef struct { void *metadata; uint64_t refcounts; const AsyncFunctionPointer *fn; void *context; } ClosureBox;

// Thick `() async throws -> Any`: (indirect Any result, callee context, closure context +0).
typedef SWIFTASYNCCALL void AnyClosureFn(void *indirect_any, ACTX AsyncContext *callee, SCTX void *closure_context);
// Return continuation of a throwing async function / task completion: (context, error).
typedef SWIFTASYNCCALL void ThrowingContinuation(ACTX AsyncContext *, SCTX void *error);
