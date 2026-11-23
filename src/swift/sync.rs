//! Calling synchronous Swift functions (getters, methods, free functions) from any thread.
//!
//! Same argument lowering as async calls; the differences are in sys/csrc/funclets.c `rs_call_sync`: an indirect
//! result address travels in its own register, and direct results come back as return registers.

use super::call::{CallError, IndirectSlot, Owned, RetRegs, SwiftCall};
use super::func::{ArgLowering, ClassMethod, SwiftArgs, SwiftObject, SwiftRet, Target};
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
    method: ClassMethod,
    fixed: *const c_void,
    looked_up: OnceLock<Option<Target>>,
}

unsafe impl Send for SwiftFn {}
unsafe impl Sync for SwiftFn {}

impl SwiftFn {
    /// A function identified by its mangled symbol name (`$s…`), resolved with `dlsym` on first use.
    pub const fn new(symbol: &'static str) -> SwiftFn {
        SwiftFn { symbol, method: ClassMethod::NONE, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// A class's own member: dispatched as [`ClassMethod`] says, else its own symbol.
    #[doc(hidden)]
    pub const fn __dispatched(symbol: &'static str, method: ClassMethod) -> SwiftFn {
        SwiftFn { symbol, method, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// # Safety
    /// `address` must be the entry point of a synchronous Swift function.
    pub const unsafe fn from_address(address: *const c_void) -> SwiftFn {
        SwiftFn { symbol: "<address>", method: ClassMethod::NONE, fixed: address, looked_up: OnceLock::new() }
    }

    /// [`from_address`](Self::from_address) for a class's own member, linked by its own symbol: dispatched as
    /// [`ClassMethod`] says when the process has the thunk or method descriptor.
    ///
    /// # Safety
    /// `address` must be the member's entry point.
    #[doc(hidden)]
    pub const unsafe fn __linked(address: *const c_void, method: ClassMethod) -> SwiftFn {
        SwiftFn { symbol: "<address>", method, fixed: address, looked_up: OnceLock::new() }
    }

    #[inline]
    fn resolve(&self) -> Option<Target> {
        *self.looked_up.get_or_init(|| {
            self.method.resolve("", || match self.fixed.is_null() {
                true => lookup(self.symbol),
                false => self.fixed as usize,
            })
        })
    }

    pub fn is_available(&self) -> bool {
        self.resolve().is_some()
    }

    /// `now`: the call is made before the arguments' borrows end (see [`SwiftFnNow`]).
    #[inline]
    fn prepare<R>(&self, args: impl SwiftArgs, slot: IndirectSlot, indirect: bool, now: bool) -> SwiftSyncCall<R> {
        let mut lowering = ArgLowering::new(false);
        lowering.now = now;
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
    #[inline]
    pub fn call_declared<R: SwiftRet>(&self, args: impl SwiftArgs, result_type: super::func::DeclaredResult) -> SwiftSyncCall<R> {
        self.declared(args, result_type, false)
    }

    #[inline]
    fn declared<R: SwiftRet>(&self, args: impl SwiftArgs, result_type: super::func::DeclaredResult, now: bool) -> SwiftSyncCall<R> {
        match R::result_slot(result_type) {
            Ok((slot, indirect)) => self.prepare(args, slot, indirect, now),
            Err(error) => {
                let mut call = self.prepare((), IndirectSlot::inline(), false, false);
                call.invalid = Some(error);
                call
            }
        }
    }

    /// Like [`call`](Self::call) for a result returned by address: a generic, large or resilient value of `ty`.
    pub fn call_returning(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftSyncCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), false), true, false)
    }

    /// The function for a call made right away (what `swift!` generates for a sync function it invokes at once).
    #[doc(hidden)]
    #[inline]
    pub const fn __now(&'static self) -> SwiftFnNow {
        SwiftFnNow(self)
    }

    /// Like [`call`](Self::call) for a result of `ty` returned in registers (at most four words of integers
    /// and references: arrays, dictionaries, small structs), kept as an opaque value.
    pub fn call_direct(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftSyncCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), true), false, false)
    }
}

/// What the scalar path makes for a call and releases after it: the Swift strings made of `&str` arguments (each one
/// owned reference, its storage's, released as the general path's `Owned::Bridge` is).
#[doc(hidden)]
#[derive(Default)]
pub struct Made {
    bridges: [u64; 4],
    count: usize,
}

impl Drop for Made {
    fn drop(&mut self) {
        for &bridge in &self.bridges[..self.count] {
            unsafe { super::rt::swift_bridgeObjectRelease(bridge as *mut c_void) };
        }
    }
}

/// A Rust string, made a Swift `String` for the call (two integer registers) and released after it.
impl ScalarArg for &str {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, made: &mut Made) -> bool {
        if *ints + 2 > regs.gpr.len() || made.count == made.bridges.len() {
            return false;
        }
        let string = super::rt::string_from_utf8(self);
        made.bridges[made.count] = string.words[1];
        made.count += 1;
        regs.gpr[*ints] = string.words[0];
        regs.gpr[*ints + 1] = string.words[1];
        *ints += 2;
        true
    }
}

impl ScalarArg for String {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, floats: &mut usize, made: &mut Made) -> bool {
        self.as_str().put(regs, ints, floats, made)
    }
}

/// Arguments that go in registers and need nothing kept alive, handed over or checked: integers, `bool`, `f64`, and
/// borrowed class instances, as [`SwiftArg`](super::func::SwiftArg) lowers them.
#[doc(hidden)]
pub trait ScalarArgs {
    /// Fills the argument registers, keeping in `made` what it makes for the call (released after it); `false` if
    /// there are more than the registers hold.
    fn put(self, regs: &mut apus_sys::CallRegs, made: &mut Made) -> bool;
}

/// One scalar argument: its integer or floating-point register.
#[doc(hidden)]
pub trait ScalarArg {
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, floats: &mut usize, made: &mut Made) -> bool;
}

macro_rules! scalar_int {
    ($($t:ty => $via:ty),*) => {$(
        impl ScalarArg for $t {
            #[inline]
            fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, _made: &mut Made) -> bool {
                let Some(slot) = regs.gpr.get_mut(*ints) else { return false };
                *slot = self as $via as u64; // sign- or zero-extended to the full register
                *ints += 1;
                true
            }
        }
    )*};
}
scalar_int!(i64 => i64, i32 => i64, isize => i64, u64 => u64, u32 => u64, usize => u64, bool => u64);

impl ScalarArg for f64 {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, _ints: &mut usize, floats: &mut usize, _made: &mut Made) -> bool {
        let Some(slot) = regs.fpr.get_mut(*floats) else { return false };
        *slot = self;
        *floats += 1;
        true
    }
}

/// A class instance borrowed for a call made right away: its reference, which the caller's borrow keeps alive (a
/// consumed one never takes this path: `swift!` sends it through the general one, which hands over +1).
impl ScalarArg for &SwiftObject {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, _made: &mut Made) -> bool {
        let Some(slot) = regs.gpr.get_mut(*ints) else { return false };
        *slot = self.as_ptr() as u64;
        *ints += 1;
        true
    }
}

/// A value borrowed for a call made right away, passed by address (a resilient struct's): its own address, which
/// the caller's borrow keeps (a consumed one takes the general path, which hands over a copy).
impl ScalarArg for &SwiftValue {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, _made: &mut Made) -> bool {
        let Some(slot) = regs.gpr.get_mut(*ints) else { return false };
        *slot = self.as_ptr() as u64;
        *ints += 1;
        true
    }
}

/// A Swift string borrowed for a call made right away: its two words (a consumed one takes the general path).
impl ScalarArg for &super::SwiftString {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, _made: &mut Made) -> bool {
        if *ints + 2 > regs.gpr.len() {
            return false;
        }
        regs.gpr[*ints] = self.0.words[0];
        regs.gpr[*ints + 1] = self.0.words[1];
        *ints += 2;
        true
    }
}

/// A receiver borrowed for a call made right away, passed as it is in the `self` register: a class instance's
/// reference, or the address of a value passed by address. Neither is retained or copied: the caller's borrow keeps
/// it, and a borrowed `self` the callee may neither change nor keep (`mutating` and `consuming` members take the
/// general path, which `swift!` sends them on).
#[doc(hidden)]
pub trait ScalarReceiver {
    fn receiver(self) -> *mut c_void;
}

impl ScalarReceiver for &SwiftObject {
    #[inline]
    fn receiver(self) -> *mut c_void {
        self.as_ptr()
    }
}

impl ScalarReceiver for &SwiftValue {
    #[inline]
    fn receiver(self) -> *mut c_void {
        self.as_ptr()
    }
}

macro_rules! scalar_args {
    ($($($a:ident)*;)*) => {$(
        impl<$($a: ScalarArg),*> ScalarArgs for ($($a,)*) {
            #[inline]
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn put(self, regs: &mut apus_sys::CallRegs, made: &mut Made) -> bool {
                let ($($a,)*) = self;
                let (mut ints, mut floats) = (0, 0);
                true $(&& $a.put(regs, &mut ints, &mut floats, made))*
            }
        }
    )*};
}
scalar_args! {
    ;
    A;
    A B;
    A B C;
    A B C D;
    A B C D E;
    A B C D E F;
    A B C D E F G;
    A B C D E F G H;
}

impl SwiftFn {
    /// A call whose arguments are all [`ScalarArgs`] and whose result comes back in registers, made right away with
    /// no call record: the registers filled, the trampoline called, the result read. What `swift!` generates for
    /// such a call (the same lowering as the general path, which keeps what other arguments need).
    #[doc(hidden)]
    #[inline]
    pub fn __call_scalar<R: SwiftRet>(
        &'static self,
        this: *mut c_void,
        args: impl ScalarArgs,
        throws: bool,
        result_type: super::func::DeclaredResult,
    ) -> Result<R, CallError> {
        // `this`: the receiver, borrowed (null without one); a class's member is looked up in its vtable through it.
        let function = self.resolve().ok_or(CallError::Unavailable(self.symbol))?.entry(this)?;
        let mut regs = apus_sys::CallRegs { gpr: [0; 8], fpr: [0.0; 8], self_: this, isolation: -1 };
        let mut made = Made::default();
        if !args.put(&mut regs, &mut made) {
            return Err(CallError::InvalidArgument("too many arguments for the argument registers"));
        }
        // A result by address (a resilient struct's: `SwiftValue`) gets its storage, as the general path's does.
        let (mut slot, indirect) = R::result_slot(result_type)?;
        slot.resolve(&[])?;
        let mut ret = RetRegs::new();
        let kind = if indirect { KIND_INDIRECT } else if R::float() { KIND_FLOATS } else { KIND_INTEGERS };
        unsafe {
            let address = if indirect { slot.address() } else { std::ptr::null_mut() };
            apus_sys::rs_call_sync(function, &regs, address, &mut ret.0, kind);
            if throws && !ret.0.error.is_null() {
                return Err(CallError::Swift(SwiftError::from_raw(ret.0.error)));
            }
            Ok(R::lift(&ret, &mut slot))
        }
    }
}

/// A synchronous function called right away, while its arguments are still borrowed: a value it borrows by address
/// (`&SwiftValue`) is passed as it is rather than copied, as for a call that may run later. What `swift!` uses for
/// a sync function's call that it invokes at once.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct SwiftFnNow(&'static SwiftFn);

impl SwiftFnNow {
    #[inline]
    pub fn call_declared<R: SwiftRet>(self, args: impl SwiftArgs, result_type: super::func::DeclaredResult) -> SwiftSyncCall<R> {
        self.0.declared(args, result_type, true)
    }

    #[inline]
    pub fn call_returning(self, ty: SwiftType, args: impl SwiftArgs) -> SwiftSyncCall<SwiftValue> {
        self.0.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), false), true, true)
    }
}

/// A prepared synchronous call.
pub struct SwiftSyncCall<R> {
    /// Its entry point, or the vtable's for the `self` bound before it runs.
    function: Option<Target>,
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
    #[inline]
    pub fn invoke(mut self) -> Result<R, CallError> {
        if let Some(error) = self.invalid.take() {
            return Err(error);
        }
        let function = self.function.ok_or(CallError::Unavailable(self.symbol))?.entry(self.lowering.regs.self_)?;
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
        let kind = if self.indirect { KIND_INDIRECT } else if R::float() { KIND_FLOATS } else { KIND_INTEGERS };
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
        let kind = if indirect { KIND_INDIRECT } else if R::float() { KIND_FLOATS } else { KIND_INTEGERS };
        let executor = match to {
            HopTo::Main => Some(unsafe { super::rt::swift_task_getMainExecutor() }),
            HopTo::SelfActor => actor_executor(lowering.regs.self_),
        };
        let (function, dispatch) = match function.map(|target| target.entry(lowering.regs.self_)) {
            Some(Ok(function)) => (Some(function), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, Some(CallError::Unavailable(symbol))),
        };
        let problem = invalid
            .or_else(|| lowering.borrows.then_some(CallError::InvalidArgument("an `inout` argument (`&mut`) cannot hop to an executor")))
            .or(dispatch)
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

impl<R> super::func::CheckedCall for SwiftSyncCall<R> {
    fn checked(mut self, check: Result<(), CallError>) -> Self {
        if let Err(error) = check {
            self.invalid = Some(error);
        }
        self
    }
}

impl<R> super::func::ClassSelf for SwiftSyncCall<R> {
    fn class_self(mut self, class: Result<SwiftType, CallError>) -> Self {
        match class {
            Ok(ty) => self.on_type(ty),
            Err(error) => {
                self.invalid.get_or_insert(error);
                self
            }
        }
    }
}
