//! libdispatch backend: futures run as `async-task` runnables scheduled with `dispatch_async_f`.
//!
//! The runnable/trampoline structure follows `dgrijalva/async-dispatch` (Apache-2.0), which in turn follows
//! the dispatcher in Zed's gpui crate.

mod ffi;

use crate::{JoinError, Priority, SpawnOptions};
use async_task::Runnable;
use ffi::*;
use std::ffi::c_void;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Queue(DispatchQueue);
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    fn raw(self) -> DispatchQueue {
        self.0
    }
}

fn global_queue(priority: Priority) -> Queue {
    let qos = match priority {
        Priority::Unspecified => Priority::Default,
        p => p,
    };
    Queue(unsafe { dispatch_get_global_queue(qos.raw() as isize, 0) })
}

extern "C" fn run_runnable(runnable: *mut c_void) {
    unsafe { Runnable::<()>::from_raw(std::ptr::NonNull::new_unchecked(runnable.cast())) }.run();
}

/// Shared between a `JoinHandle` and its task: the abort request and the waker that notices it.
struct AbortState {
    requested: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// Wraps the user future: catches panics and stops at the next poll after `abort`.
struct Guarded<F> {
    future: Option<F>,
    abort: Arc<AbortState>,
}

impl<F: Future> Future for Guarded<F> {
    type Output = Result<F::Output, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        if this.abort.requested.load(Ordering::SeqCst) {
            this.future = None;
            return Poll::Ready(Err(JoinError::Aborted));
        }
        *this.abort.waker.lock().unwrap() = Some(cx.waker().clone());
        let Some(future) = this.future.as_mut() else { return Poll::Ready(Err(JoinError::Aborted)) };
        let future = unsafe { Pin::new_unchecked(future) };
        match catch_unwind(AssertUnwindSafe(|| future.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(v)) => {
                this.future = None;
                Poll::Ready(Ok(v))
            }
            Err(payload) => {
                this.future = None;
                Poll::Ready(Err(JoinError::Panicked(payload)))
            }
        }
    }
}

/// Handle to a spawned task. Awaiting yields its output; dropping it detaches the task.
pub struct JoinHandle<T> {
    task: Option<async_task::Task<Result<T, JoinError>>>,
    abort: Arc<AbortState>,
}

impl<T> JoinHandle<T> {
    /// Stops the task at its next poll; awaiting the handle then yields `Err(JoinError::Aborted)`.
    pub fn abort(&self) {
        self.abort.requested.store(true, Ordering::SeqCst);
        if let Some(w) = self.abort.waker.lock().unwrap().take() {
            w.wake();
        }
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(self.task.as_mut().expect("JoinHandle polled after completion")).poll(cx)
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

fn spawn_on<F>(queue: Queue, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let abort = Arc::new(AbortState { requested: AtomicBool::new(false), waker: Mutex::new(None) });
    let guarded = Guarded { future: Some(future), abort: abort.clone() };
    let schedule = move |runnable: Runnable| unsafe {
        dispatch_async_f(queue.raw(), runnable.into_raw().as_ptr().cast(), run_runnable)
    };
    let (runnable, task) = async_task::spawn(guarded, schedule);
    runnable.schedule();
    JoinHandle { task: Some(task), abort }
}

/// Spawns `future` with default options.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_with(SpawnOptions::default(), future)
}

/// Spawns `future` with explicit options.
pub fn spawn_with<F>(options: SpawnOptions, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_on(global_queue(options.priority), future)
}

/// Completes after `duration`, using `dispatch_after_f`.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep { deadline: Instant::now() + duration, armed: false }
}

pub struct Sleep {
    deadline: Instant,
    armed: bool,
}

extern "C" fn wake_boxed(waker: *mut c_void) {
    unsafe { Box::from_raw(waker.cast::<Waker>()) }.wake();
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Poll::Ready(());
        }
        if !self.armed {
            self.armed = true;
            let waker = Box::into_raw(Box::new(cx.waker().clone()));
            unsafe {
                let when = dispatch_time(DISPATCH_TIME_NOW, left.as_nanos() as i64);
                dispatch_after_f(when, global_queue(Priority::Default).raw(), waker.cast(), wake_boxed);
            }
        }
        Poll::Pending
    }
}
