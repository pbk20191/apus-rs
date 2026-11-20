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
/// - An initializer (`Module.Type(kind).init(…)`, returning the type unless written: `-> T?` when failable) or a
///   static member takes the type's metatype as `self`: a class's is its metadata, a struct's or enum's is nothing.
///   For a class, declare no `self` and apus passes the declared class (looked up by name; `CallError::Unavailable`
///   if missing), or `self: &SwiftType` to pass another (a subclass). A generic class's needs `self: &SwiftType`, the
///   specialized class's metadata. The wrong one is a compile error:
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.Crate(class)<T>.init(_: T)"]
///     fn new_crate(x: &apus::swift::SwiftValue) -> apus::swift::SwiftObject; // which `Crate<T>`? needs `self: &SwiftType`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Probe.Made(struct).init(x: Int)"]
///     fn made(self: &apus::swift::SwiftType, x: i64) -> apus::swift::SwiftValue; // a struct passes no `self`
/// }
/// ```
///
/// - `async` and `throws` must be the declaration's (a getter's too: `{ get async throws }`); a mismatch is a compile
///   error. A sync member that runs on its actor is `async fn` (apus hops there).
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.Meter(struct).slowDouble: Double { get async }"]
///     fn slow_double(self: &apus::swift::SwiftValue) -> f64; // async in Swift
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.Meter(struct).checked: Double { get throws }"]
///     fn checked(self: &apus::swift::SwiftValue) -> f64; // its error would go unread
/// }
/// ```
///
/// - A protocol's static member or initializer is called on a conforming type: `self: &OpenedType`
///   (`SwiftType::conforming`), as an instance member is on an `Opened`. A `Self` result is a `SwiftValue` of that
///   type. Under `where Self == Concrete` it is a member of that type instead.
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "static Res.Maker(protocol).label: String { get }"]
///     fn label(self: &apus::swift::SwiftType) -> String; // needs the conformance too
/// }
/// ```
///
/// - A value of a class-bound generic type (`T: SomeClass`, `T: AnyObject`, `T: P(AnyObject)` for a protocol declared
///   `: AnyObject`) is a class reference, not a `SwiftValue`: `&SwiftObject` (or `&Retained<T>`) as an argument,
///   `SwiftObject` as a result. A metatype (`T.Type`, a class's, `Any.Type`) is a `SwiftType`; a struct's or enum's
///   (passed as nothing) is refused. `any Error` is an error box: `&SwiftError` as an argument, `SwiftError` as a
///   result (`Option<…>` for `(any Error)?`).
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)"]
///     fn insert(self: &apus::swift::SwiftObject, model: &apus::swift::SwiftValue); // a model is a class reference
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Probe.makeError(code: Int) -> any Error"]
///     fn make_error(code: i64) -> apus::swift::SwiftValue; // one box reference: SwiftError
/// }
/// ```
///
/// - A generic function's parameters can be bound to concrete types after the declaration:
///   `#[swift = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(…) throws -> FetchResultsCollection(struct)<T>", T = "Todos.Todo(class)"]`.
///   The Rust signature lists the declaration's parameters only: the bound types' metadata and witness tables are
///   found by name when the call is made (`CallError::Unavailable` if a type is missing, `NotConforming` if it does not
///   conform), and the result's type is the one with `T` substituted. An argument whose type names `T` must hold that
///   type exactly (`CallError::InvalidArgument`). The function is still generic: a value of type `T` goes by address
///   whatever type is bound (`&SwiftValue`, `-> SwiftValue`); a class-bound one is a reference.
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.sumOf<T: AdditiveArithmetic>(_: [T]) -> T", T = "Swift.Int"]
///     fn sum_of(values: Vec<i64>) -> i64; // returned by address: `-> SwiftValue`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.firstOf<T>(_: T, _: Int) -> Int", T = "Swift.Int"]
///     fn first_of(x: i64, n: i64) -> i64; // `x` goes by address: `&SwiftValue`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.sumOf<T: AdditiveArithmetic>(_: [T]) -> T", U = "Swift.Int"]
///     fn sum_of(values: Vec<i64>) -> apus::swift::SwiftValue; // no parameter `U`
/// }
/// ```
///
/// ```
/// apus::swift! {
///     #[swift = "Res.sumOf<T: AdditiveArithmetic>(_: [T]) -> T", T = "Swift.Int"]
///     fn sum_of(values: Vec<i64>) -> apus::swift::SwiftValue;
/// }
/// ```
///
/// - A generic struct's or enum's instance member gets `Self`'s metadata from its receiver; its static members and
///   initializers take the type's generic arguments after their own. A standard library struct's or enum's instance
///   member is refused: Swift passes that `self` in registers.
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "static Res.Tray(struct)<T>.size(plus: Int) -> Int"]
///     fn size(n: i64) -> i64; // T's metadata goes after `n`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Swift.Array<Element>.count: Int { get }"]
///     fn count(self: &apus::swift::SwiftValue) -> i64; // an array is passed as its one reference
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.same<T: AnyObject>(_: T) -> T"]
///     fn same(x: &apus::swift::SwiftValue) -> apus::swift::SwiftObject; // a SwiftValue goes by address
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.same<T: AnyObject>(_: T) -> T"]
///     fn same(x: &apus::swift::SwiftObject) -> apus::swift::SwiftValue; // the result is a reference
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.sizeOf(_: Int.Type) -> Int"]
///     fn size_of(ty: apus::swift::SwiftType) -> i64; // `Int.Type` is thin: Swift passes nothing
/// }
/// ```
///
/// - An `@autoclosure` parameter takes a `SwiftClosure`, like any closure. A `@convention(c)` function is an
///   `extern "C" fn(…)`, a `@convention(block)` one a block2 `&Block<dyn Fn(…)>` (feature `block2`), each `Option`
///   when optional; as results, `extern "C" fn(…)` and `RcBlock<…>` (+1). Their parameter and result types are checked
///   against the declared ones when the call is made (`CallError::ClosureMismatch`).
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.callBlock(_: @convention(block) (Int) -> Int) -> Int"]
///     fn call_block(f: &apus::swift::SwiftClosure) -> i64; // a Swift closure is two words, a block one
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.cDoubler() -> @convention(c) (Int) -> Int"]
///     fn c_doubler() -> i64; // a function pointer comes back
/// }
/// ```
///
/// - A struct's setter (`name: T { set }`, `subscript(…) -> T { set }`) or `mutating` method changes `self` in place:
///   declare `self: &mut SwiftValue`; an `inout` parameter is a `&mut SwiftValue`. Both only in a synchronous
///   `fn` (an async call would outlive the borrow). A setter's new value comes first, then a subscript's indices.
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.Bag(struct).name: String { set }"]
///     fn set_name(self: &apus::swift::SwiftValue, name: &str); // a copy: needs `&mut SwiftValue`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.swapInts(_: inout Int, _: inout Int)"]
///     fn swap_ints(a: &apus::swift::SwiftValue, b: &apus::swift::SwiftValue); // `inout` needs `&mut`
/// }
/// ```
///
/// ```compile_fail
/// apus::swift! {
///     #[swift = "Res.bump(_: inout Int) async"]
///     async fn bump(a: &mut apus::swift::SwiftValue); // the borrow would outlive the call
/// }
/// ```
///
/// ```
/// apus::swift! {
///     #[swift = "Res.Bag(struct).name: String { set }"]
///     fn set_name(self: &mut apus::swift::SwiftValue, name: &str);
///     #[swift = "Res.swapInts(_: inout Int, _: inout Int)"]
///     fn swap_ints(a: &mut apus::swift::SwiftValue, b: &mut apus::swift::SwiftValue);
/// }
/// ```
///
/// ```
/// apus::swift! {
///     #[swift = "Probe.Minted(class).init(n: Int)"]
///     fn minted(n: i64) -> apus::swift::SwiftObject;
///     #[swift = "Probe.Minted(class).init(n: Int)"]
///     fn minted_as(self: &apus::swift::SwiftType, n: i64) -> apus::swift::SwiftObject;
///     #[swift = "Probe.Made(struct).init(x: Int)"]
///     fn made(x: i64) -> apus::swift::SwiftValue;
/// }
/// ```
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
///
/// [`swift_type!`](crate::swift_type) and [`swift_protocol!`](crate::swift_protocol) name a type and a protocol the
/// same way, for `SwiftType::from_mangled_name` and `SwiftProtocol::new`:
///
/// ```
/// assert_eq!(apus::swift_type!("SwiftData.FetchResultsCollection(struct)<Todos.Todo(class)>"), "9SwiftData22FetchResultsCollectionVy5Todos4TodoCG");
/// assert_eq!(apus::swift_protocol!("SwiftData.PersistentModel"), "$s9SwiftData15PersistentModelMp");
/// ```
///
/// ```compile_fail
/// const NAME: &str = apus::swift_type!("Todos.Todo"); // needs its kind: `Todos.Todo(class)`
/// ```
#[macro_export]
macro_rules! swift {
    () => {};

    // The attribute says how the function is named and bound; `__swift_fn!` reads the signature after it.
    ($(#[doc = $doc:literal])* #[swift = $spec:literal $(, $param:ident = $bound:literal)* $(,)?] $($rest:tt)*) => {
        $crate::__swift_fn! { [runtime declared $spec [$($param = $bound),*]] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[swift_linked = $spec:literal $(, $param:ident = $bound:literal)* $(,)?] $($rest:tt)*) => {
        $crate::__swift_fn! { [linked declared $spec [$($param = $bound),*]] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[symbol = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [runtime literal $spec []] [$(#[doc = $doc])*] $($rest)* }
    };
    ($(#[doc = $doc:literal])* #[linked = $spec:literal] $($rest:tt)*) => {
        $crate::__swift_fn! { [linked literal $spec []] [$(#[doc = $doc])*] $($rest)* }
    };
}

/// One signature, then the rest of the `swift!` block. A leading `self: &Type` is the method's receiver.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_fn {
    // `self: &mut SwiftValue`: a struct's setter or mutating method, on the Rust value itself (sync only).
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis async fn $name:ident (self : &mut $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how async $bind [$(#[$doc])*] [$vis] $name [this : &mut $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis fn $name:ident (self : &mut $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how sync $bind [$(#[$doc])*] [$vis] $name [this : &mut $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis async fn $name:ident (self : & $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how async $bind [$(#[$doc])*] [$vis] $name [this : & $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis async fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how async $bind [$(#[$doc])*] [$vis] $name [] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis fn $name:ident (self : & $receiver:ty $(, $arg:ident : $ty:ty)* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how sync $bind [$(#[$doc])*] [$vis] $name [this : & $receiver] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
    ([$binding:ident $how:ident $spec:literal $bind:tt] [$(#[$doc:meta])*]
     $vis:vis fn $name:ident ($($arg:ident : $ty:ty),* $(,)?) $($throws:ident)? $(-> $ret:ty)? ; $($rest:tt)*) => {
        $crate::__swift_route! { $crate::__swift_item ; $binding $how sync $bind [$(#[$doc])*] [$vis] $name [] [$($arg : $ty),*] [$($throws)?] [$($ret)?] $spec }
        $crate::swift! { $($rest)* }
    };
}

/// One declared function: `binding` (runtime | linked), `how` the symbol is given (literal | declared).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_item {
    (runtime $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftAsyncFn = $crate::swift::SwiftAsyncFn::__dispatched($crate::__swift_symbol!($how $spec), $crate::__swift_thunk!($how $spec));
            let call = $crate::__swift_call!(FUNCTION [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (runtime $how:ident nonsending [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftAsyncFn = $crate::swift::SwiftAsyncFn::__dispatched($crate::__swift_symbol!($how $spec), $crate::__swift_thunk!($how $spec));
            // The caller's isolation is the hidden first argument; closures are checked like any call's.
            let isolation = $crate::swift::Isolation::current();
            let call = $crate::__swift_call!(FUNCTION [isolation $(, $arg)*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (linked $how:ident async [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_async_pointer!($how $spec)]
                static POINTER: $crate::swift::AsyncFunctionPointer;
            }
            let function = unsafe { $crate::swift::SwiftAsyncFn::from_pointer(&POINTER) };
            let call = $crate::__swift_call!(function [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (linked $how:ident nonsending [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_async_pointer!($how $spec)]
                static POINTER: $crate::swift::AsyncFunctionPointer;
            }
            let function = unsafe { $crate::swift::SwiftAsyncFn::from_pointer(&POINTER) };
            // The caller's isolation is the hidden first argument; closures are checked like any call's.
            let isolation = $crate::swift::Isolation::current();
            let call = $crate::__swift_call!(function [isolation $(, $arg)*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call
        }
    };
    (runtime $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::__dispatched($crate::__swift_symbol!($how $spec), $crate::__swift_thunk!($how $spec));
            let call = $crate::__swift_call!(FUNCTION [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind_sync($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
    (linked $how:ident sync [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> ::core::result::Result<$crate::__swift_ret!($($ret)?), $crate::swift::CallError> {
            extern "C" {
                #[link_name = $crate::__swift_symbol!($how $spec)]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let function = unsafe { $crate::swift::SwiftFn::from_address(entry) };
            let call = $crate::__swift_call!(function [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind_sync($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.invoke()
        }
    };
    (runtime $how:ident hop $target:ident [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            static FUNCTION: $crate::swift::SwiftFn = $crate::swift::SwiftFn::__dispatched($crate::__swift_symbol!($how $spec), $crate::__swift_thunk!($how $spec));
            let call = $crate::__swift_call!(FUNCTION [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind_sync($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.hop($crate::swift::HopTo::$target)
        }
    };
    (linked $how:ident hop $target:ident [$(#[$doc:meta])*] [$vis:vis] $name:ident [$($this:ident : $receiver:ty)?] [$($arg:ident : $ty:ty),*] [$($throws:ident)?] [$($ret:ty)?] [$($generic:tt)*] $spec:literal) => {
        $(#[$doc])*
        $vis fn $name($($this: $receiver,)? $($arg: $ty),*) -> $crate::swift::SwiftCall<$crate::__swift_ret!($($ret)?)> {
            extern "C" {
                #[link_name = $crate::__swift_symbol!($how $spec)]
                static ENTRY: u8;
            }
            let entry = unsafe { &ENTRY as *const u8 as *const ::core::ffi::c_void };
            let function = unsafe { $crate::swift::SwiftFn::from_address(entry) };
            let call = $crate::__swift_call!(function [$($arg),*] [$($generic)*] $crate::__swift_result_type!($how $spec));
            $(let call = $crate::swift::__bind_sync($this, call, $crate::__swift_consumes_self!($how $spec));)?
            $(let call = $crate::__swift_throws!($throws, call);)?
            call.hop($crate::swift::HopTo::$target)
        }
    };
}

/// The argument tuple: the formal arguments, then the generic arguments `swift!` fills itself (`auto`: the formal
/// arguments giving each generic parameter's type, the (argument, protocol descriptor) pairs of the witness tables in
/// canonical order, and the argument giving a typed-throws error type).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_args {
    // `[consumed…]`: the arguments the callee consumes (an initializer's, a setter's new value), wrapped last.
    ([$($arg:ident),*] [$($o:ident)*] []) => {{
        $(let $o = $crate::swift::Consumed($o);)*
        ($($arg,)*)
    }};
    // Closure arguments, checked against their declared function types (`closures [(argument "shape")…]`).
    ([$($arg:ident),*] [$($o:ident)*] [closures [$(($c:ident $shape:literal))*]]) => {{
        $(let $c = $crate::swift::DeclaredClosure($c, $shape);)*
        $(let $o = $crate::swift::Consumed($o);)*
        ($($arg,)*)
    }};
    // `extra generic`: one more argument after the formal ones, the generic arguments already built.
    ([$($arg:ident),*] [$($o:ident)*] [extra $g:ident $(closures [$(($c:ident $shape:literal))*])?]) => {{
        $($(let $c = $crate::swift::DeclaredClosure($c, $shape);)*)?
        $(let $o = $crate::swift::Consumed($o);)*
        ($($arg,)* $g,)
    }};
    ([$($arg:ident),*] [$($o:ident)*] [auto [$(($mf:ident $m:ident))*] [$(($wf:ident $w:ident $p:literal $pl:ident))*] [$($sf:ident $slot:ident)?] [$($rf:ident $result:ident)?] $(closures [$(($c:ident $shape:literal))*])?]) => {{
        let generic = $crate::swift::__generic_args(
            &[$($crate::swift::$mf(&$m)),*],
            &[$(($crate::swift::$wf(&$w), {
                static PROTOCOL: $crate::swift::SwiftProtocol = $crate::swift::SwiftProtocol::new($p);
                &PROTOCOL
            }, $crate::swift::ProtocolLayout::$pl)),*],
            {
                #[allow(unused_mut)]
                let mut slot = ::core::option::Option::None;
                $(slot = ::core::option::Option::Some($crate::swift::$sf(&$slot));)?
                slot
            },
        );
        $($(let $c = $crate::swift::DeclaredClosure($c, $shape);)*)?
        $(let $o = $crate::swift::Consumed($o);)*
        ($($arg,)* generic,)
    }};
}

/// Prepares the call: with the declared result type, or, when the result is exactly a generic parameter filled
/// automatically, with that parameter's type taken from its argument.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_call {
    // `selftype this [formal…] [rest…]`: a generic struct's or enum's instance member, which takes the receiver's type
    // metadata right after its formal arguments.
    // `[n]`: the result is the type's parameter n, whose metadata is among the receiver type's generic arguments.
    ($function:ident [$($arg:ident),*] [selftype $this:ident [$($f:ident),*] [$($g:ident),*] [$($n:literal)?] $($generic:tt)*] $declared:expr) => {{
        let __apus_self_type = $crate::swift::__self_type(&$this);
        let declared = $declared;
        $(let declared = declared.of_type(__apus_self_type.generic_argument($n));)?
        $crate::__swift_call!($function [$($f,)* __apus_self_type $(, $g)*] [$($generic)*] declared)
    }};
    // `classself "name"`: a class's initializer or static member declared without `self`: the class's metadata,
    // found by its mangled name, is `self`.
    ($function:ident [$($arg:ident),*] [classself $class:literal $($generic:tt)*] $declared:expr) => {{
        static CLASS: ::std::sync::OnceLock<::core::option::Option<$crate::swift::SwiftType>> = ::std::sync::OnceLock::new();
        let call = $crate::__swift_call!($function [$($arg),*] [$($generic)*] $declared);
        $crate::swift::ClassSelf::class_self(call, $crate::swift::__declared_class(&CLASS, $class))
    }};
    // `wrapped [(argument Wrapper)…]`: arguments whose Rust type the declaration fixes (`ClassBound`, `Metatype`,
    // `Existential`, `ErrorArg`, `CFunctionArg`, `BlockArg`).
    // A C function's or block's wrapper also takes the declared shape.
    ($function:ident [$($arg:ident),*] [wrapped [$(($wa:ident $wk:ident $($ws:literal)?))*] $($generic:tt)*] $declared:expr) => {{
        $(let $wa = $crate::swift::$wk($wa $(, $ws)?);)*
        $crate::__swift_call!($function [$($arg),*] [$($generic)*] $declared)
    }};
    ($function:ident [$($arg:ident),*] [consumed [$($o:ident)*] $($generic:tt)*] $declared:expr) => {
        $crate::__swift_call!(@ $function [$($arg),*] [$($o)*] [$($generic)*] $declared)
    };
    ($function:ident [$($arg:ident),*] [$($generic:tt)*] $declared:expr) => {
        $crate::__swift_call!(@ $function [$($arg),*] [] [$($generic)*] $declared)
    };
    // `concrete [type…] [(parameter protocol layout)…] [result?]`: generic parameters bound to the types named.
    // `[(argument function "type")…]`: arguments whose type names a bound parameter, checked against that type.
    (@ $function:ident [$($arg:ident),*] $o:tt [concrete [$($t:literal)*] [$(($wp:literal $p:literal $pl:ident))*] [$($result:literal)?] [$(($ca:ident $cf:ident $ct:literal))*] $($closures:tt)*] $declared:expr) => {{
        static TYPES: [(&str, ::std::sync::OnceLock<::core::option::Option<$crate::swift::SwiftType>>); [$($t),*].len()] =
            [$(($t, ::std::sync::OnceLock::new())),*];
        static CHECKED: [(&str, ::std::sync::OnceLock<::core::option::Option<$crate::swift::SwiftType>>); <[&str]>::len(&[$($ct),*])] =
            [$(($ct, ::std::sync::OnceLock::new())),*];
        let generic = $crate::swift::__concrete_args(
            &TYPES,
            &[$(($wp, {
                static PROTOCOL: $crate::swift::SwiftProtocol = $crate::swift::SwiftProtocol::new($p);
                &PROTOCOL
            }, $crate::swift::ProtocolLayout::$pl)),*],
            &CHECKED,
            &[$($crate::swift::$cf(&$ca)),*],
        );
        #[allow(unused_mut)]
        let mut declared = $declared;
        $(declared.name = ::core::option::Option::Some($result);)?
        $function.call_declared($crate::__swift_args!([$($arg),*] $o [extra generic $($closures)*]), declared)
    }};
    (@ $function:ident [$($arg:ident),*] $o:tt [auto $m:tt $w:tt $slot:tt [$rf:ident $result:ident] $($closures:tt)*] $declared:expr) => {
        $function.call_returning(
            $crate::swift::$rf(&$result),
            $crate::__swift_args!([$($arg),*] $o [auto $m $w $slot [$rf $result] $($closures)*]),
        )
    };
    (@ $function:ident [$($arg:ident),*] $o:tt [$($generic:tt)*] $declared:expr) => {
        $function.call_declared($crate::__swift_args!([$($arg),*] $o [$($generic)*]), $declared)
    };
}

/// The Swift result type's mangled name, known only when the function was declared in Swift syntax.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_result_type {
    (literal $symbol:literal) => { $crate::swift::DeclaredResult::UNKNOWN };
    (declared $declaration:literal) => {{
        static OPAQUE: $crate::swift::OpaqueResult = $crate::swift::OpaqueResult::new($crate::__opaque_descriptor!($declaration));
        $crate::swift::DeclaredResult {
            name: $crate::result_type!($declaration),
            opaque: ::core::option::Option::Some(&OPAQUE),
            receiver_self: $crate::__returns_self!($declaration),
            ty: ::core::option::Option::None,
        }
    }};
}

/// Whether the declaration consumes `self` (`consuming func`); unknown for a bare symbol.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_consumes_self {
    (literal $symbol:literal) => { false };
    (declared $declaration:literal) => { $crate::__consumes_self!($declaration) };
}

/// The function's symbol: the literal itself, or computed from a Swift declaration.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_symbol {
    (literal $symbol:literal) => { $symbol };
    (declared $declaration:literal) => { $crate::__call_symbol!($declaration) };
}

/// A class member's dispatch thunk (`…Tj`), tried before its own symbol; empty for anything else.
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_thunk {
    (literal $symbol:literal) => { "" };
    (declared $declaration:literal) => { $crate::__dispatch_thunk!($declaration) };
}

/// The symbol of the function's async function pointer (`…Tu`).
#[doc(hidden)]
#[macro_export]
macro_rules! __swift_async_pointer {
    (literal $symbol:literal) => { concat!($symbol, "Tu") };
    (declared $declaration:literal) => { $crate::__call_symbol!($declaration, "Tu") };
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
