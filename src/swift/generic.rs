//! Generic arguments `swift!` fills itself: type metadata, witness tables in the ABI's canonical requirement order,
//! and a typed-throws error slot, taken from the call's own arguments (apus_mangle::generic_plan says from which).
use super::func::{ArgLowering, ErrorSlot, SwiftArg, SwiftObject};
use super::value::{SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
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

/// The generic arguments of a declaration whose generic parameters `swift!` binds to concrete types
/// (`#[swift = "…", T = "Type"]`): each bound type found by its mangled name once, the declaration's requirements
/// checked, then lowered as [`GenericArgs`].
#[doc(hidden)]
pub struct ConcreteArgs(Result<&'static Resolved, Refused>);

/// A bound declaration's generic arguments, found once per call site: its metadata, witness tables (each
/// conformance looked up, its layout checked) and error type, after its superclass and associated type requirements
/// held. The bound types are fixed when the program is built, and a Swift image's types and conformances stay for
/// the life of the process (the runtime does not support unloading one); a failure is not kept, so a type an image
/// loaded later brings is found then.
#[doc(hidden)]
pub struct Resolved {
    metadata: Vec<SwiftType>,
    witnesses: Vec<WitnessTable>,
    error_slot: Option<SwiftType>,
}

/// Where `swift!` keeps a call site's [`Resolved`] arguments.
#[doc(hidden)]
pub type ResolvedOnce = std::sync::OnceLock<Resolved>;

enum Refused {
    /// A type or symbol named by the binding is not in the process.
    Missing(&'static str),
    /// A bound type does not conform to a protocol the declaration requires.
    NotConforming { ty: String, protocol: &'static str },
    /// The bindings break a requirement the declaration states, or an argument does not hold its bound type.
    Invalid(&'static str),
}

/// A type found by its mangled name once, kept where `swift!` put it.
#[doc(hidden)]
pub type Named = (&'static str, std::sync::OnceLock<Option<SwiftType>>);

fn find((name, found): &'static Named) -> Result<SwiftType, Refused> {
    found.get_or_init(|| SwiftType::from_mangled_name(name)).ok_or(Refused::Missing(name))
}

/// What a witness table or check is about: a bound type (an index into the call's names), or one's associated type
/// (the type, then a step for each name of its path: the protocol whose conformance gives it, the protocol's `…TL`
/// and the associated type's `…Tl` symbols).
#[doc(hidden)]
pub enum BoundSubject {
    Named(usize),
    Associated(usize, &'static [(usize, &'static str, &'static str)]),
}

/// `names` and `protocols`: everything the call looks up, which the rest refer to by index. `types`: the metadata
/// passed; `witnesses`: (subject, protocol, layout) in canonical order; `superclasses`: (bound type, class it must be
/// or inherit from); `same_types`: (associated type, the type it must be); `error`: a typed-throws error type;
/// `checked`: (bound type, the type an argument holds), which must be the same.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn __concrete_args(
    resolved: &'static ResolvedOnce,
    names: &'static [Named],
    protocols: &'static [SwiftProtocol],
    types: &[usize],
    witnesses: &[(BoundSubject, usize, ProtocolLayout)],
    superclasses: &[(usize, usize)],
    same_types: &[(BoundSubject, usize)],
    error: Option<usize>,
    checked: &[(usize, SwiftType)],
) -> ConcreteArgs {
    let name = |i: usize| find(&names[i]);
    let subject = |subject: &BoundSubject| -> Result<SwiftType, Refused> {
        match *subject {
            BoundSubject::Named(i) => name(i),
            BoundSubject::Associated(i, path) => {
                let mut ty = name(i)?;
                for &(p, base, descriptor) in path {
                    let protocol = &protocols[p];
                    let Some(conformance) = ty.conformance(protocol) else { return Err(not_conforming(ty, protocol)) };
                    ty = ty.associated_type(conformance, base, descriptor).ok_or(Refused::Missing(descriptor))?;
                }
                Ok(ty)
            }
        }
    };
    let build = || {
        for &(sub, class) in superclasses {
            // swift_dynamicCastMetatype (include/swift/Runtime/Casting.h): the type if it is the class or a subclass.
            if unsafe { rt::swift_dynamicCastMetatype(name(sub)?.as_ptr(), name(class)?.as_ptr()) }.is_null() {
                return Err(Refused::Invalid("a bound type is not the class its generic parameter requires, nor a subclass of it"));
            }
        }
        for (associated, expected) in same_types {
            if subject(associated)? != name(*expected)? {
                return Err(Refused::Invalid("a bound type's associated type is not the one the declaration requires"));
            }
        }
        let witness = |(on, p, layout): &(BoundSubject, usize, ProtocolLayout)| {
            let (ty, protocol) = (subject(on)?, &protocols[*p]);
            if let Some(why) = layout.mismatch(protocol) {
                return Err(Refused::Invalid(why));
            }
            ty.conformance(protocol).ok_or_else(|| not_conforming(ty, protocol))
        };
        Ok(Resolved {
            metadata: types.iter().map(|&i| name(i)).collect::<Result<_, _>>()?,
            witnesses: witnesses.iter().map(witness).collect::<Result<_, _>>()?,
            error_slot: error.map(name).transpose()?,
        })
    };
    let check = || {
        for &(expected, actual) in checked {
            if name(expected)? != actual {
                return Err(Refused::Invalid(
                    "an argument does not hold the type its generic parameter is bound to (a subclass's object: bind the subclass)",
                ));
            }
        }
        match resolved.get() {
            Some(found) => Ok(found),
            None => {
                let found = build()?;
                Ok(resolved.get_or_init(|| found))
            }
        }
    };
    ConcreteArgs(check())
}

fn not_conforming(ty: SwiftType, protocol: &SwiftProtocol) -> Refused {
    match protocol.is_available() {
        true => Refused::NotConforming { ty: ty.name(), protocol: protocol.descriptor_symbol() },
        false => Refused::Missing(protocol.descriptor_symbol()),
    }
}

impl ConcreteArgs {
    /// The arguments, or why the call is refused (as their lowering reports it on the general path).
    #[doc(hidden)]
    #[inline]
    pub fn resolved(self) -> Result<&'static Resolved, CallError> {
        match self.0 {
            Ok(resolved) => Ok(resolved),
            Err(Refused::Missing(name)) => Err(CallError::Unavailable(name)),
            Err(Refused::NotConforming { ty, protocol }) => Err(CallError::NotConforming { ty, protocol }),
            Err(Refused::Invalid(why)) => Err(CallError::InvalidArgument(why)),
        }
    }
}

/// The generic arguments after the formal ones on the scalar path (metadata, then witness tables), kept for an
/// opaque result's type; a typed-throws error slot takes the general path.
impl super::sync::ScalarArg for &'static Resolved {
    #[inline]
    fn put(self, regs: &mut apus_sys::CallRegs, ints: &mut usize, _floats: &mut usize, made: &mut super::sync::Made) -> bool {
        if self.error_slot.is_some() {
            return false;
        }
        let words = self.metadata.iter().map(|ty| ty.as_ptr()).chain(self.witnesses.iter().map(|w| w.0));
        for word in words {
            let Some(slot) = regs.gpr.get_mut(*ints) else { return false };
            *slot = word as u64;
            *ints += 1;
            made.generics.push(word);
        }
        true
    }
}

impl SwiftArg for ConcreteArgs {
    fn lower(&self, into: &mut ArgLowering) {
        match &self.0 {
            Ok(resolved) => {
                resolved.metadata.iter().for_each(|ty| ty.lower(into));
                resolved.witnesses.iter().for_each(|witness| witness.lower(into));
                if let Some(ty) = resolved.error_slot {
                    ErrorSlot::of(ty).lower(into);
                }
            }
            Err(Refused::Missing(name)) => into.fail(CallError::Unavailable(name)),
            Err(Refused::NotConforming { ty, protocol }) => into.fail(CallError::NotConforming { ty: ty.clone(), protocol }),
            Err(Refused::Invalid(why)) => into.fail(CallError::InvalidArgument(why)),
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
