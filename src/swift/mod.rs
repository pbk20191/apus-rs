//! Swift Concurrency backend: every spawned future is a genuine Swift Task.

mod any;
pub(crate) mod call;
mod closure;
mod current;
mod enums;
mod func;
mod isolation;
mod generic;
mod join;
mod macros;
pub(crate) mod rt;
mod selfcheck;
mod stream;
mod sync;
#[cfg(feature = "objc2")]
mod objc;
mod task;
mod timer;
mod value;

pub use any::{FromSwiftAny, SwiftAny, SwiftError, SwiftHashable};
pub use call::{timeout, AsyncFunctionPointer, CallError, IndirectSlot, RetRegs, SwiftAsync, SwiftCall};
pub use closure::{ByAddress, ClosureParam, ClosureResult, SwiftClosure};
#[doc(hidden)]
pub use closure::{ClosureArgs, ClosureFn, ClosureOut, DeclaredClosure, ResultKind, Shaped};
pub use enums::{EnumCases, SwiftEnumCase};
pub use stream::SwiftStream;
pub use sync::{HopTo, SwiftFn, SwiftSyncCall};
pub use value::{IntoSwift, OpaqueResult, Opened, OpenedType, SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
pub use isolation::Isolation;
#[doc(hidden)]
pub use generic::{
    __generic_args, __metatype_of, __object_type_of, __type_of, AsMetatype, ClassBound, ClassReference, GenericArgs, HasSwiftType,
    Metatype,
};
pub use func::{__bind, __bind_sync, BindAsyncReceiver, BindReceiver, Consumed, DeclaredResult, ErrorSlot, ArgLowering, DirectValue, SwiftArg, SwiftArgs, SwiftAsyncFn, SwiftObject, SwiftReceiver, SwiftRet};
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
/// let handle = apus::swift::spawn_local_on_main(async { std::rc::Rc::new(1) });
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

/// What [`async_main`]'s future may produce: `()`, or a `Result` whose error ends the process with code 1.
pub trait MainResult {
    #[doc(hidden)]
    fn report(self);
}

impl MainResult for () {
    fn report(self) {}
}

impl<E: std::fmt::Debug> MainResult for Result<(), E> {
    fn report(self) {
        if let Err(error) = self {
            eprintln!("Error: {error:?}");
            std::process::exit(1);
        }
    }
}

/// Runs `future` as the process's main, the way Swift runs an async `@main`: Swift's own `_runAsyncMain` starts it as
/// a main task (on the main actor), drains the main queue, and exits the process with code 0 when it is done. An
/// `Err` exits with code 1, a panic with 101. Never returns; call it on the main thread, once, instead of
/// [`run_main`](crate::run_main).
pub fn async_main<F>(future: F) -> !
where
    F: std::future::Future + 'static,
    F::Output: MainResult,
{
    assert!(crate::is_main_thread(), "async_main must be called on the main thread");
    selfcheck::ensure();
    struct MainOnly<T>(T);
    // SAFETY: the task runs on the main actor only, so the future stays on the main thread.
    unsafe impl<T> Send for MainOnly<T> {}
    impl<F: std::future::Future> std::future::Future for MainOnly<F>
    where
        F::Output: MainResult,
    {
        type Output = ();
        fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
            let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.poll(cx))) {
                Ok(std::task::Poll::Ready(output)) => {
                    output.report();
                    std::task::Poll::Ready(())
                }
                Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
                Err(_) => std::process::exit(101), // the panic message is already printed
            }
        }
    }
    task::run_async_main(Box::pin(MainOnly(future)))
}
