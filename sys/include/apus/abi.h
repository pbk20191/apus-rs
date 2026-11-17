// The Swift runtime ABI as apus uses it: the only place it is written down. The C funclets are compiled against
// this header, and the Rust declarations are generated from it (sys/tests/fresh.rs). Layouts and signatures follow
// swiftlang/swift at APUS_SWIFT_ABI_TAG; each item names the file and declaration it mirrors.
//
// Markers (read by sys/tests/lowering.rs and the generator):
//   APUS_C         a C-convention function: Rust calls it directly
//   APUS_DIRECT    Swift convention, proven to lower like a C call: Rust calls it directly as extern "C"
//   APUS_THUNK     Swift convention that lowers differently: Rust calls the C thunk apus_<name> instead
//   APUS_ASYNC     swiftasynccall: only the funclets call it; not visible to Rust
//   APUS_INTERNAL  Swift convention, called or implemented by the funclets only; not visible to Rust
//   APUS_DATA      a data symbol (type metadata, witness table)
// With APUS_BINDGEN defined (bindgen and the C side of the lowering test) the Swift attributes expand to nothing.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define APUS_SWIFT_ABI_TAG "swift-6.4.2-RELEASE"

// The namespace of the C<->Rust boundary symbols apus defines itself: apus-sys's version and an FNV-1a hash of it
// (checked by sys/tests/fresh.rs). Each boundary function links as apus_<tag>_<C name>, so no other library's symbol
// can clash with it. The C side renames the identifiers (not an asm label: under cross-language LTO an asm label stays
// "\01_name" in the IR and would not meet Rust's "name"); bindgen sees the plain names with their link names.
#define APUS_TAG "0_1_0_e1cf1eb8"
#define APUS_TAG_ID 0_1_0_e1cf1eb8
#define APUS_JOIN_(a, b, c) a##b##_##c
#define APUS_JOIN(a, b, c) APUS_JOIN_(a, b, c)
#define APUS_NAME(name) APUS_JOIN(apus_, APUS_TAG_ID, name)
#ifdef APUS_BINDGEN
#define APUS_BOUNDARY(name) __asm__("_apus_" APUS_TAG "_" #name)
#else
#define APUS_BOUNDARY(name)
#endif
// Only apus's own C (built with APUS_SYS_BUILD) renames: another C file including this header keeps its names.
#if defined(APUS_SYS_BUILD) && !defined(APUS_BINDGEN)
#define rs_entry_fn APUS_NAME(rs_entry_fn)
#define rs_context_size APUS_NAME(rs_context_size)
#define rs_job_metadata APUS_NAME(rs_job_metadata)
#define rs_run_job_fn APUS_NAME(rs_run_job_fn)
#define rs_string_from_utf8 APUS_NAME(rs_string_from_utf8)
#define rs_call_sync APUS_NAME(rs_call_sync)
#define apus_swift_task_isCancelled APUS_NAME(apus_swift_task_isCancelled)
#define rs_poll APUS_NAME(rs_poll)
#define rs_park APUS_NAME(rs_park)
#define rs_finish APUS_NAME(rs_finish)
#define rs_cancelled APUS_NAME(rs_cancelled)
#define rs_is_main APUS_NAME(rs_is_main)
#define rs_take_call APUS_NAME(rs_take_call)
#define rs_take_child APUS_NAME(rs_take_child)
#define rs_call_target APUS_NAME(rs_call_target)
#define rs_set_child APUS_NAME(rs_set_child)
#define rs_call_done APUS_NAME(rs_call_done)
#define rs_job_run APUS_NAME(rs_job_run)
#endif

#ifdef APUS_BINDGEN
#define APUS_SWIFTCALL
#else
#define APUS_SWIFTCALL __attribute__((swiftcall))
#endif
#define APUS_SWIFTASYNCCALL __attribute__((swiftasynccall))
#define APUS_ASYNC_CONTEXT __attribute__((swift_async_context))
#define APUS_SWIFT_CONTEXT __attribute__((swift_context))
#define APUS_SWIFT_ERROR __attribute__((swift_error_result))
#define APUS_SWIFT_INDIRECT_RESULT __attribute__((swift_indirect_result))
#define APUS_SYMBOL(s) __asm__("_" s)

#define APUS_MARK(m) __attribute__((annotate("apus:" m)))
#define APUS_C APUS_MARK("c")
#define APUS_DIRECT APUS_MARK("direct")
#define APUS_THUNK APUS_MARK("thunk")
#define APUS_ASYNC APUS_MARK("async")
#define APUS_INTERNAL APUS_MARK("internal")
#define APUS_DATA APUS_MARK("data")
// Introduced in macOS `v`: weakly linked below that floor. Rust reaches it through apus_sys::optional (a run-time
// lookup), or directly with the `macos15` feature. Kept out of bindings.rs by the generator, not by APUS_BINDGEN.
#define APUS_AVAILABLE(v) __attribute__((availability(macos, introduced = v), weak_import)) APUS_MARK("available:" #v)

// ---- values and records ----

// include/swift/ABI/Executor.h: TaskExecutorRef (macOS 15)
typedef struct { const void *identity; uintptr_t implementation; } TaskExecutorRef;

// include/swift/ABI/Executor.h: SerialExecutorRef
typedef struct { const void *identity; uintptr_t implementation; } SerialExecutorRef;
_Static_assert(sizeof(SerialExecutorRef) == 16, "SerialExecutorRef layout");

// include/swift/Runtime/Concurrency.h: AsyncTaskAndContext
typedef struct { void *task; void *initial_context; } AsyncTaskAndContext;

// include/swift/Runtime/Metadata.h: MetadataResponse
typedef struct { const void *metadata; size_t state; } MetadataResponse;

// include/swift/Runtime/Metadata.h: TypeNamePair
typedef struct { const char *data; size_t length; } TypeNamePair;

// stdlib/public/SwiftShims/swift/shims/HeapObject.h: HeapObject (metadata, inline reference counts)
typedef struct { const void *metadata; uintptr_t refcounts; } HeapObject;
_Static_assert(sizeof(HeapObject) == 16, "HeapObject layout");

// include/swift/ABI/Metadata.h: TypeMetadataHeader, the word just before a metadata's address point.
typedef struct ValueWitnessTable ValueWitnessTable;
typedef struct { const ValueWitnessTable *value_witnesses; } TypeMetadataHeader;

// stdlib/public/core/ContiguousArrayBuffer.swift, ArrayBody.swift: the header of a _ContiguousArrayStorage; the
// elements follow it (no padding for elements aligned to 16 bytes or less).
typedef struct { HeapObject header; intptr_t count; uintptr_t capacity_and_flags; } ContiguousArrayStorageHeader;
_Static_assert(sizeof(ContiguousArrayStorageHeader) == 32, "ContiguousArrayStorageHeader layout");

// A Swift String value: two words, frozen layout (stdlib/public/core/StringObject.swift).
typedef struct { uint64_t words[2]; } SwiftString;

// What `_allocateUninitializedArray` returns: the array and the address of its first element.
typedef struct { void *array; void *elements; } ArrayAndElements;

// include/swift/Runtime/Error.h: ErrorValueResult (swift_getErrorValue's out parameter)
typedef struct { const void *value; const void *type; const void *error_conformance; } ErrorValueResult;

// include/swift/ABI/TaskOptions.h: TaskOptionRecord + InitialSerialExecutorTaskOptionRecord
typedef struct { size_t flags; void *parent; SerialExecutorRef executor; } InitialSerialExecutorOption;

// lib/IRGen: AsyncFunctionPointer = { relative offset to the code, callee context size }.
typedef struct { int32_t function; uint32_t expected_context_size; } AsyncFunctionPointer;

// `final class AsyncSwiftClosure { let body: () async throws -> Any }` (the README recipe): the object header, then the
// thick function: the async function pointer and its context.
typedef struct { HeapObject header; const AsyncFunctionPointer *function; void *context; } AsyncClosureBox;

// One call shape for every Swift function apus calls by symbol: Rust fills the argument registers, the callee reads
// the ones its signature uses (integer argument registers: 8 on arm64, 6 on x86_64). Results: up to four integer or
// four floating-point registers, and the error the callee threw (src/swift/call.rs, sync.rs).
typedef struct { uint64_t gpr[8]; double fpr[8]; void *self; } CallRegs;
typedef struct { uint64_t gpr[4]; double fpr[4]; void *error; } RetRegs;

// ---- constants (include/swift/ABI/MetadataValues.h, MetadataKind.def) ----

// TaskCreateFlags: priority in bits 0-7.
#define CREATE_IS_CHILD_TASK (1u << 8)
#define CREATE_COPY_TASK_LOCALS (1u << 10)
#define CREATE_ENQUEUE_JOB (1u << 12)
#define CREATE_ADD_PENDING_GROUP_TASK_UNCONDITIONALLY (1u << 13)
#define CREATE_IS_DISCARDING_TASK (1u << 14)
#define CREATE_IS_TASK_FUNCTION_CONSUMED (1u << 15)
// TaskOptionRecordKind
#define OPTION_KIND_INITIAL_SERIAL_EXECUTOR 0u
#define OPTION_KIND_TASK_GROUP 1u
// AsyncContinuationFlags
#define CONT_HAS_EXECUTOR_OVERRIDE (1u << 1)
// TaskGroupFlags::TaskGroup_DiscardResults
#define GROUP_DISCARD_RESULTS (1u << 8)
// NumWords_TaskGroup
#define NUM_WORDS_TASK_GROUP 32u
// MetadataKind::Tuple = 1 | RuntimePrivate | NonHeap; MetadataKind::Struct = 0 | NonHeap.
#define METADATA_KIND_TUPLE 0x301u
#define METADATA_KIND_STRUCT 0x200u

// ---- value witnesses (include/swift/ABI/ValueWitness.def) ----

typedef APUS_SWIFTCALL void ValueWitnessDestroy(void *value, const void *metadata) APUS_DIRECT;
typedef APUS_SWIFTCALL void *ValueWitnessInitializeWithCopy(void *dest, void *src, const void *metadata) APUS_DIRECT;
typedef APUS_SWIFTCALL void *ValueWitnessInitializeWithTake(void *dest, void *src, const void *metadata) APUS_DIRECT;
typedef APUS_SWIFTCALL unsigned ValueWitnessGetEnumTag(const void *value, const void *metadata) APUS_DIRECT;

// ValueWitnessTable, followed for enums by the enum witnesses (EnumValueWitnessTable). TypeMetadataHeader points at it.
struct ValueWitnessTable {
  const void *initialize_buffer_with_copy_of_buffer;
  ValueWitnessDestroy *destroy;
  ValueWitnessInitializeWithCopy *initialize_with_copy;
  const void *assign_with_copy;
  ValueWitnessInitializeWithTake *initialize_with_take;
  const void *assign_with_take;
  const void *get_enum_tag_single_payload;
  const void *store_enum_tag_single_payload;
  size_t size;
  size_t stride;
  uint32_t flags;                  // low 8 bits: alignment mask
  uint32_t extra_inhabitant_count;
  ValueWitnessGetEnumTag *get_enum_tag;   // EnumValueWitnessTable only
};

// ---- swiftCore: C-convention runtime functions (include/swift/Runtime/HeapObject.h, Casting.h, Error.h) ----

extern void *swift_retain(void *object) APUS_C;
extern void swift_release(void *object) APUS_C;
extern void swift_errorRelease(void *error) APUS_C;
extern void swift_bridgeObjectRelease(void *object) APUS_C;
extern const void *swift_conformsToProtocol(const void *metadata, const void *protocol) APUS_C;
extern bool swift_dynamicCast(void *dest, void *src, const void *src_type, const void *target_type, size_t flags) APUS_C;
extern void swift_getErrorValue(const void *error, void **scratch, ErrorValueResult *out) APUS_C;
extern const void *swift_getExistentialTypeMetadata(bool class_constraint_any, const void *superclass, size_t count,
                                                    const void *protocols) APUS_C;

// ---- swiftCore: Swift-convention runtime functions (include/swift/Runtime/Metadata.h) ----

extern APUS_SWIFTCALL MetadataResponse swift_getTupleTypeMetadata(size_t request, size_t flags, const void *const *elements,
                                                                  const char *labels, const void *proposed_witnesses) APUS_DIRECT;
extern APUS_SWIFTCALL TypeNamePair swift_getTypeName(const void *metadata, bool qualified) APUS_DIRECT;
extern APUS_SWIFTCALL const void *swift_getTypeByMangledNameInContext(const char *name, size_t length, const void *context,
                                                                       const void *const *generic_arguments) APUS_DIRECT;
// `base` is the protocol's requirement base descriptor (`…TL`), `associated_type` the associated type's descriptor (`…Tl`).
extern APUS_SWIFTCALL MetadataResponse swift_getAssociatedTypeWitness(size_t request, const void *witness_table,
                                                                      const void *conforming_type, const void *base,
                                                                      const void *associated_type) APUS_DIRECT;

// ---- swiftCore: standard library entry points by mangled name ----

// Optional<T>, Array<T>, Dictionary<K, V> metadata accessors; the last argument of the dictionary one is K's Hashable
// witness table.
extern APUS_SWIFTCALL MetadataResponse optional_metadata(size_t request, const void *wrapped) APUS_SYMBOL("$sSqMa") APUS_DIRECT;
extern APUS_SWIFTCALL MetadataResponse array_metadata(size_t request, const void *element) APUS_SYMBOL("$sSaMa") APUS_DIRECT;
extern APUS_SWIFTCALL MetadataResponse dictionary_metadata(size_t request, const void *key, const void *value,
                                                           const void *key_hashable) APUS_SYMBOL("$sSDMa") APUS_DIRECT;
// `_allocateUninitializedArray<T>(_: Builtin.Word) -> ([T], Builtin.RawPointer)`: what array literals compile to.
extern APUS_SWIFTCALL ArrayAndElements allocate_uninitialized_array(size_t count, const void *element)
    APUS_SYMBOL("$ss27_allocateUninitializedArrayySayxG_BptBwlF") APUS_DIRECT;
// `String.utf8CString` getter: a +1 `ContiguousArray<Int8>` (NUL-terminated).
extern APUS_SWIFTCALL void *string_utf8_c_string(uint64_t w0, uint64_t w1)
    APUS_SYMBOL("$sSS11utf8CStrings15ContiguousArrayVys4Int8VGvg") APUS_DIRECT;
// `String.init<T>(describing: T)`: takes the value +1 (indirect), then T's metadata.
extern APUS_SWIFTCALL SwiftString string_describing(void *value, const void *metadata)
    APUS_SYMBOL("$sSS10describingSSx_tclufC") APUS_DIRECT;

// Type metadata (`…N`, at the address point) and Hashable witness tables (`…SHsWP`).
extern const uint8_t INT_METADATA APUS_SYMBOL("$sSiN") APUS_DATA;
extern const uint8_t INT32_METADATA APUS_SYMBOL("$ss5Int32VN") APUS_DATA;
extern const uint8_t UINT64_METADATA APUS_SYMBOL("$ss6UInt64VN") APUS_DATA;
extern const uint8_t DOUBLE_METADATA APUS_SYMBOL("$sSdN") APUS_DATA;
extern const uint8_t BOOL_METADATA APUS_SYMBOL("$sSbN") APUS_DATA;
extern const uint8_t STRING_METADATA APUS_SYMBOL("$sSSN") APUS_DATA;
extern const uint8_t STRING_HASHABLE APUS_SYMBOL("$sSSSHsWP") APUS_DATA;
extern const uint8_t INT_HASHABLE APUS_SYMBOL("$sSiSHsWP") APUS_DATA;
extern const uint8_t INT32_HASHABLE APUS_SYMBOL("$ss5Int32VSHsWP") APUS_DATA;
extern const uint8_t UINT64_HASHABLE APUS_SYMBOL("$ss6UInt64VSHsWP") APUS_DATA;

// ---- swift_Concurrency (include/swift/Runtime/Concurrency.h) ----

extern APUS_SWIFTCALL AsyncTaskAndContext swift_task_create_common(size_t flags, void *options, const void *result_type,
                                                                   const void *function, void *closure_context,
                                                                   size_t initial_context_size) APUS_DIRECT;
extern APUS_SWIFTCALL SerialExecutorRef swift_task_getMainExecutor(void) APUS_DIRECT;
extern APUS_SWIFTCALL void swift_continuation_resume(void *task) APUS_DIRECT;
extern APUS_SWIFTCALL void swift_task_cancel(void *task) APUS_DIRECT;
extern APUS_SWIFTCALL void *swift_task_getCurrent(void) APUS_DIRECT;
extern APUS_SWIFTCALL size_t swift_task_currentPriority(void *task) APUS_DIRECT;
extern APUS_SWIFTCALL size_t swift_task_escalate(void *task, size_t priority) APUS_DIRECT;
extern APUS_SWIFTCALL void swift_task_enqueueGlobalWithDelay(unsigned long long delay_ns, void *job) APUS_DIRECT;

// macOS 15 (Swift 6.0): task executor preference (SE-0417).
extern APUS_SWIFTCALL TaskExecutorRef swift_task_getPreferredTaskExecutor(void) APUS_DIRECT APUS_AVAILABLE(15.0);

// ---- funclets: C functions in sys/csrc that Rust calls ----

const void *rs_entry_fn(void) APUS_BOUNDARY(rs_entry_fn) APUS_C;
size_t rs_context_size(void) APUS_BOUNDARY(rs_context_size) APUS_C;
const void *rs_job_metadata(void) APUS_BOUNDARY(rs_job_metadata) APUS_C;
const void *rs_run_job_fn(void) APUS_BOUNDARY(rs_run_job_fn) APUS_C;
void rs_string_from_utf8(const uint8_t *start, intptr_t count, SwiftString *out) APUS_BOUNDARY(rs_string_from_utf8) APUS_C;
// kind: 0 = integer results, 1 = floating-point results, 2 = indirect result at `indirect`
void rs_call_sync(const void *function, const CallRegs *regs, void *indirect, RetRegs *ret, int kind) APUS_BOUNDARY(rs_call_sync) APUS_C;

// ---- thunks (sys/csrc/thunks.c): C-convention entry points for APUS_THUNK functions ----

// swift_task_isCancelled: its Bool result is not extended to a full register as a C uint8_t return would be.
uint8_t apus_swift_task_isCancelled(void *task) APUS_BOUNDARY(apus_swift_task_isCancelled) APUS_C;

#ifndef APUS_BINDGEN
// ---- for the funclets and thunks only: not visible to Rust ----

// Returns Bool: only bit 0 is defined.
extern APUS_SWIFTCALL uint8_t swift_task_isCancelled(void *task) APUS_THUNK;

// include/swift/ABI/Task.h: AsyncContext, TaskContinuationFunction
typedef struct AsyncContext AsyncContext;
typedef APUS_SWIFTASYNCCALL void TaskContinuationFunction(APUS_ASYNC_CONTEXT AsyncContext *) APUS_ASYNC;
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
_Static_assert(sizeof(TaskFutureWaitAsyncContext) == 48, "TaskFutureWaitAsyncContext layout");

// Return continuation of a throwing async function / task completion: (context, error).
typedef APUS_SWIFTASYNCCALL void ThrowingContinuation(APUS_ASYNC_CONTEXT AsyncContext *, APUS_SWIFT_CONTEXT void *error) APUS_ASYNC;
// include/swift/ABI/TaskOptions.h: TaskOptionRecord + TaskGroupTaskOptionRecord
typedef struct { size_t flags; void *parent; void *group; } TaskGroupOption;

// A plain Job's metadata (timers): the runtime hands such jobs to dispatch, which calls VTableInvoke. Job header:
// include/swift/ABI/Task.h (class Job). Shape: FullMetadata<DispatchClassMetadata> as in
// stdlib/public/Concurrency/Task.cpp (jobHeapMetadata, not exported).
typedef struct {
  void *type_layout; void (*destroy)(void *); void *value_witnesses;            // FullMetadata prefix
  uintptr_t kind; void *opaque; void *opaque_objc[3];                           // HeapMetadata + DispatchClassMetadata
  unsigned long vtable_type; void (*vtable_invoke)(void *, void *, uint32_t);
} JobMetadata;
// MetadataKind::Job = 3 | NonType | RuntimePrivate (ABI/MetadataKind.def); dispatchSwiftObjectType.
#define METADATA_KIND_JOB (3u | 0x400u | 0x100u)
#define DISPATCH_SWIFT_OBJECT_TYPE 1u

// A cancellation handler: the runtime calls it with the context it was registered with.
typedef APUS_SWIFTCALL void CancelFn(APUS_SWIFT_CONTEXT void *) APUS_INTERNAL;

// The generic async call shape (call.rs): every integer and floating-point argument register, then the async
// context and self.
#if defined(__aarch64__)
typedef APUS_SWIFTASYNCCALL void GenericAsyncFn(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t,
                                                double, double, double, double, double, double, double, double,
                                                APUS_ASYNC_CONTEXT AsyncContext *, APUS_SWIFT_CONTEXT void *) APUS_ASYNC;
#define CALL_GENERIC(fn, r, callee) \
  fn((r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], (r)->gpr[6], (r)->gpr[7], \
     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7], callee, (r)->self)
#else
typedef APUS_SWIFTASYNCCALL void GenericAsyncFn(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t,
                                                double, double, double, double, double, double, double, double,
                                                APUS_ASYNC_CONTEXT AsyncContext *, APUS_SWIFT_CONTEXT void *) APUS_ASYNC;
#define CALL_GENERIC(fn, r, callee) \
  fn((r)->gpr[0], (r)->gpr[1], (r)->gpr[2], (r)->gpr[3], (r)->gpr[4], (r)->gpr[5], \
     (r)->fpr[0], (r)->fpr[1], (r)->fpr[2], (r)->fpr[3], (r)->fpr[4], (r)->fpr[5], (r)->fpr[6], (r)->fpr[7], callee, (r)->self)
#endif

extern APUS_SWIFTCALL void *swift_continuation_init(ContinuationAsyncContext *context, size_t flags) APUS_INTERNAL;
extern APUS_SWIFTASYNCCALL void swift_continuation_await(ContinuationAsyncContext *context) APUS_ASYNC;
extern APUS_SWIFTASYNCCALL void swift_task_switch(APUS_ASYNC_CONTEXT AsyncContext *resume_context,
                                                  TaskContinuationFunction *resume, SerialExecutorRef executor) APUS_ASYNC;
extern APUS_SWIFTCALL void *swift_task_alloc(size_t size) APUS_INTERNAL;
extern APUS_SWIFTCALL void swift_task_dealloc(void *pointer) APUS_INTERNAL;
extern APUS_SWIFTCALL void *swift_task_addCancellationHandler(CancelFn *handler, void *context) APUS_INTERNAL;
extern APUS_SWIFTCALL void swift_task_removeCancellationHandler(void *record) APUS_INTERNAL;
extern APUS_SWIFTCALL void swift_taskGroup_initializeWithFlags(size_t flags, void *group, const void *result_type) APUS_INTERNAL;
extern APUS_SWIFTCALL void swift_taskGroup_destroy(void *group) APUS_INTERNAL;
extern APUS_SWIFTASYNCCALL void swift_taskGroup_waitAll(void *result, APUS_ASYNC_CONTEXT AsyncContext *caller, void *group,
                                                        void *body_error, ThrowingContinuation *resume,
                                                        AsyncContext *call_context) APUS_ASYNC;
extern APUS_SWIFTCALL void swift_job_run(void *job, SerialExecutorRef executor) APUS_INTERNAL;

// `String._fromUTF8Repairing(_: UnsafeBufferPointer<UInt8>) -> (result: String, repairsMade: Bool)`: copies the bytes
// (any length, NUL bytes included). Three result registers, which only a swiftcall declaration can receive; Rust
// reaches it through rs_string_from_utf8.
typedef struct { uint64_t words[2]; uint8_t repairs_made; } StringAndFlag;
extern APUS_SWIFTCALL StringAndFlag swift_string_from_utf8_repairing(const uint8_t *start, intptr_t count)
    APUS_SYMBOL("$sSS18_fromUTF8RepairingySS6result_Sb11repairsMadetSRys5UInt8VGFZ") APUS_INTERNAL;

// ---- implemented in Rust (src/swift/task.rs, call.rs, timer.rs), called by the funclets ----
int rs_poll(void *rust);                                                       // 1 = Ready
int rs_park(void *rust, void *continuation);                                   // 1 = parked, 0 = woken mid-poll
void rs_finish(void *rust);
void rs_cancelled(void *rust);
bool rs_is_main(void *rust);
int rs_take_call(void *rust, const AsyncFunctionPointer **function, CallRegs **regs, RetRegs **ret, void **record);  // 1 = run it in this Task
int rs_take_child(void *rust, void **record);                                  // 1 = run it as a child Task
void rs_call_target(void *record, const AsyncFunctionPointer **function, CallRegs **regs, RetRegs **ret);
void rs_set_child(void *record, void *child_task);                             // +1 child task
void rs_call_done(void *record);                                               // results are in the call's RetRegs
void rs_job_run(void *job);
#endif
