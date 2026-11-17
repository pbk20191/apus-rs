//! Generic arguments `swift!` fills itself: type metadata, witness tables in the ABI's canonical requirement order,
//! and a typed-throws error slot, taken from the call's own arguments (apus_mangle::generic_plan says from which).
use super::func::{ArgLowering, ErrorSlot, SwiftArg};
use super::value::{SwiftProtocol, SwiftType, SwiftValue};
use super::CallError;

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

#[doc(hidden)]
pub fn __type_of<T: HasSwiftType + ?Sized>(value: &T) -> SwiftType {
    value.swift_type()
}

/// The generic arguments after the formal ones: metadata, witness tables, then the error slot.
#[doc(hidden)]
pub struct GenericArgs {
    metadata: Vec<SwiftType>,
    witnesses: Vec<(SwiftType, &'static SwiftProtocol)>,
    error_slot: Option<SwiftType>,
}

#[doc(hidden)]
pub fn __generic_args(metadata: &[SwiftType], witnesses: &[(SwiftType, &'static SwiftProtocol)], error_slot: Option<SwiftType>) -> GenericArgs {
    GenericArgs { metadata: metadata.to_vec(), witnesses: witnesses.to_vec(), error_slot }
}

impl SwiftArg for GenericArgs {
    fn lower(&self, into: &mut ArgLowering) {
        for ty in &self.metadata {
            ty.lower(into);
        }
        for (ty, protocol) in &self.witnesses {
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
        __generic_args(&[int], &[(int, &MISSING)], None).lower(&mut into);
        assert!(matches!(into.failure, Some(CallError::Unavailable("$s9NoSuchMod7MissingMp"))), "{:?}", into.failure);
    }
}
