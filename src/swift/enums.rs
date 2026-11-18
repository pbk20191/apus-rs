//! Swift enum values by case tag: which case a value holds, a copy of its payload, and new values of a case.
//!
//! Only the runtime's public ABI is used: the enum value witnesses (include/swift/ABI/ValueWitness.def) and the
//! type's descriptor (include/swift/ABI/Metadata.h, `TargetEnumDescriptor`). Tags number the cases with a payload
//! first, in declaration order, then the cases without one. A library-evolution module (every Apple framework)
//! exports each case's tag as a symbol, `mangle!("case Module.Enum(enum).name(Payload)")`, read by [`SwiftEnumCase`].

use super::rt;
use super::value::{lookup, SwiftType, SwiftValue, UninitValue};
use std::sync::OnceLock;

/// How many cases an enum type has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumCases {
    pub with_payload: u32,
    pub without_payload: u32,
}

impl SwiftType {
    /// The enum's cases, or `None` for a type that is not a Swift enum (`Optional` is one). An enum imported from C
    /// (`NS_ENUM`) is refused: its value is the raw integer, not a case tag; use its `rawValue`.
    pub fn enum_cases(self) -> Option<EnumCases> {
        unsafe {
            let metadata = &*(self.as_ptr() as *const rt::EnumMetadata);
            if !matches!(metadata.kind as u32, rt::METADATA_KIND_ENUM | rt::METADATA_KIND_OPTIONAL) {
                return None;
            }
            let descriptor = &*metadata.description;
            if descriptor.flags & rt::TYPE_CONTEXT_HAS_IMPORT_INFO != 0 {
                return None;
            }
            Some(EnumCases {
                with_payload: descriptor.num_payload_cases_and_payload_size_offset & rt::ENUM_PAYLOAD_CASES_MASK,
                without_payload: descriptor.num_empty_cases,
            })
        }
    }
}

impl SwiftValue {
    /// The tag of the case the value holds, or `None` if it is not an enum (see [`SwiftType::enum_cases`]).
    pub fn enum_tag(&self) -> Option<u32> {
        self.ty().enum_cases()?;
        Some(unsafe { (rt::value_witnesses(self.ty().as_ptr()).get_enum_tag.unwrap())(self.as_ptr(), self.ty().as_ptr()) })
    }

    /// A copy of the payload of the case the value holds, as a value of `payload` (a tuple type for several associated
    /// values); `None` if the value is not an enum or its case has no payload. The value itself is untouched.
    ///
    /// # Safety
    /// `payload` must be the payload type of the case the value holds, and that case must not be `indirect`
    /// (see [`indirect_enum_payload`](Self::indirect_enum_payload)).
    pub unsafe fn enum_payload(&self, payload: SwiftType) -> Option<SwiftValue> {
        self.payload(payload, false)
    }

    /// [`enum_payload`](Self::enum_payload) for an `indirect` case, whose payload lives in a box.
    ///
    /// # Safety
    /// `payload` must be the payload type of the case the value holds, and that case must be `indirect`.
    pub unsafe fn indirect_enum_payload(&self, payload: SwiftType) -> Option<SwiftValue> {
        self.payload(payload, true)
    }

    unsafe fn payload(&self, payload: SwiftType, indirect: bool) -> Option<SwiftValue> {
        let ty = self.ty();
        let cases = ty.enum_cases()?;
        let tag = self.enum_tag()?;
        if tag >= cases.with_payload {
            return None;
        }
        let witnesses = rt::value_witnesses(ty.as_ptr());
        let payload_witnesses = rt::value_witnesses(payload.as_ptr());
        if !indirect && payload_witnesses.size > witnesses.size {
            return None; // cannot be this enum's payload
        }
        // Work on a copy: projecting strips the tag in place.
        let copy = self.clone();
        (witnesses.destructive_project_enum_data.unwrap())(copy.as_ptr(), ty.as_ptr());
        let out = UninitValue::new(payload);
        if indirect {
            // The storage holds the box; the copy keeps its reference until it is destroyed below.
            let value = rt::swift_projectBox(*(copy.as_ptr() as *const *mut std::ffi::c_void));
            (payload_witnesses.initialize_with_copy.unwrap())(out.ptr.as_ptr().cast(), value, payload.as_ptr());
            (witnesses.destructive_inject_enum_tag.unwrap())(copy.as_ptr(), tag, ty.as_ptr());
            drop(copy);
        } else {
            (payload_witnesses.initialize_with_take.unwrap())(out.ptr.as_ptr().cast(), copy.as_ptr(), payload.as_ptr());
            drop(copy.into_uninit()); // the payload moved out
        }
        Some(out.assume_init())
    }

    /// A value of `ty`'s case `tag`, a case without payload; `None` if `ty` is not an enum or `tag` is not such a case.
    pub fn enum_case(ty: SwiftType, tag: u32) -> Option<SwiftValue> {
        let cases = ty.enum_cases()?;
        if tag < cases.with_payload || tag >= cases.with_payload + cases.without_payload {
            return None;
        }
        let storage = UninitValue::new(ty);
        unsafe {
            (rt::value_witnesses(ty.as_ptr()).destructive_inject_enum_tag.unwrap())(storage.ptr.as_ptr().cast(), tag, ty.as_ptr());
            Some(storage.assume_init())
        }
    }

    /// A value of `ty`'s case `tag` holding `payload` (a tuple for several associated values), which it consumes;
    /// `payload` comes back if `ty` is not an enum or `tag` is not a case with a payload.
    ///
    /// # Safety
    /// `payload` must have the case's payload type, and the case must not be `indirect` (not supported: its box
    /// would have to be built the way the compiler builds it).
    pub unsafe fn enum_case_with(ty: SwiftType, tag: u32, payload: SwiftValue) -> Result<SwiftValue, SwiftValue> {
        let Some(cases) = ty.enum_cases() else { return Err(payload) };
        let witnesses = rt::value_witnesses(ty.as_ptr());
        if tag >= cases.with_payload || rt::value_witnesses(payload.ty().as_ptr()).size > witnesses.size {
            return Err(payload);
        }
        let storage = UninitValue::new(ty);
        let payload_ty = payload.ty();
        (rt::value_witnesses(payload_ty.as_ptr()).initialize_with_take.unwrap())(storage.ptr.as_ptr().cast(), payload.as_ptr(), payload_ty.as_ptr());
        drop(payload.into_uninit()); // moved into the enum
        (witnesses.destructive_inject_enum_tag.unwrap())(storage.ptr.as_ptr().cast(), tag, ty.as_ptr());
        Ok(storage.assume_init())
    }
}

/// A case's tag, read from the symbol a library-evolution module exports for it:
/// `SwiftEnumCase::new(apus::mangle!("case Swift.DecodingError(enum).dataCorrupted(DecodingError(enum).Context(struct))"))`.
/// A module built
/// without library evolution exports none; its tags follow the declaration (see the module docs).
pub struct SwiftEnumCase {
    symbol: &'static str,
    tag: OnceLock<usize>,
}

impl SwiftEnumCase {
    pub const fn new(tag_symbol: &'static str) -> SwiftEnumCase {
        SwiftEnumCase { symbol: tag_symbol, tag: OnceLock::new() }
    }

    /// The tag symbol (`…WC`).
    pub fn symbol(&self) -> &'static str {
        self.symbol
    }

    /// The tag, or `None` if the symbol is not in the process (a case newer than the OS's framework).
    pub fn tag(&self) -> Option<u32> {
        let address = *self.tag.get_or_init(|| lookup(self.symbol));
        (address != 0).then(|| unsafe { *(address as *const u32) })
    }
}
