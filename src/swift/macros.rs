//! `swift!`: declare Swift functions as Rust signatures.

/// Declares Swift functions by symbol, as Rust functions.
///
/// ```ignore
/// async_swift_rs::swift! {
///     /// `func greet(_ name: String, times: Int) async throws -> String` in module Greeter
///     #[symbol = "$s7Greeter5greet_5timesS2S_SitYaKF"]
///     pub async fn greet(name: &str, times: i64) throws -> String;
///
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
/// - `#[symbol = "…"]` looks the mangled name up at run time (a missing function gives
///   `CallError::Unavailable`); `#[linked = "…"]` binds it at link time (a missing function fails the build).
/// - Parameter and result types are the ones [`SwiftAsyncFn::call`](crate::swift::SwiftAsyncFn::call) accepts,
///   and must match the Swift signature: nothing checks them.
#[macro_export]
macro_rules! swift {
    () => {};

    ($(#[doc = $doc:literal])* #[symbol = $symbol:literal]
     $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $(#[doc = $doc])*
        $vis fn $name($($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftAsyncFn = $crate::swift::SwiftAsyncFn::new($symbol);
            let call = FUNCTION.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
        $crate::swift! { $($rest)* }
    };

    ($(#[doc = $doc:literal])* #[linked = $symbol:literal]
     $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $(#[doc = $doc])*
        $vis fn $name($($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = concat!($symbol, "Tu")]
                static POINTER: $crate::swift::AsyncFunctionPointer;
            }
            let call = unsafe { $crate::swift::SwiftAsyncFn::from_pointer(&POINTER) }.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
        $crate::swift! { $($rest)* }
    };

    ($(#[doc = $doc:literal])* #[symbol = $symbol:literal]
     $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $(#[doc = $doc])*
        $vis fn $name($($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::new($symbol);
            let call = FUNCTION.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
        $crate::swift! { $($rest)* }
    };

    ($(#[doc = $doc:literal])* #[linked = $symbol:literal]
     $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $(#[doc = $doc])*
        $vis fn $name($($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            extern "C" {
                #[link_name = $symbol]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let call = unsafe { $crate::swift::SwiftFn::from_address(entry) }.call(($($arg,)*));
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
        $crate::swift! { $($rest)* }
    };
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
