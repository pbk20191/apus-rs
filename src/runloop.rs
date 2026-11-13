//! Main-thread helpers. The main queue is drained by the main run loop (`CFRunLoopRun*`), never `dispatch_main`.

use dispatch2::DispatchQueue;
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

struct MainQueueNudge;

impl std::task::Wake for MainQueueNudge {
    fn wake(self: std::sync::Arc<Self>) {
        DispatchQueue::main().exec_async(|| {});
    }
}

pub fn is_main_thread() -> bool {
    unsafe { libc::pthread_main_np() == 1 }
}

/// Runs `future` as a main-thread task (a MainActor Swift Task when the `swift` feature is on), driving the
/// main run loop until it completes.
///
/// Panics when called off the main thread. A panic inside `future` is resumed here.
pub fn run_main<F>(future: F) -> F::Output
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(is_main_thread(), "run_main must be called on the main thread");
    let mut handle = pin!(spawn_main_task(future));
    // Re-poll after every main-queue item. The waker enqueues an empty main-queue item, so a completion that
    // arrives from any thread still makes CFRunLoopRunInMode return (an early CFRunLoopStop would be lost).
    let waker = Waker::from(std::sync::Arc::new(MainQueueNudge));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(result) = handle.as_mut().poll(&mut cx) {
            return match result {
                Ok(v) => v,
                Err(crate::JoinError::Panicked(payload)) => std::panic::resume_unwind(payload),
                Err(crate::JoinError::Aborted) => unreachable!("run_main task cannot be aborted"),
            };
        }
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 1e10, true);
    }
}

#[cfg(feature = "swift")]
fn spawn_main_task<F>(future: F) -> impl Future<Output = Result<F::Output, crate::JoinError>>
where
    F: Future + 'static,
    F::Output: 'static,
{
    crate::swift::spawn_local_on_main(future)
}

#[cfg(all(feature = "dispatch", not(feature = "swift")))]
fn spawn_main_task<F>(future: F) -> impl Future<Output = Result<F::Output, crate::JoinError>>
where
    F: Future + 'static,
    F::Output: 'static,
{
    crate::dispatch::spawn_local_on_main(future)
}
