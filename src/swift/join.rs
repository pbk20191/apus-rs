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
    #[allow(dead_code)] // read by abort(), which arrives with cancellation
    pub(crate) abort: Arc<AbortState>,
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
