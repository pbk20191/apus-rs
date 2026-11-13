//! libdispatch backend: futures run as `async-task` runnables scheduled with `dispatch_async_f`.
//!
//! The runnable/trampoline structure follows `dgrijalva/async-dispatch` (Apache-2.0), which in turn follows
//! the dispatcher in Zed's gpui crate.

mod block_on;
mod ffi;
mod queue;

pub use block_on::block_on;
pub use queue::SerialQueue;

use crate::common::{guard, AbortState};
use crate::{JoinError, Priority, SpawnOptions};
use async_task::Runnable;
use ffi::*;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) struct Queue(DispatchQueue);
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    fn raw(self) -> DispatchQueue {
        self.0
    }
}

/// `Unspecified` inherits the calling thread's QoS, like an unprioritized Swift `Task {}`.
fn global_queue(priority: Priority) -> Queue {
    let qos = match priority {
        Priority::Unspecified => match Priority::from_raw(unsafe { qos_class_self() } as u8) {
            Priority::Unspecified => Priority::Default,
            inherited => inherited,
        },
        p => p,
    };
    Queue(unsafe { dispatch_get_global_queue(qos.raw() as isize, 0) })
}

fn main_queue() -> Queue {
    Queue(std::ptr::addr_of!(_dispatch_main_q) as DispatchQueue)
}

thread_local! {
    /// True while a main-queue task is being polled: `spawn` inside it stays on the main queue.
    static IN_MAIN_TASK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

extern "C" fn run_runnable(runnable: *mut c_void) {
    let runnable = unsafe { Runnable::<()>::from_raw(std::ptr::NonNull::new_unchecked(runnable.cast())) };
    // `Guarded` turns task panics into JoinError; this only keeps anything else from unwinding into libdispatch.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runnable.run())).is_err() {
        eprintln!("async-swift-rs: a dispatch task panicked outside its future");
    }
}

extern "C" fn run_main_runnable(runnable: *mut c_void) {
    let previous = IN_MAIN_TASK.replace(true);
    run_runnable(runnable);
    IN_MAIN_TASK.set(previous);
}

/// Handle to a spawned task. Awaiting yields its output; dropping it detaches the task.
pub struct JoinHandle<T> {
    task: Option<async_task::Task<Result<T, JoinError>>>,
    abort: Arc<AbortState>,
}

impl<T> JoinHandle<T> {
    /// Stops the task at its next poll; awaiting the handle then yields `Err(JoinError::Aborted)`.
    pub fn abort(&self) {
        self.abort.request();
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

/// Schedules a runnable on `queue`; `keep_alive` is held by the schedule closure (serial queues).
fn scheduler<K: Send + Sync + 'static>(queue: Queue, keep_alive: K) -> impl Fn(Runnable) + Send + Sync + 'static {
    let trampoline: DispatchFunction = if queue.raw() == main_queue().raw() { run_main_runnable } else { run_runnable };
    move |runnable: Runnable| {
        let _ = &keep_alive;
        unsafe { dispatch_async_f(queue.raw(), runnable.into_raw().as_ptr().cast(), trampoline) }
    }
}

pub(crate) fn spawn_on<F, K>(queue: Queue, keep_alive: K, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
    K: Send + Sync + 'static,
{
    let (guarded, abort) = guard(future);
    let (runnable, task) = async_task::spawn(guarded, scheduler(queue, keep_alive));
    runnable.schedule();
    JoinHandle { task: Some(task), abort }
}

/// Spawns `future` on the main queue. Main-queue tasks run only while the main run loop runs.
pub fn spawn_on_main<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_on(main_queue(), (), future)
}

/// Like [`spawn_on_main`] for `!Send` futures. Panics off the main thread.
pub fn spawn_local_on_main<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(crate::is_main_thread(), "spawn_local_on_main must be called on the main thread");
    let (guarded, abort) = guard(future);
    let (runnable, task) = async_task::spawn_local(guarded, scheduler(main_queue(), ()));
    runnable.schedule();
    JoinHandle { task: Some(task), abort }
}

/// Spawns without inheriting anything from the caller (Swift's `Task.detached`): default priority, not main.
pub fn spawn_detached<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_on(global_queue(Priority::Default), (), future)
}

/// Spawns `future` with default options.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_with(SpawnOptions::default(), future)
}

/// Spawns `future` with explicit options. Inside a main-queue task the new task stays on the main queue
/// (Swift keeps the actor for `Task {}`); use [`spawn_detached`] to leave it.
pub fn spawn_with<F>(options: SpawnOptions, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    if options.main || IN_MAIN_TASK.get() {
        return spawn_on_main(future);
    }
    spawn_on(global_queue(options.priority), (), future)
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
