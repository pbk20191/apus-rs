//! Serial queues: tasks spawned on the same queue never run at the same time.

use super::ffi::*;
use super::{spawn_on, JoinHandle, Queue};
use crate::Priority;
use std::ffi::CString;
use std::future::Future;
use std::sync::Arc;

struct OwnedQueue(Queue);

impl Drop for OwnedQueue {
    fn drop(&mut self) {
        unsafe { dispatch_release(self.0.raw()) }
    }
}

/// A serial dispatch queue. Pending tasks keep it alive after the `SerialQueue` is dropped.
#[derive(Clone)]
pub struct SerialQueue(Arc<OwnedQueue>);

impl SerialQueue {
    pub fn new(label: &str, priority: Priority) -> SerialQueue {
        let label = CString::new(label).expect("queue label contains a NUL byte");
        let qos = match priority {
            Priority::Unspecified => 0,
            p => p.raw() as u32,
        };
        let queue = unsafe {
            let attr = dispatch_queue_attr_make_with_qos_class(std::ptr::null_mut(), qos, 0);
            dispatch_queue_create(label.as_ptr(), attr)
        };
        SerialQueue(Arc::new(OwnedQueue(Queue(queue))))
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        spawn_on(self.0 .0, self.0.clone(), future)
    }
}
