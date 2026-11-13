//! Swift Concurrency backend: every spawned future is a genuine Swift Task.

mod join;
pub(crate) mod rt;
mod selfcheck;
mod task;

pub use join::JoinHandle;

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
    task::spawn_with(options, future)
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
    spawn_with(SpawnOptions { priority: Priority::Default, main: false, copy_task_locals: false }, future)
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
