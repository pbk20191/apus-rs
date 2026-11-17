//! Calling Swift async functions directly: finding them, lowering Rust arguments into Swift's argument
//! registers, and lifting results back out of the result registers.
//!
//! Lowering rules (Swift's calling convention, checked by tests/swift_fn.rs on arm64 and x86_64): integers,
//! `Bool` and references take the next integer register; `Double` the next floating-point register; `String`
//! two integer registers, passed guaranteed (+0). Direct results come back in the first result registers,
//! owned (+1): `String` in two, `Array`/`Dictionary`/objects as one reference, `Int?` as value + nil flag.

use super::any::{FromSwiftAny, SwiftAny, SwiftHashable};
use super::call::{AsyncFunctionPointer, CallError, CallRegs, IndirectSlot, Owned, RetRegs, SwiftCall};
use super::rt;
use super::value::{IntoSwift, SwiftType, SwiftValue, UninitValue, WitnessTable};
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::ptr::NonNull;
use std::sync::OnceLock;

/// Integer argument registers of the platform's Swift calling convention.
pub(crate) const INT_REGISTERS: usize = if cfg!(target_arch = "aarch64") { 8 } else { 6 };
const FLOAT_REGISTERS: usize = 8;

/// A Swift async function that can be called from a Swift-backend task.
///
/// ```ignore
/// // The mangled name, as `nm` prints it (without the leading underscore). Looked up on first use.
/// static SLEEP: SwiftAsyncFn = SwiftAsyncFn::new("$sScTss5NeverORszABRs_rlE5sleep11nanosecondsys6UInt64V_tYaKFZ");
/// SLEEP.call::<()>((50_000_000u64,)).throws().await?;
/// ```
pub struct SwiftAsyncFn {
    symbol: &'static str,
    fixed: *const AsyncFunctionPointer,
    looked_up: OnceLock<usize>,
}

unsafe impl Send for SwiftAsyncFn {}
unsafe impl Sync for SwiftAsyncFn {}

use libc::{dlsym, RTLD_DEFAULT};

impl SwiftAsyncFn {
    /// A function identified by its mangled symbol name (`$s…`), resolved with `dlsym` on first use.
    /// If the symbol is missing, calls yield `CallError::Unavailable` instead of failing to launch.
    pub const fn new(symbol: &'static str) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// A function identified by its async function pointer (the symbol with the `Tu` suffix).
    ///
    /// # Safety
    /// `pointer` must be a Swift async function pointer.
    pub const unsafe fn from_pointer(pointer: &'static AsyncFunctionPointer) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol: "<pointer>", fixed: pointer, looked_up: OnceLock::new() }
    }

    fn resolve(&self) -> Option<*const AsyncFunctionPointer> {
        if !self.fixed.is_null() {
            return Some(self.fixed);
        }
        let address = *self.looked_up.get_or_init(|| {
            let name = CString::new(format!("{}Tu", self.symbol)).expect("symbol contains a NUL byte");
            unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) as usize }
        });
        (address != 0).then_some(address as *const AsyncFunctionPointer)
    }

    /// Whether the function exists in this process.
    pub fn is_available(&self) -> bool {
        self.resolve().is_some()
    }

    /// Prepares a call with `args` (a tuple, in declaration order) returning `R`. Nothing runs until awaited.
    /// The argument and result types must match the Swift signature: the compiler cannot check them.
    pub fn call<R: SwiftRet>(&self, args: impl SwiftArgs) -> SwiftCall<R> {
        self.call_declared(args, None)
    }

    /// [`call`](Self::call) for a function declared with its Swift result type (what `swift!` generates).
    #[doc(hidden)]
    pub fn call_declared<R: SwiftRet>(&self, args: impl SwiftArgs, result_type: Option<&'static str>) -> SwiftCall<R> {
        match R::result_slot(result_type) {
            Ok((slot, indirect)) => self.prepare(args, slot, indirect),
            Err(error) => SwiftCall::build(std::ptr::null(), ArgLowering::new(false), Some(error), IndirectSlot::inline(), false),
        }
    }

    /// Like [`call`](Self::call) for a result returned by address: a value of generic type, or a large or
    /// resilient struct (every struct of an Apple framework). `ty` is the result's Swift type.
    pub fn call_returning(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), false), true)
    }

    /// Like [`call`](Self::call) for a result of `ty` returned in registers (at most four words of integers
    /// and references: arrays, dictionaries, small structs), kept as an opaque value.
    pub fn call_direct(&self, ty: SwiftType, args: impl SwiftArgs) -> SwiftCall<SwiftValue> {
        self.prepare(args, IndirectSlot::boxed(UninitValue::new(ty), true), false)
    }

    fn prepare<R: SwiftRet>(&self, args: impl SwiftArgs, slot: IndirectSlot, indirect: bool) -> SwiftCall<R> {
        let mut lowering = ArgLowering::new(indirect);
        args.lower(&mut lowering);
        match self.resolve() {
            Some(function) => SwiftCall::build(function, lowering, None, slot, indirect),
            None => SwiftCall::build(std::ptr::null(), lowering, Some(CallError::Unavailable(self.symbol)), slot, indirect),
        }
    }
}

/// Collects a call's arguments into registers.
#[doc(hidden)]
pub struct ArgLowering {
    pub(crate) regs: CallRegs,
    pub(crate) ints: usize,
    floats: usize,
    pub(crate) owned: Vec<Owned>,
    pub(crate) error: Option<&'static str>,
    /// The error slot of a typed-throws call (`ErrorSlot`): written only when the callee throws.
    pub(crate) typed_error: Option<UninitValue>,
    /// A reason not to make the call at all, found while lowering (the first one wins).
    pub(crate) failure: Option<CallError>,
}

impl ArgLowering {
    /// `indirect_result`: the first integer register is reserved for the result address.
    pub(crate) fn new(indirect_result: bool) -> ArgLowering {
        ArgLowering {
            regs: CallRegs { gpr: [0; 8], fpr: [0.0; 8], self_: std::ptr::null_mut(), isolation: -1 },
            ints: indirect_result as usize,
            floats: 0,
            owned: Vec::new(),
            error: None,
            typed_error: None,
            failure: None,
        }
    }

    /// Records why the call must not be made; it is reported instead of running it.
    pub(crate) fn fail(&mut self, error: CallError) {
        self.failure.get_or_insert(error);
    }

    /// The error slot of a typed-throws call: storage for an error of `ty`, passed by address.
    pub(crate) fn error_slot(&mut self, ty: SwiftType) {
        let storage = UninitValue::new(ty);
        self.int(storage.ptr.as_ptr() as u64);
        self.typed_error = Some(storage);
    }

    pub fn int(&mut self, value: u64) {
        if self.ints == INT_REGISTERS {
            self.error = Some("too many integer arguments for the argument registers");
            return;
        }
        self.regs.gpr[self.ints] = value;
        self.ints += 1;
    }

    /// An `isolated (any Actor)?` argument: two integer registers the call fills when it runs.
    pub(crate) fn isolation(&mut self) {
        let first = self.ints;
        self.int(0);
        self.int(0);
        if self.error.is_none() {
            self.regs.isolation = first as i32;
        }
    }

    pub fn float(&mut self, value: f64) {
        if self.floats == FLOAT_REGISTERS {
            self.error = Some("too many floating-point arguments for the argument registers");
            return;
        }
        self.regs.fpr[self.floats] = value;
        self.floats += 1;
    }

    /// A Swift `String` built from `value`, passed guaranteed and released when the call is over.
    pub fn string(&mut self, value: &str) {
        let string = rt::string_from_utf8(value);
        self.owned.push(Owned::Bridge(string.words[1]));
        self.int(string.words[0]);
        self.int(string.words[1]);
    }
}

/// A Rust value that can be passed as a Swift argument.
pub trait SwiftArg {
    #[doc(hidden)]
    fn lower(&self, into: &mut ArgLowering);
}

macro_rules! int_arg {
    ($($t:ty => $via:ty),*) => {$(
        impl SwiftArg for $t {
            fn lower(&self, into: &mut ArgLowering) {
                into.int(*self as $via as u64) // sign- or zero-extended to the full register
            }
        }
    )*};
}
int_arg!(i64 => i64, i32 => i64, isize => i64, u64 => u64, u32 => u64, usize => u64, bool => u64);

impl SwiftArg for f64 {
    fn lower(&self, into: &mut ArgLowering) {
        into.float(*self)
    }
}

impl SwiftArg for &str {
    fn lower(&self, into: &mut ArgLowering) {
        into.string(self)
    }
}

impl SwiftArg for String {
    fn lower(&self, into: &mut ArgLowering) {
        into.string(self)
    }
}

/// A class instance, passed guaranteed; the call keeps it alive.
impl SwiftArg for &SwiftObject {
    fn lower(&self, into: &mut ArgLowering) {
        into.owned.push(Owned::Object(unsafe { rt::swift_unknownObjectRetain(self.as_ptr()) }));
        into.int(self.as_ptr() as u64)
    }
}

/// A type argument of a generic function.
impl SwiftArg for SwiftType {
    fn lower(&self, into: &mut ArgLowering) {
        into.int(self.as_ptr() as u64)
    }
}

/// A conformance argument of a constrained generic function.
impl SwiftArg for WitnessTable {
    fn lower(&self, into: &mut ArgLowering) {
        into.int(self.0 as u64)
    }
}

/// Where a typed-throws function with a generic error type (`throws(E)`) writes its error: pass it after the generic
/// arguments, with `E`'s type. A thrown error comes back as `CallError::Typed`.
#[derive(Clone, Copy, Debug)]
pub struct ErrorSlot(SwiftType);

impl ErrorSlot {
    pub fn of(error_type: SwiftType) -> ErrorSlot {
        ErrorSlot(error_type)
    }
}

impl SwiftArg for ErrorSlot {
    fn lower(&self, into: &mut ArgLowering) {
        into.error_slot(self.0);
    }
}

/// A value passed by address: an argument of generic type, or a large or resilient struct.
impl SwiftArg for &SwiftValue {
    fn lower(&self, into: &mut ArgLowering) {
        let copy = (*self).clone(); // the call may outlive the borrow
        into.int(copy.as_ptr() as u64);
        into.owned.push(Owned::Value(copy));
    }
}

/// A concrete `[T]` parameter: an array is one reference, passed directly.
impl<T: IntoSwift + Clone> SwiftArg for Vec<T> {
    fn lower(&self, into: &mut ArgLowering) {
        let array = SwiftValue::new(self.clone());
        into.int(unsafe { *(array.as_ptr() as *const u64) });
        into.owned.push(Owned::Value(array));
    }
}

/// A tuple of arguments, in declaration order.
pub trait SwiftArgs {
    #[doc(hidden)]
    fn lower(&self, into: &mut ArgLowering);
}

macro_rules! tuple_args {
    ($($($name:ident)*;)*) => {$(
        impl<$($name: SwiftArg),*> SwiftArgs for ($($name,)*) {
            #[allow(non_snake_case, unused_variables)]
            fn lower(&self, into: &mut ArgLowering) {
                let ($($name,)*) = self;
                $($name.lower(into);)*
            }
        }
    )*};
}
tuple_args! {
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

/// A Rust type a Swift result can be read as.
pub trait SwiftRet: Sized {
    /// The Swift type is returned indirectly (through an address in the first integer argument).
    #[doc(hidden)]
    const INDIRECT: bool = false;

    /// The Swift type is returned in floating-point registers.
    #[doc(hidden)]
    const FLOAT: bool = false;

    /// Takes the owned result out of the result registers (or the indirect slot).
    #[doc(hidden)]
    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self;

    /// Where the result goes and whether the callee writes it through an address. `result_type` is the
    /// mangled name of the Swift result type when the function was declared with one.
    #[doc(hidden)]
    fn result_slot(_result_type: Option<&'static str>) -> Result<(IndirectSlot, bool), CallError> {
        Ok((IndirectSlot::inline(), Self::INDIRECT))
    }
}

macro_rules! direct_ret {
    ($($t:ty: |$ret:ident| $value:expr),* $(,)?) => {$(
        impl SwiftRet for $t {
            unsafe fn lift($ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
                $value
            }
        }
    )*};
}
direct_ret! {
    (): |_ret| (),
    i64: |ret| ret.0.gpr[0] as i64,
    i32: |ret| ret.0.gpr[0] as i32,
    u64: |ret| ret.0.gpr[0],
    bool: |ret| ret.0.gpr[0] & 1 == 1,
    // `Int?`: the value, then a flag whose low bit is set for nil.
    Option<i64>: |ret| (ret.0.gpr[1] & 1 == 0).then_some(ret.0.gpr[0] as i64),
    String: |ret| take_string(ret.0.gpr[0], ret.0.gpr[1]),
    // `String?`: nil is the all-zero owner word.
    Option<String>: |ret| (ret.0.gpr[1] != 0).then(|| take_string(ret.0.gpr[0], ret.0.gpr[1])),
    SwiftObject: |ret| SwiftObject::from_raw(ret.0.gpr[0] as *mut c_void),
    Option<SwiftObject>: |ret| NonNull::new(ret.0.gpr[0] as *mut c_void).map(SwiftObject),
}

impl SwiftRet for f64 {
    const FLOAT: bool = true;

    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        ret.0.fpr[0]
    }
}

unsafe fn take_string(w0: u64, w1: u64) -> String {
    let mut string = rt::SwiftString { words: [w0, w1] };
    let out = rt::string_to_rust(&string);
    rt::destroy_string(&mut string);
    out
}

/// Reads a collection returned as one owned reference, then releases it.
unsafe fn take_collection<C: FromSwiftAny>(reference: u64) -> C {
    let out = C::from_value((&reference as *const u64).cast());
    rt::swift_bridgeObjectRelease(reference as *mut c_void);
    out
}

impl<T: FromSwiftAny> SwiftRet for Vec<T> {
    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        take_collection(ret.0.gpr[0])
    }
}

impl<K: SwiftHashable, V: FromSwiftAny> SwiftRet for HashMap<K, V> {
    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        take_collection(ret.0.gpr[0])
    }
}

impl SwiftRet for SwiftAny {
    const INDIRECT: bool = true;

    unsafe fn lift(_ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        SwiftAny::from_raw_initialized(indirect.inline)
    }
}

fn declared_type(result_type: Option<&'static str>) -> Result<SwiftType, CallError> {
    let name = result_type.ok_or(CallError::InvalidArgument(
        "an opaque result needs its Swift type: declare the function with #[swift = \"…\"], or use call_returning / call_direct",
    ))?;
    SwiftType::from_mangled_name(name).ok_or(CallError::Unavailable(name))
}

/// A value returned by address: a large or resilient struct (every struct of an Apple framework), or a value
/// of generic type. Its Swift type comes from the function's declaration, `call_returning` or `call_direct`.
impl SwiftRet for SwiftValue {
    const INDIRECT: bool = true;

    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        let storage = indirect.boxed.take().expect("a SwiftValue result always has storage");
        if indirect.direct {
            std::ptr::copy_nonoverlapping(ret.0.gpr.as_ptr().cast::<u8>(), storage.ptr.as_ptr(), storage.ty.size());
        }
        storage.assume_init()
    }

    fn result_slot(result_type: Option<&'static str>) -> Result<(IndirectSlot, bool), CallError> {
        Ok((IndirectSlot::boxed(UninitValue::new(declared_type(result_type)?), false), true))
    }
}

/// An opaque value that Swift returns in registers rather than by address: at most four words of integers and
/// references, such as an array or dictionary whose elements Rust does not read (`[Product]`).
pub struct DirectValue(pub SwiftValue);

impl SwiftRet for DirectValue {
    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        DirectValue(SwiftValue::lift(ret, indirect))
    }

    fn result_slot(result_type: Option<&'static str>) -> Result<(IndirectSlot, bool), CallError> {
        let ty = declared_type(result_type)?;
        if ty.size() > 32 {
            return Err(CallError::InvalidArgument("a type larger than four words is returned by address: use SwiftValue"));
        }
        Ok((IndirectSlot::boxed(UninitValue::new(ty), true), false))
    }
}

/// What a method can be called on: a class instance, or a value passed by address.
pub trait SwiftReceiver {
    #[doc(hidden)]
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R>;
    #[doc(hidden)]
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R>;
}

impl SwiftReceiver for super::value::Opened {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on_opened(self)
    }
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on_opened(self)
    }
}

impl SwiftReceiver for SwiftObject {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on(self)
    }
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on(self)
    }
}

/// `self` by address: a large or resilient struct. (A small struct of the caller's own module is passed in
/// registers instead; that is not supported as a receiver.)
impl SwiftReceiver for SwiftValue {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on_value(self)
    }
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on_value(self)
    }
}

/// An owned reference to a class instance: a native Swift object or an Objective-C-backed one (any `AnyObject`).
pub struct SwiftObject(NonNull<c_void>);

// Swift reference counting is atomic; the object itself is only touched by Swift code.
unsafe impl Send for SwiftObject {}

impl SwiftObject {
    /// Takes ownership of a reference (+1).
    ///
    /// # Safety
    /// `object` must be an owned, non-null reference to a Swift object.
    pub unsafe fn from_raw(object: *mut c_void) -> SwiftObject {
        SwiftObject(NonNull::new(object).expect("null Swift object"))
    }

    /// Gives up ownership of the reference (+1).
    pub(crate) fn into_raw(self) -> *mut c_void {
        std::mem::ManuallyDrop::new(self).0.as_ptr()
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Clone for SwiftObject {
    fn clone(&self) -> Self {
        unsafe { SwiftObject::from_raw(rt::swift_unknownObjectRetain(self.as_ptr())) }
    }
}

impl Drop for SwiftObject {
    fn drop(&mut self) {
        unsafe { rt::swift_unknownObjectRelease(self.as_ptr()) }
    }
}

impl std::fmt::Debug for SwiftObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftObject({:p})", self.as_ptr())
    }
}
