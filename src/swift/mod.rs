//! Swift Concurrency backend: every spawned future is a genuine Swift Task.

mod join;
pub(crate) mod rt;
mod selfcheck;
mod task;
mod timer;

pub use join::{JoinHandle, MainOnlyHandle};
pub use timer::{sleep, Sleep};

use crate::{Priority, SpawnOptions};
use std::ffi::c_void;
use std::future::Future;

/// Spawns `future` like Swift's `Task {}`.
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
    task::spawn_with(options, true, future)
}

/// Spawns `future` on the MainActor (main thread).
pub fn spawn_on_main<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_with(SpawnOptions { main: true, ..SpawnOptions::default() }, future)
}

/// Spawns without inheriting anything (Swift's `Task.detached`).
pub fn spawn_detached<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    task::spawn_with(SpawnOptions { priority: Priority::Default, main: false, copy_task_locals: false }, false, future)
}

/// Like [`spawn_on_main`] for `!Send` futures: a MainActor task only ever runs on the main thread.
/// Panics off the main thread.
pub fn spawn_local_on_main<F>(future: F) -> MainOnlyHandle<impl Send + 'static, F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    assert!(crate::is_main_thread(), "spawn_local_on_main must be called on the main thread");
    struct MainOnly<T>(T);
    // SAFETY: the task is pinned to the main executor, so the future and its output stay on the main thread.
    unsafe impl<T> Send for MainOnly<T> {}
    impl<F: Future> Future for MainOnly<F> {
        type Output = MainOnly<F::Output>;
        fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
            unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx).map(MainOnly)
        }
    }
    let inner = task::spawn_with(SpawnOptions { main: true, ..SpawnOptions::default() }, false, MainOnly(future));
    join::unwrap_main_only(inner, |m: MainOnly<F::Output>| m.0)
}

/// A Swift task, not owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskRef(*mut c_void);

impl TaskRef {
    pub fn as_ptr(self) -> *mut c_void {
        self.0
    }
}

/// The Swift task the caller runs in, if any.
pub fn current_task() -> Option<TaskRef> {
    let task = unsafe { rt::swift_task_getCurrent() };
    (!task.is_null()).then_some(TaskRef(task))
}
