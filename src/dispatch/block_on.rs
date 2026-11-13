//! Blocking wait for a future on a non-main thread.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark()
    }
}

/// Parks the current thread until `future` completes. Panics on the main thread, where waiting would
/// block the main queue that main-thread work needs; use `run_main` there.
pub fn block_on<F: Future>(future: F) -> F::Output {
    assert!(
        !crate::is_main_thread(),
        "block_on on the main thread cannot wait for main-queue work; use run_main"
    );
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}
