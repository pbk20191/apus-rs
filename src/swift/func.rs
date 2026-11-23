//! Calling Swift async functions directly: finding them, lowering Rust arguments into Swift's argument
//! registers, and lifting results back out of the result registers.
//!
//! Lowering rules (Swift's calling convention, checked by tests/swift_fn.rs on arm64 and x86_64): integers,
//! `Bool` and references take the next integer register; `Double` the next floating-point register; `String`
//! two integer registers, passed guaranteed (+0). Direct results come back in the first result registers,
//! owned (+1): `String` in two, `Array`/`Dictionary`/objects as one reference, `Int?` as value + nil flag.

use super::any::{FromSwiftAny, SwiftAny, SwiftError, SwiftHashable};
use super::call::{AsyncFunctionPointer, CallError, CallRegs, IndirectSlot, Owned, RetRegs, SwiftCall};
use super::rt;
use super::value::{IntoSwift, SwiftType, SwiftValue, UninitValue, WitnessTable};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::OnceLock;

/// Integer argument registers of the platform's Swift calling convention.
pub(crate) const INT_REGISTERS: usize = if cfg!(target_arch = "aarch64") { 8 } else { 6 };
pub(crate) const FLOAT_REGISTERS: usize = 8;

/// A Swift async function that can be called from a Swift-backend task.
///
/// ```ignore
/// // The mangled name, as `nm` prints it (without the leading underscore). Looked up on first use.
/// static SLEEP: SwiftAsyncFn = SwiftAsyncFn::new("$sScTss5NeverORszABRs_rlE5sleep11nanosecondsys6UInt64V_tYaKFZ");
/// SLEEP.call::<()>((50_000_000u64,)).throws().await?;
/// ```
pub struct SwiftAsyncFn {
    symbol: &'static str,
    method: ClassMethod,
    fixed: *const AsyncFunctionPointer,
    looked_up: OnceLock<Option<Target>>,
}

/// How a class's own member dispatches to an override, as Swift calls it (`apus_mangle::ClassMethod`): its dispatch
/// thunk (`…Tj`) where the module has one (library evolution), else the class's vtable entry for `self`, found by
/// `swift_lookUpClassMethod` from the member's method descriptor (`…Tq`) and the class's descriptor (`…Mn`). A final
/// member has neither and is called by its own symbol.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ClassMethod {
    thunk: &'static str,
    method: &'static str,
    class: &'static str,
    class_side: bool,
}

impl ClassMethod {
    pub const NONE: ClassMethod = ClassMethod { thunk: "", method: "", class: "", class_side: false };

    pub const fn __new((thunk, method, class, class_side): (&'static str, &'static str, &'static str, bool)) -> ClassMethod {
        ClassMethod { thunk, method, class, class_side }
    }

    /// Where the call goes: the thunk (`…Tj`, with `suffix`: `Tu` for an async function pointer), the vtable, or
    /// `own` (the member's own symbol's address, 0 if missing). Looked up once: a Swift image is not unloaded.
    #[inline]
    pub(crate) fn resolve(&self, suffix: &str, own: impl FnOnce() -> usize) -> Option<Target> {
        if !self.thunk.is_empty() {
            let thunk = lookup(&format!("{}{suffix}", self.thunk));
            if thunk != 0 {
                return Some(Target::Entry(thunk));
            }
            let (method, class) = (lookup(self.method), lookup(self.class));
            if method != 0 && class != 0 {
                return Some(Target::VTable { method, class, class_side: self.class_side });
            }
        }
        let own = own();
        (own != 0).then_some(Target::Entry(own))
    }
}

/// A call's entry point, or the class's vtable entry for its `self`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    Entry(usize),
    VTable { method: usize, class: usize, class_side: bool },
}

impl Target {
    /// The entry point for a call whose `self` is `this` (an object, or a class's metadata for a static member).
    #[inline]
    pub(crate) fn entry(self, this: *mut c_void) -> Result<*const c_void, CallError> {
        let Target::VTable { method, class, class_side } = self else {
            let Target::Entry(entry) = self else { unreachable!() };
            return Ok(entry as *const c_void);
        };
        if this.is_null() {
            return Err(CallError::InvalidArgument("a class's member is called on an instance, or a static one on the class"));
        }
        unsafe {
            let metadata = if class_side { this as *const c_void } else { super::rt::swift_getObjectType(this) };
            // swift_lookUpClassMethod reads the vtable of `class`'s layout: metadata must be its class or a subclass.
            if !descends(metadata, class as *const c_void) {
                return Err(CallError::InvalidArgument("the receiver is not of the member's class, nor of a subclass"));
            }
            Ok(super::rt::swift_lookUpClassMethod(metadata, method as *const c_void, class as *const c_void))
        }
    }
}

/// Whether `metadata` is the class `descriptor` describes, or a subclass of it.
unsafe fn descends(mut metadata: *const c_void, descriptor: *const c_void) -> bool {
    // include/swift/ABI/MetadataKind.def: a class's metadata starts with its isa pointer, above the last enumerated
    // kind (0x7FF); any other kind (a struct, an Objective-C class wrapper, …) is no Swift class.
    const LAST_ENUMERATED: usize = 0x7FF;
    while !metadata.is_null() && *(metadata as *const usize) > LAST_ENUMERATED {
        if super::rt::swift_getTypeContextDescriptor(metadata) == descriptor {
            return true;
        }
        // include/swift/ABI/Metadata.h TargetAnyClassMetadata: the superclass follows the isa pointer.
        metadata = *(metadata as *const *const c_void).add(1);
    }
    false
}

unsafe impl Send for SwiftAsyncFn {}
unsafe impl Sync for SwiftAsyncFn {}

use super::value::lookup;

impl SwiftAsyncFn {
    /// A function identified by its mangled symbol name (`$s…`), resolved with `dlsym` on first use.
    /// If the symbol is missing, calls yield `CallError::Unavailable` instead of failing to launch.
    pub const fn new(symbol: &'static str) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol, method: ClassMethod::NONE, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// A class's own member: dispatched as [`ClassMethod`] says, else its own async function pointer.
    #[doc(hidden)]
    pub const fn __dispatched(symbol: &'static str, method: ClassMethod) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol, method, fixed: std::ptr::null(), looked_up: OnceLock::new() }
    }

    /// A function identified by its async function pointer (the symbol with the `Tu` suffix).
    ///
    /// # Safety
    /// `pointer` must be a Swift async function pointer.
    pub const unsafe fn from_pointer(pointer: &'static AsyncFunctionPointer) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol: "<pointer>", method: ClassMethod::NONE, fixed: pointer, looked_up: OnceLock::new() }
    }

    /// [`from_pointer`](Self::from_pointer) for a class's own member, linked by its own symbol: dispatched as
    /// [`ClassMethod`] says when the process has the thunk or method descriptor.
    ///
    /// # Safety
    /// `pointer` must be the member's async function pointer.
    #[doc(hidden)]
    pub const unsafe fn __linked(pointer: &'static AsyncFunctionPointer, method: ClassMethod) -> SwiftAsyncFn {
        SwiftAsyncFn { symbol: "<pointer>", method, fixed: pointer, looked_up: OnceLock::new() }
    }

    #[inline]
    fn resolve(&self) -> Option<Target> {
        *self.looked_up.get_or_init(|| {
            self.method.resolve("Tu", || match self.fixed.is_null() {
                true => lookup(&format!("{}Tu", self.symbol)),
                false => self.fixed as usize,
            })
        })
    }

    /// Whether the function exists in this process.
    pub fn is_available(&self) -> bool {
        self.resolve().is_some()
    }

    /// Prepares a call with `args` (a tuple, in declaration order) returning `R`. Nothing runs until awaited.
    /// The argument and result types must match the Swift signature: the compiler cannot check them.
    pub fn call<R: SwiftRet>(&self, args: impl SwiftArgs) -> SwiftCall<R> {
        self.call_declared(args, DeclaredResult::UNKNOWN)
    }

    /// [`call`](Self::call) for a function declared with its Swift result type (what `swift!` generates).
    #[doc(hidden)]
    pub fn call_declared<R: SwiftRet>(&self, args: impl SwiftArgs, result_type: DeclaredResult) -> SwiftCall<R> {
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
        if lowering.borrows {
            lowering.fail(CallError::InvalidArgument("an `inout` argument (`&mut`) needs a synchronous call"));
        }
        match self.resolve() {
            Some(Target::Entry(function)) => SwiftCall::build(function as *const AsyncFunctionPointer, lowering, None, slot, indirect),
            // The vtable entry for the `self` bound next, found when the call starts.
            Some(target) => SwiftCall::build(std::ptr::null(), lowering, None, slot, indirect).dispatched(target),
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
    /// An argument or `self` is a Rust value borrowed for the call (`inout`): only a synchronous call may run it.
    pub(crate) borrows: bool,
    /// The generic arguments passed (metadata, witness tables), in order: what an opaque result type depends on.
    pub(crate) generic_args: Vec<*const c_void>,
    /// The call is made before the arguments' borrows end: a borrowed value need not be copied.
    pub(crate) now: bool,
    /// An argument the callee consumes is being lowered (`Consumed`): it takes what is lowered, so a copy.
    pub(crate) consuming: bool,
}

impl ArgLowering {
    /// `indirect_result`: the first integer register is reserved for the result address.
    #[inline]
    pub(crate) fn new(indirect_result: bool) -> ArgLowering {
        ArgLowering {
            regs: CallRegs { gpr: [0; 8], fpr: [0.0; 8], self_: std::ptr::null_mut(), isolation: -1 },
            ints: indirect_result as usize,
            floats: 0,
            owned: Vec::new(),
            error: None,
            typed_error: None,
            failure: None,
            borrows: false,
            generic_args: Vec::new(),
            now: false,
            consuming: false,
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
        into.generic_args.push(self.as_ptr());
        into.int(self.as_ptr() as u64)
    }
}

/// A conformance argument of a constrained generic function.
impl SwiftArg for WitnessTable {
    fn lower(&self, into: &mut ArgLowering) {
        into.generic_args.push(self.0);
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
        // Borrowed by a call made right away: its address (the callee may neither change nor keep it).
        if into.now && !into.consuming {
            return into.int(self.as_ptr() as u64);
        }
        let copy = (*self).clone(); // the call may outlive the borrow, or the callee consumes it
        into.int(copy.as_ptr() as u64);
        into.owned.push(Owned::Value(copy));
    }
}

/// An `any P` argument (optional or not). A class existential, whose protocols are class-bound, is passed in
/// registers as its reference and witness tables; any other existential by address, as a `&SwiftValue` is. `swift!`
/// wraps an argument declared `any P`; a generic parameter's value goes by address whatever its type, so pass that
/// one unwrapped.
pub struct Existential<'a>(pub &'a SwiftValue);

impl SwiftArg for Existential<'_> {
    fn lower(&self, into: &mut ArgLowering) {
        let Some(words) = self.0.ty().class_existential_words() else {
            return self.0.lower(into);
        };
        if words > 4 {
            // swiftcall expands an aggregate of at most four words into registers, and passes a larger one by address.
            return into.fail(CallError::InvalidArgument("a class existential of more than three protocols is not supported"));
        }
        let copy = self.0.clone(); // the call may outlive the borrow
        let container = copy.as_ptr() as *const u64;
        for word in 0..words {
            into.int(unsafe { *container.add(word) });
        }
        into.owned.push(Owned::Value(copy));
    }
}

/// An `any Error` argument, optional or not: one error box reference, passed guaranteed; the call keeps it alive.
/// `swift!` wraps an argument declared `any Error` (`&SwiftError`) or `(any Error)?` (`Option<&SwiftError>`).
#[doc(hidden)]
pub struct ErrorArg<A>(pub A);

#[doc(hidden)]
pub trait ErrorRef {
    fn boxed(&self) -> Option<&SwiftError>;
}

impl ErrorRef for &SwiftError {
    fn boxed(&self) -> Option<&SwiftError> {
        Some(self)
    }
}

impl ErrorRef for Option<&SwiftError> {
    fn boxed(&self) -> Option<&SwiftError> {
        *self
    }
}

impl<A: ErrorRef> SwiftArg for ErrorArg<A> {
    fn lower(&self, into: &mut ArgLowering) {
        match self.0.boxed() {
            Some(error) => {
                into.owned.push(Owned::Error(unsafe { rt::swift_errorRetain(error.as_ptr()) }));
                into.int(error.as_ptr() as u64)
            }
            None => into.int(0), // nil: a null box
        }
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

/// An `inout` argument: the callee reads and writes the value in place. Synchronous calls only (an async call
/// refuses it with `CallError::InvalidArgument`): the borrow lasts as long as the call.
impl SwiftArg for &mut SwiftValue {
    fn lower(&self, into: &mut ArgLowering) {
        into.borrows = true;
        into.int(self.as_ptr() as u64);
    }
}

/// The receiver of a `swift!` function: `&T` for any [`SwiftReceiver`], or `&mut SwiftValue` (sync calls only).
#[doc(hidden)]
pub trait BindReceiver {
    fn bind_sync<R>(self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R>;
}

/// The receiver of an async `swift!` function: `&T` for any [`SwiftReceiver`].
#[doc(hidden)]
pub trait BindAsyncReceiver {
    fn bind<R>(self, call: SwiftCall<R>) -> SwiftCall<R>;
}

impl<T: SwiftReceiver + ?Sized> BindReceiver for &T {
    fn bind_sync<R>(self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        self.bind_sync(call)
    }
}

impl<T: SwiftReceiver + ?Sized> BindAsyncReceiver for &T {
    fn bind<R>(self, call: SwiftCall<R>) -> SwiftCall<R> {
        SwiftReceiver::bind(self, call)
    }
}

/// `inout self`: a struct's setter or `mutating` method, working on the value in place.
impl BindReceiver for &mut SwiftValue {
    fn bind_sync<R>(self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on_inout_value(self)
    }
}

/// The class a `swift!` initializer or static member belongs to when no `self` is declared: its metadata, looked up by
/// mangled name once (`swift_getTypeByMangledNameInContext`), is `self` (swiftself), as `self: &SwiftType` passes it.
#[doc(hidden)]
pub fn __declared_class(class: &'static OnceLock<Option<SwiftType>>, name: &'static str) -> Result<SwiftType, CallError> {
    class.get_or_init(|| SwiftType::from_mangled_name(name)).ok_or(CallError::Unavailable(name))
}

/// How [`__check_receiver`] compares a receiver's runtime type with the declaration's.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum ReceiverKind {
    /// A class's object: an instance of the class, or of a subclass (`swift_dynamicCastUnknownClass`).
    Instance,
    /// A struct's or enum's value: of exactly the type (concrete metadata is unique).
    Exactly,
    /// A generic struct's or enum's value: its qualified type name, without generic arguments (`swift_getTypeName`).
    Named,
}

/// A receiver whose Swift type Rust does not know (`SwiftObject`, `SwiftValue`): checked when the call is made.
#[doc(hidden)]
pub trait ErasedReceiver {
    /// The object, if the receiver is one.
    fn erased_object(&self) -> Option<*mut c_void>;
    /// The value's type, if the receiver is a value.
    fn erased_type(&self) -> Option<SwiftType>;
}

impl ErasedReceiver for SwiftObject {
    fn erased_object(&self) -> Option<*mut c_void> {
        Some(self.as_ptr())
    }
    fn erased_type(&self) -> Option<SwiftType> {
        None
    }
}

impl ErasedReceiver for SwiftValue {
    fn erased_object(&self) -> Option<*mut c_void> {
        None
    }
    fn erased_type(&self) -> Option<SwiftType> {
        Some(self.ty())
    }
}

/// Whether `receiver` has the declaration's type `name` (mangled, or for `Named` the qualified name); the type's
/// metadata is looked up once (for `Named`, the first type found to have that name is kept).
#[doc(hidden)]
pub fn __check_receiver<R: ErasedReceiver + ?Sized>(
    receiver: &R,
    kind: ReceiverKind,
    expected: &'static OnceLock<Option<SwiftType>>,
    name: &'static str,
) -> Result<(), CallError> {
    let mismatch = Err(CallError::InvalidArgument("the receiver is not of the declaration's type"));
    let declared = || expected.get_or_init(|| SwiftType::from_mangled_name(name)).ok_or(CallError::Unavailable(name));
    match kind {
        ReceiverKind::Instance => {
            let Some(object) = receiver.erased_object() else { return mismatch };
            let class = declared()?;
            if unsafe { super::rt::swift_dynamicCastUnknownClass(object, class.as_ptr()) }.is_null() {
                return mismatch;
            }
        }
        ReceiverKind::Exactly => {
            if receiver.erased_type() != Some(declared()?) {
                return mismatch;
            }
        }
        ReceiverKind::Named => {
            let Some(ty) = receiver.erased_type() else { return mismatch };
            // `expected` keeps the first type that passed (concrete metadata is unique): the same one again is not
            // compared by name, whose reading allocates.
            if expected.get() == Some(&Some(ty)) {
                return Ok(());
            }
            if without_generic_arguments(&ty.name()) != name {
                return mismatch;
            }
            let _ = expected.set(Some(ty));
        }
    }
    Ok(())
}

/// `Res.Tray<Swift.Array<Swift.Int>>.Inner` → `Res.Tray.Inner`.
fn without_generic_arguments(name: &str) -> String {
    let mut depth = 0usize;
    name.chars()
        .filter(|&c| {
            match c {
                '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                _ => return depth == 0,
            }
            false
        })
        .collect()
}

/// Records a failed check on a prepared call, which then fails with it instead of running.
#[doc(hidden)]
pub trait CheckedCall: Sized {
    fn checked(self, check: Result<(), CallError>) -> Self;
}

/// Binds [`__declared_class`]'s result as `self`; a class not found is reported when the call is made.
#[doc(hidden)]
pub trait ClassSelf: Sized {
    fn class_self(self, class: Result<SwiftType, CallError>) -> Self;
}

/// Binds `self`; for a `consuming` method (`consumes`), what the binding keeps alive goes to the callee instead:
/// the receiver is a copy (a value) or an extra reference (an object) that the callee consumes.
#[doc(hidden)]
pub fn __bind_sync<B: BindReceiver, R>(this: B, mut call: super::SwiftSyncCall<R>, consumes: bool) -> super::SwiftSyncCall<R> {
    let start = call.lowering.owned.len();
    call = this.bind_sync(call);
    if consumes {
        hand_over(&mut call.lowering.owned, start);
    }
    call
}

#[doc(hidden)]
pub fn __bind<B: BindAsyncReceiver, R>(this: B, mut call: SwiftCall<R>, consumes: bool) -> SwiftCall<R> {
    if !consumes {
        return this.bind(call);
    }
    let start = call.owned_count();
    let mut call = this.bind(call);
    call.hand_over_owned(start);
    call
}

/// Marks the entries from `start` on for the callee to consume, handed over when the call starts.
pub(crate) fn hand_over(owned: &mut Vec<Owned>, start: usize) {
    let lowered: Vec<Owned> = owned.drain(start..).collect();
    owned.extend(lowered.into_iter().map(Owned::pending));
}

/// What `swift!` knows of a declaration's result: its type's mangled name, or that it is a `some P`.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct DeclaredResult {
    pub name: Option<&'static str>,
    pub opaque: Option<&'static super::value::OpaqueResult>,
    /// The result is the protocol's `Self`: of the type the call is made on (an `Opened`'s or `OpenedType`'s).
    pub receiver_self: bool,
    /// The result's type, read at run time: a generic struct's parameter, from the receiver's metadata.
    pub ty: Option<SwiftType>,
    /// The call site's cache of `name`'s metadata, looked up once (a Swift image is not unloaded).
    pub resolved: Option<&'static OnceLock<Option<SwiftType>>>,
}

impl DeclaredResult {
    pub const UNKNOWN: DeclaredResult = DeclaredResult { name: None, opaque: None, receiver_self: false, ty: None, resolved: None };

    pub const fn named(name: Option<&'static str>) -> DeclaredResult {
        DeclaredResult { name, opaque: None, receiver_self: false, ty: None, resolved: None }
    }

    /// The same, with the result's type known at run time.
    pub fn of_type(self, ty: SwiftType) -> DeclaredResult {
        DeclaredResult { ty: Some(ty), ..self }
    }
}

/// An argument the callee consumes (+1): an initializer's arguments, a setter's new value. `swift!` wraps them.
#[doc(hidden)]
pub struct Consumed<A>(pub A);

impl<A: SwiftArg> SwiftArg for Consumed<A> {
    fn lower(&self, into: &mut ArgLowering) {
        let start = into.owned.len();
        let consuming = std::mem::replace(&mut into.consuming, true);
        self.0.lower(into);
        into.consuming = consuming;
        hand_over(&mut into.owned, start);
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

    /// [`FLOAT`](Self::FLOAT), for a type that knows it only at run time (a C value's layout).
    #[doc(hidden)]
    fn float() -> bool {
        Self::FLOAT
    }

    /// Takes the owned result out of the result registers (or the indirect slot).
    #[doc(hidden)]
    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self;

    /// Where the result goes and whether the callee writes it through an address. `result_type` is the
    /// mangled name of the Swift result type when the function was declared with one.
    #[doc(hidden)]
    fn result_slot(_result_type: DeclaredResult) -> Result<(IndirectSlot, bool), CallError> {
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
    // A thick metatype (`T.Type`, a class's, `Any.Type`): the type's metadata, which is not reference counted.
    SwiftType: |ret| SwiftType::from_metadata(ret.0.gpr[0] as *const c_void),
    Option<SwiftObject>: |ret| NonNull::new(ret.0.gpr[0] as *mut c_void).map(SwiftObject),
    // `any Error`: one owned box reference; `(any Error)?`: nil is null.
    SwiftError: |ret| SwiftError::from_raw(ret.0.gpr[0] as *mut c_void),
    Option<SwiftError>: |ret| (ret.0.gpr[0] != 0).then(|| SwiftError::from_raw(ret.0.gpr[0] as *mut c_void)),
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

fn declared_type(result_type: DeclaredResult) -> Result<SwiftType, CallError> {
    if let Some(ty) = result_type.ty {
        return Ok(ty);
    }
    let name = result_type.name.ok_or(CallError::InvalidArgument(
        "an opaque result needs its Swift type: declare the function with #[swift = \"…\"], or use call_returning / call_direct",
    ))?;
    let ty = match result_type.resolved {
        Some(resolved) => *resolved.get_or_init(|| SwiftType::from_mangled_name(name)),
        None => SwiftType::from_mangled_name(name),
    };
    ty.ok_or(CallError::Unavailable(name))
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

    fn result_slot(result_type: DeclaredResult) -> Result<(IndirectSlot, bool), CallError> {
        if let Some(opaque) = result_type.opaque.filter(|o| o.is_opaque()) {
            // `some P`: always by address; the storage is sized once the generic arguments are known.
            return Ok((IndirectSlot::opaque(opaque), true));
        }
        if result_type.receiver_self {
            // `Self`: by address, of the receiver's type, which binding the receiver adds.
            return Ok((IndirectSlot::receiver_self(), true));
        }
        let ty = declared_type(result_type)?;
        if result_type.ty.is_none() && ty.class_existential_words().is_some() {
            // A declared `any P` whose protocols are class-bound: in registers (a generic parameter's value, whose type
            // comes in `ty`, is returned by address whatever its type).
            return DirectValue::result_slot(result_type);
        }
        Ok((IndirectSlot::boxed(UninitValue::new(ty), false), true))
    }
}

/// An opaque value that Swift returns in registers rather than by address: at most four words of integers and
/// references, such as an array or dictionary whose elements Rust does not read (`[Product]`).
pub struct DirectValue(pub SwiftValue);

impl SwiftRet for DirectValue {
    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        DirectValue(SwiftValue::lift(ret, indirect))
    }

    fn result_slot(result_type: DeclaredResult) -> Result<(IndirectSlot, bool), CallError> {
        if result_type.opaque.is_some_and(|o| o.is_opaque()) || result_type.receiver_self {
            return Err(CallError::InvalidArgument("a `some P` or `Self` result is returned by address: use SwiftValue"));
        }
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

impl SwiftReceiver for super::value::OpenedType {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on_opened_type(self)
    }
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on_opened_type(self)
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

/// A class as `self`: its initializers and static members (a struct's or enum's take no `self`).
impl SwiftReceiver for SwiftType {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on_type(*self)
    }
    fn bind_sync<R>(&self, call: super::SwiftSyncCall<R>) -> super::SwiftSyncCall<R> {
        call.on_type(*self)
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
