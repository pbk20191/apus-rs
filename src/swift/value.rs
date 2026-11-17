//! Swift types (metadata), protocol conformances (witness tables) and owned Swift values of any type.
//!
//! Generic Swift functions take their type arguments as trailing parameters: the metadata of each generic
//! parameter, then one witness table per conformance requirement, and they take and return values of generic
//! type by address. Large or resilient structs (every struct of an Apple framework) also travel by address.
//! [`SwiftValue`] is such a value: a box sized and destroyed through the type's value witness table.

use super::any::FromSwiftAny;
use super::rt;
use std::alloc::Layout;
use std::ffi::{c_void, CString};
use std::ptr::NonNull;
use std::sync::OnceLock;

use libc::{dlsym, RTLD_DEFAULT};

pub(crate) fn lookup(symbol: &str) -> usize {
    let name = CString::new(symbol).expect("symbol contains a NUL byte");
    unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) as usize }
}

/// A Swift type: its metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SwiftType(*const c_void);

unsafe impl Send for SwiftType {}
unsafe impl Sync for SwiftType {}

impl SwiftType {
    /// The Swift type a Rust type reads from: `i64` → `Int`, `Vec<String>` → `[String]`, …
    pub fn of<T: FromSwiftAny>() -> SwiftType {
        SwiftType(T::swift_metadata())
    }

    /// Resolves a type from its mangled name (no `$s` prefix), e.g. `"8StoreKit7ProductV"` or `"SaySSG"`.
    /// `None` if the type does not exist in this process.
    pub fn from_mangled_name(name: &str) -> Option<SwiftType> {
        let metadata = unsafe { rt::swift_getTypeByMangledNameInContext(name.as_ptr().cast(), name.len(), std::ptr::null(), std::ptr::null()) };
        (!metadata.is_null()).then_some(SwiftType(metadata))
    }

    /// # Safety
    /// `metadata` must be Swift type metadata.
    pub unsafe fn from_metadata(metadata: *const c_void) -> SwiftType {
        SwiftType(metadata)
    }

    pub fn as_ptr(self) -> *const c_void {
        self.0
    }

    /// The qualified name, e.g. `Swift.Array<Swift.String>`.
    pub fn name(self) -> String {
        unsafe { rt::type_name(self.0) }
    }

    /// The size of a value in bytes.
    pub fn size(self) -> usize {
        unsafe { rt::value_witnesses(self.0) }.size
    }

    pub(crate) fn stride(self) -> usize {
        unsafe { rt::value_witnesses(self.0) }.stride
    }

    fn layout(self) -> Layout {
        let witnesses = unsafe { rt::value_witnesses(self.0) };
        Layout::from_size_align(witnesses.size.max(1), (witnesses.flags as usize & 0xff) + 1).unwrap()
    }

    /// `Optional<Self>`.
    pub fn optional(self) -> SwiftType {
        SwiftType(unsafe { rt::optional_metadata(0, self.0) }.metadata)
    }

    /// The type a conformance binds to an associated type. `base` and `associated_type` are the protocol's
    /// requirement base descriptor (`…TL`) and the associated type's descriptor (`…Tl`) symbols.
    pub(crate) fn associated_type(self, conformance: WitnessTable, base: &str, associated_type: &str) -> Option<SwiftType> {
        let (base, associated_type) = (lookup(base), lookup(associated_type));
        if base == 0 || associated_type == 0 {
            return None;
        }
        let metadata = unsafe { rt::swift_getAssociatedTypeWitness(0, conformance.0, self.0, base as *const c_void, associated_type as *const c_void) }.metadata;
        (!metadata.is_null()).then_some(SwiftType(metadata))
    }

    /// The witness table of this type's conformance to `protocol`, if it conforms.
    pub fn conformance(self, protocol: &SwiftProtocol) -> Option<WitnessTable> {
        let descriptor = protocol.descriptor()?;
        let table = unsafe { rt::swift_conformsToProtocol(self.0, descriptor) };
        (!table.is_null()).then_some(WitnessTable(table))
    }
}

impl std::fmt::Debug for SwiftType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftType({})", self.name())
    }
}

/// A Swift protocol, named by its protocol descriptor symbol (`$sSlMp` is `Collection`).
pub struct SwiftProtocol {
    symbol: &'static str,
    descriptor: OnceLock<usize>,
}

impl SwiftProtocol {
    pub const fn new(descriptor_symbol: &'static str) -> SwiftProtocol {
        SwiftProtocol { symbol: descriptor_symbol, descriptor: OnceLock::new() }
    }

    /// The descriptor's symbol (`$sSHMp` for Hashable).
    pub fn descriptor_symbol(&self) -> &'static str {
        self.symbol
    }

    fn descriptor(&self) -> Option<*const c_void> {
        let address = *self.descriptor.get_or_init(|| lookup(self.symbol));
        (address != 0).then_some(address as *const c_void)
    }
}

/// A protocol witness table: the trailing argument a constrained generic function takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WitnessTable(pub(crate) *const c_void);

unsafe impl Send for WitnessTable {}
unsafe impl Sync for WitnessTable {}

/// Storage for a value of a Swift type that has not been initialized yet (an indirect result).
#[doc(hidden)]
pub struct UninitValue {
    pub(crate) ptr: NonNull<u8>,
    pub(crate) ty: SwiftType,
}

impl UninitValue {
    pub(crate) fn new(ty: SwiftType) -> UninitValue {
        let ptr = NonNull::new(unsafe { std::alloc::alloc(ty.layout()) }).expect("allocation failed");
        UninitValue { ptr, ty }
    }

    /// # Safety
    /// The storage must now hold an initialized value.
    pub(crate) unsafe fn assume_init(self) -> SwiftValue {
        let this = std::mem::ManuallyDrop::new(self);
        SwiftValue { ptr: this.ptr, ty: this.ty }
    }
}

impl Drop for UninitValue {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr.as_ptr(), self.ty.layout()) }
    }
}

/// An owned value of any Swift type.
pub struct SwiftValue {
    ptr: NonNull<u8>,
    ty: SwiftType,
}

// As for SwiftAny: copies and destruction go through atomic reference counting.
unsafe impl Send for SwiftValue {}

impl SwiftValue {
    /// Converts a Rust value into the corresponding Swift value.
    pub fn new<T: IntoSwift>(value: T) -> SwiftValue {
        let storage = UninitValue::new(T::swift_type());
        unsafe {
            value.write(storage.ptr.as_ptr().cast());
            storage.assume_init()
        }
    }

    pub fn ty(&self) -> SwiftType {
        self.ty
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.ptr.as_ptr().cast()
    }

    /// Copies the value out as `T` if it is exactly that Swift type.
    pub fn get<T: FromSwiftAny>(&self) -> Option<T> {
        (self.ty.0 == T::swift_metadata()).then(|| unsafe { T::from_value(self.as_ptr()) })
    }

    /// Gives up the value: the storage, no longer initialized (after something consumed the value in place).
    pub(crate) fn into_uninit(self) -> UninitValue {
        let this = std::mem::ManuallyDrop::new(self);
        UninitValue { ptr: this.ptr, ty: this.ty }
    }

    /// For a value of type `Optional<payload>`: the payload, or `None` for nil.
    pub(crate) fn into_optional_payload(self, payload: SwiftType) -> Option<SwiftValue> {
        unsafe {
            // Optional: case some = 0 (payload at offset 0), none = 1.
            if (rt::value_witnesses(self.ty.0).get_enum_tag.unwrap())(self.as_ptr(), self.ty.0) == 1 {
                return None;
            }
            let storage = UninitValue::new(payload);
            (rt::value_witnesses(payload.0).initialize_with_take.unwrap())(storage.ptr.as_ptr().cast(), self.as_ptr(), payload.0);
            drop(self.into_uninit()); // the payload was moved out
            Some(storage.assume_init())
        }
    }

    /// `String(describing:)`.
    pub fn description(&self) -> String {
        unsafe {
            let copy = self.clone();
            let mut text = rt::string_describing(copy.as_ptr(), copy.ty.0); // consumes the copy's value
            drop(UninitValue { ptr: copy.ptr, ty: copy.ty });
            std::mem::forget(copy);
            let out = rt::string_to_rust(&text);
            rt::destroy_string(&mut text);
            out
        }
    }
}

impl Clone for SwiftValue {
    fn clone(&self) -> Self {
        let storage = UninitValue::new(self.ty);
        unsafe {
            (rt::value_witnesses(self.ty.0).initialize_with_copy.unwrap())(storage.ptr.as_ptr().cast(), self.as_ptr(), self.ty.0);
            storage.assume_init()
        }
    }
}

impl Drop for SwiftValue {
    fn drop(&mut self) {
        unsafe {
            (rt::value_witnesses(self.ty.0).destroy.unwrap())(self.as_ptr(), self.ty.0);
            std::alloc::dealloc(self.ptr.as_ptr(), self.ty.layout());
        }
    }
}

impl std::fmt::Debug for SwiftValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftValue({}: {})", self.ty.name(), self.description())
    }
}

/// Rust values that can become Swift values: integers, `bool`, `f64`, strings, and `Vec`s of them.
pub trait IntoSwift {
    #[doc(hidden)]
    fn swift_type() -> SwiftType;

    /// Initializes `dest` (uninitialized storage of `swift_type()`) with the value.
    #[doc(hidden)]
    unsafe fn write(self, dest: *mut c_void);
}

macro_rules! plain_into {
    ($($t:ty),*) => {$(
        impl IntoSwift for $t {
            fn swift_type() -> SwiftType {
                SwiftType::of::<$t>()
            }
            unsafe fn write(self, dest: *mut c_void) {
                (dest as *mut $t).write(self)
            }
        }
    )*};
}
plain_into!(i64, i32, u64, f64);

impl IntoSwift for bool {
    fn swift_type() -> SwiftType {
        SwiftType::of::<bool>()
    }
    unsafe fn write(self, dest: *mut c_void) {
        (dest as *mut u8).write(self as u8)
    }
}

impl IntoSwift for &str {
    fn swift_type() -> SwiftType {
        SwiftType::of::<String>()
    }
    unsafe fn write(self, dest: *mut c_void) {
        (dest as *mut rt::SwiftString).write(rt::string_from_utf8(self))
    }
}

impl IntoSwift for String {
    fn swift_type() -> SwiftType {
        SwiftType::of::<String>()
    }
    unsafe fn write(self, dest: *mut c_void) {
        self.as_str().write(dest)
    }
}

/// `[T]`, built with the standard library's array-literal entry point.
impl<T: IntoSwift> IntoSwift for Vec<T> {
    fn swift_type() -> SwiftType {
        SwiftType(unsafe { rt::array_metadata(0, T::swift_type().0) }.metadata)
    }
    unsafe fn write(self, dest: *mut c_void) {
        let element = T::swift_type();
        let allocated = rt::allocate_uninitialized_array(self.len(), element.0);
        let stride = element.stride();
        for (i, item) in self.into_iter().enumerate() {
            item.write(allocated.elements.byte_add(i * stride));
        }
        (dest as *mut *mut c_void).write(allocated.array);
    }
}

/// An `any P` value opened: its value's address, its dynamic type and the witness table of `P`, as Swift has them
/// inside an existential. Made by [`SwiftValue::open`]; a receiver (`self: &Opened`) for `P`'s requirements, which
/// are called through their dispatch thunks.
pub struct Opened {
    value: *mut c_void,
    ty: SwiftType,
    witness: WitnessTable,
    /// The existential it was opened from (a copy), which owns the value; calls on it hold it too.
    container: std::sync::Arc<KeepAlive>,
}

/// A value a call only keeps alive (never touches), so sharing it across threads is fine.
pub(crate) struct KeepAlive(#[allow(dead_code)] SwiftValue);
unsafe impl Sync for KeepAlive {}

unsafe impl Send for Opened {}
unsafe impl Sync for Opened {}

impl Opened {
    /// The value's dynamic type.
    pub fn ty(&self) -> SwiftType {
        self.ty
    }

    pub(crate) fn value(&self) -> *mut c_void {
        self.value
    }

    pub(crate) fn witness(&self) -> WitnessTable {
        self.witness
    }

    pub(crate) fn keep_alive(&self) -> std::sync::Arc<KeepAlive> {
        self.container.clone()
    }
}

impl SwiftValue {
    /// Opens an `any P` value of one protocol (not class-bound): its value, dynamic type and `P`'s witness table.
    /// `None` for any other value. The container layout is the ABI's: a three-word buffer holding the value inline,
    /// or a box when the value does not fit, then the metadata, then the witness table.
    pub fn open(&self) -> Option<Opened> {
        let metadata = self.ty().as_ptr();
        unsafe {
            if *(metadata as *const usize) != rt::METADATA_KIND_EXISTENTIAL as usize {
                return None;
            }
            let flags = *(metadata.byte_add(std::mem::size_of::<usize>()) as *const u32);
            let opaque = flags & rt::EXISTENTIAL_ANY_CLASS_CONSTRAINT != 0;
            let special = flags & rt::EXISTENTIAL_SPECIAL_PROTOCOL_MASK != 0;
            if !opaque || special || flags & rt::EXISTENTIAL_WITNESS_TABLES_MASK != 1 {
                return None;
            }
            // A copy keeps the value alive: inline values are copied, a box is shared.
            let container = self.clone();
            let words = container.as_ptr() as *const usize;
            let ty = SwiftType::from_metadata(*words.add(3) as *const c_void);
            let witness = WitnessTable(*words.add(4) as *const c_void);
            let witnesses = rt::value_witnesses(ty.as_ptr());
            let value = if witnesses.flags & rt::VALUE_WITNESS_IS_NON_INLINE != 0 {
                let alignment = (witnesses.flags as usize & 0xff) + 1;
                let header = std::mem::size_of::<rt::HeapObject>().next_multiple_of(alignment);
                (*words as *mut u8).add(header).cast()
            } else {
                container.as_ptr()
            };
            Some(Opened { value, ty, witness, container: std::sync::Arc::new(KeepAlive(container)) })
        }
    }
}
