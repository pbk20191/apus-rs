//! Running a Swift async call inside the current Swift Task.
//!
//! A [`SwiftCall`] registers itself on the task while being polled. When the poll ends Pending with exactly one
//! registered call and no other wake, the funclet calls the callee in this same Task (Swift's `await f()`): it
//! loads the argument registers from the call record and tail-calls; the callee returns to `after_call`, which
//! stores the result registers, reports through `rs_call_done`, and polls again. Concurrent calls become child
//! Tasks (`rs_take_child`).
//!
//! Every Swift async function has the same shape at this level (apus-sys include/apus/abi.h `GenericAsyncFn`), so one
//! path serves direct function calls ([`super::SwiftAsyncFn`]) and closures ([`SwiftAsync`]).

use super::any::{SwiftAny, SwiftError};
use super::func::{ArgLowering, SwiftRet};
use super::value::{SwiftValue, UninitValue};
use super::{rt, task};
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicPtr, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// What a Swift async function symbol with the `Tu` suffix points at (lib/IRGen `AsyncFunctionPointer`):
/// the code's offset relative to this record, and the size of the context the caller must allocate.
#[repr(transparent)]
pub struct AsyncFunctionPointer(apus_sys::AsyncFunctionPointer);

/// Argument registers of a call (apus-sys `CallRegs`).
pub(crate) use apus_sys::CallRegs;

/// Result registers of a finished call (apus-sys `RetRegs`).
#[doc(hidden)]
#[repr(transparent)]
pub struct RetRegs(pub(crate) apus_sys::RetRegs);

impl RetRegs {
    pub(crate) const fn new() -> RetRegs {
        RetRegs(apus_sys::RetRegs { gpr: [0; 4], fpr: [0.0; 4], error: std::ptr::null_mut() })
    }
}

pub enum CallError {
    /// The callee threw.
    Swift(SwiftError),
    /// Awaited outside a Swift task: there is no Task to run the call in.
    NoCurrentTask,
    /// The function's symbol is not present in this process (e.g. an API newer than the running OS).
    Unavailable(&'static str),
    /// The arguments cannot be passed (too many for the argument registers, or a string with a NUL byte).
    InvalidArgument(&'static str),
}

impl fmt::Debug for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Swift(e) => write!(f, "Swift({e:?})"),
            CallError::NoCurrentTask => f.write_str("NoCurrentTask"),
            CallError::Unavailable(symbol) => write!(f, "Unavailable({symbol})"),
            CallError::InvalidArgument(why) => write!(f, "InvalidArgument({why})"),
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Swift(e) => write!(f, "Swift error: {e}"),
            CallError::NoCurrentTask => f.write_str("a Swift async call was awaited outside a Swift task"),
            CallError::Unavailable(symbol) => write!(f, "Swift function {symbol} is not available"),
            CallError::InvalidArgument(why) => write!(f, "invalid argument: {why}"),
        }
    }
}

impl std::error::Error for CallError {}

/// Something the call keeps alive until it is over; released when dropped.
pub(crate) enum Owned {
    Object(*mut c_void),
    /// A `String`'s owner word.
    Bridge(u64),
    /// A value passed by address; dropped with the record.
    Value(#[allow(dead_code)] SwiftValue),
    /// Shared storage the callee works on in place (an `inout` self).
    Shared(#[allow(dead_code)] Arc<dyn std::any::Any + Send + Sync>),
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            match *self {
                Owned::Object(object) => rt::swift_release(object),
                Owned::Bridge(word) => rt::swift_bridgeObjectRelease(word as *mut c_void),
                Owned::Value(_) | Owned::Shared(_) => {} // release themselves
            }
        }
    }
}

/// Where an indirect result is written: four inline words (`Any`) or storage sized for a given Swift type.
#[doc(hidden)]
pub struct IndirectSlot {
    pub(crate) inline: [usize; 4],
    pub(crate) boxed: Option<UninitValue>,
    /// The boxed type is returned in registers, not through an address: copy the result words into the box.
    pub(crate) direct: bool,
}

impl IndirectSlot {
    pub(crate) fn inline() -> IndirectSlot {
        IndirectSlot { inline: [0; 4], boxed: None, direct: false }
    }

    pub(crate) fn boxed(storage: UninitValue, direct: bool) -> IndirectSlot {
        assert!(!direct || storage.ty.size() <= 32, "a type larger than four words is not returned in registers");
        IndirectSlot { inline: [0; 4], boxed: Some(storage), direct }
    }

    pub(crate) fn address(&mut self) -> *mut c_void {
        match &self.boxed {
            Some(storage) => storage.ptr.as_ptr().cast(),
            None => self.inline.as_mut_ptr().cast(),
        }
    }
}

enum CallState {
    Registered,
    Running,
    /// Finished; the results are in `ret` / `indirect`.
    Done,
    Taken,
    /// The awaiting future was dropped; whoever finishes the call discards the result.
    Abandoned,
}

pub(crate) struct CallShared {
    function: *const AsyncFunctionPointer,
    regs: UnsafeCell<CallRegs>,
    ret: UnsafeCell<RetRegs>,
    indirect: UnsafeCell<IndirectSlot>,
    /// Whether the callee is `throws`: only then is the error register meaningful.
    throws: bool,
    owned: Vec<Owned>,
    /// Drops a finished result nobody will read (monomorphized for the call's result type).
    discard: unsafe fn(&CallShared),
    state: Mutex<CallState>,
    waker: Mutex<Option<Waker>>,
    child: AtomicPtr<c_void>, // +1 child AsyncTask when run as a child
    force_child: bool,
}

unsafe impl Send for CallShared {}
unsafe impl Sync for CallShared {}

impl CallShared {
    /// `indirect`: the callee writes its result through the slot's address, passed as first integer argument
    /// (`lowering` reserved that register).
    fn new<R: SwiftRet>(function: *const AsyncFunctionPointer, lowering: ArgLowering, slot: IndirectSlot, indirect: bool) -> Arc<CallShared> {
        let shared = Arc::new(CallShared {
            function,
            regs: UnsafeCell::new(lowering.regs),
            ret: UnsafeCell::new(RetRegs::new()),
            indirect: UnsafeCell::new(slot),
            throws: false,
            owned: lowering.owned,
            discard: discard_as::<R>,
            state: Mutex::new(CallState::Registered),
            waker: Mutex::new(None),
            child: AtomicPtr::default(),
            force_child: false,
        });
        if indirect {
            unsafe { (*shared.regs.get()).gpr[0] = (*shared.indirect.get()).address() as u64 };
        }
        shared
    }

    /// Takes the finished call's result. Call exactly once, after `Done`.
    unsafe fn take_result<R: SwiftRet>(&self) -> Result<R, CallError> {
        let ret = &*self.ret.get();
        if self.throws && !ret.0.error.is_null() {
            return Err(CallError::Swift(SwiftError::from_raw(ret.0.error)));
        }
        Ok(R::lift(ret, &mut *self.indirect.get()))
    }
}

unsafe fn discard_as<R: SwiftRet>(call: &CallShared) {
    drop(call.take_result::<R>());
}

impl Drop for CallShared {
    fn drop(&mut self) {
        unsafe {
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

unsafe fn describe_target(call: &CallShared, function: *mut *const AsyncFunctionPointer, regs: *mut *mut CallRegs, ret: *mut *mut RetRegs) {
    *function = call.function;
    *regs = call.regs.get();
    *ret = call.ret.get();
}

#[export_name = apus_sys::boundary_name!(rs_take_call)]
extern "C" fn rs_take_call(
    rust: *mut c_void,
    function: *mut *const AsyncFunctionPointer,
    regs: *mut *mut CallRegs,
    ret: *mut *mut RetRegs,
    record: *mut *const c_void,
) -> i32 {
    let t = unsafe { &*(rust as *const task::Task) };
    let Some(call) = take_registered(t, true) else { return 0 };
    unsafe {
        describe_target(&call, function, regs, ret);
        *record = Arc::into_raw(call).cast();
    }
    1
}

/// Calls the funclet must run as child Tasks (discarding TaskGroup): everything still registered after
/// `rs_take_call` declined.
#[export_name = apus_sys::boundary_name!(rs_take_child)]
extern "C" fn rs_take_child(rust: *mut c_void, record: *mut *const c_void) -> i32 {
    let t = unsafe { &*(rust as *const task::Task) };
    let Some(call) = take_registered(t, false) else { return 0 };
    unsafe { *record = hand_to_child(call) };
    1
}

#[export_name = apus_sys::boundary_name!(rs_call_target)]
extern "C" fn rs_call_target(record: *const c_void, function: *mut *const AsyncFunctionPointer, regs: *mut *mut CallRegs, ret: *mut *mut RetRegs) {
    unsafe { describe_target(&*(record as *const CallShared), function, regs, ret) }
}

/// Stores the child Task (+1) and releases the reference hand_to_child added for this call.
#[export_name = apus_sys::boundary_name!(rs_set_child)]
extern "C" fn rs_set_child(record: *const c_void, child: *mut c_void) {
    let call = unsafe { Arc::from_raw(record as *const CallShared) };
    call.child.store(child, SeqCst);
}

#[export_name = apus_sys::boundary_name!(rs_call_done)]
extern "C" fn rs_call_done(record: *const c_void) {
    let call = unsafe { Arc::from_raw(record as *const CallShared) };
    let mut state = call.state.lock().unwrap();
    if matches!(*state, CallState::Abandoned) {
        drop(state);
        unsafe { (call.discard)(&call) }; // nobody will read it
        return;
    }
    *state = CallState::Done;
    drop(state);
    let waker = call.waker.lock().unwrap().take();
    if let Some(w) = waker {
        w.wake();
    }
}

/// A Swift async call about to run, or running, in the current Swift Task. Awaiting yields its result.
///
/// Awaited on its own it runs in this Task, like Swift's `await f()`. Polled together with other calls
/// (`join!`) or marked [`child`](Self::child) it runs as a child Task.
pub struct SwiftCall<R> {
    shared: Arc<CallShared>,
    /// A problem found while building the call; reported instead of running it.
    invalid: Option<CallError>,
    registered: bool,
    result: PhantomData<fn() -> R>,
}

/// A Swift `() async throws -> Any` closure handed over by the README's `asyncSwiftClosure` helper.
pub type SwiftAsync = SwiftCall<SwiftAny>;

impl<R: SwiftRet> SwiftCall<R> {
    pub(crate) fn build(
        function: *const AsyncFunctionPointer,
        mut lowering: ArgLowering,
        unavailable: Option<CallError>,
        slot: IndirectSlot,
        indirect: bool,
    ) -> SwiftCall<R> {
        let invalid = unavailable.or(lowering.error.take().map(CallError::InvalidArgument));
        SwiftCall { shared: CallShared::new::<R>(function, lowering, slot, indirect), invalid, registered: false, result: PhantomData }
    }
}

impl<R> SwiftCall<R> {
    fn configure(&mut self) -> &mut CallShared {
        Arc::get_mut(&mut self.shared).expect("a Swift call is configured before it is first awaited")
    }

    /// Declares the callee `throws`: its error becomes `Err(CallError::Swift(_))`.
    /// Leave it off for a non-throwing function, whose error register holds no meaningful value.
    pub fn throws(mut self) -> Self {
        self.configure().throws = true;
        self
    }

    /// Calls a method: `object` is the callee's `self`.
    pub fn on(mut self, object: &super::SwiftObject) -> Self {
        let call = self.configure();
        call.regs.get_mut().self_ = object.as_ptr();
        call.owned.push(Owned::Object(unsafe { rt::swift_retain(object.as_ptr()) }));
        self
    }

    /// Calls a method of a value passed by address (a large or resilient struct): `value` is `self`.
    pub fn on_value(mut self, value: &SwiftValue) -> Self {
        let copy = value.clone();
        let call = self.configure();
        call.regs.get_mut().self_ = copy.as_ptr();
        call.owned.push(Owned::Value(copy));
        self
    }

    /// Calls a `mutating` method: the callee works on the value at `address`, which `keep_alive` owns.
    pub(crate) fn on_inout(mut self, address: *mut c_void, keep_alive: Arc<dyn std::any::Any + Send + Sync>) -> Self {
        let call = self.configure();
        call.regs.get_mut().self_ = address;
        call.owned.push(Owned::Shared(keep_alive));
        self
    }

    /// Runs the call in a child Task (Swift's `async let`) instead of this Task, so other futures polled
    /// alongside it (`select`, timeouts) keep running. Dropping the future then cancels the child.
    pub fn child(mut self) -> Self {
        self.configure().force_child = true;
        self
    }
}

impl SwiftCall<SwiftAny> {
    /// Takes ownership of an `AsyncSwiftClosure` box (+1), as returned by the README recipe.
    ///
    /// # Safety
    /// `closure_box` must be an owned `AsyncSwiftClosure` instance.
    pub unsafe fn from_raw(closure_box: *mut c_void) -> SwiftAsync {
        // The thick function's context word is passed as the callee's self.
        let closure = &*(closure_box as *const apus_sys::AsyncClosureBox);
        let function = closure.function as *const AsyncFunctionPointer;
        let mut lowering = ArgLowering::new(true);
        lowering.regs.self_ = closure.context;
        lowering.owned.push(Owned::Object(closure_box));
        SwiftCall::build(function, lowering, None, IndirectSlot::inline(), true).throws()
    }
}

impl<R: SwiftRet> Future for SwiftCall<R> {
    type Output = Result<R, CallError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(error) = self.invalid.take() {
            return Poll::Ready(Err(error));
        }
        {
            let mut state = self.shared.state.lock().unwrap();
            if matches!(*state, CallState::Done) {
                *state = CallState::Taken;
                drop(state);
                return Poll::Ready(unsafe { self.shared.take_result::<R>() });
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

impl<R> Drop for SwiftCall<R> {
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
            CallState::Done => {
                *state = CallState::Taken;
                drop(state);
                unsafe { (self.shared.discard)(&self.shared) };
            }
            CallState::Taken | CallState::Abandoned => {}
        }
    }
}

/// Awaits `call` for at most `duration`. The call runs as a child Task; on timeout it is cancelled and `None`
/// is returned.
pub async fn timeout<R: SwiftRet>(duration: std::time::Duration, call: SwiftCall<R>) -> Option<Result<R, CallError>> {
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
        let mut call = SwiftCall::<()>::build(std::ptr::null(), ArgLowering::new(false), None, IndirectSlot::inline(), false);
        call.configure().force_child = force_child;
        call.shared.clone() // the SwiftCall is dropped: the record is Abandoned…
    }

    fn revive(call: &CallShared) {
        *call.state.lock().unwrap() = CallState::Registered; // …so tests put it back
    }

    /// spawn_child calls rs_set_child after creating the child, and the child may already have finished
    /// (rs_call_done consumed the record) and the awaiting future may be gone. The record must still be alive.
    #[test]
    fn child_record_outlives_early_completion() {
        let call = registered(true);
        revive(&call);
        let calls = Mutex::new(vec![call.clone()]);
        let taken = take_registered_from(&calls, false, false).expect("a registered call");
        let record = hand_to_child(taken);
        rs_call_done(record); // the child finished first
        assert!(Arc::strong_count(&call) >= 2, "only the test's reference is left: rs_set_child would read freed memory");
        drop(call); // the awaiting future is gone too
        rs_set_child(record, std::ptr::null_mut()); // must be the last user of the record
    }

    /// A call abandoned before the funclet takes it must never be started.
    #[test]
    fn abandoned_call_is_not_taken() {
        let call = registered(false);
        let calls = Mutex::new(vec![call.clone()]);
        assert!(take_registered_from(&calls, false, false).is_none());
    }
}
