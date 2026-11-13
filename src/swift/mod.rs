//! Swift Concurrency backend: every spawned future is a genuine Swift Task.

mod any;
pub(crate) mod call;
mod current;
mod func;
mod join;
mod macros;
pub(crate) mod rt;
mod selfcheck;
mod stream;
mod sync;
mod task;
mod timer;
mod value;

pub use any::{FromSwiftAny, SwiftAny, SwiftError, SwiftHashable};
pub use call::{timeout, AsyncFunctionPointer, CallError, IndirectSlot, RetRegs, SwiftAsync, SwiftCall};
pub use stream::SwiftStream;
pub use sync::{SwiftFn, SwiftSyncCall};
pub use value::{IntoSwift, SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
pub use func::{ArgLowering, SwiftArg, SwiftArgs, SwiftAsyncFn, SwiftObject, SwiftRet};
pub use current::{cancelled, current_task, is_cancelled, priority, Cancelled, TaskRef};
pub use join::{JoinHandle, MainOnlyHandle};
pub use timer::{sleep, within, Sleep};

use crate::{Priority, SpawnOptions};
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
///
/// The handle is `!Send`, so a `!Send` output can only be awaited on the main thread:
///
/// ```compile_fail
/// fn assert_send<T: Send>(_: &T) {}
/// let handle = async_swift_rs::swift::spawn_local_on_main(async { std::rc::Rc::new(1) });
/// assert_send(&handle);
/// ```
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
