//! The few libdispatch functions this backend needs (no bindgen).

use std::ffi::c_void;

pub type DispatchQueue = *mut c_void;
pub type DispatchFunction = extern "C" fn(*mut c_void);

pub const DISPATCH_TIME_NOW: u64 = 0;

extern "C" {
    pub fn dispatch_get_global_queue(identifier: isize, flags: usize) -> DispatchQueue;
    pub fn dispatch_async_f(queue: DispatchQueue, context: *mut c_void, work: DispatchFunction);
    pub fn dispatch_after_f(when: u64, queue: DispatchQueue, context: *mut c_void, work: DispatchFunction);
    pub fn dispatch_time(when: u64, delta: i64) -> u64;
}
