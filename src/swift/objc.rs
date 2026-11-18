//! objc2 objects as Swift class references (feature `objc2`). On Apple platforms every Swift class instance is an
//! Objective-C object, so a `Retained<T>` and a [`SwiftObject`] are the same owned reference.
use super::call::{IndirectSlot, RetRegs, SwiftCall};
use super::func::{ArgLowering, SwiftArg, SwiftReceiver, SwiftRet};
use super::sync::SwiftSyncCall;
use super::SwiftObject;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DowncastTarget, Message};

/// Hands the reference over (+1 stays +1).
impl<T: Message> From<Retained<T>> for SwiftObject {
    fn from(object: Retained<T>) -> SwiftObject {
        unsafe { SwiftObject::from_raw(Retained::into_raw(object).cast()) }
    }
}

impl SwiftObject {
    /// The same object as an objc2 reference (+1 stays +1).
    pub fn into_objc(self) -> Retained<AnyObject> {
        let object = std::mem::ManuallyDrop::new(self).as_ptr();
        unsafe { Retained::from_raw(object.cast()) }.expect("a SwiftObject is never null")
    }

    /// The object as `T` if it is a `T` (`isKindOfClass:`), else the object back.
    pub fn downcast<T: DowncastTarget>(self) -> Result<Retained<T>, SwiftObject> {
        self.into_objc().downcast::<T>().map_err(SwiftObject::from)
    }
}

/// A Swift class argument; the call keeps the object alive until it is over.
impl<T: Message> SwiftArg for &Retained<T> {
    fn lower(&self, into: &mut ArgLowering) {
        (&SwiftObject::from(Retained::clone(self))).lower(into)
    }
}

impl<T: Message> super::ClassReference for &Retained<T> {
    fn object(&self) -> *mut std::ffi::c_void {
        Retained::as_ptr(self) as *mut std::ffi::c_void
    }
}

/// A Swift class result (+1). `T` is what the declaration says the function returns; it is not checked.
impl<T: Message> SwiftRet for Retained<T> {
    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        Retained::from_raw(ret.0.gpr[0] as *mut T).expect("a non-optional Swift class result is never nil")
    }
}

/// An optional Swift class result: nil is the null pointer.
impl<T: Message> SwiftRet for Option<Retained<T>> {
    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        Retained::from_raw(ret.0.gpr[0] as *mut T)
    }
}

/// `self` of a Swift method.
impl<T: Message> SwiftReceiver for Retained<T> {
    fn bind<R>(&self, call: SwiftCall<R>) -> SwiftCall<R> {
        call.on(&SwiftObject::from(Retained::clone(self)))
    }
    fn bind_sync<R>(&self, call: SwiftSyncCall<R>) -> SwiftSyncCall<R> {
        call.on(&SwiftObject::from(Retained::clone(self)))
    }
}
