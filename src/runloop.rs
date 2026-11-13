//! Main-thread helpers. The main queue is drained by the main run loop (`CFRunLoopRun*`), never `dispatch_main`.

use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext};
use std::ffi::c_void;
use std::future::Future;
use std::pin::pin;
use std::cell::RefCell;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

pub fn is_main_thread() -> bool {
    unsafe { libc::pthread_main_np() == 1 }
}

/// Wakes `run_main` by signaling its run loop source, whose callback polls the task's handle.
/// `CFRunLoopSourceSignal` and `CFRunLoopWakeUp` may be called from any thread.
struct MainLoopWaker {
    source: OnceLock<CFRetained<CFRunLoopSource>>,
    run_loop: CFRetained<CFRunLoop>,
}

unsafe impl Send for MainLoopWaker {}
unsafe impl Sync for MainLoopWaker {}

impl Wake for MainLoopWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(source) = self.source.get() {
            source.signal();
            self.run_loop.wake_up();
        }
    }
}

/// The source's perform callback: `info` points at the `&mut dyn FnMut()` that polls.
unsafe extern "C-unwind" fn source_fired(info: *mut c_void) {
    let poll = &mut *(info as *mut &mut dyn FnMut());
    poll();
}

/// Runs `future` as a main-thread task (a MainActor Swift Task when the `swift` feature is on), driving the
/// main run loop until it completes.
///
/// The task's handle is polled inside a run loop source; when it is ready the source stops the run loop.
/// Panics when called off the main thread. A panic inside `future` is resumed here.
pub fn run_main<F>(future: F) -> F::Output
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(is_main_thread(), "run_main must be called on the main thread");
    let run_loop = CFRunLoop::main().expect("no main run loop");
    let mode = unsafe { kCFRunLoopDefaultMode };
    let state = Arc::new(MainLoopWaker { source: OnceLock::new(), run_loop: run_loop.clone() });
    let result = RefCell::new(None);
    {
        let waker = Waker::from(state.clone());
        let mut cx = Context::from_waker(&waker);
        let mut handle = pin!(spawn_main_task(future));
        let mut poll = || {
            if result.borrow().is_none() {
                if let Poll::Ready(output) = handle.as_mut().poll(&mut cx) {
                    *result.borrow_mut() = Some(output);
                }
            }
            if result.borrow().is_some() {
                // CFRunLoopStop ends the innermost run. If that was a nested run started inside the task,
                // fire again so that our own run below is stopped too.
                run_loop.stop();
                state.wake_by_ref();
            }
        };
        let mut poll: &mut dyn FnMut() = &mut poll;
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: (&mut poll as *mut &mut dyn FnMut()).cast(),
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
        run_loop.add_source(Some(&source), mode);
        let _ = state.source.set(source);
        state.wake_by_ref(); // the first poll
        while result.borrow().is_none() {
            CFRunLoop::run_in_mode(mode, 1e10, false);
        }
        // Invalidate before `poll` goes away: the source must never fire again.
        state.source.get().expect("source was set").invalidate();
    }
    match result.into_inner().expect("run loop ended with a result") {
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
