//! Typed wrappers (`swift! { pub class Todo = "Todos.Todo(class)"; }`): a `SwiftObject` or `SwiftValue` known to hold
//! one Swift type, so that `swift!` checks receivers, arguments and results against declarations at compile time.

use super::any::FromSwiftAny;
use super::call::{CallError, IndirectSlot, RetRegs, SwiftCall};
use super::func::{ArgLowering, DeclaredResult, SwiftArg, SwiftObject, SwiftReceiver, SwiftRet};
use super::rt;
use super::sync::SwiftSyncCall;
use super::value::{SwiftType, SwiftValue};
use std::ffi::c_void;

/// A Rust type standing for one Swift type: what `swift! { class … = "…"; }` (or `struct`, `enum`) declares.
///
/// # Safety
/// `erased` must hold a value of the type `MANGLED` names (or for a class, a subclass's object).
pub unsafe trait SwiftWrapper: Sized {
    /// `SwiftObject` for a class, `SwiftValue` for a struct or enum.
    type Erased: Erased;
    /// The type's mangled name (`5Todos4TodoC`).
    const MANGLED: &'static str;
    /// The type's qualified name without generic arguments (`Todos.Todo`).
    const NAME: &'static str;
    /// A class's wrapper (holding a `SwiftObject`).
    const CLASS: bool;
    /// The type's metadata, looked up by name once.
    fn swift_type() -> Option<SwiftType>;
    fn erased(&self) -> &Self::Erased;
    fn into_erased(self) -> Self::Erased;
    /// # Safety
    /// `erased` must hold a value of the type (see the trait's safety section).
    unsafe fn from_erased_unchecked(erased: Self::Erased) -> Self;

    /// `erased` as this type, if it holds one: a class's object (or a subclass's), a struct's or enum's value of exactly
    /// the type. `InvalidArgument` if not, `Unavailable` if the type is not in the process.
    fn try_wrap(erased: Self::Erased) -> Result<Self, CallError> {
        let ty = Self::swift_type().ok_or(CallError::Unavailable(Self::MANGLED))?;
        if !erased.holds(ty) {
            return Err(CallError::InvalidArgument("the value is not of the wrapper's Swift type"));
        }
        Ok(unsafe { Self::from_erased_unchecked(erased) })
    }
}

/// What a wrapper holds: `SwiftObject` or `SwiftValue`.
#[doc(hidden)]
pub trait Erased: SwiftReceiver + SwiftRet + Clone + super::func::ErasedReceiver {
    /// Whether this holds a value of `ty` (an object: an instance of `ty` or of a subclass).
    fn holds(&self, ty: SwiftType) -> bool;
    fn lower_erased(&self, into: &mut ArgLowering);
    /// A copy of the initialized value of type `ty` at `value`.
    unsafe fn copy_from(value: *const c_void, ty: SwiftType) -> Self;
    /// What a borrowed receiver passes as `self`: the object's reference, or the value's address.
    fn receiver(&self) -> *mut c_void;
}

impl Erased for SwiftObject {
    fn holds(&self, ty: SwiftType) -> bool {
        !unsafe { rt::swift_dynamicCastUnknownClass(self.as_ptr(), ty.as_ptr()) }.is_null()
    }
    fn lower_erased(&self, into: &mut ArgLowering) {
        self.lower(into)
    }
    unsafe fn copy_from(value: *const c_void, _ty: SwiftType) -> Self {
        // A class-typed value is one reference (TypeLayout.rst): a copy is one more retain.
        SwiftObject::from_raw(rt::swift_unknownObjectRetain(*(value as *const *mut c_void)))
    }
    fn receiver(&self) -> *mut c_void {
        self.as_ptr()
    }
}

impl Erased for SwiftValue {
    fn holds(&self, ty: SwiftType) -> bool {
        self.ty() == ty
    }
    fn lower_erased(&self, into: &mut ArgLowering) {
        self.lower(into)
    }
    unsafe fn copy_from(value: *const c_void, ty: SwiftType) -> Self {
        SwiftValue::copy_of(value, ty)
    }
    fn receiver(&self) -> *mut c_void {
        self.as_ptr()
    }
}

/// A wrapper as a borrowed argument of the scalar path: what it holds, a reference or a value's address, as a
/// `&SwiftObject` or `&SwiftValue` is passed.
impl<W: SwiftWrapper> super::sync::ScalarArg for &W {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, _made: &mut super::sync::Made) -> bool {
        let Some(slot) = regs.gpr.get_mut(*ints) else { return false };
        *slot = self.erased().receiver() as u64;
        *ints += 1;
        true
    }
}

/// A wrapper as a borrowed receiver of the scalar path: what it holds, its type known (checked when wrapped, and
/// against the declaration's when compiling, or by cast for another class's wrapper).
impl<W: SwiftWrapper> super::sync::ScalarReceiver for &W {
    #[inline]
    fn receiver(self) -> *mut c_void {
        self.erased().receiver()
    }
}

impl<W: SwiftWrapper> SwiftReceiver for W {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        self.erased().bind(call)
    }
    fn bind_sync<R>(&self, call: SwiftSyncCall<R>) -> SwiftSyncCall<R> {
        self.erased().bind_sync(call)
    }
}

/// A class wrapper as a class-bound generic parameter's value: one reference.
impl<W: SwiftWrapper<Erased = SwiftObject>> super::generic::ClassReference for &W {
    fn object(&self) -> *mut c_void {
        self.erased().as_ptr()
    }
}

impl<W: SwiftWrapper> SwiftArg for &W {
    fn lower(&self, into: &mut ArgLowering) {
        self.erased().lower_erased(into)
    }
}

/// A result of the wrapper's type: the declaration says what it is, which `swift!` checked at compile time.
impl<W: SwiftWrapper> SwiftRet for W {
    const INDIRECT: bool = <W::Erased as SwiftRet>::INDIRECT;
    const FLOAT: bool = <W::Erased as SwiftRet>::FLOAT;

    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        W::from_erased_unchecked(<W::Erased as SwiftRet>::lift(ret, indirect))
    }

    fn result_slot(result_type: DeclaredResult) -> Result<(IndirectSlot, bool), CallError> {
        <W::Erased as SwiftRet>::result_slot(result_type)
    }
}

/// An optional class instance: nil is `None`.
impl<W: SwiftWrapper<Erased = SwiftObject>> SwiftRet for Option<W> {
    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        <Option<SwiftObject> as SwiftRet>::lift(ret, indirect).map(|object| W::from_erased_unchecked(object))
    }
}

/// An element of an array result (`-> Vec<Todo>`), which is read through its `Sequence` conformance.
impl<W: SwiftWrapper> FromSwiftAny for W {
    const ITERATE: bool = true;

    fn swift_metadata() -> *const c_void {
        W::swift_type().expect("a wrapper's Swift type, which a result of it shows is loaded").as_ptr()
    }
    unsafe fn from_value(value: *const c_void) -> Self {
        let ty = SwiftType::from_metadata(Self::swift_metadata());
        W::from_erased_unchecked(<W::Erased as Erased>::copy_from(value, ty))
    }
}

/// Compile-time check that a wrapper is the Swift type a declaration names: `mangled` its mangled name when the
/// declaration's type is concrete (else empty), `name` its qualified name without generic arguments, `class` whether
/// it is a class. Another type fails the build, except that a class's wrapper where another class is declared, as a
/// receiver or argument (`by_cast`), is checked when the call is made: `true` then.
#[doc(hidden)]
pub const fn __expect_wrapper<W: SwiftWrapper>(mangled: &str, name: &str, class: bool, by_cast: bool) -> bool {
    let same = if mangled.is_empty() { same_str(W::NAME, name) } else { same_str(W::MANGLED, mangled) };
    if same {
        return false;
    }
    if !(by_cast && class && W::CLASS && !mangled.is_empty()) {
        panic!("this wrapper stands for another Swift type than the declaration's");
    }
    true
}

/// The run-time check [`__expect_wrapper`] leaves: the wrapper's object is an instance of the class `mangled` names.
#[doc(hidden)]
pub fn __check_wrapper<W: SwiftWrapper>(
    wrapper: &W,
    expected: &'static std::sync::OnceLock<Option<SwiftType>>,
    accepted: &'static std::sync::atomic::AtomicPtr<std::ffi::c_void>,
    mangled: &'static str,
) -> Result<(), CallError> {
    super::func::__check_receiver(wrapper.erased(), super::func::ReceiverKind::Instance, expected, accepted, mangled)
}

const fn same_str(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
