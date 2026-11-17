//! libdispatch backend: futures run as `async-task` runnables scheduled on GCD queues (via `dispatch2`).
//!
//! The runnable/trampoline structure follows `dgrijalva/async-dispatch` (Apache-2.0), which in turn follows
//! the dispatcher in Zed's gpui crate.

mod block_on;
mod queue;

pub use block_on::block_on;
pub use queue::SerialQueue;

use crate::common::{guard, AbortState, TimerSlot};
use crate::{JoinError, Priority, SpawnOptions};
use async_task::Runnable;
use dispatch2::{DispatchQoS, DispatchQueue, DispatchRetained, DispatchTime, GlobalQueueIdentifier};
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

pub use crate::common::Sleep;

extern "C" {
    /// <pthread/qos.h>; bound by neither dispatch2 nor libc.
    fn qos_class_self() -> u32;
}

/// `Unspecified` inherits the calling thread's QoS, like an unprioritized Swift `Task {}`.
fn global_queue(priority: Priority) -> DispatchRetained<DispatchQueue> {
    let qos = match priority {
        Priority::Unspecified => match Priority::from_raw(unsafe { qos_class_self() } as u8) {
            Priority::Unspecified => Priority::Default,
            inherited => inherited,
        },
        p => p,
    };
    DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(DispatchQoS(qos.raw().into())))
}

thread_local! {
    /// True while a main-queue task is being polled: `spawn` inside it stays on the main queue.
    static IN_MAIN_TASK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while a main-queue task is being polled on this thread.
#[allow(dead_code)] // used by run_main without the swift backend
pub(crate) fn in_main_task() -> bool {
    IN_MAIN_TASK.get()
}

extern "C" fn run_runnable(runnable: *mut c_void) {
    let runnable = unsafe { Runnable::<()>::from_raw(std::ptr::NonNull::new_unchecked(runnable.cast())) };
    // `Guarded` turns task panics into JoinError; this only keeps anything else from unwinding into libdispatch.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runnable.run())).is_err() {
        eprintln!("apus: a dispatch task panicked outside its future");
    }
}

extern "C" fn run_main_runnable(runnable: *mut c_void) {
    let previous = IN_MAIN_TASK.replace(true);
    run_runnable(runnable);
    IN_MAIN_TASK.set(previous);
}

/// Where a task runs. Holding the queue keeps a serial queue alive while it has tasks.
#[derive(Clone)]
pub(crate) enum Target {
    Main,
    Queue(DispatchRetained<DispatchQueue>),
}

/// Schedules runnables with `dispatch_async_f` (no allocation per wake, unlike the closure API).
fn scheduler(target: Target) -> impl Fn(Runnable) + Send + Sync + 'static {
    move |runnable: Runnable| {
        let work = runnable.into_raw().as_ptr().cast();
        unsafe {
            match &target {
                Target::Main => DispatchQueue::main().exec_async_f(work, run_main_runnable),
                Target::Queue(queue) => queue.exec_async_f(work, run_runnable),
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

pub(crate) fn spawn_on<F>(target: Target, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (guarded, abort) = guard(future);
    let (runnable, task) = async_task::spawn(guarded, scheduler(target));
    runnable.schedule();
    JoinHandle { task: Some(task), abort }
}

/// Spawns `future` on the main queue. Main-queue tasks run only while the main run loop runs.
pub fn spawn_on_main<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_on(Target::Main, future)
}

/// Like [`spawn_on_main`] for `!Send` futures. Panics off the main thread.
pub fn spawn_local_on_main<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(crate::is_main_thread(), "spawn_local_on_main must be called on the main thread");
    let (guarded, abort) = guard(future);
    let (runnable, task) = async_task::spawn_local(guarded, scheduler(Target::Main));
    runnable.schedule();
    JoinHandle { task: Some(task), abort }
}

/// Spawns without inheriting anything from the caller (Swift's `Task.detached`): default priority, not main.
pub fn spawn_detached<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_on(Target::Queue(global_queue(Priority::Default)), future)
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
    spawn_on(Target::Queue(global_queue(options.priority)), future)
}

/// Completes after `duration`, using `dispatch_after`.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep::new(duration, arm_timer)
}

fn arm_timer(delay: Duration, slot: Arc<TimerSlot>) {
    let when = DispatchTime::try_from(delay).unwrap_or(DispatchTime::FOREVER);
    global_queue(Priority::Default).after(when, move || slot.fire()).expect("dispatch_after time overflow");
}
