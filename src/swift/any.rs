//! Owned Swift `Any` values and `any Error` boxes, read through the runtime's casting and value witnesses.

use super::rt;
use std::ffi::c_void;

/// An owned Swift `Any` (opaque existential: 3-word value buffer + type metadata).
pub struct SwiftAny {
    container: [usize; 4],
}

// Swift values in `Any` are reference-counted atomically and never mutated through this type.
unsafe impl Send for SwiftAny {}

impl SwiftAny {
    /// Takes ownership of an initialized `Any` container.
    ///
    /// # Safety
    /// `container` must hold an initialized Swift `Any` that nothing else will destroy.
    pub unsafe fn from_raw_initialized(container: [usize; 4]) -> SwiftAny {
        SwiftAny { container }
    }

    fn dynamic_type(&self) -> *const c_void {
        self.container[3] as *const c_void
    }

    /// The qualified Swift name of the value's dynamic type, e.g. `Swift.Int`.
    pub fn type_name(&self) -> String {
        unsafe { rt::type_name(self.dynamic_type()) }
    }

    /// Copies the value out as `T`, if it is one (`swift_dynamicCast`; optionals are unwrapped as Swift does).
    pub fn downcast<T: FromSwiftAny>(&self) -> Option<T> {
        T::from_any(self)
    }

    /// Casts a copy of the value into `dest` (sized for `target`). The source stays intact.
    fn cast_into(&self, target: *const c_void, dest: *mut c_void) -> bool {
        let src = self.container.as_ptr() as *mut c_void;
        unsafe { rt::swift_dynamicCast(dest, src, rt::any_metadata(), target, 0) }
    }
}

impl std::fmt::Debug for SwiftAny {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftAny({})", self.type_name())
    }
}

impl Drop for SwiftAny {
    fn drop(&mut self) {
        let any = rt::any_metadata();
        unsafe { (rt::value_witnesses(any).destroy)(self.container.as_mut_ptr().cast(), any) }
    }
}

/// Types a `SwiftAny` can be read as.
pub trait FromSwiftAny: Sized {
    #[doc(hidden)]
    fn swift_metadata() -> *const c_void;
    #[doc(hidden)]
    fn from_any(any: &SwiftAny) -> Option<Self>;
}

macro_rules! plain {
    ($t:ty, $sym:ident) => {
        impl FromSwiftAny for $t {
            fn swift_metadata() -> *const c_void {
                rt::metadata_of(unsafe { &rt::$sym })
            }
            fn from_any(any: &SwiftAny) -> Option<Self> {
                let mut out = std::mem::MaybeUninit::<$t>::zeroed();
                any.cast_into(Self::swift_metadata(), out.as_mut_ptr().cast()).then(|| unsafe { out.assume_init() })
            }
        }
    };
}
plain!(i64, INT_METADATA);
plain!(i32, INT32_METADATA);
plain!(u64, UINT64_METADATA);
plain!(f64, DOUBLE_METADATA);

impl FromSwiftAny for bool {
    fn swift_metadata() -> *const c_void {
        rt::metadata_of(unsafe { &rt::BOOL_METADATA })
    }
    fn from_any(any: &SwiftAny) -> Option<Self> {
        let mut out = 0u8;
        any.cast_into(Self::swift_metadata(), (&mut out as *mut u8).cast()).then_some(out & 1 == 1)
    }
}

impl FromSwiftAny for () {
    fn swift_metadata() -> *const c_void {
        rt::empty_tuple_metadata()
    }
    fn from_any(any: &SwiftAny) -> Option<Self> {
        let mut nothing = 0u8;
        any.cast_into(Self::swift_metadata(), (&mut nothing as *mut u8).cast()).then_some(())
    }
}

impl FromSwiftAny for String {
    fn swift_metadata() -> *const c_void {
        rt::metadata_of(unsafe { &rt::STRING_METADATA })
    }
    fn from_any(any: &SwiftAny) -> Option<Self> {
        let mut s = rt::SwiftString { words: [0; 2] };
        if !any.cast_into(Self::swift_metadata(), (&mut s as *mut rt::SwiftString).cast()) {
            return None;
        }
        unsafe {
            let out = rt::string_to_rust(&s);
            rt::destroy_string(&mut s);
            Some(out)
        }
    }
}

/// `Optional<T>`: `Some(None)` for a nil optional, `Some(Some(v))` for a value of `T`.
impl<T: FromSwiftAny> FromSwiftAny for Option<T> {
    fn swift_metadata() -> *const c_void {
        unsafe { rt::optional_metadata(0, T::swift_metadata()) }.metadata
    }
    fn from_any(any: &SwiftAny) -> Option<Self> {
        let metadata = Self::swift_metadata();
        let witnesses = unsafe { rt::value_witnesses(metadata) };
        let align = (witnesses.flags as usize & 0xff) + 1;
        let layout = std::alloc::Layout::from_size_align(witnesses.size.max(1), align.max(16)).unwrap();
        unsafe {
            let buf = std::alloc::alloc(layout).cast::<c_void>();
            let is_optional = any.cast_into(metadata, buf);
            let none = is_optional && (witnesses.get_enum_tag)(buf, metadata) == 1; // Optional: some = 0, none = 1
            if is_optional {
                (witnesses.destroy)(buf, metadata);
            }
            std::alloc::dealloc(buf.cast(), layout);
            match (is_optional, none) {
                (_, true) => Some(None),
                _ => T::from_any(any).map(Some),
            }
        }
    }
}

/// An owned Swift `any Error`.
pub struct SwiftError {
    boxed: *mut c_void,
}

unsafe impl Send for SwiftError {}

impl SwiftError {
    /// Takes ownership of an `any Error` box (+1).
    ///
    /// # Safety
    /// `boxed` must be an owned Swift error box.
    pub unsafe fn from_raw(boxed: *mut c_void) -> SwiftError {
        SwiftError { boxed }
    }

    fn opened(&self) -> rt::ErrorValueResult {
        let mut scratch = std::ptr::null_mut();
        let mut out = rt::ErrorValueResult { value: std::ptr::null(), type_: std::ptr::null(), error_conformance: std::ptr::null() };
        unsafe { rt::swift_getErrorValue(self.boxed, &mut scratch, &mut out) };
        out
    }

    /// The qualified Swift name of the error's dynamic type.
    pub fn type_name(&self) -> String {
        unsafe { rt::type_name(self.opened().type_) }
    }

    /// `String(describing: error)`.
    pub fn description(&self) -> String {
        let opened = self.opened();
        unsafe {
            let witnesses = rt::value_witnesses(opened.type_);
            let align = (witnesses.flags as usize & 0xff) + 1;
            let layout = std::alloc::Layout::from_size_align(witnesses.size.max(1), align.max(16)).unwrap();
            let copy = std::alloc::alloc(layout).cast::<c_void>();
            (witnesses.initialize_with_copy)(copy, opened.value as *mut c_void, opened.type_);
            let mut text = rt::string_describing(copy, opened.type_); // consumes the copy
            std::alloc::dealloc(copy.cast(), layout);
            let out = rt::string_to_rust(&text);
            rt::destroy_string(&mut text);
            out
        }
    }
}

impl Drop for SwiftError {
    fn drop(&mut self) {
        unsafe { rt::swift_errorRelease(self.boxed) }
    }
}

impl std::fmt::Debug for SwiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftError({}: {})", self.type_name(), self.description())
    }
}

impl std::fmt::Display for SwiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description())
    }
}

impl std::error::Error for SwiftError {}
