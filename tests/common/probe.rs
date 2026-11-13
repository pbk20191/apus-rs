//! Swift probes (tests/support/probe.swift, linked when the `test-probe` feature is on).
#![allow(dead_code)]

use std::ffi::{c_char, c_void, CStr};

extern "C" {
    pub fn probe_has_task() -> bool;
    pub fn probe_is_cancelled() -> bool;
    pub fn probe_priority() -> u8;
    pub fn probe_assume_main() -> bool;
    pub fn probe_with_local(value: *const c_char, body: extern "C" fn());
    pub fn probe_read_local(buf: *mut c_char, len: usize);
    pub fn probe_spawn_swift_task(ctx: *const c_void, done: extern "C" fn(*const c_void, u8));
}

pub fn read_local() -> String {
    let mut buf = [0 as c_char; 128];
    unsafe {
        probe_read_local(buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Starts a Swift `Task {}` from the current context and returns the priority it observed.
pub fn swift_child_priority() -> u8 {
    let (tx, rx) = std::sync::mpsc::channel::<u8>();
    extern "C" fn done(ctx: *const c_void, priority: u8) {
        let tx = unsafe { Box::from_raw(ctx as *mut std::sync::mpsc::Sender<u8>) };
        let _ = tx.send(priority);
    }
    unsafe { probe_spawn_swift_task(Box::into_raw(Box::new(tx)).cast(), done) };
    rx.recv_timeout(std::time::Duration::from_secs(2)).expect("Swift Task {} never ran")
}

extern "C" {
    pub fn probe_closure_identity() -> *mut c_void;
    pub fn probe_closure_sleep(ms: isize) -> *mut c_void;
    pub fn probe_closure_throw_eio() -> *mut c_void;
    pub fn probe_closure_void() -> *mut c_void;
    pub fn probe_closure_read_local() -> *mut c_void;
    pub fn probe_closure_block_ms(ms: isize) -> *mut c_void;
    pub fn probe_closure_deinit_flag(flag: *mut bool) -> *mut c_void;
}

extern "C" {
    pub fn probe_closure_sleep_flag_cancel(ms: isize, cancelled: *mut bool) -> *mut c_void;
}
