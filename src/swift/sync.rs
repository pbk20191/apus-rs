//! Calling synchronous Swift functions (getters, methods, free functions) from any thread.
//!
//! Same argument lowering as async calls; the differences are in sys/csrc/funclets.c `rs_call_sync`: an indirect
//! result address travels in its own register, and direct results come back as return registers.

use super::call::{CallError, IndirectSlot, Owned, RetRegs, SwiftCall};
use super::func::{ArgLowering, SwiftArgs, SwiftObject, SwiftRet};
use super::value::{lookup, SwiftType, SwiftValue, UninitValue};
use super::SwiftError;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::OnceLock;

const KIND_INTEGERS: i32 = 0;
const KIND_FLOATS: i32 = 1;
const KIND_INDIRECT: i32 = 2;

/// A synchronous Swift function, named by its mangled symbol.
pub struct SwiftFn {
    symbol: &'static str,
    /// A class member's dispatch thunk, tried first; empty if none.
    thunk: &'static str,
    fixed: *const c_void,
    looked_up: OnceLock<usize>,
}

unsafe impl Send for SwiftFn {}
unsafe impl Sync for SwiftFn {}

impl SwiftFn {
    /// A function identified by its mangled symbol name (`$s…`), resolved with `dlsym` on first use.
    pub const fn new(symbol: &'static str) -> SwiftFn {
        SwiftFn { symbol, thunk: "", fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// A class member: its dispatch thunk (`…Tj`) if the process has it, else its own symbol.
    #[doc(hidden)]
    pub const fn __dispatched(symbol: &'static str, thunk: &'static str) -> SwiftFn {
        SwiftFn { symbol, thunk, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// # Safety
    /// `address` must be the entry point of a synchronous Swift function.
    pub const unsafe fn from_address(address: *const c_void) -> SwiftFn {
        SwiftFn { symbol: "<address>", thunk: "", fixed: address, looked_up: OnceLock::new() }
    }

    fn resolve(&self) -> Option<*const c_void> {
        if !self.fixed.is_null() {
            return Some(self.fixed);
        }
        let address = *self.looked_up.get_or_init(|| match self.thunk {
            "" => lookup(self.symbol),
            thunk => Some(lookup(thunk)).filter(|&a| a != 0).unwrap_or_else(|| lookup(self.symbol)),
        });
        (address != 0).then_some(address as *const c_void)
    }

    pub fn is_available(&self) -> bool {
        self.resolve().is_some()
    }

    fn prepare<R>(&self, args: impl SwiftArgs, slot: IndirectSlot, indirect: bool) -> SwiftSyncCall<R> {
        let mut lowering = ArgLowering::new(false);
        args.lower(&mut lowering);
        SwiftSyncCall { function: self.resolve(), symbol: self.symbol, lowering, slot, indirect, throws: false, invalid: None, result: PhantomData }
    }

    /// Prepares a call returning `R`; run it with [`SwiftSyncCall::invoke`].
    /// The argument and result types must match the Swift signature: the compiler cannot check them.
    pub fn call<R: SwiftRet>(&self, args: impl SwiftArgs) -> SwiftSyncCall<R> {
        self.call_declared(args, super::func::DeclaredResult::UNKNOWN)
    }

    /// [`call`](Self::call) for a function declared with its Swift result type (what `swift!` generates).
    #[doc(hidden)]
    pub fn call_declared<R: SwiftRet>(&self, args: impl SwiftArgs, result_type: super::func::DeclaredResult) -> SwiftSyncCall<R> {
        match R::result_slot(result_type) {
            Ok((slot, indirect)) => self.prepare(args, slot, indirect),
            Err(error) => {
                let mut call = self.prepare((), IndirectSlot::inline(), false);
                call.invalid = Some(error);
                call
            }
        }
    }

    /// Like [`call`](Self::call) for a result returned by address: a generic, large or resilient value of `ty`.
    pub fn call_returning(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftSyncCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), false), true)
    }

    /// Like [`call`](Self::call) for a result of `ty` returned in registers (at most four words of integers
    /// and references: arrays, dictionaries, small structs), kept as an opaque value.
    pub fn call_direct(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftSyncCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), true), false)
    }
}

/// A prepared synchronous call.
pub struct SwiftSyncCall<R> {
    function: Option<*const c_void>,
    symbol: &'static str,
    pub(crate) lowering: ArgLowering,
    slot: IndirectSlot,
    indirect: bool,
    throws: bool,
    /// A `self` the callee consumes (`__consuming` methods): passed by address, not destroyed afterwards.
    /// A problem found while preparing the call; reported by `invoke` instead of running it.
    invalid: Option<CallError>,
    result: PhantomData<fn() -> R>,
}

impl<R> SwiftSyncCall<R> {
    /// Declares the callee `throws`.
    pub fn throws(mut self) -> Self {
        self.throws = true;
        self
    }

    /// Calls a method of a class: `object` is `self`.
    pub fn on(mut self, object: &SwiftObject) -> Self {
        self.lowering.regs.self_ = object.as_ptr();
        self.lowering.owned.push(Owned::Object(unsafe { super::rt::swift_unknownObjectRetain(object.as_ptr()) })); // may outlive the borrow
        self
    }

    /// Calls a struct's setter or `mutating` method: the callee changes `value` in place (`inout self`).
    pub fn on_inout_value(mut self, value: &mut SwiftValue) -> Self {
        self.lowering.regs.self_ = value.as_ptr();
        self.lowering.borrows = true;
        self
    }

    /// Calls a class's initializer or static member: the class (its metadata, which lives forever) is `self`.
    pub fn on_type(mut self, ty: SwiftType) -> Self {
        self.lowering.regs.self_ = ty.as_ptr() as *mut std::ffi::c_void;
        self
    }

    /// Calls a protocol requirement on an opened `any P`: the value's address is `self`, and the dynamic type and
    /// the witness table follow the arguments.
    pub fn on_opened(mut self, opened: &super::value::Opened) -> Self {
        let copy = opened.copy();
        self.lowering.regs.self_ = opened.self_of(&copy);
        self.lowering.int(opened.ty().as_ptr() as u64);
        self.lowering.int(opened.witness().0 as u64);
        self.lowering.generic_args.extend([opened.ty().as_ptr(), opened.witness().0]);
        self.lowering.owned.push(Owned::Value(copy)); // `.hop()` runs the call later
        self
    }

    /// Calls a protocol's static member or initializer on a conforming type: its metadata (a thick metatype, which
    /// lives forever) is `self`, and the type and the witness table follow the arguments.
    pub fn on_opened_type(mut self, opened: &super::value::OpenedType) -> Self {
        self.lowering.regs.self_ = opened.ty().as_ptr() as *mut std::ffi::c_void;
        self.lowering.int(opened.ty().as_ptr() as u64);
        self.lowering.int(opened.witness().0 as u64);
        self.lowering.generic_args.extend([opened.ty().as_ptr(), opened.witness().0]);
        self
    }

    /// Calls a method of a value passed by address (a large or resilient struct): `value` is `self`.
    pub fn on_value(mut self, value: &SwiftValue) -> Self {
        let copy = value.clone();
        self.lowering.regs.self_ = copy.as_ptr();
        self.lowering.owned.push(Owned::Value(copy));
        self
    }
}

impl<R> SwiftSyncCall<R> {
    /// Calls a `__consuming` method of a value passed by address: the callee takes ownership of `value`.
    pub(crate) fn on_consumed(mut self, value: SwiftValue) -> Self {
        self.lowering.regs.self_ = value.as_ptr();
        self.lowering.owned.push(Owned::Value(value).pending());
        self
    }
}

impl<R: SwiftRet> SwiftSyncCall<R> {
    /// Runs the call on the current thread.
    pub fn invoke(mut self) -> Result<R, CallError> {
        if let Some(error) = self.invalid.take() {
            return Err(error);
        }
        let function = self.function.ok_or(CallError::Unavailable(self.symbol))?;
        if let Some(failure) = self.lowering.failure.take() {
            return Err(failure);
        }
        if let Some(why) = self.lowering.error {
            return Err(CallError::InvalidArgument(why));
        }
        self.slot.resolve(&self.lowering.generic_args)?;
        let mut ret = RetRegs::new();
        // A sync call runs right here: on the main actor if this is a main task's poll.
        let main_task = super::task::current().is_some_and(|task| task.main);
        unsafe { apus_sys::apus_fill_isolation(&mut self.lowering.regs, main_task) };
        let kind = if self.indirect { KIND_INDIRECT } else if R::FLOAT { KIND_FLOATS } else { KIND_INTEGERS };
        unsafe {
            super::call::start_all(&mut self.lowering.owned);
            apus_sys::rs_call_sync(function, &self.lowering.regs, self.slot.address(), &mut ret.0, kind);
            let typed_error = self.lowering.typed_error.take();
            let result = if let (Some(slot), false) = (typed_error, ret.0.error.is_null()) {
                Err(CallError::Typed(slot.assume_init())) // the callee threw: the slot holds its error
            } else if self.throws && !ret.0.error.is_null() {
                Err(CallError::Swift(SwiftError::from_raw(ret.0.error)))
            } else {
                Ok(R::lift(&ret, &mut self.slot))
            };
            result
        }
    }
}


/// Where a hopping call runs: what `swift!` generates for the sync members of an actor or of the main actor.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum HopTo {
    /// The `self` actor's executor (`Actor.unownedExecutor`).
    SelfActor,
    /// The main actor's executor.
    Main,
}

impl<R: SwiftRet> SwiftSyncCall<R> {
    /// The call as an awaitable that runs on `to`'s executor: the Task hops there, calls, and hops back to its own
    /// executor. An object that is not an actor makes it `CallError::NotAnActor`.
    #[doc(hidden)]
    pub fn hop(self, to: HopTo) -> SwiftCall<R> {
        let SwiftSyncCall { function, symbol, lowering, slot, indirect, throws, invalid, .. } = self;
        let kind = if indirect { KIND_INDIRECT } else if R::FLOAT { KIND_FLOATS } else { KIND_INTEGERS };
        let executor = match to {
            HopTo::Main => Some(unsafe { super::rt::swift_task_getMainExecutor() }),
            HopTo::SelfActor => actor_executor(lowering.regs.self_),
        };
        let problem = invalid
            .or_else(|| lowering.borrows.then_some(CallError::InvalidArgument("an `inout` argument (`&mut`) cannot hop to an executor")))
            .or_else(|| function.is_none().then_some(CallError::Unavailable(symbol)))
            .or_else(|| executor.is_none().then_some(CallError::NotAnActor));
        // The sync result address is the hop's own (rs_call_sync), not the first integer argument.
        let mut call = SwiftCall::build(std::ptr::null(), lowering, problem, slot, false);
        if throws {
            call = call.throws();
        }
        if let (Some(function), Some(executor)) = (function, executor) {
            call.set_hop(apus_sys::HopCall { function, indirect: std::ptr::null_mut(), executor, kind });
        }
        call
    }
}

/// The executor of `actor`: its `Actor` conformance's `unownedExecutor` (a default actor's own, or a custom one).
fn actor_executor(actor: *mut c_void) -> Option<super::rt::SerialExecutorRef> {
    use super::rt;
    if actor.is_null() {
        return None;
    }
    unsafe {
        let ty = rt::swift_getObjectType(actor);
        let witness = rt::swift_conformsToProtocol(ty, &rt::ACTOR_PROTOCOL as *const u8 as *const c_void);
        (!witness.is_null()).then(|| rt::apus_actor_unowned_executor(actor, ty, witness))
    }
}
