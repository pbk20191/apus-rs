//! The few libdispatch functions this backend needs (no bindgen).

use std::ffi::c_void;

pub type DispatchQueue = *mut c_void;
pub type DispatchFunction = extern "C" fn(*mut c_void);

pub const DISPATCH_TIME_NOW: u64 = 0;

/// The main queue object (`dispatch_get_main_queue()` is a macro for its address).
#[repr(C)]
pub struct DispatchMainQueue {
    _opaque: [u8; 0],
}

extern "C" {
    pub static _dispatch_main_q: DispatchMainQueue;
    pub fn qos_class_self() -> u32;
    pub fn dispatch_queue_create(label: *const std::ffi::c_char, attr: *mut c_void) -> DispatchQueue;
    pub fn dispatch_queue_attr_make_with_qos_class(attr: *mut c_void, qos: u32, relative_priority: i32) -> *mut c_void;
    pub fn dispatch_release(object: *mut c_void);
    pub fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchQueue;
    pub fn dispatch_async_f(queue: DispatchQueue, context: *mut c_void, work: DispatchFunction);
    pub fn dispatch_after_f(when: u64, queue: DispatchQueue, context: *mut c_void, work: DispatchFunction);
    pub fn dispatch_time(when: u64, delta: i64) -> u64;
}
