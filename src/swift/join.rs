//! `JoinHandle` for Swift-backed tasks. It resolves when the *Swift Task* completes (after its children),
//! like Swift's `Task.value`, not when the Rust future returns.

use super::rt;
use crate::common::AbortState;
use crate::JoinError;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub(crate) struct JoinState<T> {
    value: Mutex<(Option<Result<T, JoinError>>, Option<Waker>)>,
}

impl<T> JoinState<T> {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(JoinState { value: Mutex::new((None, None)) })
    }

    pub(crate) fn complete(&self, result: Result<T, JoinError>) {
        let waker = {
            let mut g = self.value.lock().unwrap();
            g.0 = Some(result);
            g.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

pub struct JoinHandle<T> {
    pub(crate) state: Arc<JoinState<T>>,
    pub(crate) task: *mut c_void, // +1 AsyncTask
    pub(crate) abort: Arc<AbortState>,
}

impl<T> JoinHandle<T> {
    /// Cooperative cancellation, exactly Swift's `task.cancel()`: `cancelled()` resolves and
    /// `is_cancelled()` turns true inside the task; Swift code it calls sees `Task.isCancelled`.
    pub fn cancel(&self) {
        unsafe { rt::swift_task_cancel(self.task) }
    }

    /// Cancels the task and drops its future at the next poll; awaiting then yields `Err(JoinError::Aborted)`.
    pub fn abort(&self) {
        self.cancel();
        self.abort.request();
    }

    /// Raises the task's priority, as Swift does when a higher-priority task awaits it.
    pub fn escalate(&self, priority: crate::Priority) {
        unsafe { rt::swift_task_escalate(self.task, priority.raw() as usize) };
    }
}

unsafe impl<T: Send> Send for JoinHandle<T> {}
unsafe impl<T: Send> Sync for JoinHandle<T> {}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut g = self.state.value.lock().unwrap();
        match g.0.take() {
            Some(result) => Poll::Ready(result),
            None => {
                g.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        unsafe { rt::swift_release(self.task) }
    }
}

/// Re-types a handle whose output was wrapped to cross the `Send` bound (main-thread-only tasks).
pub(crate) fn unwrap_main_only<W: 'static, T: 'static>(inner: JoinHandle<W>, unwrap: fn(W) -> T) -> MainOnlyHandle<W, T> {
    MainOnlyHandle { inner, unwrap, main_thread_only: std::marker::PhantomData }
}

/// Handle for a `!Send` main-thread task: itself `!Send`/`!Sync`, so the output stays on the main thread.
pub struct MainOnlyHandle<W, T> {
    inner: JoinHandle<W>,
    unwrap: fn(W) -> T,
    main_thread_only: std::marker::PhantomData<*const ()>,
}

impl<W, T> Future for MainOnlyHandle<W, T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let unwrap = self.unwrap;
        Pin::new(&mut self.inner).poll(cx).map(|r| r.map(unwrap))
    }
}
