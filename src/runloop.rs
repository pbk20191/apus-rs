//! Main-thread helpers. The main queue is drained by the main run loop (`CFRunLoopRun*`), never `dispatch_main`.

use std::ffi::c_void;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
}

extern "C" {
    fn pthread_main_np() -> i32;
}

pub fn is_main_thread() -> bool {
    unsafe { pthread_main_np() == 1 }
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
    // Re-poll after every main-queue item: the task only ever completes on the main queue, so a noop waker
    // is enough, and nothing can be lost the way an early CFRunLoopStop would be.
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(result) = handle.as_mut().poll(&mut cx) {
            return match result {
                Ok(v) => v,
                Err(crate::JoinError::Panicked(payload)) => std::panic::resume_unwind(payload),
                Err(crate::JoinError::Aborted) => unreachable!("run_main task cannot be aborted"),
            };
        }
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1e10, 1) };
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
