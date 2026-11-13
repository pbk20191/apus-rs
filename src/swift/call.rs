//! Awaiting a Swift `() async throws -> Any` closure inside the current Swift Task.
//!
//! A `SwiftAsync` registers itself on the task while being polled. When the poll ends Pending with exactly one
//! registered call and no other wake, the funclet calls the closure in this same Task (Swift's `await f()`);
//! the closure returns to `after_call`, which reports through `rs_call_done` and polls again. Concurrent calls
//! become child Tasks (rs_take_child).
//!
//! The closure arrives as an `AsyncSwiftClosure` box (+1) built by the user's Swift code (see README):
//! `final class AsyncSwiftClosure { let body: () async throws -> Any }`.

use super::any::{SwiftAny, SwiftError};
use super::{rt, task};
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicPtr, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub enum CallError {
    /// The closure threw.
    Swift(SwiftError),
    /// Awaited outside a Swift task: there is no Task to run the closure in.
    NoCurrentTask,
}

impl fmt::Debug for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Swift(e) => write!(f, "Swift({e:?})"),
            CallError::NoCurrentTask => f.write_str("NoCurrentTask"),
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Swift(e) => write!(f, "Swift error: {e}"),
            CallError::NoCurrentTask => f.write_str("a Swift async call was awaited outside a Swift task"),
        }
    }
}

impl std::error::Error for CallError {}

enum CallState {
    Registered,
    Running,
    /// Finished; error box (+1) or null for a result in `result`.
    Done(*mut c_void),
    Taken,
    /// The awaiting future was dropped; whoever finishes the call cleans up.
    Abandoned,
}

pub(crate) struct CallShared {
    closure: *mut c_void, // AsyncSwiftClosure box, +1
    result: UnsafeCell<[usize; 4]>,
    state: Mutex<CallState>,
    waker: Mutex<Option<Waker>>,
    child: AtomicPtr<c_void>, // +1 child AsyncTask when run as a child
    force_child: bool,
}

unsafe impl Send for CallShared {}
unsafe impl Sync for CallShared {}

impl CallShared {
    fn new(closure: *mut c_void, force_child: bool) -> CallShared {
        CallShared {
            closure,
            result: UnsafeCell::new([0; 4]),
            state: Mutex::new(CallState::Registered),
            waker: Mutex::new(None),
            child: AtomicPtr::default(),
            force_child,
        }
    }

    fn result_ptr(&self) -> *mut c_void {
        self.result.get().cast()
    }

    fn take_value(&self, error: *mut c_void) -> Result<SwiftAny, CallError> {
        if error.is_null() {
            Ok(unsafe { SwiftAny::from_raw_initialized(*self.result.get()) })
        } else {
            Err(CallError::Swift(unsafe { SwiftError::from_raw(error) }))
        }
    }
}

impl Drop for CallShared {
    fn drop(&mut self) {
        unsafe {
            rt::swift_release(self.closure);
            let child = *self.child.get_mut();
            if !child.is_null() {
                rt::swift_release(child);
            }
        }
    }
}

/// Pops a registered call. Same-Task only when it is the poll's only call, no wake is pending and it is not
/// forced to a child; otherwise any registered call (for the child path).
pub(crate) fn take_registered(t: &task::Task, same_task: bool) -> Option<Arc<CallShared>> {
    take_registered_from(&t.calls, same_task, task::notified(t))
}

fn take_registered_from(calls: &Mutex<Vec<Arc<CallShared>>>, same_task: bool, notified: bool) -> Option<Arc<CallShared>> {
    let mut calls = calls.lock().unwrap();
    calls.retain(|c| matches!(*c.state.lock().unwrap(), CallState::Registered));
    if same_task && !(calls.len() == 1 && !calls[0].force_child && !notified) {
        return None;
    }
    // Check and transition under the call's own lock: a future dropped meanwhile (Abandoned) is skipped.
    while let Some(call) = calls.pop() {
        let mut state = call.state.lock().unwrap();
        if matches!(*state, CallState::Registered) {
            *state = CallState::Running;
            drop(state);
            return Some(call);
        }
    }
    None
}

/// The record pointer given to the funclet for a child Task. It carries two references: one consumed by
/// rs_call_done when the child finishes, one by rs_set_child. The child may finish before spawn_child gets
/// to rs_set_child, so the second keeps the record alive until then.
fn hand_to_child(call: Arc<CallShared>) -> *const c_void {
    let record = Arc::into_raw(call);
    unsafe { Arc::increment_strong_count(record) };
    record.cast()
}

#[no_mangle]
extern "C" fn rs_take_call(rust: *mut c_void, closure: *mut *mut c_void, result: *mut *mut c_void, record: *mut *const c_void) -> i32 {
    let t = unsafe { &*(rust as *const task::Task) };
    let Some(call) = take_registered(t, true) else { return 0 };
    unsafe {
        *closure = call.closure;
        *result = call.result_ptr();
        *record = Arc::into_raw(call).cast();
    }
    1
}

/// Calls the funclet must run as child Tasks (discarding TaskGroup): everything still registered after
/// `rs_take_call` declined.
#[no_mangle]
extern "C" fn rs_take_child(rust: *mut c_void, record: *mut *const c_void) -> i32 {
    let t = unsafe { &*(rust as *const task::Task) };
    let Some(call) = take_registered(t, false) else { return 0 };
    unsafe { *record = hand_to_child(call) };
    1
}

#[no_mangle]
extern "C" fn rs_call_closure(record: *const c_void, closure: *mut *mut c_void, result: *mut *mut c_void) {
    let call = unsafe { &*(record as *const CallShared) };
    unsafe {
        *closure = call.closure;
        *result = call.result_ptr();
    }
}

/// Stores the child Task (+1) and releases the reference hand_to_child added for this call.
#[no_mangle]
extern "C" fn rs_set_child(record: *const c_void, child: *mut c_void) {
    let call = unsafe { Arc::from_raw(record as *const CallShared) };
    call.child.store(child, SeqCst);
}

#[no_mangle]
extern "C" fn rs_call_done(record: *const c_void, error: *mut c_void) {
    let call = unsafe { Arc::from_raw(record as *const CallShared) };
    let mut state = call.state.lock().unwrap();
    if matches!(*state, CallState::Abandoned) {
        drop(state);
        drop(call.take_value(error)); // nobody will read it
        return;
    }
    *state = CallState::Done(error);
    drop(state);
    let waker = call.waker.lock().unwrap().take();
    if let Some(w) = waker {
        w.wake();
    }
}

/// An owned Swift async closure; awaiting it runs the closure in the current Swift Task.
pub struct SwiftAsync {
    shared: Arc<CallShared>,
    registered: bool,
}

impl SwiftAsync {
    /// Takes ownership of an `AsyncSwiftClosure` box (+1), as returned by the README recipe.
    ///
    /// # Safety
    /// `closure_box` must be an owned `AsyncSwiftClosure` instance.
    pub unsafe fn from_raw(closure_box: *mut c_void) -> SwiftAsync {
        Self::with_mode(closure_box, false)
    }

    /// Runs the closure in a child Task (Swift's `async let`) instead of this Task, so other futures polled
    /// alongside it (`select`, timeouts) keep running. Dropping the future then cancels the child.
    pub fn child(mut self) -> SwiftAsync {
        Arc::get_mut(&mut self.shared).expect("child() must be called before the call is awaited").force_child = true;
        self
    }

    fn with_mode(closure_box: *mut c_void, force_child: bool) -> SwiftAsync {
        SwiftAsync { registered: false, shared: Arc::new(CallShared::new(closure_box, force_child)) }
    }
}

impl Future for SwiftAsync {
    type Output = Result<SwiftAny, CallError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            let mut state = self.shared.state.lock().unwrap();
            if let CallState::Done(error) = *state {
                *state = CallState::Taken;
                drop(state);
                return Poll::Ready(self.shared.take_value(error));
            }
        }
        *self.shared.waker.lock().unwrap() = Some(cx.waker().clone());
        if self.registered {
            return Poll::Pending;
        }
        let Some(t) = task::current() else { return Poll::Ready(Err(CallError::NoCurrentTask)) };
        self.registered = true;
        t.calls.lock().unwrap().push(self.shared.clone());
        Poll::Pending
    }
}

impl Drop for SwiftAsync {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        match *state {
            CallState::Running => {
                *state = CallState::Abandoned;
                let child = self.shared.child.load(SeqCst);
                if !child.is_null() {
                    unsafe { rt::swift_task_cancel(child) };
                }
            }
            CallState::Registered => *state = CallState::Abandoned,
            CallState::Done(error) => {
                *state = CallState::Taken;
                drop(state);
                drop(self.shared.take_value(error));
            }
            CallState::Taken | CallState::Abandoned => {}
        }
    }
}

/// Awaits `call` for at most `duration`. The call runs as a child Task; on timeout it is cancelled and `None`
/// is returned.
pub async fn timeout(duration: std::time::Duration, call: SwiftAsync) -> Option<Result<SwiftAny, CallError>> {
    let mut call = std::pin::pin!(call.child());
    let mut deadline = std::pin::pin!(super::sleep(duration));
    std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = call.as_mut().poll(cx) {
            return Poll::Ready(Some(result));
        }
        deadline.as_mut().poll(cx).map(|()| None)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(force_child: bool) -> Arc<CallShared> {
        // A null closure: CallShared::drop's swift_release(NULL) is a no-op.
        Arc::new(CallShared::new(std::ptr::null_mut(), force_child))
    }

    /// spawn_child calls rs_set_child after creating the child, and the child may already have finished
    /// (rs_call_done consumed the record) and the awaiting future may be gone. The record must still be alive.
    #[test]
    fn child_record_outlives_early_completion() {
        let call = registered(true);
        let calls = Mutex::new(vec![call.clone()]);
        let taken = take_registered_from(&calls, false, false).expect("a registered call");
        let record = hand_to_child(taken);
        rs_call_done(record, std::ptr::null_mut()); // the child finished first
        assert!(Arc::strong_count(&call) >= 2, "only the test's reference is left: rs_set_child would read freed memory");
        drop(call); // the awaiting future is gone too
        rs_set_child(record, std::ptr::null_mut()); // must be the last user of the record
    }

    /// A call abandoned before the funclet takes it must never be started.
    #[test]
    fn abandoned_call_is_not_taken() {
        let call = registered(false);
        let calls = Mutex::new(vec![call.clone()]);
        *call.state.lock().unwrap() = CallState::Abandoned;
        assert!(take_registered_from(&calls, false, false).is_none());
    }
}
