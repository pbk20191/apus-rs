//! Safe helpers over the Swift runtime ABI. The declarations themselves (functions, records, constants, data
//! symbols) come from apus-sys, generated from its include/apus/abi.h; `rt::X` names them all.

pub(crate) use apus_sys::*;
use std::ffi::c_void;
use std::sync::OnceLock;

/// The value witness table, from the metadata header just before a metadata's address point.
pub unsafe fn value_witnesses<'a>(metadata: *const c_void) -> &'a ValueWitnessTable {
    &*(*(metadata as *const TypeMetadataHeader).sub(1)).value_witnesses
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
    let header = array as *const ContiguousArrayStorageHeader;
    let count = (*header).count as usize;
    let bytes = std::slice::from_raw_parts(header.add(1) as *const u8, count.saturating_sub(1));
    let out = String::from_utf8_lossy(bytes).into_owned();
    swift_release(array);
    out
}

/// Copies a Swift `String` value into `dest` (+1 on its storage).
pub unsafe fn copy_string(dest: &mut SwiftString, source: &SwiftString) {
    let metadata = metadata_of(&STRING_METADATA);
    value_witnesses(metadata).initialize_with_copy.unwrap()((dest as *mut SwiftString).cast(), (source as *const SwiftString).cast_mut().cast(), metadata);
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
