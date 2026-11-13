//! Test harness: the main thread drains the main queue with `CFRunLoopRun` (never `dispatch_main`, which
//! drains it on a worker thread) while libtest-mimic runs the trials on other threads.
#![allow(dead_code)]

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

pub use libtest_mimic::{Failed, Trial};

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopRun();
}

pub fn run(tests: Vec<Trial>) -> ! {
    let args = libtest_mimic::Arguments::from_args();
    std::thread::spawn(move || std::process::exit(if libtest_mimic::run(&args, tests).has_failed() { 101 } else { 0 }));
    unsafe { CFRunLoopRun() };
    unreachable!("main run loop returned")
}

struct Unpark(std::thread::Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark()
    }
}

/// Parks the calling (non-main) thread until `f` completes.
pub fn block<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&waker)) {
            return v;
        }
        std::thread::park();
    }
}

pub fn check(cond: bool, msg: impl Into<String>) -> Result<(), Failed> {
    if cond { Ok(()) } else { Err(msg.into().into()) }
}

extern "C" {
    fn qos_class_self() -> u32;
}
pub fn current_qos() -> u32 {
    unsafe { qos_class_self() }
}

/// A value whose `Drop` panics.
pub struct PanicOnDrop;
impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("drop panicked");
    }
}

/// Awaits `f` on a helper thread; `None` if it does not complete within `limit`.
pub fn block_with_timeout<F: Future + Send + 'static>(f: F, limit: std::time::Duration) -> Option<F::Output>
where
    F::Output: Send,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(block(f)); });
    rx.recv_timeout(limit).ok()
}
