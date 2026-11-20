//! Generic arguments `swift!` fills itself: type metadata, witness tables in the ABI's canonical requirement order,
//! and a typed-throws error slot, taken from the call's own arguments (apus_mangle::generic_plan says from which).
use super::func::{ArgLowering, ErrorSlot, SwiftArg, SwiftObject};
use super::value::{SwiftProtocol, SwiftType, SwiftValue};
use super::{rt, CallError};

/// A value whose Swift type is known at run time: what an automatic generic argument is taken from.
#[doc(hidden)]
pub trait HasSwiftType {
    fn swift_type(&self) -> SwiftType;
}

impl HasSwiftType for SwiftValue {
    fn swift_type(&self) -> SwiftType {
        self.ty()
    }
}

impl<T: HasSwiftType + ?Sized> HasSwiftType for &T {
    fn swift_type(&self) -> SwiftType {
        (**self).swift_type()
    }
}

/// The receiver of a generic struct's or enum's instance member: its type is the `Self` metadata the member takes.
#[doc(hidden)]
pub trait ValueReceiver {
    fn value_type(&self) -> SwiftType;
}

impl ValueReceiver for &SwiftValue {
    fn value_type(&self) -> SwiftType {
        self.ty()
    }
}

impl ValueReceiver for &mut SwiftValue {
    fn value_type(&self) -> SwiftType {
        self.ty()
    }
}

#[doc(hidden)]
pub fn __self_type<R: ValueReceiver>(receiver: &R) -> SwiftType {
    receiver.value_type()
}

#[doc(hidden)]
pub fn __type_of<T: HasSwiftType + ?Sized>(value: &T) -> SwiftType {
    value.swift_type()
}

/// A class reference: what a value of a class-bound generic type (`T: SomeClass`, `T: AnyObject`) is passed as.
pub trait ClassReference: SwiftArg {
    #[doc(hidden)]
    fn object(&self) -> *mut std::ffi::c_void;
}

impl ClassReference for &SwiftObject {
    fn object(&self) -> *mut std::ffi::c_void {
        self.as_ptr()
    }
}

/// An argument of a class-bound generic type: Swift passes it as one reference (guaranteed), not by address, so
/// only a [`ClassReference`] is accepted. `swift!` wraps such arguments itself.
#[doc(hidden)]
pub struct ClassBound<A>(pub A);

impl<A: ClassReference> SwiftArg for ClassBound<A> {
    fn lower(&self, into: &mut ArgLowering) {
        self.0.lower(into)
    }
}

/// The type a class-bound generic parameter takes from its argument: the object's own (dynamic) class.
#[doc(hidden)]
pub fn __object_type_of<A: ClassReference>(value: &ClassBound<A>) -> SwiftType {
    // swift_getObjectType (include/swift/Runtime/Metadata.h): a native or Objective-C object's metadata.
    unsafe { SwiftType::from_metadata(rt::swift_getObjectType(value.0.object())) }
}

/// What a metatype argument (`T.Type`, a class's `.Type`, `Any.Type`) is given as: the type's metadata.
pub trait AsMetatype {
    #[doc(hidden)]
    fn metatype(&self) -> SwiftType;
}

impl AsMetatype for SwiftType {
    fn metatype(&self) -> SwiftType {
        *self
    }
}

impl AsMetatype for &SwiftType {
    fn metatype(&self) -> SwiftType {
        **self
    }
}

/// A metatype argument: a thick metatype is the type's metadata pointer, passed directly. `swift!` wraps such
/// arguments itself; a struct's or enum's metatype (thin: nothing is passed) is refused by the declaration check.
#[doc(hidden)]
pub struct Metatype<A>(pub A);

impl<A: AsMetatype> SwiftArg for Metatype<A> {
    fn lower(&self, into: &mut ArgLowering) {
        into.int(self.0.metatype().as_ptr() as u64)
    }
}

/// The type a `T.Type` argument names.
#[doc(hidden)]
pub fn __metatype_of<A: AsMetatype>(value: &Metatype<A>) -> SwiftType {
    value.0.metatype()
}

/// The generic arguments after the formal ones: metadata, witness tables, then the error slot.
#[doc(hidden)]
pub struct GenericArgs {
    metadata: Vec<SwiftType>,
    witnesses: Vec<(SwiftType, &'static SwiftProtocol, ProtocolLayout)>,
    error_slot: Option<SwiftType>,
}

#[doc(hidden)]
pub fn __generic_args(
    metadata: &[SwiftType],
    witnesses: &[(SwiftType, &'static SwiftProtocol, ProtocolLayout)],
    error_slot: Option<SwiftType>,
) -> GenericArgs {
    GenericArgs { metadata: metadata.to_vec(), witnesses: witnesses.to_vec(), error_slot }
}

/// The generic arguments of a function whose generic parameters `swift!` binds to concrete types
/// (`#[swift = "…", T = "Type"]`): each bound type found by its mangled name once, then lowered as [`GenericArgs`].
#[doc(hidden)]
pub struct ConcreteArgs(Result<GenericArgs, Refused>);

enum Refused {
    /// A type named by the binding is not in the process.
    Missing(&'static str),
    /// An argument does not hold the type its declared type is with the bindings substituted.
    Mismatch,
}

/// A type found by its mangled name once, kept where `swift!` put it.
type Named = (&'static str, std::sync::OnceLock<Option<SwiftType>>);

fn find((name, found): &'static Named) -> Result<SwiftType, Refused> {
    found.get_or_init(|| SwiftType::from_mangled_name(name)).ok_or(Refused::Missing(name))
}

/// `types`: each generic parameter's bound type; `witnesses`: (generic parameter index, protocol, layout) in canonical
/// order; `checked` and `actual`: the type each checked argument must hold, and the one it holds.
#[doc(hidden)]
pub fn __concrete_args(
    types: &'static [Named],
    witnesses: &[(usize, &'static SwiftProtocol, ProtocolLayout)],
    checked: &'static [Named],
    actual: &[SwiftType],
) -> ConcreteArgs {
    let build = || {
        let metadata = types.iter().map(find).collect::<Result<Vec<_>, _>>()?;
        for (expected, actual) in checked.iter().zip(actual) {
            if find(expected)? != *actual {
                return Err(Refused::Mismatch);
            }
        }
        Ok(GenericArgs {
            witnesses: witnesses.iter().map(|&(param, protocol, layout)| (metadata[param], protocol, layout)).collect(),
            metadata,
            error_slot: None,
        })
    };
    ConcreteArgs(build())
}

impl SwiftArg for ConcreteArgs {
    fn lower(&self, into: &mut ArgLowering) {
        match &self.0 {
            Ok(generic) => generic.lower(into),
            Err(Refused::Missing(name)) => into.fail(CallError::Unavailable(name)),
            Err(Refused::Mismatch) => into.fail(CallError::InvalidArgument(
                "an argument does not hold the type its generic parameter is bound to (a subclass's object: bind the subclass)",
            )),
        }
    }
}

/// What the declaration says of a required protocol (`apus_mangle::ProtocolLayout`): a parameter is passed as a
/// reference exactly when Swift knows its conforming types are classes, which for a protocol only its descriptor tells.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum ProtocolLayout {
    /// Marked `P(AnyObject)`: the parameter is passed as a reference, so P must be class-bound.
    Class,
    /// The parameter is passed by address, so P must not be class-bound.
    Any,
    /// The parameter is class-bound anyway.
    Either,
}

impl ProtocolLayout {
    /// Why the declared lowering is not Swift's, if it is not.
    fn mismatch(self, protocol: &SwiftProtocol) -> Option<&'static str> {
        match (self, protocol.is_class_bound()?) {
            (ProtocolLayout::Class, false) => Some("a protocol marked `(AnyObject)` is not class-bound: remove the mark"),
            (ProtocolLayout::Any, true) => {
                Some("the protocol is class-bound (`: AnyObject`): mark it `P(AnyObject)`, and pass the value as a class reference")
            }
            _ => None,
        }
    }
}

impl SwiftArg for GenericArgs {
    fn lower(&self, into: &mut ArgLowering) {
        for ty in &self.metadata {
            ty.lower(into);
        }
        for (ty, protocol, layout) in &self.witnesses {
            if let Some(why) = layout.mismatch(protocol) {
                into.fail(CallError::InvalidArgument(why));
                into.int(0);
                continue;
            }
            match ty.conformance(protocol) {
                Some(witness) => witness.lower(into),
                None => {
                    into.fail(if protocol.is_available() {
                        CallError::NotConforming { ty: ty.name(), protocol: protocol.descriptor_symbol() }
                    } else {
                        CallError::Unavailable(protocol.descriptor_symbol())
                    });
                    into.int(0);
                }
            }
        }
        if let Some(ty) = self.error_slot {
            ErrorSlot::of(ty).lower(into);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swift::func::ArgLowering;

    /// A protocol whose descriptor is not in the process is unavailable, not something the type fails to conform to.
    #[test]
    fn missing_protocol_is_unavailable() {
        static MISSING: SwiftProtocol = SwiftProtocol::new("$s9NoSuchMod7MissingMp");
        let int = SwiftType::of::<i64>();
        let mut into = ArgLowering::new(false);
        __generic_args(&[int], &[(int, &MISSING, ProtocolLayout::Any)], None).lower(&mut into);
        assert!(matches!(into.failure, Some(CallError::Unavailable("$s9NoSuchMod7MissingMp"))), "{:?}", into.failure);
    }
}
