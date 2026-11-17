//! `sleep` on the Swift runtime's own timer: a plain Swift Job enqueued with
//! `swift_task_enqueueGlobalWithDelay` that wakes the sleeping task when it runs.

use super::rt;
use crate::common::TimerSlot;
pub use crate::common::Sleep;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

/// include/swift/ABI/Task.h `class Job` (64 bytes), followed by our payload.
#[repr(C, align(16))]
struct WakeJob {
    metadata: *const c_void,
    refcounts: u64,
    scheduler_private: [*mut c_void; 2],
    flags: u32, // JobFlags: kind in bits 0-7 (1: unused by the runtime), priority in bits 8-15
    id: u32,
    voucher: *mut c_void,
    reserved: *mut c_void,
    run_job: *const c_void,
    slot: Arc<TimerSlot>,
}

const JOB_KIND_PLAIN: u32 = 1;
const PRIORITY_DEFAULT: u32 = 0x15;

#[export_name = apus_sys::boundary_name!(rs_job_run)]
extern "C" fn rs_job_run(job: *mut c_void) {
    unsafe { Box::from_raw(job.cast::<WakeJob>()) }.slot.fire();
}

fn arm_timer(delay: Duration, slot: Arc<TimerSlot>) {
    let job = Box::new(WakeJob {
        metadata: unsafe { rt::rs_job_metadata() },
        refcounts: 0,
        scheduler_private: [std::ptr::null_mut(); 2],
        flags: JOB_KIND_PLAIN | PRIORITY_DEFAULT << 8,
        id: 0,
        voucher: std::ptr::null_mut(),
        reserved: std::ptr::null_mut(),
        run_job: unsafe { rt::rs_run_job_fn() },
        slot,
    });
    let nanos = u64::try_from(delay.as_nanos()).unwrap_or(u64::MAX);
    unsafe { rt::swift_task_enqueueGlobalWithDelay(nanos, Box::into_raw(job).cast()) };
}

/// Completes after `duration`.
pub fn sleep(duration: Duration) -> Sleep {
    // The timer is a Job the runtime runs: check the runtime first, even before any spawn.
    super::selfcheck::ensure();
    Sleep::new(duration, arm_timer)
}

/// Awaits `future` for at most `duration`: `None` if it has not completed by then. The future is dropped on
/// timeout; whether that cancels the work behind it is up to the future (a [`SwiftStream`](super::SwiftStream)
/// element stays pending for the next call, a Swift call marked `.child()` is cancelled).
pub async fn within<F: std::future::Future>(duration: Duration, future: F) -> Option<F::Output> {
    use std::future::Future as _;
    let mut future = std::pin::pin!(future);
    let mut deadline = std::pin::pin!(sleep(duration));
    std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(cx) {
            return std::task::Poll::Ready(Some(output));
        }
        deadline.as_mut().poll(cx).map(|()| None)
    })
    .await
}
