//! `swift!`: declare Swift functions as Rust signatures.

/// Declares Swift functions as Rust functions.
///
/// ```ignore
/// apus::swift! {
///     /// The symbol is computed from the Swift declaration.
///     #[swift = "Greeter.greet(_: String, times: Int) async throws -> String"]
///     pub async fn greet(name: &str, times: i64) throws -> String;
///
///     /// Or given as it appears in the binary.
///     #[linked = "$s7Greeter5countSiyF"]
///     fn count() -> i64;
/// }
///
/// let text = greet("Kim", 2).await?;     // a `SwiftCall<String>`: `.child()` and `timeout` apply
/// let n = count()?;                       // synchronous: `Result<i64, CallError>`
/// ```
///
/// - `async fn` declares a Swift `async` function and returns a [`SwiftCall`](crate::swift::SwiftCall);
///   a plain `fn` declares a synchronous one and returns `Result<_, CallError>`.
/// - `throws` after the parameters marks a Swift `throws` function. It must match the Swift declaration.
/// - Naming the function, one attribute each:
///   - `#[swift = "Module.function(_: Int) async -> Int"]`: the symbol is computed from the Swift declaration
///     (see [`mangle!`](crate::mangle)) and looked up at run time; a missing function gives
///     `CallError::Unavailable`.
///   - `#[swift_linked = "…"]`: the same, bound at link time; a missing function fails the build.
///   - `#[symbol = "$s…"]` / `#[linked = "$s…"]`: the mangled symbol itself, at run time / at link time.
/// - A leading `self: &SwiftObject` or `self: &SwiftValue` declares a method or property of that receiver:
///   `fn total(self: &SwiftValue) -> i64;` is called as `total(&cart)`.
/// - Parameter and result types are the ones [`SwiftAsyncFn::call`](crate::swift::SwiftAsyncFn::call) accepts,
///   and must match the Swift signature: nothing checks them. A `SwiftValue` (returned by address) or
///   `DirectValue` (returned in registers) result takes its Swift type from the `#[swift = …]` declaration.
///
/// A declaration that cannot be mangled is a compile error:
///
/// ```compile_fail
/// // `Mystery` is not a standard library type: it needs its kind, e.g. `Mystery(struct)`.
/// const SYMBOL: &str = apus::mangle!("Probe.f(_: Mystery) -> Int");
/// ```
///
/// ```
/// const SYMBOL: &str = apus::mangle!("Probe.f(_: Mystery(struct)) -> Int");
/// assert_eq!(SYMBOL, "$s5Probe1fySiAA7MysteryVF");
/// ```
#[macro_export]
macro_rules! swift {
    () => {};

    // The attribute says how the function is named and bound; `__swift_fn!` reads the signature after it.
    ($(#[doc = $doc:literal])* #[swift = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [runtime declared $spec] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift_linked = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [linked declared $spec] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[symbol = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [runtime literal $spec] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[linked = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [linked literal $spec] [$(#[doc = $doc])*] $($rest)* }
    };
}

/// One signature, then the rest of the `swift!` block. A leading `self: &Type` is the method's receiver.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_fn {
    ([$binding:ident $how:ident $spec:literal] [$(#[$doc:meta])*]
     $vis:vis async fn $name:ident (self : & $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how async [$(#[$doc])*] [$vis] $name [this : & $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal] [$(#[$doc:meta])*]
     $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how async [$(#[$doc])*] [$vis] $name [] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal] [$(#[$doc:meta])*]
     $vis:vis fn $name:ident (self : & $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how sync [$(#[$doc])*] [$vis] $name [this : & $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal] [$(#[$doc:meta])*]
     $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how sync [$(#[$doc])*] [$vis] $name [] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
}

/// One declared function: `binding` (runtime | linked), `how` the symbol is given (literal | declared).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_item {
    (runtime $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftAsyncFn = $crate::swift::SwiftAsyncFn::new($crate::__swift_symbol!($how $spec));
            let call = FUNCTION.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (linked $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_async_pointer!($how $spec)]
                static POINTER: $crate::swift::AsyncFunctionPointer;
            }
            let function = unsafe { $crate::swift::SwiftAsyncFn::from_pointer(&POINTER) };
            let call = function.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (runtime $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::new($crate::__swift_symbol!($how $spec));
            let call = FUNCTION.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind_sync($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
    (linked $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            extern "C" {
                #[link_name = $crate::__swift_symbol!($how $spec)]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let function = unsafe { $crate::swift::SwiftFn::from_address(entry) };
            let call = function.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind_sync($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
    (runtime $how:ident hop $target:ident [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::new($crate::__swift_symbol!($how $spec));
            let call = FUNCTION.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind_sync($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.hop($crate::swift::HopTo::$target)
        }
    };
    (linked $how:ident hop $target:ident [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_symbol!($how $spec)]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let function = unsafe { $crate::swift::SwiftFn::from_address(entry) };
            let call = function.call_declared(($($arg,)*), $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::SwiftReceiver::bind_sync($this, call);)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.hop($crate::swift::HopTo::$target)
        }
    };
}

/// The Swift result type's mangled name, known only when the function was declared in Swift syntax.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_result_type {
    (literal $symbol:literal) => { ::core::option::Option::None };
    (declared $declaration:literal) => { $crate::result_type!($declaration) };
}

/// The function's symbol: the literal itself, or computed from a Swift declaration.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_symbol {
    (literal $symbol:literal) => { $symbol };
    (declared $declaration:literal) => { $crate::mangle!($declaration) };
}

/// The symbol of the function's async function pointer (`…Tu`).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_async_pointer {
    (literal $symbol:literal) => { concat!($symbol, "Tu") };
    (declared $declaration:literal) => { $crate::mangle!($declaration, "Tu") };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __swift_ret {
    () => { () };
    ($ret:ty) => { $ret };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __swift_throws {
    (throws, $call:expr) => { $call.throws() };
    ($other:ident, $call:expr) => { compile_error!(concat!("expected `throws`, found `", stringify!($other), "`")) };
}
