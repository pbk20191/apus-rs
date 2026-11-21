//! Values of types imported from C (`CGRect`, `NSRange`, an `NS_OPTIONS` set, an `NS_TYPED_ENUM` string): Swift passes
//! and returns them by swiftcall's lowering of their C layout (clang's `SwiftCallingConv.cpp`, `swiftcall`): the value
//! is split into pointer-sized chunks; a chunk of integer or pointer data is one integer register, a `float` or
//! `double` alone in its chunk one floating-point register, in order; more than four registers go by address (an
//! argument as the address of a copy the callee only reads, a result through the indirect result address).
//!
//! The Rust type is the one the objc2 crates define (`objc2_core_foundation::CGRect`, …), its layout read from its
//! Objective-C type encoding (`objc2::Encode`, feature `objc2`); without the feature an integer or `f64` stands for a C
//! scalar. apus refuses what it does not lower (a chunk mixing integers and floating point, two `float`s in a chunk,
//! unions and bitfields, an object inside a struct) with `CallError::InvalidArgument`.

use super::call::{CallError, IndirectSlot, Owned, RetRegs};
use super::func::{ArgLowering, DeclaredResult, SwiftArg, SwiftRet};
use super::rt;
use std::ffi::c_void;
use std::mem::MaybeUninit;

/// A scalar of a C value's layout, at its offset.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leaf {
    Int { offset: usize, size: usize, signed: bool },
    Float { offset: usize },
    Double { offset: usize },
    /// An Objective-C object or block: a reference the call keeps alive (objc2's types only).
    #[cfg_attr(not(feature = "objc2"), allow(dead_code))]
    Object { offset: usize },
}

/// One register of a lowered C value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    /// The bytes `offset..offset + size`, little-endian; `signed`: a lone signed scalar, sign-extended.
    Int { offset: usize, size: usize, signed: bool },
    Float { offset: usize },
    Double { offset: usize },
    Object { offset: usize },
}

/// How a C value crosses: in these registers, or by address.
enum Lowered {
    Registers(Vec<Piece>),
    Indirect { size: usize },
}

/// A C value's layout: its scalars and its size.
#[doc(hidden)]
pub struct CLayout {
    leaves: Vec<Leaf>,
    size: usize,
}

impl CLayout {
    #[cfg(not(feature = "objc2"))]
    fn scalar(size: usize, signed: bool) -> CLayout {
        CLayout { leaves: vec![Leaf::Int { offset: 0, size, signed }], size }
    }

    /// The registers, as swiftcall splits the value, or by address.
    fn lower(&self) -> Result<Lowered, &'static str> {
        const CHUNK: usize = std::mem::size_of::<usize>();
        let offset = |leaf: &Leaf| match leaf {
            Leaf::Int { offset, .. } | Leaf::Float { offset } | Leaf::Double { offset } | Leaf::Object { offset } => *offset,
        };
        let chunks: Vec<Vec<&Leaf>> = (0..self.size.div_ceil(CHUNK))
            .map(|chunk| self.leaves.iter().filter(|leaf| (chunk * CHUNK..(chunk + 1) * CHUNK).contains(&offset(leaf))).collect())
            .collect();
        // Every chunk holding data takes at least one register, so more than four such chunks go by address whatever
        // their mix (clang's `shouldPassIndirectly`: more than four scalars).
        if chunks.iter().filter(|leaves| !leaves.is_empty()).count() > 4 {
            if self.leaves.iter().any(|leaf| matches!(leaf, Leaf::Object { .. })) {
                return Err("a C struct holding an object is not supported");
            }
            return Ok(Lowered::Indirect { size: self.size });
        }
        let mut pieces = Vec::new();
        for (chunk, leaves) in chunks.iter().enumerate() {
            let start = chunk * CHUNK;
            match leaves.as_slice() {
                [] => {}
                [Leaf::Float { offset }] => pieces.push(Piece::Float { offset: *offset }),
                [Leaf::Double { offset }] => pieces.push(Piece::Double { offset: *offset }),
                [Leaf::Object { offset }] if self.leaves.len() == 1 => pieces.push(Piece::Object { offset: *offset }),
                leaves if leaves.iter().any(|l| matches!(l, Leaf::Object { .. })) => {
                    return Err("a C struct holding an object is not supported");
                }
                leaves if leaves.iter().all(|l| matches!(l, Leaf::Int { .. })) => {
                    let covered = leaves.iter().map(|l| match l {
                        Leaf::Int { offset, size, .. } => offset + size,
                        _ => unreachable!(),
                    });
                    let size = covered.max().unwrap() - start;
                    let signed = self.leaves.len() == 1 && matches!(leaves[0], Leaf::Int { signed: true, .. });
                    pieces.push(Piece::Int { offset: start, size, signed });
                }
                _ => return Err("a C value with integers and floating point (or two floats) in one word is not supported"),
            }
        }
        Ok(Lowered::Registers(pieces))
    }
}

/// A Rust type standing for a C type: its layout. Implemented for every `objc2::Encode` type (feature `objc2`; the
/// objc2 framework crates' `CGRect`, `NSRange`, option sets, `&NSString`, …), else for the integers and `f64`.
///
/// # Safety
/// `layout` describes the type's memory exactly, and every value Swift returns of the C type is a valid `Self`.
pub unsafe trait AsCValue: Sized {
    #[doc(hidden)]
    fn layout() -> Result<CLayout, &'static str>;
}

#[cfg(not(feature = "objc2"))]
macro_rules! scalar_c_value {
    ($($t:ty => $signed:expr),*) => {$(
        unsafe impl AsCValue for $t {
            fn layout() -> Result<CLayout, &'static str> {
                Ok(CLayout::scalar(std::mem::size_of::<$t>(), $signed))
            }
        }
    )*};
}
#[cfg(not(feature = "objc2"))]
scalar_c_value!(i8 => true, i16 => true, i32 => true, i64 => true, isize => true, u8 => false, u16 => false, u32 => false, u64 => false, usize => false, bool => false);

#[cfg(not(feature = "objc2"))]
unsafe impl AsCValue for f64 {
    fn layout() -> Result<CLayout, &'static str> {
        Ok(CLayout { leaves: vec![Leaf::Double { offset: 0 }], size: 8 })
    }
}

#[cfg(not(feature = "objc2"))]
unsafe impl AsCValue for f32 {
    fn layout() -> Result<CLayout, &'static str> {
        Ok(CLayout { leaves: vec![Leaf::Float { offset: 0 }], size: 4 })
    }
}

#[cfg(feature = "objc2")]
unsafe impl<T: objc2::Encode> AsCValue for T {
    fn layout() -> Result<CLayout, &'static str> {
        let mut leaves = Vec::new();
        let (size, _) = encoded(&T::ENCODING, 0, &mut leaves)?;
        if size != std::mem::size_of::<T>() {
            return Err("the Rust type's size is not its encoding's C layout");
        }
        Ok(CLayout { leaves, size })
    }
}

/// The scalars of an encoded C type at `base`, with natural alignment (Apple's 64-bit C ABIs); its (size, alignment).
#[cfg(feature = "objc2")]
fn encoded(encoding: &objc2::Encoding, base: usize, leaves: &mut Vec<Leaf>) -> Result<(usize, usize), &'static str> {
    use objc2::Encoding as E;
    let int = |leaves: &mut Vec<Leaf>, size: usize, signed: bool| {
        leaves.push(Leaf::Int { offset: base, size, signed });
        Ok((size, size))
    };
    match encoding {
        E::Char => int(leaves, 1, true),
        E::UChar | E::Bool => int(leaves, 1, false),
        E::Short => int(leaves, 2, true),
        E::UShort => int(leaves, 2, false),
        E::Int => int(leaves, 4, true),
        E::UInt => int(leaves, 4, false),
        E::Long | E::LongLong => int(leaves, 8, true),
        E::ULong | E::ULongLong => int(leaves, 8, false),
        // A pointer that is not an object: not reference counted.
        E::Pointer(_) | E::String | E::Sel | E::Class => int(leaves, 8, false),
        E::Float => {
            leaves.push(Leaf::Float { offset: base });
            Ok((4, 4))
        }
        E::Double => {
            leaves.push(Leaf::Double { offset: base });
            Ok((8, 8))
        }
        E::Object | E::Block => {
            leaves.push(Leaf::Object { offset: base });
            Ok((8, 8))
        }
        E::Atomic(inner) => encoded(inner, base, leaves),
        E::Array(count, element) => {
            let (mut size, mut align) = (0usize, 1usize);
            for _ in 0..*count {
                let (s, a) = encoded(element, base + size, leaves)?;
                size += s;
                align = a;
            }
            Ok((size, align))
        }
        E::Struct(_, fields) => {
            let (mut size, mut align) = (0usize, 1usize);
            for field in fields.iter() {
                // The field's alignment, from a dry run: its offset is rounded up to it.
                let (_, field_align) = encoded(field, 0, &mut Vec::new())?;
                size = size.next_multiple_of(field_align);
                let (field_size, _) = encoded(field, base + size, leaves)?;
                size += field_size;
                align = align.max(field_align);
            }
            Ok((size.next_multiple_of(align), align))
        }
        _ => Err("a C type with a union, bitfield, long double, complex or unknown field is not supported"),
    }
}

/// An argument of a type imported from C; `swift!` wraps such arguments itself, `"?"` marking an optional one.
#[doc(hidden)]
pub struct CValueArg<A>(pub A, pub &'static str);

impl<A: AsCValue> SwiftArg for CValueArg<A> {
    fn lower(&self, into: &mut ArgLowering) {
        let pieces = match A::layout().and_then(|layout| layout.lower()) {
            Ok(Lowered::Registers(pieces)) => pieces,
            Ok(Lowered::Indirect { .. }) if self.1 == "?" => {
                return into.fail(CallError::InvalidArgument("an optional C struct is not supported"));
            }
            Ok(Lowered::Indirect { size }) => {
                // A copy: the call may outlive the borrow.
                let mut copy = vec![0u64; size.div_ceil(8)].into_boxed_slice();
                unsafe { std::ptr::copy_nonoverlapping(&self.0 as *const A as *const u8, copy.as_mut_ptr() as *mut u8, size) };
                into.int(copy.as_ptr() as u64);
                into.owned.push(Owned::Words(copy));
                return;
            }
            Err(why) => return into.fail(CallError::InvalidArgument(why)),
        };
        // Swift's optional of a C type is its nullable pointer only when the type is one (a typed string's object).
        if self.1 == "?" && !matches!(pieces.as_slice(), [Piece::Object { .. }] | [Piece::Int { size: 8, .. }]) {
            return into.fail(CallError::InvalidArgument("an optional C struct is not supported"));
        }
        let bytes = &self.0 as *const A as *const u8;
        let read = |offset: usize, size: usize| {
            let mut word = [0u8; 8];
            unsafe { std::ptr::copy_nonoverlapping(bytes.add(offset), word.as_mut_ptr(), size) };
            u64::from_le_bytes(word)
        };
        for piece in pieces {
            match piece {
                Piece::Int { offset, size, signed } => {
                    let value = read(offset, size);
                    let shift = 64 - 8 * size as u32;
                    into.int(if signed && shift > 0 { (((value << shift) as i64) >> shift) as u64 } else { value })
                }
                Piece::Double { offset } => into.float(f64::from_bits(read(offset, 8))),
                // A `float` is the low half of its register.
                Piece::Float { offset } => into.float(f64::from_bits(read(offset, 4))),
                Piece::Object { offset } => {
                    let object = read(offset, 8) as *mut c_void;
                    if !object.is_null() {
                        // The call may outlive the borrow.
                        into.owned.push(Owned::Object(unsafe { rt::swift_unknownObjectRetain(object) }));
                    }
                    into.int(object as u64)
                }
            }
        }
    }
}

/// A result of a type imported from C, as its Rust type (an objc2 crate's): `-> CValue<CGRect>`. A typed string's
/// object is a class reference instead: `-> Retained<NSString>`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CValue<T>(pub T);

impl<T: AsCValue> SwiftRet for CValue<T> {
    fn float() -> bool {
        matches!(T::layout().and_then(|layout| layout.lower()), Ok(Lowered::Registers(p)) if !p.is_empty() && p.iter().all(|p| matches!(p, Piece::Float { .. } | Piece::Double { .. })))
    }

    unsafe fn lift(ret: &RetRegs, indirect: &mut IndirectSlot) -> Self {
        let pieces = match T::layout().and_then(|layout| layout.lower()).expect("checked by result_slot") {
            Lowered::Registers(pieces) => pieces,
            Lowered::Indirect { .. } => return CValue(indirect.address().cast::<T>().read()),
        };
        let mut value = MaybeUninit::<T>::zeroed();
        let bytes = value.as_mut_ptr() as *mut u8;
        let write = |offset: usize, word: u64, size: usize| std::ptr::copy_nonoverlapping(word.to_le_bytes().as_ptr(), bytes.add(offset), size);
        let (mut ints, mut floats) = (0, 0);
        for piece in pieces {
            match piece {
                Piece::Int { offset, size, .. } => {
                    write(offset, ret.0.gpr[ints], size);
                    ints += 1;
                }
                Piece::Double { offset } => {
                    write(offset, ret.0.fpr[floats].to_bits(), 8);
                    floats += 1;
                }
                Piece::Float { offset } => {
                    write(offset, ret.0.fpr[floats].to_bits(), 4);
                    floats += 1;
                }
                Piece::Object { .. } => unreachable!("refused by result_slot"),
            }
        }
        CValue(value.assume_init())
    }

    fn result_slot(_result_type: DeclaredResult) -> Result<(IndirectSlot, bool), CallError> {
        let pieces = match T::layout().and_then(|layout| layout.lower()).map_err(CallError::InvalidArgument)? {
            Lowered::Registers(pieces) => pieces,
            Lowered::Indirect { size } => return Ok((IndirectSlot::words(size), true)),
        };
        if pieces.iter().any(|p| matches!(p, Piece::Object { .. })) {
            return Err(CallError::InvalidArgument("a typed string comes back as its object (+1): declare `-> Retained<T>`"));
        }
        // A synchronous call reads either the integer or the floating-point result registers.
        let floats = pieces.iter().filter(|p| matches!(p, Piece::Float { .. } | Piece::Double { .. })).count();
        if floats != 0 && floats != pieces.len() {
            return Err(CallError::InvalidArgument("a C value returned in integer and floating-point registers is not supported"));
        }
        Ok((IndirectSlot::inline(), false))
    }
}
