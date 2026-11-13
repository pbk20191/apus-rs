//! Types shared by both backends.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Task priority. Raw values are QoS classes, which Swift's `JobPriority` also uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    UserInteractive,
    UserInitiated,
    Default,
    Utility,
    Background,
    /// Inherit from the current task, or from the current thread's QoS.
    Unspecified,
}

impl Priority {
    pub fn raw(self) -> u8 {
        match self {
            Priority::UserInteractive => 0x21,
            Priority::UserInitiated => 0x19,
            Priority::Default => 0x15,
            Priority::Utility => 0x11,
            Priority::Background => 0x09,
            Priority::Unspecified => 0,
        }
    }

    /// Maps a raw value to the nearest class at or below it.
    pub fn from_raw(raw: u8) -> Priority {
        match raw {
            0 => Priority::Unspecified,
            0x21.. => Priority::UserInteractive,
            0x19.. => Priority::UserInitiated,
            0x15.. => Priority::Default,
            0x11.. => Priority::Utility,
            _ => Priority::Background,
        }
    }
}

/// Why a task produced no value.
pub enum JoinError {
    Aborted,
    Panicked(Box<dyn Any + Send>),
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Aborted => f.write_str("Aborted"),
            JoinError::Panicked(_) => f.write_str("Panicked(..)"),
        }
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Aborted => f.write_str("task was aborted"),
            JoinError::Panicked(_) => f.write_str("task panicked"),
        }
    }
}

impl std::error::Error for JoinError {}

/// How to spawn a task. `Default` matches Swift's `Task {}`: inherit priority, copy task-locals, not main.
#[derive(Clone, Copy, Debug)]
pub struct SpawnOptions {
    pub priority: Priority,
    pub main: bool,
    pub copy_task_locals: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        SpawnOptions { priority: Priority::Unspecified, main: false, copy_task_locals: true }
    }
}

/// Shared between a `JoinHandle` and its task: the abort request and the waker that notices it.
pub(crate) struct AbortState {
    requested: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// Wraps the user future: catches panics and stops at the next poll after `abort`.
pub(crate) struct Guarded<F> {
    future: Option<F>,
    abort: Arc<AbortState>,
}

impl<F> Guarded<F> {
    /// Drops the user future; a panicking `Drop` becomes `JoinError::Panicked` instead of unwinding further.
    fn drop_future(&mut self) -> Result<(), JoinError> {
        catch_unwind(AssertUnwindSafe(|| self.future = None)).map_err(|payload| {
            std::mem::forget(self.future.take()); // already partly dropped by the panic; never touch it again
            JoinError::Panicked(payload)
        })
    }
}

impl<F: Future> Future for Guarded<F> {
    type Output = Result<F::Output, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        if this.abort.requested.load(Ordering::SeqCst) {
            return Poll::Ready(this.drop_future().and(Err(JoinError::Aborted)));
        }
        *this.abort.waker.lock().unwrap() = Some(cx.waker().clone());
        let Some(future) = this.future.as_mut() else { return Poll::Ready(Err(JoinError::Aborted)) };
        let future = unsafe { Pin::new_unchecked(future) };
        match catch_unwind(AssertUnwindSafe(|| future.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(v)) => Poll::Ready(this.drop_future().map(|()| v)),
            Err(payload) => {
                let _ = this.drop_future();
                Poll::Ready(Err(JoinError::Panicked(payload)))
            }
        }
    }
}

impl AbortState {
    /// Asks the task to stop at its next poll and wakes it so that poll happens.
    pub(crate) fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        if let Some(w) = self.waker.lock().unwrap().take() {
            w.wake();
        }
    }
}

pub(crate) fn guard<F>(future: F) -> (Guarded<F>, Arc<AbortState>) {
    let abort = Arc::new(AbortState { requested: AtomicBool::new(false), waker: Mutex::new(None) });
    (Guarded { future: Some(future), abort: abort.clone() }, abort)
}
