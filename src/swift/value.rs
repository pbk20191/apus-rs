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

    /// `any Error`, resolved once.
    pub(crate) fn any_error() -> Option<SwiftType> {
        static ANY_ERROR: OnceLock<Option<SwiftType>> = OnceLock::new();
        *ANY_ERROR.get_or_init(|| SwiftType::from_mangled_name("s5Error_p"))
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

    /// The metadata of a generic struct's or enum's argument `index`: TargetStructMetadata's and TargetEnumMetadata's
    /// generic arguments follow their kind and description words (`getGenericArgumentOffset`,
    /// include/swift/ABI/Metadata.h), the parameters' metadata first.
    #[doc(hidden)]
    pub fn generic_argument(self, index: usize) -> SwiftType {
        let kind = unsafe { *(self.0 as *const usize) } as u32;
        assert!(
            matches!(kind, rt::METADATA_KIND_STRUCT | rt::METADATA_KIND_ENUM | rt::METADATA_KIND_OPTIONAL),
            "{} is not a generic struct or enum",
            self.name()
        );
        unsafe { SwiftType(*(self.0 as *const *const c_void).add(2 + index)) }
    }

    /// Whether this is a class: a Swift class, an Objective-C class or a Core Foundation type
    /// (include/swift/ABI/MetadataKind.def; a Swift class's kind word is its isa pointer).
    pub(crate) fn is_class(self) -> bool {
        let kind = unsafe { *(self.0 as *const usize) };
        kind == rt::METADATA_KIND_CLASS as usize
            || kind > rt::METADATA_KIND_LAST_ENUMERATED as usize
            || matches!(kind as u32, rt::METADATA_KIND_OBJC_CLASS_WRAPPER | rt::METADATA_KIND_FOREIGN_CLASS)
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

    /// The type with its conformance to `protocol`, if it conforms: what a protocol's static members and
    /// initializers are called on.
    pub fn conforming(self, protocol: &SwiftProtocol) -> Option<OpenedType> {
        Some(OpenedType { ty: self, witness: self.conformance(protocol)? })
    }
}

/// A type with its conformance to a protocol: the receiver of the protocol's static members and initializers
/// (`self: &OpenedType`), as [`Opened`] is of its instance members. From [`SwiftType::conforming`].
#[derive(Clone, Copy, Debug)]
pub struct OpenedType {
    ty: SwiftType,
    witness: WitnessTable,
}

impl OpenedType {
    pub fn ty(&self) -> SwiftType {
        self.ty
    }

    pub(crate) fn witness(&self) -> WitnessTable {
        self.witness
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

    /// Whether the descriptor is in the process (a protocol of a framework the OS lacks is not).
    pub fn is_available(&self) -> bool {
        self.descriptor().is_some()
    }

    /// Whether the protocol is class-bound (`protocol P: AnyObject`), from its descriptor; `None` if it is not in the
    /// process. ContextDescriptorFlags' kind-specific flags (the high 16 bits) of a protocol are
    /// ProtocolContextDescriptorFlags, whose bit 0 is the ProtocolClassConstraint: 0 for `Class`
    /// (include/swift/ABI/MetadataValues.h).
    pub(crate) fn is_class_bound(&self) -> Option<bool> {
        let flags = unsafe { *(self.descriptor()? as *const u32) };
        Some((flags >> 16) & 1 == 0)
    }

    fn descriptor(&self) -> Option<*const c_void> {
        let address = *self.descriptor.get_or_init(|| lookup(self.symbol));
        (address != 0).then_some(address as *const c_void)
    }
}

/// A declaration's opaque result type (`some P`), named by its opaque type descriptor (`…QOMQ`, looked up once).
#[doc(hidden)]
pub struct OpaqueResult {
    symbol: Option<&'static str>,
    descriptor: OnceLock<usize>,
}

impl OpaqueResult {
    /// `None`: the declaration has no `some P` result.
    pub const fn new(descriptor_symbol: Option<&'static str>) -> OpaqueResult {
        OpaqueResult { symbol: descriptor_symbol, descriptor: OnceLock::new() }
    }

    pub(crate) fn is_opaque(&self) -> bool {
        self.symbol.is_some()
    }

    /// The actual type, given the generic arguments of the declaration's signature (metadata, then witness tables:
    /// what the call passes). `swift_getOpaqueTypeMetadata2`, index 0: the one `some P` of the result.
    pub(crate) fn metadata(&self, arguments: &[*const c_void]) -> Result<SwiftType, super::CallError> {
        let symbol = self.symbol.expect("an opaque result");
        let descriptor = *self.descriptor.get_or_init(|| lookup(symbol));
        if descriptor == 0 {
            return Err(super::CallError::Unavailable(symbol));
        }
        let response = unsafe { rt::swift_getOpaqueTypeMetadata2(0, arguments.as_ptr(), descriptor as *const c_void, 0) };
        Ok(SwiftType(response.metadata))
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

    /// The class instance this value is, when its type is a class (a generic result such as an array's element):
    /// a class-typed value is one reference (TypeLayout.rst), which the object takes over. Any other value back.
    pub fn into_object(self) -> Result<super::SwiftObject, SwiftValue> {
        if !self.ty.is_class() {
            return Err(self);
        }
        let object = unsafe { *(self.as_ptr() as *const *mut c_void) };
        drop(self.into_uninit()); // the reference moved to the object
        Ok(unsafe { super::SwiftObject::from_raw(object) })
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
/// are called through their dispatch thunks. Like a `&SwiftValue` receiver, each call gets its own copy of the value:
/// a `mutating` requirement changes that copy, never this value or the one it was opened from.
pub struct Opened {
    value: *mut c_void,
    ty: SwiftType,
    witness: WitnessTable,
    /// The existential it was opened from (a copy), which owns the value (a box may be shared with the original).
    #[allow(dead_code)]
    container: SwiftValue,
}

// Calls only copy the value (Swift copies with atomic reference counting), so they may run on several threads.
unsafe impl Send for Opened {}
unsafe impl Sync for Opened {}

impl Opened {
    /// The value's dynamic type.
    pub fn ty(&self) -> SwiftType {
        self.ty
    }

    pub(crate) fn witness(&self) -> WitnessTable {
        self.witness
    }

    /// A copy of the value, of its dynamic type: what a call works on.
    pub(crate) fn copy(&self) -> SwiftValue {
        let storage = UninitValue::new(self.ty);
        unsafe {
            (rt::value_witnesses(self.ty.0).initialize_with_copy.unwrap())(storage.ptr.as_ptr().cast(), self.value, self.ty.0);
            storage.assume_init()
        }
    }
}

impl SwiftValue {
    /// The value as a `P` its type conforms to (`swift_conformsToProtocol`), to call `P`'s requirements and extension
    /// methods on it: a `some P` result, for one. `None` if the type does not conform.
    pub fn conforming(&self, protocol: &SwiftProtocol) -> Option<Opened> {
        let witness = self.ty.conformance(protocol)?;
        let container = self.clone();
        Some(Opened { value: container.as_ptr(), ty: self.ty, witness, container })
    }

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
            Some(Opened { value, ty, witness, container })
        }
    }
}
