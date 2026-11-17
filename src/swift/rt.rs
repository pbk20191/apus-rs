//! Safe helpers over the Swift runtime ABI. The declarations themselves (functions, records, constants, data
//! symbols) come from apus-sys, generated from its include/apus/abi.h; `rt::X` names them all.

pub(crate) use apus_sys::*;
use std::ffi::c_void;
use std::sync::OnceLock;

/// The value witness table sits one word before a metadata's address point.
pub unsafe fn value_witnesses<'a>(metadata: *const c_void) -> &'a ValueWitnessTable {
    &**(metadata as *const *const ValueWitnessTable).sub(1)
}

pub fn is_cancelled(task: *mut c_void) -> bool {
    unsafe { apus_swift_task_isCancelled(task) & 1 == 1 }
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
    value_witnesses(metadata).destroy.unwrap()((s as *mut SwiftString).cast(), metadata);
}

pub unsafe fn type_name(metadata: *const c_void) -> String {
    let pair = swift_getTypeName(metadata, true);
    String::from_utf8_lossy(std::slice::from_raw_parts(pair.data.cast::<u8>(), pair.length)).into_owned()
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

/// A Swift `String` holding a copy of `value` (owned).
pub fn string_from_utf8(value: &str) -> SwiftString {
    let mut string = SwiftString { words: [0; 2] };
    unsafe { rs_string_from_utf8(value.as_ptr(), value.len() as isize, &mut string) };
    string
}
