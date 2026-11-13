//! Swift runtime functions called directly from Rust.
//!
//! They are `SWIFT_CC(swift)`, declared here as `extern "C"`: for these signatures clang lowers both
//! conventions identically on arm64 and x86_64 (tests/abi_equivalence.rs, list in rt_sigs.txt). Swift-CC `bool`
//! returns define only bit 0, so they are read as `u8` and masked. Signatures: include/swift/Runtime/Concurrency.h,
//! Runtime/Metadata.h @ swift-6.4.2-RELEASE.

// A binding table: some entries are only used by later features (cancellation, escalation, calls).
#![allow(dead_code)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AsyncTaskAndContext {
    pub task: *mut c_void,
    pub initial_context: *mut c_void,
}

/// include/swift/ABI/Executor.h
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SerialExecutorRef {
    pub identity: *const c_void,
    pub implementation: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetadataResponse {
    pub metadata: *const c_void,
    pub state: usize,
}

/// `TaskOptionRecord` + `InitialSerialExecutorTaskOptionRecord` (include/swift/ABI/TaskOptions.h).
#[repr(C)]
pub struct InitialSerialExecutorOption {
    pub flags: usize,
    pub parent: *mut c_void,
    pub executor: SerialExecutorRef,
}

// TaskCreateFlags (include/swift/ABI/MetadataValues.h): priority in bits 0-7.
pub const CREATE_COPY_TASK_LOCALS: usize = 1 << 10;
pub const CREATE_ENQUEUE_JOB: usize = 1 << 12;
pub const CREATE_IS_TASK_FUNCTION_CONSUMED: usize = 1 << 15;
// TaskOptionRecordKind (include/swift/ABI/MetadataValues.h).
pub const OPTION_KIND_INITIAL_SERIAL_EXECUTOR: usize = 0;
// MetadataKind::Tuple = 1 | RuntimePrivate | NonHeap (include/swift/ABI/MetadataKind.def).
pub const METADATA_KIND_TUPLE: usize = 0x301;

#[link(name = "swiftCore")]
extern "C" {
    pub fn swift_retain(object: *mut c_void) -> *mut c_void;
    pub fn swift_release(object: *mut c_void);
    pub fn swift_getTupleTypeMetadata(
        request: usize,
        flags: usize,
        elements: *const *const c_void,
        labels: *const c_char,
        proposed_witnesses: *const c_void,
    ) -> MetadataResponse;
}

#[link(name = "swift_Concurrency")]
extern "C" {
    pub fn swift_task_create_common(
        flags: usize,
        options: *mut c_void,
        result_type: *const c_void,
        function: *const c_void,
        closure_context: *mut c_void,
        initial_context_size: usize,
    ) -> AsyncTaskAndContext;
    pub fn swift_task_getMainExecutor() -> SerialExecutorRef;
    pub fn swift_continuation_resume(task: *mut c_void);
    pub fn swift_task_cancel(task: *mut c_void);
    pub fn swift_task_getCurrent() -> *mut c_void;
    pub fn swift_task_isCancelled(task: *mut c_void) -> u8;
    pub fn swift_task_currentPriority(task: *mut c_void) -> usize;
    pub fn swift_task_escalate(task: *mut c_void, priority: usize) -> usize;
}

/// Every runtime symbol this crate binds, Rust side and funclets. The self-check resolves each one.
pub const REQUIRED_SYMBOLS: &[&str] = &[
    "swift_retain", "swift_release", "swift_getTupleTypeMetadata",
    "swift_task_create_common", "swift_task_getMainExecutor", "swift_continuation_resume", "swift_task_cancel",
    "swift_task_getCurrent", "swift_task_isCancelled", "swift_task_currentPriority", "swift_task_escalate",
    "swift_continuation_init", "swift_continuation_await", "swift_task_switch", "swift_task_alloc",
    "swift_task_dealloc", "swift_task_addCancellationHandler", "swift_task_removeCancellationHandler",
    "swift_taskGroup_initializeWithFlags", "swift_taskGroup_destroy", "swift_taskGroup_waitAll",
];

pub fn is_cancelled(task: *mut c_void) -> bool {
    unsafe { swift_task_isCancelled(task) & 1 == 1 }
}

/// Metadata for `()`. Not the `$sytN` symbol: that is the full metadata (prefix included).
pub fn empty_tuple_metadata() -> *const c_void {
    struct Ptr(*const c_void);
    unsafe impl Send for Ptr {}
    unsafe impl Sync for Ptr {}
    static METADATA: OnceLock<Ptr> = OnceLock::new();
    METADATA
        .get_or_init(|| Ptr(unsafe { swift_getTupleTypeMetadata(0, 0, std::ptr::null(), std::ptr::null(), std::ptr::null()) }.metadata))
        .0
}

// Funclets (csrc/funclets.c).
extern "C" {
    pub fn rs_entry_fn() -> *const c_void;
    pub fn rs_context_size() -> usize;
}
