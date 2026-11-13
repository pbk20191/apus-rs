//! `sleep` on the Swift runtime's own timer: a plain Swift Job enqueued with
//! `swift_task_enqueueGlobalWithDelay` that wakes the sleeping task when it runs.

use super::rt;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

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
    waker: Waker,
}

const JOB_KIND_PLAIN: u32 = 1;
const PRIORITY_DEFAULT: u32 = 0x15;

#[no_mangle]
extern "C" fn rs_job_run(job: *mut c_void) {
    unsafe { Box::from_raw(job.cast::<WakeJob>()) }.waker.wake();
}

fn wake_after(delay: Duration, waker: Waker) {
    let job = Box::new(WakeJob {
        metadata: unsafe { rt::rs_job_metadata() },
        refcounts: 0,
        scheduler_private: [std::ptr::null_mut(); 2],
        flags: JOB_KIND_PLAIN | PRIORITY_DEFAULT << 8,
        id: 0,
        voucher: std::ptr::null_mut(),
        reserved: std::ptr::null_mut(),
        run_job: unsafe { rt::rs_run_job_fn() },
        waker,
    });
    let nanos = u64::try_from(delay.as_nanos()).unwrap_or(u64::MAX);
    unsafe { rt::swift_task_enqueueGlobalWithDelay(nanos, Box::into_raw(job).cast()) };
}

/// Completes after `duration`.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep { deadline: Instant::now() + duration, armed: false }
}

pub struct Sleep {
    deadline: Instant,
    armed: bool,
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Poll::Ready(());
        }
        if !self.armed {
            self.armed = true;
            wake_after(left, cx.waker().clone());
        }
        Poll::Pending
    }
}
