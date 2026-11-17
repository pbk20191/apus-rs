//! Test harness: the main thread drains the main queue with `CFRunLoopRun` (never `dispatch_main`, which
//! drains it on a worker thread) while libtest-mimic runs the trials on other threads.
#![allow(dead_code)]

use std::future::{poll_fn, Future};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

pub use libtest_mimic::{Failed, Trial};

pub fn run(tests: Vec<Trial>) -> ! {
    let args = libtest_mimic::Arguments::from_args();
    std::thread::spawn(move || std::process::exit(if libtest_mimic::run(&args, tests).has_failed() { 101 } else { 0 }));
    objc2_core_foundation::CFRunLoop::run();
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

/// Polls `sleep` with waker A, then with waker B, and reports whether B was woken once it elapsed.
pub fn latest_waker_woken<S: Future<Output = ()>>(sleep: S) -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Flag(AtomicBool);
    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst)
        }
    }
    let (a, b) = (Arc::new(Flag(AtomicBool::new(false))), Arc::new(Flag(AtomicBool::new(false))));
    let mut sleep = pin!(sleep);
    let wa = Waker::from(a.clone());
    let wb = Waker::from(b.clone());
    assert!(sleep.as_mut().poll(&mut Context::from_waker(&wa)).is_pending());
    assert!(sleep.as_mut().poll(&mut Context::from_waker(&wb)).is_pending());
    std::thread::sleep(std::time::Duration::from_millis(200));
    b.0.load(Ordering::SeqCst)
}

/// Polls two futures together, like `join!`.
pub async fn join2<A, B>(a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut ra, mut rb) = (None, None);
    poll_fn(|cx| {
        if ra.is_none() { if let Poll::Ready(v) = a.as_mut().poll(cx) { ra = Some(v) } }
        if rb.is_none() { if let Poll::Ready(v) = b.as_mut().poll(cx) { rb = Some(v) } }
        if ra.is_some() && rb.is_some() { Poll::Ready((ra.take().unwrap(), rb.take().unwrap())) } else { Poll::Pending }
    })
    .await
}

/// Polls a very long sleep once: it must be pending (and arm nothing that fires early), not panic.
pub fn pending_after_one_poll<S: Future<Output = ()>>(sleep: S) -> bool {
    let mut sleep = pin!(sleep);
    sleep.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending()
}
