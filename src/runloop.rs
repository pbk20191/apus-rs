//! Main-thread helpers. The main queue is drained by the main run loop (`CFRunLoopRun*`), never `dispatch_main`.

use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext};
use std::ffi::c_void;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

pub fn is_main_thread() -> bool {
    unsafe { libc::pthread_main_np() == 1 }
}

/// Wakes `run_main`: marks it woken and signals a run loop source so the main run loop returns to it.
/// `CFRunLoopSourceSignal` and `CFRunLoopWakeUp` may be called from any thread.
struct MainLoopWaker {
    woken: AtomicBool,
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}

unsafe impl Send for MainLoopWaker {}
unsafe impl Sync for MainLoopWaker {}

impl Wake for MainLoopWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        self.source.signal();
        self.run_loop.wake_up();
    }
}

/// The source only has to be "handled" for the run loop to return; the polling happens in `run_main`.
unsafe extern "C-unwind" fn source_fired(_info: *mut c_void) {}

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
    let run_loop = CFRunLoop::main().expect("no main run loop");
    let mut context = CFRunLoopSourceContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(source_fired),
    };
    let source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }.expect("CFRunLoopSourceCreate failed");
    let mode = unsafe { kCFRunLoopDefaultMode };
    run_loop.add_source(Some(&source), mode);
    let state = Arc::new(MainLoopWaker { woken: AtomicBool::new(true), source, run_loop });
    let waker = Waker::from(state.clone());
    let mut cx = Context::from_waker(&waker);

    let mut handle = pin!(spawn_main_task(future));
    let result = loop {
        // Poll only when woken: the run loop also returns for unrelated sources and main-queue work.
        if state.woken.swap(false, Ordering::SeqCst) {
            if let Poll::Ready(result) = handle.as_mut().poll(&mut cx) {
                break result;
            }
        }
        CFRunLoop::run_in_mode(mode, 1e10, true);
    };
    state.source.invalidate();
    match result {
        Ok(v) => v,
        Err(crate::JoinError::Panicked(payload)) => std::panic::resume_unwind(payload),
        Err(crate::JoinError::Aborted) => unreachable!("run_main task cannot be aborted"),
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
