//! Serial queues: tasks spawned on the same queue never run at the same time.

use super::{spawn_on, JoinHandle, Target};
use crate::Priority;
use dispatch2::{DispatchQoS, DispatchQueue, DispatchQueueAttr, DispatchRetained};
use std::future::Future;

/// A serial dispatch queue. Pending tasks keep it alive after the `SerialQueue` is dropped.
#[derive(Clone)]
pub struct SerialQueue(DispatchRetained<DispatchQueue>);

impl SerialQueue {
    pub fn new(label: &str, priority: Priority) -> SerialQueue {
        let qos = DispatchQoS(match priority {
            Priority::Unspecified => 0,
            p => p.raw().into(),
        });
        let attr = DispatchQueueAttr::with_qos_class(None, qos, 0);
        SerialQueue(DispatchQueue::new(label, Some(&attr)))
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        spawn_on(Target::Queue(self.0.clone()), future)
    }
}
