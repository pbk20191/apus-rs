//! Calling synchronous Swift functions (getters, methods, free functions) from any thread.
//!
//! Same argument lowering as async calls; the differences are in csrc/funclets.c `rs_call_sync`: an indirect
//! result address travels in its own register, and direct results come back as return registers.

use super::call::{CallError, CallRegs, IndirectSlot, Owned, RetRegs};
use super::func::{ArgLowering, SwiftArgs, SwiftObject, SwiftRet};
use super::value::{lookup, SwiftType, SwiftValue, UninitValue};
use super::SwiftError;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::OnceLock;

extern "C" {
    fn rs_call_sync(function: *const c_void, regs: *const CallRegs, indirect: *mut c_void, ret: *mut RetRegs, kind: i32);
}
const KIND_INTEGERS: i32 = 0;
const KIND_FLOATS: i32 = 1;
const KIND_INDIRECT: i32 = 2;

/// A synchronous Swift function, named by its mangled symbol.
pub struct SwiftFn {
    symbol: &'static str,
    fixed: *const c_void,
    looked_up: OnceLock<usize>,
}

unsafe impl Send for SwiftFn {}
unsafe impl Sync for SwiftFn {}

impl SwiftFn {
    /// A function identified by its mangled symbol name (`$s…`), resolved with `dlsym` on first use.
    pub const fn new(symbol: &'static str) -> SwiftFn {
        SwiftFn { symbol, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// # Safety
    /// `address` must be the entry point of a synchronous Swift function.
    pub const unsafe fn from_address(address: *const c_void) -> SwiftFn {
        SwiftFn { symbol: "<address>", fixed: address, looked_up: OnceLock::new() }
    }

    fn resolve(&self) -> Option<*const c_void> {
        if !self.fixed.is_null() {
            return Some(self.fixed);
        }
        let address = *self.looked_up.get_or_init(|| lookup(self.symbol));
        (address != 0).then_some(address as *const c_void)
    }

    pub fn is_available(&self) -> bool {
        self.resolve().is_some()
    }

    fn prepare<R>(&self, args: impl SwiftArgs, slot: IndirectSlot, indirect: bool) -> SwiftSyncCall<R> {
        let mut lowering = ArgLowering::new(false);
        args.lower(&mut lowering);
        SwiftSyncCall { function: self.resolve(), symbol: self.symbol, lowering, slot, indirect, throws: false, consumed: None, result: PhantomData }
    }

    /// Prepares a call returning `R`; run it with [`SwiftSyncCall::invoke`].
    /// The argument and result types must match the Swift signature: the compiler cannot check them.
    pub fn call<R: SwiftRet>(&self, args: impl SwiftArgs) -> SwiftSyncCall<R> {
        self.prepare(args, IndirectSlot::inline(), R::INDIRECT)
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
    lowering: ArgLowering,
    slot: IndirectSlot,
    indirect: bool,
    throws: bool,
    /// A `self` the callee consumes (`__consuming` methods): passed by address, not destroyed afterwards.
    consumed: Option<SwiftValue>,
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
        self.lowering.regs.this = object.as_ptr();
        self.lowering.owned.push(Owned::Object(unsafe { super::rt::swift_retain(object.as_ptr()) })); // may outlive the borrow
        self
    }

    /// Calls a method of a value passed by address (a large or resilient struct): `value` is `self`.
    pub fn on_value(mut self, value: &SwiftValue) -> Self {
        let copy = value.clone();
        self.lowering.regs.this = copy.as_ptr();
        self.lowering.owned.push(Owned::Value(copy));
        self
    }
}

impl<R> SwiftSyncCall<R> {
    /// Calls a `__consuming` method of a value passed by address: the callee takes ownership of `value`.
    pub(crate) fn on_consumed(mut self, value: SwiftValue) -> Self {
        self.lowering.regs.this = value.as_ptr();
        self.consumed = Some(value);
        self
    }
}

impl<R: SwiftRet> SwiftSyncCall<R> {
    /// Runs the call on the current thread.
    pub fn invoke(mut self) -> Result<R, CallError> {
        let function = self.function.ok_or(CallError::Unavailable(self.symbol))?;
        if let Some(why) = self.lowering.error {
            return Err(CallError::InvalidArgument(why));
        }
        let mut ret = RetRegs { gpr: [0; 4], fpr: [0.0; 4], error: std::ptr::null_mut() };
        let kind = if self.indirect { KIND_INDIRECT } else if R::FLOAT { KIND_FLOATS } else { KIND_INTEGERS };
        unsafe {
            rs_call_sync(function, &self.lowering.regs, self.slot.address(), &mut ret, kind);
            if let Some(consumed) = self.consumed.take() {
                drop(consumed.into_uninit()); // the callee destroyed the value; only the storage is ours
            }
            let result = if self.throws && !ret.error.is_null() {
                Err(CallError::Swift(SwiftError::from_raw(ret.error)))
            } else {
                Ok(R::lift(&ret, &mut self.slot))
            };
            result
        }
    }
}

