//! C function pointers and Objective-C blocks as Swift's `@convention(c)` and `@convention(block)` functions.
//!
//! Both are one pointer: a C function pointer, called with the C calling convention; a block object, called and
//! reference counted as the Blocks runtime specifies (clang's Block-ABI-Apple, `_Block_copy` / `_Block_release`). A
//! block argument is passed guaranteed and a block result comes back +1, as for any Swift class reference; an
//! initializer consumes its block argument. `swift!` checks the Rust function's or block's parameter and result types
//! against the declared ones (`CallError::ClosureMismatch`), as Objective-C type encodings.

use super::call::{CallError, IndirectSlot, RetRegs};
use super::func::{ArgLowering, SwiftArg, SwiftRet};
use std::ffi::c_void;

/// Whether a Rust function's or block's shape (`(id)q`) is the declared one, where `?` declares a type apus does not
/// check (a pointer, an object, a struct).
fn shape_matches(ours: &str, declared: &str) -> bool {
    ours.len() == declared.len() && ours.chars().zip(declared.chars()).all(|(a, b)| a == b || b == '?')
}

fn check_shape(into: &mut ArgLowering, ours: String, declared: &str) {
    if !shape_matches(&ours, declared) {
        into.fail(CallError::ClosureMismatch(format!(
            "the function is `{ours}`, the declared function type `{declared}` (Objective-C type encodings: q Int, i Int32, \
             d Double, B Bool, v no result, ? any other type)"
        )));
    }
}

/// A type a C function takes or returns, by its Objective-C type encoding.
pub trait CType {
    #[doc(hidden)]
    const CODE: char;
}

macro_rules! c_types {
    ($($t:ty: $code:literal),* $(,)?) => {$(
        impl CType for $t {
            const CODE: char = $code;
        }
    )*};
}
c_types! {
    i64: 'q', isize: 'q', u64: 'Q', usize: 'Q', i32: 'i', u32: 'I', i16: 's', u16: 'S', i8: 'c', u8: 'C',
    f64: 'd', f32: 'f', bool: 'B',
}

impl<T> CType for *const T {
    const CODE: char = '?';
}

impl<T> CType for *mut T {
    const CODE: char = '?';
}

/// A C function's result type: a [`CType`] or `()`.
pub trait CReturn {
    #[doc(hidden)]
    const CODE: char;
}

impl CReturn for () {
    const CODE: char = 'v';
}

impl<T: CType> CReturn for T {
    const CODE: char = T::CODE;
}

/// A Rust function pointer Swift can call as a `@convention(c)` function: `extern "C" fn(A, B) -> R`.
pub trait CFunctionPointer: Copy {
    #[doc(hidden)]
    fn shape() -> String;
    #[doc(hidden)]
    fn address(self) -> *const c_void;
}

macro_rules! c_function_pointers {
    ($($($a:ident)*;)*) => {$(
        impl<R: CReturn, $($a: CType),*> CFunctionPointer for extern "C" fn($($a),*) -> R {
            fn shape() -> String {
                format!("({}){}", <String as FromIterator<char>>::from_iter([$(<$a as CType>::CODE),*]), R::CODE)
            }
            fn address(self) -> *const c_void {
                self as *const c_void
            }
        }

        impl<R: CReturn, $($a: CType),*> CFunctionPointer for unsafe extern "C" fn($($a),*) -> R {
            fn shape() -> String {
                format!("({}){}", <String as FromIterator<char>>::from_iter([$(<$a as CType>::CODE),*]), R::CODE)
            }
            fn address(self) -> *const c_void {
                self as *const c_void
            }
        }

        /// A `@convention(c)` result: Swift's function pointer, never null.
        impl<R: CReturn, $($a: CType),*> SwiftRet for extern "C" fn($($a),*) -> R {
            unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
                let address = ret.0.gpr[0] as *const c_void;
                assert!(!address.is_null(), "a non-optional C function result is never null");
                std::mem::transmute_copy(&address)
            }
        }

        /// An optional `@convention(c)` result: nil is the null pointer.
        impl<R: CReturn, $($a: CType),*> SwiftRet for Option<extern "C" fn($($a),*) -> R> {
            unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
                let address = ret.0.gpr[0] as *const c_void;
                std::mem::transmute_copy(&address)
            }
        }
    )*};
}
c_function_pointers! {
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

/// What a `@convention(c)` argument is given as: a C function pointer, or for an optional one, `Option` of one.
#[diagnostic::on_unimplemented(
    message = "a `@convention(c)` parameter takes an `extern \"C\" fn(…)` (`Option<…>` if optional), not `{Self}`"
)]
pub trait AsCFunction {
    #[doc(hidden)]
    fn shape() -> String;
    #[doc(hidden)]
    fn address(&self) -> *const c_void;
}

impl<F: CFunctionPointer> AsCFunction for F {
    fn shape() -> String {
        F::shape()
    }
    fn address(&self) -> *const c_void {
        CFunctionPointer::address(*self)
    }
}

impl<F: CFunctionPointer> AsCFunction for Option<F> {
    fn shape() -> String {
        F::shape()
    }
    fn address(&self) -> *const c_void {
        self.map_or(std::ptr::null(), CFunctionPointer::address)
    }
}

/// A `@convention(c)` argument with its declared shape. `swift!` wraps such arguments itself.
#[doc(hidden)]
pub struct CFunctionArg<A>(pub A, pub &'static str);

impl<A: AsCFunction> SwiftArg for CFunctionArg<A> {
    fn lower(&self, into: &mut ArgLowering) {
        check_shape(into, A::shape(), self.1);
        into.int(self.0.address() as u64)
    }
}

/// What a `@convention(block)` argument is given as: a block2 block (`&Block<dyn Fn(…) -> R>`, which an `RcBlock` or a
/// `StackBlock` gives), or for an optional one, `Option` of one. Needs the feature `block2`.
#[diagnostic::on_unimplemented(
    message = "a `@convention(block)` parameter takes a block2 `&Block<dyn Fn(…)>` (`Option<…>` if optional; feature `block2`), not `{Self}`"
)]
pub trait AsBlock {
    #[doc(hidden)]
    fn shape() -> String;
    /// The block, or null.
    #[doc(hidden)]
    fn block(&self) -> *const c_void;
}

/// A `@convention(block)` argument with its declared shape. `swift!` wraps such arguments itself.
#[doc(hidden)]
pub struct BlockArg<A>(pub A, pub &'static str);

impl<A: AsBlock> SwiftArg for BlockArg<A> {
    /// The block, guaranteed: a copy (`_Block_copy`, which moves a stack block to the heap) is held until the call is
    /// over, and handed over when the callee consumes it.
    fn lower(&self, into: &mut ArgLowering) {
        check_shape(into, A::shape(), self.1);
        let block = self.0.block();
        if block.is_null() {
            return into.int(0);
        }
        #[cfg(feature = "block2")]
        {
            let copy = unsafe { block2::ffi::_Block_copy(block) };
            into.owned.push(super::call::Owned::Block(copy));
            into.int(copy as u64)
        }
        #[cfg(not(feature = "block2"))]
        unreachable!("no block without the feature `block2`")
    }
}

#[cfg(feature = "block2")]
mod blocks {
    use super::*;
    use block2::{Block, BlockFn, RcBlock};
    use objc2::encode::{EncodeArguments, EncodeReturn, Encoding};

    fn code(encoding: &Encoding) -> char {
        match encoding {
            Encoding::Long | Encoding::LongLong => 'q',
            Encoding::ULong | Encoding::ULongLong => 'Q',
            Encoding::Int => 'i',
            Encoding::UInt => 'I',
            Encoding::Short => 's',
            Encoding::UShort => 'S',
            Encoding::Char => 'c',
            Encoding::UChar => 'C',
            Encoding::Double => 'd',
            Encoding::Float => 'f',
            Encoding::Bool => 'B',
            Encoding::Void => 'v',
            _ => '?',
        }
    }

    fn shape<F: ?Sized + BlockFn>() -> String {
        let params: String = F::Args::ENCODINGS.iter().map(code).collect();
        format!("({params}){}", code(&F::Output::ENCODING_RETURN))
    }

    impl<F: ?Sized + BlockFn> AsBlock for &Block<F> {
        fn shape() -> String {
            shape::<F>()
        }
        fn block(&self) -> *const c_void {
            *self as *const Block<F> as *const c_void
        }
    }

    impl<F: ?Sized + BlockFn> AsBlock for Option<&Block<F>> {
        fn shape() -> String {
            shape::<F>()
        }
        fn block(&self) -> *const c_void {
            self.map_or(std::ptr::null(), |block| block as *const Block<F> as *const c_void)
        }
    }

    /// A `@convention(block)` result (+1). `F` is what the declaration says the block is; it is not checked.
    impl<F: ?Sized + BlockFn> SwiftRet for RcBlock<F> {
        unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
            RcBlock::from_raw(ret.0.gpr[0] as *mut Block<F>).expect("a non-optional block result is never nil")
        }
    }

    /// An optional `@convention(block)` result: nil is the null pointer.
    impl<F: ?Sized + BlockFn> SwiftRet for Option<RcBlock<F>> {
        unsafe fn lift(ret: &RetRegs, _indirect: &mut IndirectSlot) -> Self {
            RcBlock::from_raw(ret.0.gpr[0] as *mut Block<F>)
        }
    }
}
