//! `swift!`: declare Swift functions as Rust signatures.

/// Declares Swift functions as Rust functions.
///
/// ```ignore
/// async_swift_rs::swift! {
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
/// - Parameter and result types are the ones [`SwiftAsyncFn::call`](crate::swift::SwiftAsyncFn::call) accepts,
///   and must match the Swift signature: nothing checks them.
///
/// A declaration that cannot be mangled is a compile error:
///
/// ```compile_fail
/// // `Mystery` is not a standard library type: it needs its kind, e.g. `Mystery(struct)`.
/// const SYMBOL: &str = async_swift_rs::mangle!("Probe.f(_: Mystery) -> Int");
/// ```
///
/// ```
/// const SYMBOL: &str = async_swift_rs::mangle!("Probe.f(_: Mystery(struct)) -> Int");
/// assert_eq!(SYMBOL, "$s5Probe1fySiAA7MysteryVF");
/// ```
#[macro_export]
macro_rules! swift {
    () => {};

    ($(#[doc = $doc:literal])* #[symbol = $spec:literal] $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { runtime literal async [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift = $spec:literal] $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { runtime declared async [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[linked = $spec:literal] $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { linked literal async [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift_linked = $spec:literal] $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { linked declared async [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[symbol = $spec:literal] $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { runtime literal sync [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift = $spec:literal] $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { runtime declared sync [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[linked = $spec:literal] $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { linked literal sync [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift_linked = $spec:literal] $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_item! { linked declared sync [$(#[doc = $doc])*] [$vis] $name [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
}

/// One declared function: `binding` (runtime | linked), `how` the symbol is given (literal | declared).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_item {
    (runtime $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftAsyncFn = $crate::swift::SwiftAsyncFn::new($crate::__swift_symbol!($how $spec));
            let call = FUNCTION.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (linked $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_async_pointer!($how $spec)]
                static POINTER: $crate::swift::AsyncFunctionPointer;
            }
            let call = unsafe { $crate::swift::SwiftAsyncFn::from_pointer(&POINTER) }.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (runtime $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::new($crate::__swift_symbol!($how $spec));
            let call = FUNCTION.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
    (linked $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            extern "C" {
                #[link_name = $crate::__swift_symbol!($how $spec)]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let call = unsafe { $crate::swift::SwiftFn::from_address(entry) }.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
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
