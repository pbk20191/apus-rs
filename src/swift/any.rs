//! Owned Swift `Any` values and `any Error` boxes, read through the runtime's casting and value witnesses.

use super::rt;
use super::value::{SwiftType, SwiftValue, UninitValue};
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
        unsafe { (rt::value_witnesses(any).destroy.unwrap())(self.container.as_mut_ptr().cast(), any) }
    }
}

/// Types a `SwiftAny` can be read as: `()`, `bool`, `i64`, `i32`, `u64`, `f64`, `String`, and `Option`,
/// `Vec` and `HashMap` of them (Swift `Optional`, `Array`, `Dictionary`).
pub trait FromSwiftAny: Sized {
    /// Metadata of the Swift type this reads.
    #[doc(hidden)]
    fn swift_metadata() -> *const c_void;

    /// Copies a Rust value out of an initialized Swift value of that type, without consuming it.
    #[doc(hidden)]
    unsafe fn from_value(value: *const c_void) -> Self;

    #[doc(hidden)]
    fn from_any(any: &SwiftAny) -> Option<Self> {
        let metadata = Self::swift_metadata();
        let witnesses = unsafe { rt::value_witnesses(metadata) };
        let align = (witnesses.flags as usize & 0xff) + 1;
        let layout = std::alloc::Layout::from_size_align(witnesses.size.max(1), align.max(16)).unwrap();
        unsafe {
            let buf = std::alloc::alloc(layout).cast::<c_void>();
            let out = any.cast_into(metadata, buf).then(|| {
                let out = Self::from_value(buf);
                (witnesses.destroy.unwrap())(buf, metadata);
                out
            });
            std::alloc::dealloc(buf.cast(), layout);
            out
        }
    }
}

macro_rules! plain {
    ($t:ty, $sym:ident) => {
        impl FromSwiftAny for $t {
            fn swift_metadata() -> *const c_void {
                rt::metadata_of(unsafe { &rt::$sym })
            }
            unsafe fn from_value(value: *const c_void) -> Self {
                *(value as *const $t)
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
    unsafe fn from_value(value: *const c_void) -> Self {
        *(value as *const u8) & 1 == 1
    }
}

impl FromSwiftAny for () {
    fn swift_metadata() -> *const c_void {
        rt::empty_tuple_metadata()
    }
    unsafe fn from_value(_value: *const c_void) -> Self {}
}

impl FromSwiftAny for String {
    fn swift_metadata() -> *const c_void {
        rt::metadata_of(unsafe { &rt::STRING_METADATA })
    }
    unsafe fn from_value(value: *const c_void) -> Self {
        rt::string_to_rust(&*(value as *const rt::SwiftString))
    }
}

/// `Optional<T>`. Downcasting yields `Some(None)` for nil and `Some(Some(v))` for a value (wrapped or not).
impl<T: FromSwiftAny> FromSwiftAny for Option<T> {
    fn swift_metadata() -> *const c_void {
        unsafe { rt::optional_metadata(0, T::swift_metadata()) }.metadata
    }
    unsafe fn from_value(value: *const c_void) -> Self {
        let metadata = Self::swift_metadata();
        // Optional: case some = 0 (payload at offset 0), none = 1.
        let none = (rt::value_witnesses(metadata).get_enum_tag.unwrap())(value, metadata) == 1;
        (!none).then(|| T::from_value(value))
    }
}

/// `Array<T>`. Every supported element type is a value type, for which the array is always native
/// contiguous storage (never a bridged NSArray): HeapObject (16 bytes), `_SwiftArrayBodyStorage`
/// { count, capacityAndFlags }, then the elements (stdlib/public/SwiftShims/swift/shims/GlobalObjects.h).
impl<T: FromSwiftAny> FromSwiftAny for Vec<T> {
    fn swift_metadata() -> *const c_void {
        unsafe { rt::array_metadata(0, T::swift_metadata()) }.metadata
    }
    unsafe fn from_value(value: *const c_void) -> Self {
        let storage = *(value as *const *const u8);
        let count = *(storage.add(16) as *const isize) as usize;
        let witnesses = rt::value_witnesses(T::swift_metadata());
        let align_mask = witnesses.flags as usize & 0xff;
        let first = (32 + align_mask) & !align_mask;
        (0..count).map(|i| T::from_value(storage.add(first + i * witnesses.stride).cast())).collect()
    }
}

/// Swift types usable as `Dictionary` keys here.
pub trait SwiftHashable: FromSwiftAny + Eq + std::hash::Hash {
    /// The type's `Hashable` witness table.
    #[doc(hidden)]
    fn hashable_witness() -> *const c_void;
}

macro_rules! hashable {
    ($t:ty, $sym:ident) => {
        impl SwiftHashable for $t {
            fn hashable_witness() -> *const c_void {
                rt::metadata_of(unsafe { &rt::$sym })
            }
        }
    };
}
hashable!(String, STRING_HASHABLE);
hashable!(i64, INT_HASHABLE);
hashable!(i32, INT32_HASHABLE);
hashable!(u64, UINT64_HASHABLE);

/// `Dictionary<K, V>`. With value-type keys the dictionary is always native (`guaranteedNative`,
/// stdlib/public/core/DictionaryVariant.swift). `__RawDictionaryStorage`
/// (stdlib/public/core/DictionaryStorage.swift): HeapObject, _count (16), _capacity (24), _scale: Int8 (32),
/// _reservedScale, _extra, _age, _seed (40), _rawKeys (48), _rawValues (56), then the occupancy bitmap words.
impl<K: SwiftHashable, V: FromSwiftAny> FromSwiftAny for std::collections::HashMap<K, V> {
    fn swift_metadata() -> *const c_void {
        unsafe { rt::dictionary_metadata(0, K::swift_metadata(), V::swift_metadata(), K::hashable_witness()) }.metadata
    }
    unsafe fn from_value(value: *const c_void) -> Self {
        let storage = *(value as *const *const u8);
        let count = *(storage.add(16) as *const isize) as usize;
        let bucket_count = 1usize << *(storage.add(32) as *const i8);
        let keys = *(storage.add(48) as *const *const u8);
        let values = *(storage.add(56) as *const *const u8);
        let bitmap = storage.add(64) as *const u64;
        let key_stride = rt::value_witnesses(K::swift_metadata()).stride;
        let value_stride = rt::value_witnesses(V::swift_metadata()).stride;
        let mut out = Self::with_capacity(count);
        for bucket in 0..bucket_count {
            if *bitmap.add(bucket / 64) >> (bucket % 64) & 1 == 1 {
                out.insert(K::from_value(keys.add(bucket * key_stride).cast()), V::from_value(values.add(bucket * value_stride).cast()));
            }
        }
        out
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

    /// The box, still owned by this.
    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.boxed
    }

    /// Gives up ownership of the box (+1).
    pub(crate) fn into_raw(self) -> *mut c_void {
        std::mem::ManuallyDrop::new(self).boxed
    }

    /// Runs `f` on the opened error: its value's address, dynamic type and Error conformance. For an NSError the
    /// runtime stores the object in `scratch` and the value is the address of `scratch`, so it lives only as long
    /// as this call.
    fn with_opened<R>(&self, f: impl FnOnce(&rt::ErrorValueResult) -> R) -> R {
        let mut scratch = std::ptr::null_mut();
        let mut out = rt::ErrorValueResult { value: std::ptr::null(), type_: std::ptr::null(), error_conformance: std::ptr::null() };
        unsafe { rt::swift_getErrorValue(self.boxed, &mut scratch, &mut out) };
        f(&out)
    }

    /// The error's dynamic type.
    pub fn ty(&self) -> SwiftType {
        self.with_opened(|opened| unsafe { SwiftType::from_metadata(opened.type_) })
    }

    /// A copy of the error as a value of `ty`, if it is one (`swift_dynamicCast`); the error itself is kept.
    pub fn downcast(&self, ty: SwiftType) -> Option<SwiftValue> {
        let storage = UninitValue::new(ty);
        let mut boxed = self.boxed; // the `any Error` value: one reference, not consumed (no TakeOnSuccess)
        let any_error = SwiftType::any_error()?;
        let cast = unsafe {
            rt::swift_dynamicCast(storage.ptr.as_ptr().cast(), (&mut boxed as *mut *mut c_void).cast(), any_error.as_ptr(), ty.as_ptr(), 0)
        };
        cast.then(|| unsafe { storage.assume_init() })
    }

    /// [`downcast`](Self::downcast) to the Swift type of `T`, copied out as `T`.
    pub fn downcast_value<T: FromSwiftAny>(&self) -> Option<T> {
        self.downcast(SwiftType::of::<T>())?.get::<T>()
    }

    /// The error's domain, as `NSError.domain` gives it: the NSError's own for an NSError, else the type's name.
    pub fn domain(&self) -> String {
        self.with_opened(|opened| unsafe {
            let mut domain = rt::apus_error_domain(opened.value, opened.type_, opened.error_conformance);
            let out = rt::string_to_rust(&domain);
            rt::destroy_string(&mut domain);
            out
        })
    }

    /// The error's code, as `NSError.code` gives it.
    pub fn code(&self) -> i64 {
        self.with_opened(|opened| unsafe { rt::apus_error_code(opened.value, opened.type_, opened.error_conformance) as i64 })
    }

    /// The qualified Swift name of the error's dynamic type.
    pub fn type_name(&self) -> String {
        self.with_opened(|opened| unsafe { rt::type_name(opened.type_) })
    }

    /// `String(describing: error)`.
    pub fn description(&self) -> String {
        self.with_opened(|opened| unsafe {
            let witnesses = rt::value_witnesses(opened.type_);
            let align = (witnesses.flags as usize & 0xff) + 1;
            let layout = std::alloc::Layout::from_size_align(witnesses.size.max(1), align.max(16)).unwrap();
            let copy = std::alloc::alloc(layout).cast::<c_void>();
            (witnesses.initialize_with_copy.unwrap())(copy, opened.value as *mut c_void, opened.type_);
            let mut text = rt::string_describing(copy, opened.type_); // consumes the copy
            std::alloc::dealloc(copy.cast(), layout);
            let out = rt::string_to_rust(&text);
            rt::destroy_string(&mut text);
            out
        })
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
