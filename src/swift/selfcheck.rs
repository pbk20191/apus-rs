//! One-time check, before the first spawn, that the Swift runtime matches the ABI this crate was written
//! against. A mismatch must fail loudly, never corrupt memory quietly.
//!
//! The structural checks run inline. The probe Task is verified by a watchdog thread instead of the caller:
//! the first spawn may run on a cooperative-pool thread (inside a Swift Task), and blocking it while the probe
//! needs a pool thread could starve the pool.

use super::{rt, task};
use crate::SpawnOptions;
use std::ffi::CString;
use std::sync::mpsc;
use std::sync::Once;
use std::time::Duration;

use libc::{dlsym, RTLD_DEFAULT};

const FAILED: &str = "apus: Swift runtime ABI self-check failed";

pub(crate) fn ensure() {
    static CHECK: Once = Once::new();
    CHECK.call_once(|| {
        if let Err(detail) = structural() {
            panic!("{FAILED}: {detail}");
        }
        start_probe();
    });
}

fn structural() -> Result<(), String> {
    // With `macos15` the macOS 15 symbols are required too (they are linked, not looked up).
    #[cfg(feature = "macos15")]
    let optional: &[(&str, (u32, u32))] = rt::OPTIONAL_SYMBOLS;
    #[cfg(not(feature = "macos15"))]
    let optional: &[(&str, (u32, u32))] = &[];
    let missing: Vec<_> = rt::REQUIRED_SYMBOLS
        .iter()
        .copied()
        .chain(optional.iter().map(|(name, _)| *name))
        .filter(|name| unsafe { dlsym(RTLD_DEFAULT, CString::new(*name).unwrap().as_ptr()) }.is_null())
        .collect();
    if !missing.is_empty() {
        return Err(format!("runtime symbols not found: {missing:?}"));
    }
    let metadata = rt::empty_tuple_metadata();
    if metadata.is_null() || unsafe { *(metadata as *const usize) } != (rt::METADATA_KIND_TUPLE as usize) {
        return Err("metadata for () is not a tuple metadata".into());
    }
    if unsafe { *(rt::metadata_of(&rt::INT_METADATA) as *const usize) } != (rt::METADATA_KIND_STRUCT as usize) {
        return Err("$sSiN does not point at Int's metadata address point".into());
    }
    if unsafe { rt::swift_task_getMainExecutor() }.identity.is_null() {
        return Err("main executor has no identity".into());
    }
    Ok(())
}

/// A probe Task: it must see itself as the current task, suspend once and resume, then complete. If it does
/// not within 5 s, the runtime does not behave as this crate assumes: abort with a message rather than let
/// tasks misbehave.
fn start_probe() {
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
    unsafe { rt::swift_release(probe) };
    std::thread::spawn(move || {
        let result = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("probe task did not complete".into()));
        if let Err(detail) = result {
            eprintln!("{FAILED}: {detail}");
            std::process::abort();
        }
    });
}
