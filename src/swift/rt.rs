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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypeNamePair {
    pub data: *const u8,
    pub length: usize,
}

/// A Swift `String` value (2 words, frozen layout).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwiftString {
    pub words: [u64; 2],
}

/// `ErrorValueResult` (include/swift/Runtime/Error.h).
#[repr(C)]
pub struct ErrorValueResult {
    pub value: *const c_void,
    pub type_: *const c_void,
    pub error_conformance: *const c_void,
}

/// `ValueWitnessTable` (include/swift/ABI/ValueWitness.def), followed for enums by the enum witnesses.
/// Witnesses are Swift-CC; their shapes are covered by tests/abi_equivalence.rs (`vwt_*`).
#[repr(C)]
pub struct ValueWitnessTable {
    pub initialize_buffer_with_copy_of_buffer: *const c_void,
    pub destroy: unsafe extern "C" fn(value: *mut c_void, metadata: *const c_void),
    pub initialize_with_copy: unsafe extern "C" fn(dest: *mut c_void, src: *mut c_void, metadata: *const c_void) -> *mut c_void,
    pub assign_with_copy: *const c_void,
    pub initialize_with_take: *const c_void,
    pub assign_with_take: *const c_void,
    pub get_enum_tag_single_payload: *const c_void,
    pub store_enum_tag_single_payload: *const c_void,
    pub size: usize,
    pub stride: usize,
    pub flags: u32, // low 8 bits: alignment mask
    pub extra_inhabitant_count: u32,
    // EnumValueWitnessTable only:
    pub get_enum_tag: unsafe extern "C" fn(value: *const c_void, metadata: *const c_void) -> u32,
}

/// The value witness table sits one word before a metadata's address point.
pub unsafe fn value_witnesses<'a>(metadata: *const c_void) -> &'a ValueWitnessTable {
    &**(metadata as *const *const ValueWitnessTable).sub(1)
}

#[link(name = "swiftCore")]
extern "C" {
    pub fn swift_retain(object: *mut c_void) -> *mut c_void; // C-CC
    pub fn swift_release(object: *mut c_void); // C-CC
    pub fn swift_errorRelease(error: *mut c_void); // C-CC
    pub fn swift_dynamicCast(dest: *mut c_void, src: *mut c_void, src_type: *const c_void, target_type: *const c_void, flags: usize) -> bool; // C-CC
    pub fn swift_getErrorValue(error: *const c_void, scratch: *mut *mut c_void, out: *mut ErrorValueResult); // C-CC
    pub fn swift_getExistentialTypeMetadata(class_constraint_any: bool, superclass: *const c_void, count: usize, protocols: *const c_void) -> *const c_void; // C-CC
    pub fn swift_getTypeName(metadata: *const c_void, qualified: bool) -> TypeNamePair;
    /// `Optional<T>` metadata accessor.
    #[link_name = "$sSqMa"]
    pub fn optional_metadata(request: usize, wrapped: *const c_void) -> MetadataResponse;
    /// `Array<T>` metadata accessor.
    #[link_name = "$sSaMa"]
    pub fn array_metadata(request: usize, element: *const c_void) -> MetadataResponse;
    /// `Dictionary<K, V>` metadata accessor; the last argument is K's `Hashable` witness table.
    #[link_name = "$sSDMa"]
    pub fn dictionary_metadata(request: usize, key: *const c_void, value: *const c_void, key_hashable: *const c_void) -> MetadataResponse;
    #[link_name = "$sSSSHsWP"]
    pub static STRING_HASHABLE: u8;
    #[link_name = "$sSiSHsWP"]
    pub static INT_HASHABLE: u8;
    #[link_name = "$ss5Int32VSHsWP"]
    pub static INT32_HASHABLE: u8;
    #[link_name = "$ss6UInt64VSHsWP"]
    pub static UINT64_HASHABLE: u8;
    /// `String.utf8CString` getter: a +1 `ContiguousArray<Int8>` (NUL-terminated).
    #[link_name = "$sSS11utf8CStrings15ContiguousArrayVys4Int8VGvg"]
    pub fn string_utf8_c_string(w0: u64, w1: u64) -> *mut c_void;
    /// `String.init<T>(describing: T)`: takes the value +1 (indirect), then T's metadata.
    #[link_name = "$sSS10describingSSx_tclufC"]
    pub fn string_describing(value: *mut c_void, metadata: *const c_void) -> SwiftString;
    #[link_name = "$sSiN"]
    pub static INT_METADATA: u8;
    #[link_name = "$ss5Int32VN"]
    pub static INT32_METADATA: u8;
    #[link_name = "$ss6UInt64VN"]
    pub static UINT64_METADATA: u8;
    #[link_name = "$sSdN"]
    pub static DOUBLE_METADATA: u8;
    #[link_name = "$sSbN"]
    pub static BOOL_METADATA: u8;
    #[link_name = "$sSSN"]
    pub static STRING_METADATA: u8;
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
    pub fn swift_task_enqueueGlobalWithDelay(delay_ns: u64, job: *mut c_void);
}

/// Every runtime symbol this crate binds, Rust side and funclets. The self-check resolves each one.
pub const REQUIRED_SYMBOLS: &[&str] = &[
    "swift_retain", "swift_release", "swift_getTupleTypeMetadata",
    "swift_task_create_common", "swift_task_getMainExecutor", "swift_continuation_resume", "swift_task_cancel",
    "swift_task_getCurrent", "swift_task_isCancelled", "swift_task_currentPriority", "swift_task_escalate",
    "swift_continuation_init", "swift_continuation_await", "swift_task_switch", "swift_task_alloc",
    "swift_task_dealloc", "swift_task_addCancellationHandler", "swift_task_removeCancellationHandler",
    "swift_taskGroup_initializeWithFlags", "swift_taskGroup_destroy", "swift_taskGroup_waitAll",
    "swift_task_enqueueGlobalWithDelay", "swift_job_run",
    "swift_errorRelease", "swift_dynamicCast", "swift_getErrorValue", "swift_getExistentialTypeMetadata",
    "swift_getTypeName", "$sSqMa", "$sSS11utf8CStrings15ContiguousArrayVys4Int8VGvg", "$sSS10describingSSx_tclufC",
    "$sSiN", "$ss5Int32VN", "$ss6UInt64VN", "$sSdN", "$sSbN", "$sSSN",
    "$sSaMa", "$sSDMa", "$sSSSHsWP", "$sSiSHsWP", "$ss5Int32VSHsWP", "$ss6UInt64VSHsWP",
];

// MetadataKind::Struct = 0 | NonHeap (include/swift/ABI/MetadataKind.def). Nominal `…N` symbols point at the
// address point (unlike `$sytN`); the self-check verifies that by reading the kind.
pub const METADATA_KIND_STRUCT: usize = 0x200;

pub fn is_cancelled(task: *mut c_void) -> bool {
    unsafe { swift_task_isCancelled(task) & 1 == 1 }
}

pub fn metadata_of(symbol: &'static u8) -> *const c_void {
    symbol as *const u8 as *const c_void
}

/// Metadata for `Any` (an existential with no protocols and no class constraint).
pub fn any_metadata() -> *const c_void {
    struct Ptr(*const c_void);
    unsafe impl Send for Ptr {}
    unsafe impl Sync for Ptr {}
    static METADATA: OnceLock<Ptr> = OnceLock::new();
    METADATA
        .get_or_init(|| Ptr(unsafe { swift_getExistentialTypeMetadata(true, std::ptr::null(), 0, std::ptr::null()) }))
        .0
}

/// Copies a Swift `String` out as UTF-8. Does not consume `s`.
pub unsafe fn string_to_rust(s: &SwiftString) -> String {
    let array = string_utf8_c_string(s.words[0], s.words[1]);
    // _ContiguousArrayStorage: HeapObject (16) + _ArrayBody { count, capacityAndFlags } (16), elements at 32.
    let count = *(array.byte_add(16) as *const isize) as usize;
    let bytes = std::slice::from_raw_parts(array.byte_add(32) as *const u8, count.saturating_sub(1));
    let out = String::from_utf8_lossy(bytes).into_owned();
    swift_release(array);
    out
}

/// Destroys a Swift `String` value.
pub unsafe fn destroy_string(s: &mut SwiftString) {
    let metadata = metadata_of(&STRING_METADATA);
    (value_witnesses(metadata).destroy)((s as *mut SwiftString).cast(), metadata);
}

pub unsafe fn type_name(metadata: *const c_void) -> String {
    let pair = swift_getTypeName(metadata, true);
    String::from_utf8_lossy(std::slice::from_raw_parts(pair.data, pair.length)).into_owned()
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
    pub fn rs_job_metadata() -> *const c_void;
    pub fn rs_run_job_fn() -> *const c_void;
}
