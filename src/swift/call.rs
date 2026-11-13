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
    pub(crate) force_child: bool,
}

unsafe impl Send for CallShared {}
unsafe impl Sync for CallShared {}

impl CallShared {
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
    let mut calls = t.calls.lock().unwrap();
    calls.retain(|c| matches!(*c.state.lock().unwrap(), CallState::Registered));
    if same_task && !(calls.len() == 1 && !calls[0].force_child && !task::notified(t)) {
        return None;
    }
    let call = calls.pop()?;
    *call.state.lock().unwrap() = CallState::Running;
    Some(call)
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

#[no_mangle]
extern "C" fn rs_take_child(_rust: *mut c_void, _record: *mut *const c_void) -> i32 {
    0 // child Tasks: next step
}

#[no_mangle]
extern "C" fn rs_call_closure(record: *const c_void, closure: *mut *mut c_void, result: *mut *mut c_void) {
    let call = unsafe { &*(record as *const CallShared) };
    unsafe {
        *closure = call.closure;
        *result = call.result_ptr();
    }
}

#[no_mangle]
extern "C" fn rs_set_child(record: *const c_void, child: *mut c_void) {
    unsafe { &*(record as *const CallShared) }.child.store(child, SeqCst);
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

    fn with_mode(closure_box: *mut c_void, force_child: bool) -> SwiftAsync {
        SwiftAsync {
            registered: false,
            shared: Arc::new(CallShared {
                closure: closure_box,
                result: UnsafeCell::new([0; 4]),
                state: Mutex::new(CallState::Registered),
                waker: Mutex::new(None),
                child: AtomicPtr::default(),
                force_child,
            }),
        }
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
