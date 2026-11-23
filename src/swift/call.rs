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
use super::lock::Lock;
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
    /// A sync member declared on an `(actor)` was called on an object that does not conform to `Actor`.
    NotAnActor,
    /// A typed-throws callee (`throws(E)`) threw: its error, of the `ErrorSlot`'s type.
    Typed(SwiftValue),
    /// An automatic generic argument's type does not conform to a protocol the function requires; not called.
    NotConforming { ty: String, protocol: &'static str },
    /// A `SwiftClosure` argument does not have the declared function type's shape; not called.
    ClosureMismatch(String),
}

impl fmt::Debug for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Swift(e) => write!(f, "Swift({e:?})"),
            CallError::NoCurrentTask => f.write_str("NoCurrentTask"),
            CallError::Unavailable(symbol) => write!(f, "Unavailable({symbol})"),
            CallError::NotAnActor => f.write_str("NotAnActor"),
            CallError::Typed(value) => write!(f, "Typed({value:?})"),
            CallError::NotConforming { ty, protocol } => write!(f, "NotConforming({ty}, {protocol})"),
            CallError::InvalidArgument(why) => write!(f, "InvalidArgument({why})"),
            CallError::ClosureMismatch(why) => write!(f, "ClosureMismatch({why})"),
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
            CallError::NotAnActor => f.write_str("the object is not an actor"),
            CallError::Typed(value) => write!(f, "Swift error: {}", value.description()),
            CallError::NotConforming { ty, protocol } => write!(f, "{ty} does not conform to the required protocol {protocol}"),
            CallError::ClosureMismatch(why) => write!(f, "closure does not match its declared function type: {why}"),
        }
    }
}

impl std::error::Error for CallError {}

/// Something the call keeps alive until it is over; released when dropped.
pub(crate) enum Owned {
    /// A class reference (AnyObject), released with swift_unknownObjectRelease.
    Object(*mut c_void),
    /// A native heap object that is not a class instance (a closure context), released with swift_release:
    /// swift_unknownObjectRelease would read its metadata as a class and may take it for an ObjC object.
    Native(*mut c_void),
    /// An `any Error` box, released with swift_errorRelease.
    Error(*mut c_void),
    /// A `String`'s owner word.
    Bridge(u64),
    /// A value passed by address; dropped with the record.
    Value(#[allow(dead_code)] SwiftValue),
    /// The storage of a value the callee took (an owned argument passed by address): freed, not destroyed.
    Storage(#[allow(dead_code)] UninitValue),
    /// A C value's copy passed by address.
    Words(#[allow(dead_code)] Box<[u64]>),
    /// Shared storage the callee works on in place (an `inout` self).
    Shared(#[allow(dead_code)] Arc<dyn std::any::Any + Send + Sync>),
    /// An Objective-C block (a `_Block_copy`), released with `_Block_release`.
    #[cfg(feature = "block2")]
    Block(*mut c_void),
    /// What a consumed argument keeps alive, handed over to the callee when the call starts ([`start_all`]);
    /// released as usual if the call never runs (refused, or its future dropped first).
    Pending(Box<Owned>),
}

/// The call is starting: pending hand-overs become the callee's.
#[inline]
pub(crate) fn start_all(owned: &mut Vec<Owned>) {
    if owned.iter().any(|o| matches!(o, Owned::Pending(_))) {
        let all: Vec<Owned> = owned.drain(..).collect();
        owned.extend(all.into_iter().filter_map(Owned::start));
    }
}

impl Owned {
    /// Marks what this keeps alive for the callee to consume (+1), handed over when the call starts.
    pub(crate) fn pending(self) -> Owned {
        match self {
            Owned::Object(_) | Owned::Native(_) | Owned::Error(_) | Owned::Bridge(_) | Owned::Value(_) => Owned::Pending(Box::new(self)),
            #[cfg(feature = "block2")]
            Owned::Block(_) => Owned::Pending(Box::new(self)),
            other => other,
        }
    }

    fn start(self) -> Option<Owned> {
        match self {
            Owned::Pending(_) => {
                let this = std::mem::ManuallyDrop::new(self);
                let Owned::Pending(inner) = &*this else { unreachable!() };
                // Moved out of `this`, which is never dropped.
                unsafe { std::ptr::read(inner) }.hand_over()
            }
            other => Some(other),
        }
    }

    /// Hands what this keeps alive over to the callee, for an argument it consumes (+1): a reference's +1 becomes
    /// the callee's; a value passed by address becomes the callee's, its storage still ours until the call is over.
    fn hand_over(self) -> Option<Owned> {
        match self {
            Owned::Object(_) | Owned::Native(_) | Owned::Error(_) | Owned::Bridge(_) => {
                std::mem::forget(self);
                None
            }
            #[cfg(feature = "block2")]
            Owned::Block(_) => {
                std::mem::forget(self);
                None
            }
            Owned::Value(_) => {
                let this = std::mem::ManuallyDrop::new(self);
                let Owned::Value(value) = &*this else { unreachable!() };
                // Moved out of `this`, which is never dropped.
                let value = unsafe { std::ptr::read(value) };
                Some(Owned::Storage(value.into_uninit()))
            }
            other => Some(other),
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            match *self {
                Owned::Object(object) => rt::swift_unknownObjectRelease(object),
                Owned::Native(object) => rt::swift_release(object),
                Owned::Error(error) => rt::swift_errorRelease(error),
                Owned::Bridge(word) => rt::swift_bridgeObjectRelease(word as *mut c_void),
                #[cfg(feature = "block2")]
                Owned::Block(block) => block2::ffi::_Block_release(block),
                Owned::Value(_) | Owned::Storage(_) | Owned::Words(_) | Owned::Shared(_) | Owned::Pending(_) => {} // release themselves
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
    /// A `some P` result whose storage is allocated by [`resolve`](Self::resolve), once the generic arguments are known.
    pub(crate) opaque: Option<&'static super::value::OpaqueResult>,
    /// A protocol's `Self` result, allocated by [`resolve`](Self::resolve) for the receiver's type.
    pub(crate) receiver_self: bool,
    /// Storage for a C value returned by address (more than four words).
    pub(crate) words: Option<Box<[u64]>>,
}

impl IndirectSlot {
    #[inline]
    pub(crate) fn inline() -> IndirectSlot {
        IndirectSlot { inline: [0; 4], boxed: None, direct: false, opaque: None, receiver_self: false, words: None }
    }

    pub(crate) fn receiver_self() -> IndirectSlot {
        IndirectSlot { receiver_self: true, ..IndirectSlot::inline() }
    }

    pub(crate) fn boxed(storage: UninitValue, direct: bool) -> IndirectSlot {
        assert!(!direct || storage.ty.size() <= 32, "a type larger than four words is not returned in registers");
        IndirectSlot { boxed: Some(storage), direct, ..IndirectSlot::inline() }
    }

    pub(crate) fn opaque(result: &'static super::value::OpaqueResult) -> IndirectSlot {
        IndirectSlot { opaque: Some(result), ..IndirectSlot::inline() }
    }

    /// Storage of `size` bytes, 8-aligned: a C value returned by address.
    pub(crate) fn words(size: usize) -> IndirectSlot {
        IndirectSlot { words: Some(vec![0; size.div_ceil(8)].into_boxed_slice()), ..IndirectSlot::inline() }
    }

    /// Whether the storage still waits for [`resolve`](Self::resolve).
    pub(crate) fn pending(&self) -> bool {
        (self.opaque.is_some() || self.receiver_self) && self.boxed.is_none()
    }

    /// Allocates a `some P` result's storage for its actual type, given the call's generic arguments.
    #[inline]
    pub(crate) fn resolve(&mut self, generic_args: &[*const c_void]) -> Result<(), CallError> {
        if let (Some(opaque), None) = (self.opaque, &self.boxed) {
            self.boxed = Some(UninitValue::new(opaque.metadata(generic_args)?));
        }
        if self.receiver_self && self.boxed.is_none() {
            // A protocol member's generic arguments end with Self's metadata and witness table (the witness-method
            // convention); binding the receiver adds them last.
            let [.., self_type, _witness] = generic_args else {
                return Err(CallError::InvalidArgument("a `Self` result needs an `Opened` or `OpenedType` receiver"));
            };
            self.boxed = Some(UninitValue::new(unsafe { super::SwiftType::from_metadata(*self_type) }));
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn address(&mut self) -> *mut c_void {
        assert!(!self.pending(), "apus: an opaque result's storage is allocated before the call runs");
        match (&self.boxed, &mut self.words) {
            (Some(storage), _) => storage.ptr.as_ptr().cast(),
            (None, Some(words)) => words.as_mut_ptr().cast(),
            (None, None) => self.inline.as_mut_ptr().cast(),
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
    /// A class member's vtable dispatch, resolved against `self` when the call starts (`function` null until then).
    target: Option<super::func::Target>,
    regs: UnsafeCell<CallRegs>,
    ret: UnsafeCell<RetRegs>,
    indirect: UnsafeCell<IndirectSlot>,
    /// A typed-throws call's error slot (`ErrorSlot`).
    typed_error: UnsafeCell<Option<UninitValue>>,
    /// Whether the callee is `throws`: only then is the error register meaningful.
    throws: bool,
    /// Locked only when the call starts (pending hand-overs) and while it is configured.
    owned: Lock<Vec<Owned>>,
    /// The generic arguments passed, for an opaque result's type (see `IndirectSlot::resolve`).
    generic_args: Vec<*const c_void>,
    /// The first integer register holds the result's address.
    indirect_result: bool,
    /// Drops a finished result nobody will read (monomorphized for the call's result type).
    discard: unsafe fn(&CallShared),
    state: Lock<CallState>,
    waker: Lock<Option<Waker>>,
    child: AtomicPtr<c_void>, // +1 child AsyncTask when run as a child
    /// A sync call that hops to another executor (`function` null otherwise); `indirect` is filled when it runs.
    pub(crate) hop: apus_sys::HopCall,
    /// The next free integer argument register (receivers append arguments after the formal ones).
    next_int: usize,
    force_child: bool,
}

unsafe impl Send for CallShared {}
unsafe impl Sync for CallShared {}

impl CallShared {
    /// `indirect`: the callee writes its result through the slot's address, passed as first integer argument
    /// (`lowering` reserved that register).
    fn new<R: SwiftRet>(function: *const AsyncFunctionPointer, mut lowering: ArgLowering, slot: IndirectSlot, indirect: bool) -> Arc<CallShared> {
        let typed_error = lowering.typed_error.take();
        let next_int = lowering.ints;
        let shared = Arc::new(CallShared {
            function,
            target: None,
            regs: UnsafeCell::new(lowering.regs),
            ret: UnsafeCell::new(RetRegs::new()),
            indirect: UnsafeCell::new(slot),
            typed_error: UnsafeCell::new(typed_error),
            throws: false,
            owned: Lock::new(lowering.owned),
            generic_args: lowering.generic_args,
            indirect_result: indirect,
            discard: discard_as::<R>,
            state: Lock::new(CallState::Registered),
            waker: Lock::new(None),
            child: AtomicPtr::default(),
            force_child: false,
            next_int,
            hop: apus_sys::HopCall { function: std::ptr::null(), indirect: std::ptr::null_mut(), executor: rt::SerialExecutorRef { identity: std::ptr::null(), implementation: 0 }, kind: 0 },
        });
        // An opaque result's address is set once its storage is allocated, at the first poll.
        if indirect && !unsafe { (*shared.indirect.get()).pending() } {
            unsafe { (*shared.regs.get()).gpr[0] = (*shared.indirect.get()).address() as u64 };
        }
        shared
    }

    /// Takes the finished call's result. Call exactly once, after `Done`.
    unsafe fn take_result<R: SwiftRet>(&self) -> Result<R, CallError> {
        let ret = &*self.ret.get();
        if let (Some(slot), false) = ((*self.typed_error.get()).take(), ret.0.error.is_null()) {
            return Err(CallError::Typed(slot.assume_init())); // the callee threw: the slot holds its error
        }
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
    calls.retain(|c| matches!(*c.state.lock(), CallState::Registered));
    if same_task && !(calls.len() == 1 && !calls[0].force_child && !notified) {
        return None;
    }
    // Check and transition under the call's own lock: a future dropped meanwhile (Abandoned) is skipped.
    while let Some(call) = calls.pop() {
        let mut state = call.state.lock();
        if matches!(*state, CallState::Registered) {
            *state = CallState::Running;
            drop(state);
            start_all(&mut call.owned.lock());
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

unsafe fn describe_target(
    call: &CallShared,
    function: *mut *const AsyncFunctionPointer,
    regs: *mut *mut CallRegs,
    ret: *mut *mut RetRegs,
    hop: *mut apus_sys::HopCall,
) {
    *function = call.function;
    *regs = call.regs.get();
    *ret = call.ret.get();
    if !call.hop.function.is_null() {
        *hop = apus_sys::HopCall { indirect: (&mut *call.indirect.get()).address(), ..call.hop };
    }
}

#[export_name = apus_sys::boundary_name!(rs_take_call)]
extern "C" fn rs_take_call(
    rust: *mut c_void,
    function: *mut *const AsyncFunctionPointer,
    regs: *mut *mut CallRegs,
    ret: *mut *mut RetRegs,
    record: *mut *const c_void,
    hop: *mut apus_sys::HopCall,
) -> i32 {
    let t = unsafe { &*(rust as *const task::Task) };
    let Some(call) = take_registered(t, true) else { return 0 };
    unsafe {
        describe_target(&call, function, regs, ret, hop);
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
extern "C" fn rs_call_target(
    record: *const c_void,
    function: *mut *const AsyncFunctionPointer,
    regs: *mut *mut CallRegs,
    ret: *mut *mut RetRegs,
    hop: *mut apus_sys::HopCall,
) {
    unsafe { describe_target(&*(record as *const CallShared), function, regs, ret, hop) }
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
    let mut state = call.state.lock();
    if matches!(*state, CallState::Abandoned) {
        drop(state);
        unsafe { (call.discard)(&call) }; // nobody will read it
        return;
    }
    *state = CallState::Done;
    drop(state);
    let waker = call.waker.lock().take();
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
        let invalid = unavailable.or(lowering.failure.take()).or(lowering.error.take().map(CallError::InvalidArgument));
        SwiftCall { shared: CallShared::new::<R>(function, lowering, slot, indirect), invalid, registered: false, result: PhantomData }
    }
}

impl<R> SwiftCall<R> {
    /// Dispatches through the class's vtable, for the `self` bound before the call starts.
    pub(crate) fn dispatched(mut self, target: super::func::Target) -> Self {
        self.configure().target = Some(target);
        self
    }

    /// Turns this call into a hopping sync call (see `SwiftSyncCall::hop`).
    pub(crate) fn set_hop(&mut self, hop: apus_sys::HopCall) {
        self.configure().hop = hop;
    }

    pub(crate) fn owned_count(&mut self) -> usize {
        self.configure().owned.get_mut().len()
    }

    pub(crate) fn hand_over_owned(&mut self, start: usize) {
        super::func::hand_over(self.configure().owned.get_mut(), start);
    }

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
        call.owned.get_mut().push(Owned::Object(unsafe { rt::swift_unknownObjectRetain(object.as_ptr()) }));
        self
    }

    /// Calls a class's initializer or static member: the class (its metadata, which lives forever) is `self`.
    pub fn on_type(mut self, ty: super::SwiftType) -> Self {
        self.configure().regs.get_mut().self_ = ty.as_ptr() as *mut c_void;
        self
    }

    /// Calls a protocol requirement on an opened `any P`: the value's address is `self`, and the dynamic type and
    /// the witness table follow the arguments.
    pub fn on_opened(mut self, opened: &super::value::Opened) -> Self {
        let call = self.configure();
        if call.next_int + 2 > super::func::INT_REGISTERS {
            self.invalid.get_or_insert(CallError::InvalidArgument("too many integer arguments for the argument registers"));
            return self;
        }
        let copy = opened.copy();
        let regs = call.regs.get_mut();
        regs.self_ = opened.self_of(&copy);
        regs.gpr[call.next_int] = opened.ty().as_ptr() as u64;
        regs.gpr[call.next_int + 1] = opened.witness().0 as u64;
        call.generic_args.extend([opened.ty().as_ptr(), opened.witness().0]);
        call.next_int += 2;
        call.owned.get_mut().push(Owned::Value(copy));
        self
    }

    /// Calls a protocol's static member or initializer on a conforming type: its metadata (a thick metatype, which
    /// lives forever) is `self`, and the type and the witness table follow the arguments.
    pub fn on_opened_type(mut self, opened: &super::value::OpenedType) -> Self {
        let call = self.configure();
        if call.next_int + 2 > super::func::INT_REGISTERS {
            self.invalid.get_or_insert(CallError::InvalidArgument("too many integer arguments for the argument registers"));
            return self;
        }
        let regs = call.regs.get_mut();
        regs.self_ = opened.ty().as_ptr() as *mut c_void;
        regs.gpr[call.next_int] = opened.ty().as_ptr() as u64;
        regs.gpr[call.next_int + 1] = opened.witness().0 as u64;
        call.generic_args.extend([opened.ty().as_ptr(), opened.witness().0]);
        call.next_int += 2;
        self
    }

    /// Calls a method of a value passed by address (a large or resilient struct): `value` is `self`.
    pub fn on_value(mut self, value: &SwiftValue) -> Self {
        let copy = value.clone();
        let call = self.configure();
        call.regs.get_mut().self_ = copy.as_ptr();
        call.owned.get_mut().push(Owned::Value(copy));
        self
    }

    /// Calls a `mutating` method: the callee works on the value at `address`, which `keep_alive` owns.
    pub(crate) fn on_inout(mut self, address: *mut c_void, keep_alive: Arc<dyn std::any::Any + Send + Sync>) -> Self {
        let call = self.configure();
        call.regs.get_mut().self_ = address;
        call.owned.get_mut().push(Owned::Shared(keep_alive));
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
            let mut state = self.shared.state.lock();
            if matches!(*state, CallState::Done) {
                *state = CallState::Taken;
                drop(state);
                return Poll::Ready(unsafe { self.shared.take_result::<R>() });
            }
        }
        *self.shared.waker.lock() = Some(cx.waker().clone());
        if self.registered && !matches!(*self.shared.state.lock(), CallState::Registered) {
            return Poll::Pending; // running: the waker above hears when it is done
        }
        let Some(t) = task::current() else { return Poll::Ready(Err(CallError::NoCurrentTask)) };
        if !self.registered {
            // Configured now (receiver bound): an opaque result can be sized, a vtable entry found.
            let call = self.configure();
            if let Some(target) = call.target.take() {
                match target.entry(call.regs.get_mut().self_) {
                    Ok(function) => call.function = function as *const AsyncFunctionPointer,
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            let slot = call.indirect.get_mut();
            if slot.pending() {
                if let Err(error) = slot.resolve(&call.generic_args) {
                    return Poll::Ready(Err(error));
                }
                if call.indirect_result {
                    call.regs.get_mut().gpr[0] = call.indirect.get_mut().address() as u64;
                }
            }
        }
        // Still waiting to run: the task it was registered in may have finished without running it (the call
        // was returned out of it), so it registers in this task unless it is there already. Whichever task takes
        // it first runs it.
        let mut calls = t.calls.lock().unwrap();
        if !calls.iter().any(|c| Arc::ptr_eq(c, &self.shared)) {
            calls.push(self.shared.clone());
        }
        drop(calls);
        self.registered = true;
        Poll::Pending
    }
}

impl<R> Drop for SwiftCall<R> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
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
        *call.state.lock() = CallState::Registered; // …so tests put it back
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

impl<R> super::func::CheckedCall for SwiftCall<R> {
    fn checked(mut self, check: Result<(), CallError>) -> Self {
        if let Err(error) = check {
            self.invalid = Some(error);
        }
        self
    }
}

impl<R> super::func::ClassSelf for SwiftCall<R> {
    fn class_self(mut self, class: Result<super::SwiftType, CallError>) -> Self {
        match class {
            Ok(ty) => self.on_type(ty),
            Err(error) => {
                self.invalid.get_or_insert(error);
                self
            }
        }
    }
}
