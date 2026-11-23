//! A Swift `String` kept as Swift's own two-word value, made once from Rust text: passed to Swift with no
//! conversion (a `&str` argument is converted for every call), and returned without one.

use super::call::{IndirectSlot, Owned, RetRegs};
use super::func::{ArgLowering, SwiftArg, SwiftRet};
use super::rt;
use std::fmt;

/// A Swift `String`. Cloning it retains its storage; dropping it releases it.
pub struct SwiftString(pub(crate) rt::SwiftString);

// A Swift `String` is a value whose storage is immutable and reference counted atomically.
unsafe impl Send for SwiftString {}
unsafe impl Sync for SwiftString {}

impl SwiftString {
    /// A Swift `String` holding a copy of `text`.
    pub fn new(text: &str) -> SwiftString {
        SwiftString(rt::string_from_utf8(text))
    }

    /// The text, copied out as UTF-8.
    pub fn to_rust(&self) -> String {
        unsafe { rt::string_to_rust(&self.0) }
    }

    /// Its two words, the copy's own +1 kept by `into` (released when it is done, or handed over if consumed).
    fn lower_retained(&self, into: &mut ArgLowering) {
        let copy = std::mem::ManuallyDrop::new(self.clone());
        into.owned.push(Owned::Bridge(copy.0.words[1]));
        into.int(copy.0.words[0]);
        into.int(copy.0.words[1]);
    }
}

impl From<&str> for SwiftString {
    fn from(text: &str) -> SwiftString {
        SwiftString::new(text)
    }
}

impl Clone for SwiftString {
    fn clone(&self) -> SwiftString {
        // The String type's value witness (include/swift/ABI/ValueWitness.def: initializeWithCopy).
        let mut copy = rt::SwiftString { words: [0; 2] };
        unsafe { rt::copy_string(&mut copy, &self.0) };
        SwiftString(copy)
    }
}

impl Drop for SwiftString {
    fn drop(&mut self) {
        unsafe { rt::destroy_string(&mut self.0) }
    }
}

impl fmt::Debug for SwiftString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_rust(), f)
    }
}

/// Borrowed by a call made right away: its two words as they are. A call that consumes it, or may run after the
/// borrow ends, gets a copy (+1).
impl SwiftArg for &SwiftString {
    fn lower(&self, into: &mut ArgLowering) {
        if into.now && !into.consuming {
            into.int(self.0.words[0]);
            into.int(self.0.words[1]);
        } else {
            self.lower_retained(into);
        }
    }
}

/// A `String` result, owned, kept as Swift's value.
impl SwiftRet for SwiftString {
    unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
        SwiftString(rt::SwiftString { words: [ret.0.gpr[0], ret.0.gpr[1]] })
    }
}
