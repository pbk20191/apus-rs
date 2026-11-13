//! One-time check, before the first spawn, that the Swift runtime matches the ABI this crate was written
//! against. A mismatch must fail loudly, never corrupt memory quietly.

use super::{rt, task};
use crate::SpawnOptions;
use std::ffi::{c_void, CString};
use std::sync::mpsc;
use std::sync::Once;
use std::time::Duration;

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

pub(crate) fn ensure() {
    static CHECK: Once = Once::new();
    CHECK.call_once(|| {
        if let Err(detail) = run() {
            panic!("async-swift-rs: Swift runtime ABI self-check failed: {detail}");
        }
    });
}

fn run() -> Result<(), String> {
    let missing: Vec<_> = rt::REQUIRED_SYMBOLS
        .iter()
        .filter(|name| unsafe { dlsym(RTLD_DEFAULT, CString::new(**name).unwrap().as_ptr()) }.is_null())
        .collect();
    if !missing.is_empty() {
        return Err(format!("runtime symbols not found: {missing:?}"));
    }
    let metadata = rt::empty_tuple_metadata();
    if metadata.is_null() || unsafe { *(metadata as *const usize) } != rt::METADATA_KIND_TUPLE {
        return Err("metadata for () is not a tuple metadata".into());
    }
    if unsafe { *(rt::metadata_of(&rt::INT_METADATA) as *const usize) } != rt::METADATA_KIND_STRUCT {
        return Err("$sSiN does not point at Int's metadata address point".into());
    }
    if unsafe { rt::swift_task_getMainExecutor() }.identity.is_null() {
        return Err("main executor has no identity".into());
    }
    // A probe task: it must see itself as the current task, suspend once and resume, then complete.
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let probe_tx = tx.clone();
    let mut suspended = false;
    let future = Box::pin(std::future::poll_fn(move |cx| {
        if unsafe { rt::swift_task_getCurrent() }.is_null() {
            let _ = probe_tx.send(Err("no current task inside a spawned task".into()));
            return std::task::Poll::Ready(());
        }
        if !suspended {
            suspended = true;
            let waker = cx.waker().clone();
            std::thread::spawn(move || waker.wake());
            return std::task::Poll::Pending;
        }
        std::task::Poll::Ready(())
    }));
    let probe = task::create(SpawnOptions::default(), future, Box::new(move || { let _ = tx.send(Ok(())); }));
    let result = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("probe task did not complete".into()));
    unsafe { rt::swift_release(probe) };
    result
}
