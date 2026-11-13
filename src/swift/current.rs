//! The Swift task the caller is running in.

use super::{rt, task};
use crate::Priority;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering::SeqCst;
use std::task::{Context, Poll};

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

/// Whether the current Swift task is cancelled (`Task.isCancelled`). False outside a task.
pub fn is_cancelled() -> bool {
    current_task().is_some_and(|t| rt::is_cancelled(t.0))
}

/// The current task's priority (`Task.currentPriority`), including escalation.
/// Outside a task: `Unspecified`.
pub fn priority() -> Priority {
    match current_task() {
        Some(t) => Priority::from_raw(unsafe { rt::swift_task_currentPriority(t.0) } as u8),
        None => Priority::Unspecified,
    }
}

/// Resolves when the task this future is polled in gets cancelled. Never resolves outside a spawned task.
pub fn cancelled() -> Cancelled {
    Cancelled(())
}

pub struct Cancelled(());

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(t) = task::current() else { return Poll::Pending };
        if t.cancelled.load(SeqCst) {
            return Poll::Ready(());
        }
        t.cancel_wakers.lock().unwrap().push(cx.waker().clone());
        // The handler may have run between the check and the registration.
        if t.cancelled.load(SeqCst) { Poll::Ready(()) } else { Poll::Pending }
    }
}
