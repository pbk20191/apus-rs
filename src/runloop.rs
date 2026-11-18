//! Main-thread helpers. The main queue is drained by the main run loop (`CFRunLoopRun*`), never `dispatch_main`.

use objc2_core_foundation::{
    kCFRunLoopDefaultMode, CFRetained, CFRunLoop, CFRunLoopActivity, CFRunLoopObserver, CFRunLoopObserverContext, CFRunLoopSource,
    CFRunLoopSourceContext,
};
use std::ffi::c_void;
use std::future::Future;
use std::pin::pin;
use std::cell::{Cell, RefCell};
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

/// What `run_main`'s run loop callbacks share. Everything here runs on the main thread.
struct Driver<'a> {
    /// Polls the task's handle; true once the result is in.
    poll: RefCell<&'a mut dyn FnMut() -> bool>,
    done: Cell<bool>,
    /// Default-mode run loop activations currently on the stack, counted since `run_main` began.
    depth: Cell<usize>,
    /// The depth at which `run_main`'s own run executes.
    own_depth: Cell<usize>,
    waker: &'a Arc<MainLoopWaker>,
}

impl Driver<'_> {
    /// `CFRunLoopStop` ends the innermost run, so it is only issued when that run is ours. A nested run
    /// (a run loop callback running its own loop, like a modal session) is left alone.
    fn stop_if_ours(&self) {
        if self.done.get() && self.depth.get() == self.own_depth.get() {
            self.waker.run_loop.stop();
        }
    }
}

/// The source's perform callback: polls, and stops our run once the result is in.
unsafe extern "C-unwind" fn source_fired(info: *mut c_void) {
    let driver = &*(info as *const Driver);
    if !driver.done.get() {
        if let Ok(mut poll) = driver.poll.try_borrow_mut() {
            driver.done.set(poll());
        }
    }
    driver.stop_if_ours();
}

/// The observer's callback: tracks nesting. When a nested run ends after the result came in, the source is
/// fired again so that it runs, and stops the loop, at our own level.
unsafe extern "C-unwind" fn activity_observed(_observer: *mut CFRunLoopObserver, activity: CFRunLoopActivity, info: *mut c_void) {
    let driver = &*(info as *const Driver);
    if activity == CFRunLoopActivity::Entry {
        driver.depth.set(driver.depth.get() + 1);
    } else if activity == CFRunLoopActivity::Exit {
        driver.depth.set(driver.depth.get().saturating_sub(1));
        if driver.done.get() && driver.depth.get() == driver.own_depth.get() {
            driver.waker.wake_by_ref();
        }
    }
}

/// Runs `future` as a main-thread task (a MainActor Swift Task when the `swift` feature is on), driving the
/// main run loop until it completes.
///
/// The task's handle is polled inside a run loop source, which stops the run loop when the result is in.
/// A run loop observer tracks nested runs, so a nested run in progress at that moment finishes undisturbed.
/// Panics when called off the main thread. A panic inside `future` is resumed here.
pub fn run_main<F>(future: F) -> F::Output
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(is_main_thread(), "run_main must be called on the main thread");
    // The main queue runs one task at a time: inside a main task, the task run_main starts would never run.
    assert!(!inside_main_task(), "run_main called inside a main task: await the future instead");
    let run_loop = CFRunLoop::main().expect("no main run loop");
    let mode = unsafe { kCFRunLoopDefaultMode };
    let state = Arc::new(MainLoopWaker { source: OnceLock::new(), run_loop: run_loop.clone() });
    let mut result = None;
    {
        let waker = Waker::from(state.clone());
        let mut cx = Context::from_waker(&waker);
        let mut handle = pin!(spawn_main_task(future));
        let mut poll = || match handle.as_mut().poll(&mut cx) {
            Poll::Ready(output) => {
                result = Some(output);
                true
            }
            Poll::Pending => false,
        };
        let driver = Driver { poll: RefCell::new(&mut poll), done: Cell::new(false), depth: Cell::new(0), own_depth: Cell::new(1), waker: &state };
        let info = &driver as *const Driver as *mut c_void;

        let mut source_context = CFRunLoopSourceContext {
            version: 0,
            info,
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(source_fired),
        };
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut source_context) }.expect("CFRunLoopSourceCreate failed");
        let mut observer_context = CFRunLoopObserverContext { version: 0, info, retain: None, release: None, copyDescription: None };
        let activities = (CFRunLoopActivity::Entry | CFRunLoopActivity::Exit).0;
        let observer = unsafe { CFRunLoopObserver::new(None, activities, true, 0, Some(activity_observed), &mut observer_context) }
            .expect("CFRunLoopObserverCreate failed");
        run_loop.add_observer(Some(&observer), mode);
        run_loop.add_source(Some(&source), mode);
        let _ = state.source.set(source);

        state.wake_by_ref(); // the first poll
        while !driver.done.get() {
            driver.own_depth.set(driver.depth.get() + 1);
            CFRunLoop::run_in_mode(mode, 1e10, false);
        }
        // Invalidate before the driver goes away: neither callback may run again.
        observer.invalidate();
        state.source.get().expect("source was set").invalidate();
    }
    match result.expect("run loop ended with a result") {
        Ok(v) => v,
        Err(crate::JoinError::Panicked(payload)) => std::panic::resume_unwind(payload),
        Err(crate::JoinError::Aborted) => unreachable!("run_main task cannot be aborted"),
    }
}

/// Whether a task is running on this (main) thread right now: then it is a main task, of either runtime.
fn inside_main_task() -> bool {
    #[cfg(feature = "swift")]
    if !unsafe { crate::swift::rt::swift_task_getCurrent() }.is_null() {
        return true;
    }
    #[cfg(feature = "dispatch")]
    if crate::dispatch::in_main_task() {
        return true;
    }
    false
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
