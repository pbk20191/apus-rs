# apus

Rust ↔ Swift at the runtime ABI (repository `apus-rs`; *Apus* is the swifts' genus). Formerly `async-swift-rs`: the
dated design documents under `docs/superpowers/` keep that name. Other Swift interop crates and how this one differs:
[docs/prior-art.md](docs/prior-art.md).

Run Rust futures as genuine **Swift Concurrency Tasks** on macOS, or on **libdispatch**.

With the `swift` backend every spawned future *is* a Swift Task, created through the Swift runtime's own ABI
(no Swift source in this crate). Swift code sees it as one: `Task.isCancelled`, `Task.currentPriority` and
priority escalation, `@TaskLocal` copying, MainActor isolation, Instruments' Swift Tasks view. Rust code can
await Swift `async` closures **inside the same Task**.

```toml
[dependencies]
apus = { path = "…" }                                          # swift backend (default)
apus = { path = "…", default-features = false, features = ["dispatch"] }
```

| Feature | Backend |
|---|---|
| `swift` (default) | `apus::swift`: tasks are Swift Tasks on the cooperative pool / MainActor |
| `dispatch` | `apus::dispatch`: `async-task` runnables on GCD queues via [`dispatch2`](https://crates.io/crates/dispatch2) (QoS, main queue, serial queues) |
| `stream` | `impl futures_core::Stream for SwiftStream` |
| `macos15` | raises the deployment floor from macOS 14 to 15: macOS 15 runtime functions become required instead of looked up at run time |
| `xlto` | compiles apus's C side as LLVM bitcode for cross-language LTO (see below) |
| `objc2` | [`objc2`](https://crates.io/crates/objc2) objects (`Retained<T>`) as Swift class references (see below) |
| `block2` | [`block2`](https://crates.io/crates/block2) blocks as Swift `@convention(block)` functions (implies `objc2`) |
| `test-probe` | test-only Swift probes (needs `swiftc`) |

## API

Both backends:

```rust
let h = spawn(async { 1 });                              // like Swift `Task {}`
let h = spawn_with(SpawnOptions { priority: Priority::Utility, ..Default::default() }, fut);
let h = spawn_on_main(fut);                              // MainActor / main queue
let h = spawn_detached(fut);                             // like `Task.detached`
sleep(Duration::from_millis(10)).await;
h.await -> Result<T, JoinError>                          // JoinError::{Aborted, Panicked}
h.abort();                                               // drops the future at its next poll
drop(h);                                                 // detaches; the task keeps running
let v = run_main(async { … });                           // on the main thread: runs a main task, drives the run loop
```

Swift backend only: `h.cancel()` (cooperative, = Swift `task.cancel()`), `h.escalate(p)`, `cancelled().await`,
`is_cancelled()`, `priority()`, `current_task()`, `SwiftAsyncFn`, `SwiftFn`, `SwiftCall`, `SwiftAsync`, `timeout()`,
`SwiftType`, `SwiftProtocol`, `SwiftValue`, `SwiftAny`, `SwiftObject`, `SwiftError`.
Dispatch backend only: `SerialQueue`, `block_on` (not on the main thread).

### Inheritance

| | priority | task-locals | main / MainActor |
|---|---|---|---|
| `spawn` | inherited (`Unspecified`) | copied | stays on main if spawned from a main task |
| `spawn_detached` | `Default` | not copied | no |

`JoinHandle` resolves when the **Swift Task** completes, i.e. after any child Tasks, like Swift's `Task.value`.

## The main run loop

Main tasks run when the main queue is drained by `CFRunLoopRun`/`NSApplication.run` (or `run_main`, which drives
the main run loop through `objc2-core-foundation` ; the task handle is polled inside a `CFRunLoopSource`, which stops the run loop when the task is done, and a
`CFRunLoopObserver` tracks nesting so a nested run loop in progress is never cut short).
**Do not use `dispatch_main()`**: it drains the main queue on a worker thread, so MainActor code runs off the
main thread.

## Calling Swift async functions directly

No Swift code is needed for an existing Swift async function: name its symbol, pass Rust values, await.

```rust
use apus::swift::SwiftAsyncFn;

// `func greet(_ name: String, times: Int, sleepMs: Int) async throws -> String` in module Probe.
// The mangled name comes from `nm -gU <binary> | grep greet` (drop the leading underscore).
static GREET: SwiftAsyncFn = SwiftAsyncFn::new("$s5Probe5greet_5times7sleepMsS2S_S2itYaKF");

let text: String = GREET.call(("Kim", 2i64, 10i64)).throws().await?;
let total: i64 = BUMP.call((5i64,)).on(&counter).await?;          // a method: `counter` is `self`
```

- The call runs **in the calling Swift Task** (same cancellation, priority, task-locals); see the concurrency
  rule below for `.child()` and `timeout`.
- The symbol is resolved with `dlsym` on first use. A function missing on the running OS gives
  `CallError::Unavailable` (check with `is_available()`), not a launch failure.
- Arguments: integers, `bool`, `f64`, `&str`/`String`, `Vec<T>`, `&SwiftObject`, `&SwiftValue`, `SwiftType`,
  `WitnessTable`. Results: `()`, integers, `bool`, `f64`,
  `String`, `Option<i64>`, `Option<String>`, `Vec<T>`, `HashMap<K, V>`, `SwiftObject`, `SwiftAny`.
- `.throws()` must be added exactly when the Swift function is `throws`.
- **The Rust types must match the Swift signature; nothing can check them.** A mismatch is undefined behavior.
  Arguments must fit the argument registers (8 integer words on arm64, 6 on x86_64; a `String` takes two).

### Declaring functions with `swift!`

Write the Swift declaration; the symbol is computed at compile time.

```rust
apus::swift! {
    #[swift = "Greeter.greet(_: String, times: Int) async throws -> String"]
    pub async fn greet(name: &str, times: i64) throws -> String;

    #[swift_linked = "Greeter.Counter(class).total: Int { get }"]     // bound at link time
    fn total() -> i64;
}

let text = greet("Kim", 2).await?;          // a SwiftCall<String>: `.child()` and `timeout` apply
```

- `async fn` is a Swift `async` function, a plain `fn` a synchronous one (it returns `Result<_, CallError>`);
  `throws` after the parameters marks a throwing function.
- `#[swift = "…"]` resolves the function at run time (missing → `CallError::Unavailable`), `#[swift_linked = "…"]`
  at link time. A member of an SDK module (`SwiftData`, `StoreKit`, the standard library, …) whose symbol the SDK's
  module does not export (its `.tbd` text stub) is a compile error: a wrong name, label, type or kind shows when it
  is written, not when it is called. Modules not in the SDK are not checked; `APUS_NO_SDK_CHECK=1` skips the check. `#[symbol = "$s…"]` / `#[linked = "$s…"]` take a mangled symbol as is.
- **Methods and properties**: a leading `self: &SwiftObject` (a class instance) or `self: &SwiftValue` (a struct
  passed by address) is the receiver. With a declaration (`#[swift]`, `#[swift_linked]`) its type is checked when
  the call is made, before Swift runs (`CallError::InvalidArgument` if it is another's): an object must be an
  instance of the class or a subclass, a value of exactly the struct or enum, or for a generic one of the same type
  whatever its generic arguments. A generic class's and a protocol's members are not checked. A class's own members dispatch to an override as Swift calls
  them (`#[swift]` and `#[swift_linked]`): through their dispatch thunks (`…Tj`) where the module has them (a
  library-evolution class, every Apple framework's, exports a non-final member only that way), else through the
  class's vtable, found by `swift_lookUpClassMethod` from the member's method descriptor (`…Tq`) for the receiver's
  class (which must be the member's class or a subclass, else `InvalidArgument`). A final member has neither and is
  called by its own symbol. `#[swift_linked]` finds the thunk or method descriptor with `dlsym`: a static archive
  whose symbols the program does not export dispatches to the member's own symbol. A standard library struct's or enum's
  instance member (`Array`'s, `String`'s) is refused: the type is frozen and Swift passes `self` in registers.
- **Opaque results**: `-> SwiftValue` (returned by address: large and framework structs) and `-> DirectValue`
  (returned in registers: an array or dictionary whose elements stay opaque) take their Swift type from the
  declaration.

```rust
apus::swift! {
    #[swift = "static StoreKit.Product(struct).products<I: Collection>(for: I) async throws -> [Product(struct)] where I.Element == String"]
    async fn products(ids: &SwiftValue, ids_type: SwiftType, collection: WitnessTable) throws -> DirectValue;

    #[swift = "StoreKit.Product(struct).displayName: String { get }"]
    fn display_name(self: &SwiftValue) -> String;
}
let name = display_name(&product)?;
```

- `apus::mangle!("…")` is the symbol alone, as a string literal:

```rust
static PRODUCTS: SwiftAsyncFn = SwiftAsyncFn::new(apus::mangle!(
    "static StoreKit.Product(struct).products<I: Collection>(for: I) async throws -> [Product(struct)] where I.Element == String"
));
```

**Declaration syntax**: Swift's, fully qualified, without parameter names, and with the kind of every type outside
the standard library spelled out:

```text
[static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name[<U: P>]([label|_]: Type, …)
    [async] [throws] [-> Type] [where …]
[static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name: Type { get [async] [throws] | set }
[static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….subscript[<U: P>]([label|_]: Type, …) -> Type { get [async] [throws] | set }
[static] [prefix | postfix] [Module.Type(kind)….]+(_: Type[, _: Type]) -> Type   (an operator: infix with two parameters)
[mutating | consuming | borrowing] … (how the method takes `self`; the symbol does not show it)
… ([label|_]: [_const] [inout | consuming | borrowing | __owned | __shared | sending] Type, …) [-> [sending] Type]   (`_const`: AppIntents' compile-time constants)
@preconcurrency …   (its symbol leaves out @Sendable and global actors in function types)
[(extension in Module [where …])] Module.Type(kind)[<T, …>]….init[<U: P>]([label|_]: Type, …) [async] [throws] [-> Type?]
```

- Types: `Int`, `String`, `Bool`, `Double`, `UInt8`, … ; `[T]`, `[K: V]`, `Set<T>`, `T?`, tuples, `Any`,
  `AnyObject`, `any Proto`, `Task<…>` and the other concurrency types, generic parameters, `T.Associated`.
- Other types name their kind: `Cart(struct)`, `Customer(class)`, `Color(enum)` (`(actor)` too). A bare name is in
  the declaration's module, `Other.Name(kind)` in another; nested types are `Outer(kind).Inner(kind)`; generic
  arguments follow the kind: `Box(struct)<Int>`.
- Members of generic types name the type's parameters, with the type's own constraints:
  `Shop.Box(struct)<T>.get() -> T?`, `Keyed(struct)<K: Hashable>`. An instance member of a generic struct or enum
  takes `Self`'s metadata after its arguments, which `swift!` takes from the receiver; a result of the type's own
  parameter (`Box<T>.item: T`) gets its type from there too. Its static members and initializers take the type's
  generic arguments after their own (each parameter's `SwiftType`, then witness tables), passed explicitly.
- A parameter that the member's `where` or its extension fixes to a type (`init(second: B) where A == Int`,
  `extension Pair where A == Int`) is no generic argument (observed, accepted 2026-11-22): a static member or
  initializer takes only the others (`B`'s), and an instance member whose parameters are all fixed
  (`sum() where A == Int, B == Int`) takes not even `Self`'s metadata.
- Protocol members: `Shop.Shape(protocol).area() -> Double`; the conforming type is `Self`. Call them on an
  `any Shape` value opened with `SwiftValue::open()` (`self: &Opened`); an existential of one protocol opens,
  a class existential (`any Named`, `Named: AnyObject`) too, as its object's dynamic class. A class-bound protocol's
  members take `self` as the reference itself, which `Opened` passes. Static members and initializers
  (`static Shop.Shape(protocol).make(_: Int) -> Self`, `Shop.Shape(protocol).init(n: Int)`) are called on a type with
  its conformance, `SwiftType::conforming(&protocol)` (`self: &OpenedType`). A `Self` result is a `SwiftValue` of the
  receiver's type. Under `where Self == Concrete` (SwiftUI's `.bordered`) a member belongs to that type and is called
  like that type's own.
- Initializers: `Shop.Cart(struct).init(items: [String])`, `Shop.Level(enum).init(rawValue: Int) -> Level(enum)?`
  (the result is the type unless written; a failable one returns an optional). An initializer or `static` member of a
  class or actor takes the class's metadata as `self`: declare no `self` and `swift!` passes the declared class
  (looked up by name once; `CallError::Unavailable` if it is missing), or `self: &SwiftType` to pass another (a
  subclass, or a generic class specialized, which needs it). A struct's or enum's takes no `self`.
  `swift!` refuses the wrong one. An initializer consumes its arguments (Swift passes them +1): `swift!` hands them
  over. Protocol initializers and static protocol requirements are not supported.
- A value receiver (`self: &SwiftValue`, `self: &Opened`) is passed as a copy. A struct's setter
  (`Shop.Cart(struct).note: String { set }`, `subscript(_: Int) -> Int { set }`) or `mutating` method changes the value
  itself: declare `self: &mut SwiftValue`. An `inout` parameter is a `&mut SwiftValue`. Both work only in a
  synchronous `fn`, since an async call would outlive the borrow. `swift!` refuses a mismatch. A setter takes the new
  value first, then a subscript's indices; Swift consumes the new value (+1), and `swift!` hands it over. Subscript
  labels are the call site's: `subscript(_: Int)` for `v[i]`.
- The Rust signature's `async` and `throws` must be the declaration's, getters' effects included
  (`total: Int { get async throws }` is `async fn total(self: &SwiftValue) throws -> i64`): `swift!` refuses a mismatch,
  which would call an async function the sync way or leave a thrown error unread. A sync member that must run on its
  actor is the exception: it is `async fn` in Rust, and apus hops there.
- A `rethrows` function is called as a `throws` one (`fn f(g: &SwiftClosure) throws -> i64`): its symbol is the same, and
  a closure's error comes back through it.
- Ownership: a `consuming` (`__owned`) or `sending` parameter, every initializer parameter not marked `borrowing`,
  and a setter's new value are passed +1, and `swift!` hands them over. A `sending` result is returned as any result
  is; in a closure type, `-> sending R` is accepted, a `sending` parameter refused (Swift would hand the closure it +1). A `consuming func` takes a copy of `self` (or an
  extra reference) for the callee to consume.
- Operators: `static Shop.Money(struct).+(_: Money(struct), _: Money(struct)) -> Money(struct)`, `static prefix …-(…)`,
  `Shop.~~(_: Money(struct), _: Money(struct)) -> Bool` for one declared at the top level.
- `(extension in Module)` marks a member of an extension that Swift mangles as one: an extension in another
  module than the type's, any constrained extension, every protocol extension. For example
  `Task.sleep(nanoseconds:)`:

```text
static (extension in _Concurrency where Success == Never, Failure == Never)
    _Concurrency.Task<Success, Failure>.sleep(nanoseconds: UInt64) async throws
```

- A type declared in such an extension has it in its context, marked before the type's name:
  `case __C.NSScanner(class).(extension in Foundation) NumberRepresentation(enum).decimal`, and a constrained one
  `case M.Holder(struct)<T>.(extension in M where T: Tagged) Mode(enum).on`. A parameter or result type may carry an
  unconstrained one.
- An associated type of a parameter that has several protocols (marker protocols such as `Sendable` aside) names
  the protocol that declares it, as its symbol does: `T.Value(of Opening)` in
  `M.f<T: Opening & Hashable>(_: T, _: T.Value(of Opening))`; without it the declaration is refused.
- A nested associated type (`T.A.B`) is written as a path whose names after the first each name the protocol
  declaring them, explicitly, as the symbol has it: `(of P)` where the symbol leaves the protocol out, `(mangled P)`
  where it names it (`where S.Element: Sequence, S.Element.Element(of Sequence) == String`). Neither is inferred:
  whether the symbol names it depends on protocols the declaration does not show (one may inherit another, the
  base's own protocol may require more), and a wrong one is refused (by the SDK's exports, or the linker for a module
  of your own). A bound call finds the type a step at a time, through each protocol's conformance. Declarations by
  name write these themselves, as the USR has them. Rather than spell such a chain, a program may expose a
  specialized function from its own Swift module (`func flattenedStrings(_ rows: [[String]]) -> String`) and call
  that.
- `@preconcurrency` declarations: their symbols leave out `@Sendable`, global actors in function types and `Sendable`
  requirements.
- A member type of a generic type takes each level's arguments where Swift writes them:
  `SwiftUI.FocusState(struct)<Bool>.Binding(struct)`, `M.Outer(struct)<Int>.E(enum)<String>`. In a type, a
  constrained extension in the path (`Outer(struct)<T>.(extension in M where T: P) Inner(struct)`) must be the one
  the declaration or its enclosing type is declared in: its `where` clause speaks of that type's parameters.
- A superclass constraint names the class with its kind, `<T: __C.NSObject(class)>`; `T: AnyObject` is written as
  such. A protocol declared `: AnyObject` is marked so in a constraint, `<T: PersistentModel(AnyObject)>` (its symbol
  is the same): only the protocol's descriptor tells, which the call checks (`CallError::InvalidArgument` for a
  missing or wrong mark). A value of a class-bound generic type (the declaration's, its type's: `Measurement<UnitType: NSUnit>`) is a
  class reference, not a `SwiftValue`: `&SwiftObject` (or `&Retained<T>`) as an argument, `SwiftObject` as a
  result (+1); T is then the object's own class. Anywhere else (`T?`, a tuple, a closure) it is refused.
- Metatypes: `T.Type`, a class's `Base(class).Type` and `Any.Type` are `SwiftType`s, as arguments and results; a
  `T.Type` argument also gives T (`decode<T: Decodable>(_: T.Type, from: …) -> T`). A struct's or enum's metatype
  (Swift passes nothing for it), an optional one and `any P.Type` are refused.
- An `any P` argument or result (optional or not) is a `SwiftValue`. When its protocols are class-bound
  (`any Named`, `Named: AnyObject`), Swift passes and returns it in registers, as its reference and one witness
  table per protocol; any other existential by address. Only the type's metadata tells which, so the call reads it
  (`Existential` wraps an argument declared `any P`; up to three protocols). A generic parameter's value goes by
  address whatever its type.
- `any Error` is one error box reference, in a register: `&SwiftError` as an argument (borrowed), `SwiftError` as a
  result (+1), `Option<…>` for `(any Error)?` (nil is null). Any other Rust type is a compile error.
- These rest on swiftc's type lowering (one pointer each; a class existential's words in registers), which the
  calling-convention document does not spell out; Apple's frameworks export functions compiled that way, so ABI
  stability keeps it. So does a generic struct's members taking `Self`'s metadata.
- A variadic generic type's pack argument is written as Swift's demangler prints it, since the use site does not
  show that the type is variadic: `Foundation.Predicate(struct)<Pack{T}>` for `Predicate<T>`,
  `M.Mixed(struct)<Int, Pack{String, Bool}>`. Declarations with packs of their own (`<each T>`) are not supported.
- Isolation: `isolated` parameters, `@MainActor`, `nonisolated`, `nonisolated(nonsending)`, `(actor)`, `throws(E)`.
- Not supported yet: non-ASCII names (and so non-ASCII operators),
  a declaration inside more than one generic enclosing type. A declaration that cannot be mangled
  is a compile error; for anything unsupported, pass the symbol itself with `#[symbol]`.

The mangler (`mangle/`) ports the Swift compiler's identifier, substitution and generic-signature rules and is
checked against the symbols `swiftc` actually emits for 110 declarations, plus real StoreKit and concurrency
symbols.

### Generic functions, framework structs, synchronous calls

Generic Swift functions take their type arguments as trailing parameters: the metadata of each generic
parameter, then a witness table per conformance requirement. Values of generic type, and large or resilient
structs (every struct of an Apple framework), are passed and returned by address, as a `SwiftValue`.

```rust
use apus::swift::{SwiftAsyncFn, SwiftFn, SwiftProtocol, SwiftType, SwiftValue};

// StoreKit 2: static func Product.products<I: Collection>(for: I) async throws -> [Product] where I.Element == String
static PRODUCTS: SwiftAsyncFn = SwiftAsyncFn::new("$s8StoreKit7ProductV8products3forSayACGx_tYaKSlRzSS7ElementRtzlFZ");
static COLLECTION: SwiftProtocol = SwiftProtocol::new("$sSlMp");

let ids = SwiftValue::new(vec!["com.example.pro".to_string()]);            // a Swift [String]
let collection = ids.ty().conformance(&COLLECTION).expect("[String]: Collection");
let products_type = SwiftType::from_mangled_name("Say8StoreKit7ProductVG").expect("[Product]");
let products: SwiftValue = PRODUCTS
    .call_direct(products_type, (&ids, ids.ty(), collection))               // value, I.self, I: Collection
    .throws()
    .await?;
println!("{}", products.description());                                     // String(describing:)
```

- `SwiftType`: `of::<T>()` for the built-in mappings, `from_mangled_name("8StoreKit7ProductV")` for any type in
  the process (`None` if absent), `.conformance(&protocol)` for witness tables.
- `SwiftValue`: `new(rust_value)` (integers, `bool`, `f64`, strings, `Vec`s), `get::<T>()`, `description()`, `clone()`,
  `into_object()` (a value whose type is a class, such as a generic collection's element, as a `SwiftObject`).
- Results: `call::<R>` for the types listed above; `call_returning(ty, args)` for a value returned by address
  (generic, large or resilient); `call_direct(ty, args)` for an opaque value returned in registers (arrays,
  dictionaries, structs of up to four integer words).
- `SwiftFn` calls **synchronous** Swift functions the same way, on the current thread:
  `TOTAL.call::<i64>(()).on_value(&big).invoke()?` (a getter of a struct passed by address).

**Symbol graphs.** A declaration need not spell everything out when its module has a symbol graph: an SDK
module's is extracted once (`swift-symbolgraph-extract`, about four seconds, cached under the temporary directory per
SDK) with a compact copy beside it (written the first time, up to ten seconds for SwiftUI's 500 MB graph): an index
read by each build (SwiftUI's in about 0.2 s) and the symbols, each read when looked up. A program's own module's
graph is emitted by `apus-build` (and found through `APUS_SYMBOL_GRAPHS`).

- Kinds may be left out: `SwiftData.ModelContext.fetch<T: PersistentModel>(_: FetchDescriptor<T>) throws -> [T]`
  (`ModelContext` is a class, `PersistentModel` class-bound, as `(class)` and `(AnyObject)` would say). A kind or mark
  written wins.
- Or the declaration is written by name, its labels, and `{ get }`/`{ set }` for a property or subscript:
  `SwiftData.ModelContext.fetch(_:)`, `Todos.Todo.title { get }`, `Todos.Todo.init(title:priority:)`. The rest comes
  from the graph (`static`, `mutating`, effects, generic requirements); bindings (`T = "Todos.Todo"`) apply as usual.
  A member of an extension is written with its `(extension in M where …)` when Swift mangles it so (a protocol's, a
  constrained one's, another module's), and requirements a member adds to its type's parameters go in its `where`.
  Default values are left out (`= #isolation` aside), a module's typealias is written as the type it names, an
  Objective-C class by its Objective-C name (`__C.CLLocation(class)`), a C type by its C name (`__C.CGRect(struct)`,
  `NSCalendar.Unit` as `__C.NSCalendarUnit(struct)`, `URLResourceKey` as `__C.NSURLResourceKey(typedef struct)`,
  `TimeInterval` as `Double`), what a module declares in an imported type with that extension
  (`__C.NSRunLoop(class).(extension in Foundation) SchedulerTimeType(struct)`), and a variadic type's arguments as
  its pack (`Foundation.Predicate(struct)<Pack{T}>`). A type declared in another module's extension of a type, or
  in a constrained extension of the type enclosing it, is marked with it in the context and in the signature
  (`Swift.String(struct).(extension in Foundation) Encoding(struct)`,
  `Foundation.Measurement(struct)<UnitType>.(extension in Foundation where UnitType: __C.NSDimension(class))
  AttributedStyle(struct)`). The graph leaves out `__shared` and `_const`, which the symbol keeps: the parameters the
  symbol marks so are written with them (`__shared`: borrowed, an initializer's too). A requirement's generic arguments, which the graph names
  without their USRs (`Value.ValueType == Measurement<UnitTemperature>`), are looked up by name in the graphs of
  their type's module and of the declaring module. A name with several declarations or none, or one apus cannot
  write (an associated type of a parameter with several protocols, a standard library typealias), is a compile error: write that one in full. An
  `@objc` enum of a Swift module (`POSIXErrorCode`, `MachErrorCode`) is written in its module
  (`_DarwinFoundation1.POSIXErrorCode(enum)`; `Darwin.POSIXErrorCode(enum)` too) and mangled by that module's ABI
  name (`-module-abi-name Darwin`).
- Objective-C methods and C functions have no Swift symbol: call them through objc2 and its framework crates
  (`objc2-foundation`, …), not apus; `Retained<T>` and blocks cross into Swift calls as they are.
- A value of a C type (struct, enum, option set, typedef struct) is passed and returned by swiftcall's lowering of
  its C layout (clang's `SwiftCallingConv.cpp`): each pointer-sized word of integers or pointers is one integer
  register, a `double` or `float` alone in its word one floating-point register; a value of more than four words
  holding data (`CGAffineTransform`, `CMTimeRange`) goes by address, an argument as a copy the call keeps until it
  ends, a result through the indirect result address. Declare it as its Rust type from the
  objc2 crates (`objc2_core_foundation::CGRect`, `objc2_foundation::NSRange`, `NSCalendarUnit`, `&NSString` for an
  `NS_TYPED_ENUM`; any `objc2::Encode` type, feature `objc2`, else an integer or float for a C scalar), which `swift!`
  lowers from its type encoding; a result is `-> CValue<T>` (`.0` is the value), a typed string's `-> Retained<T>`.
  An `@objc` enum (found so by its module's symbol graph) is one too: its raw integer (`i32` for `POSIXErrorCode`).
  `SwiftValue` for one is a compile error. Refused with `CallError::InvalidArgument`: in registers, a word mixing integers and floating point or holding two `float`s; unions and bitfields, an object
  inside a struct, an optional struct, enum or option set (argument or result), and a result in both kinds of
  registers.

```rust
apus::swift! {
    #[swift = "MyUI.inset(_: __C.CGRect(struct), by: Double) -> __C.CGRect(struct)"]
    fn inset(rect: CGRect, by: f64) -> CValue<CGRect>; // objc2_core_foundation::CGRect
}
```
- A type that moved to another module keeps the module it was defined in in its symbols (`@_originallyDefinedIn`,
  docs/ReferenceGuides/UnderscoredAttributes.md): apus reads which ones did from the SDK's `.swiftinterface`
  (`CoreFoundation.CGFloat` is mangled as CoreGraphics', SwiftUICore's types as SwiftUI's).
- apus still mangles the declaration itself; one written from a graph must mangle to the symbol the graph names, and
  an SDK module's must be exported by the SDK (see `#[swift]` above), or it is a compile error.

Names need not be mangled by hand: `apus::swift_type!("SwiftData.FetchResultsCollection(struct)<Todos.Todo(class)>")`
and `apus::swift_protocol!("SwiftData.PersistentModel")` expand to the mangled type name and the protocol descriptor
symbol, for `SwiftType::from_mangled_name` and `SwiftProtocol::new`.

In `swift!`, a generic function whose generic parameters are the types of its arguments gets its generic arguments
filled in automatically. One whose parameters are not (`fetch<T>(_: FetchDescriptor<T>)`) can have them **bound to
concrete types** after the declaration; the Rust signature then lists only the Swift parameters, and the result's type
is the one with the bindings substituted:

```rust
apus::swift! {
    #[swift = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>) \
               throws -> [T]", T = "Todos.Todo(class)"]
    fn fetch_todos(self: &SwiftObject, descriptor: &SwiftValue) throws -> DirectValue;
}
```

The bound types and their conformances are looked up when the call is made (`CallError::Unavailable` if a type is
missing, `NotConforming` if it does not conform), and a `SwiftValue` or class reference argument whose type names `T`
must hold exactly that type (`InvalidArgument` otherwise; for an object of a subclass, bind the subclass). The
function is still generic, so a value of type `T` goes by address whatever is bound (`&SwiftValue`, `-> SwiftValue`;
`swift!` refuses another Rust type), and `T` may appear only as a parameter or result itself or among a type's
generic arguments (not in `T?`, a tuple or a closure). A class-bound `T` takes a class.

Bindings name every generic parameter, a generic type's too, which specializes the call:

- A member of a generic type: `#[swift = "Res.Tray(struct)<T>.describe<U>(_: U, _: Int) -> String", T = "Swift.String",
  U = "Swift.Double"]`. The receiver must be the bound type (`InvalidArgument` otherwise: exactly, for a struct or enum;
  the class or a subclass, for a class). A struct's or enum's static member or initializer gets the type's generic
  arguments, a class's gets the bound class as `self`: neither is declared in Rust.
- Several protocols on one parameter (`T: Hashable & Comparable`), associated types (`where S.Element == String` is
  checked, `where S.Element: P` gets that conformance's witness table), a superclass (`T: Animal`: the bound class must
  be it or a subclass), typed throws of a generic error type (`throws(E)`: its error slot). A requirement the bound
  types break is `InvalidArgument` (`NotConforming` for a conformance), before any Swift code runs.
- The bound types' metadata, witness tables and requirement checks are found once per call site (a Swift image is
  not unloaded); only the types the arguments hold are checked on every call.
- A generic declaration written by name (`Res.renamed(_:suffix:)`) must bind every parameter.
- Not bindable: a protocol member's `Self` (call it on `Opened`), `nonisolated(nonsending)` functions, typed throws of
  an associated type. A parameter fixed to a type (`where A == Int`) is not bound: bind the others.
- A member of a generic type that adds a conformance to the type's parameters (`extension Box where T: P`, or
  `func f() where T: P`) takes that conformance's witness table besides what `self` gives (observed, accepted
  2026-11-21): bound, apus finds it from the bound type and passes it; unbound, it is refused (a struct's or enum's
  static member or initializer takes all its generic arguments explicitly, as before). A protocol extension's member
  that adds one (`where Self: P`) is refused.

### Enums

An enum value is a `SwiftValue` (pass and return it by address, like a framework struct). Its cases are read and made
through the runtime's enum value witnesses, by **tag**: the cases with a payload first, in declaration order, then
the cases without one. A library-evolution module (every Apple framework) exports each case's tag as a symbol, which
`mangle!("case …")` names and `SwiftEnumCase` reads, so a case added or reordered in a newer OS is still found:

```rust
use apus::swift::{SwiftEnumCase, SwiftType, SwiftValue};

static CORRUPTED: SwiftEnumCase =
    SwiftEnumCase::new(apus::mangle!("case Swift.DecodingError(enum).dataCorrupted(DecodingError(enum).Context(struct))"));

if error.enum_tag() == CORRUPTED.tag() {
    let context = unsafe { error.enum_payload(SwiftType::from_mangled_name("s13DecodingErrorO7ContextV").unwrap()) };
}
```

- A case of a generic enum spells out the enum's constraints, which its symbol carries:
  `case M.Keyed(enum)<K: Hashable>.one(K)`.
- `SwiftType::enum_cases()`: how many cases have a payload and how many do not; `None` for a type that is not an enum.
- `enum_tag()`; `enum_payload(ty)` and `indirect_enum_payload(ty)` copy the payload out (a tuple type for several
  associated values; the value is untouched); `SwiftValue::enum_case(ty, tag)` and `enum_case_with(ty, tag, payload)`
  make a value. Tags and case kinds are checked; the payload type and `indirect` are yours to get right (`unsafe`).
- Not supported: making a value of an `indirect` case (its box would have to match what the compiler builds); enums
  imported from C (`NS_ENUM`, refused: the value is the raw integer, use `rawValue`); passing a small enum of a module
  without library evolution in registers (its layout is the compiler's to compute: use a function that takes or
  returns it by address, or a generic one).

### Opaque results (`some P`)

A declaration returning `some P` returns a `SwiftValue` of the actual type, which the Swift runtime gives from the
declaration's opaque type descriptor (`…QOMQ`, exported by library-evolution modules) and the call's generic
arguments (`swift_getOpaqueTypeMetadata2`). `conforming(&protocol)` makes it an `Opened` to call `P`'s requirements
and extension methods, so calls chain:

```rust
apus::swift! {
    #[swift = "Res.makeSomeShape(side: Double) -> some Shape"]
    fn make_some_shape(side: f64) -> SwiftValue;
    #[swift = "(extension in Res) Res.Shape(protocol).enlarged() -> some Shape"]
    fn enlarged(self: &Opened) -> SwiftValue;
}
let square = make_some_shape(3.0)?.conforming(&SHAPE).unwrap();
let bigger = enlarged(&square)?;
```

A `some P` parameter is an anonymous generic parameter (pass its type metadata and witness table like any other).
One `some P` per result. Its protocol may take its primary associated types: a result's (`-> some Collection<Int>`)
leave its symbol, as Swift mangles them; a parameter's (`_: some Collection<Int>`) are requirements on that anonymous
parameter (`T.Element == Int`). They are named by the protocol's declaration (the standard library's, or the symbol
graph's for a framework's), or written by name (`some Holding<Held == Int>`), which a protocol of your own needs.
`some P<some Q>` is not Swift.

### SwiftData

A model is Swift: `@Model` is a macro that writes the `PersistentModel` conformance, and a container's schema takes
`any PersistentModel.Type` values, whose lowering apus does not rely on. The rest is called from Rust
(tests/swift_data.rs, with the model in tests/support/notes.swift). `examples/swift-data` is the same as a program
of its own: its build.rs compiles the model into a static library, and Rust inserts, edits, deletes, saves, fetches
with a `#Predicate` and reopens the store (`cargo run -p apus-example-swift-data`):

```swift
@Model public final class Note { public var title: String; public init(title: String) { self.title = title } }
public func noteContainer() throws -> ModelContainer {
    try ModelContainer(for: Note.self, configurations: ModelConfiguration(isStoredInMemoryOnly: true))
}
```

```rust
apus::swift! {
    class Todo = "Todos.Todo(class)";
    class ModelContext = "SwiftData.ModelContext(class)";
    struct Query = "SwiftData.FetchDescriptor(struct)<Todos.Todo(class)>";

    impl ModelContext {
        #[swift = "SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)"]
        fn insert(&self, model: &Todo);
        #[swift = "SwiftData.ModelContext(class).save() throws"]
        fn save(&self) throws;
        #[swift = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>) \
                   throws -> [T]", T = "Todos.Todo(class)"]
        fn fetch(&self, query: &Query) throws -> Vec<Todo>;
    }
}
```

`fetch<T>(_:) -> [T]` is generic over the model, which no argument's type gives: bind `T = "Todos.Todo(class)"` (see
Generic functions above); the array comes back as `Vec<Todo>` (see Typed wrappers below). Sort descriptors and predicates take key paths and
`#Predicate`, which only the compiler makes: they come from Swift too.

### Using a Swift module of your own

The Swift a program still needs (a `@Model`, a `#Predicate`) is a module of its own, which
[`apus-build`](builder/src/lib.rs) compiles from `build.rs` into a static library and links, built like Apple's
frameworks (library evolution) for the program's `MACOSX_DEPLOYMENT_TARGET`:

```toml
# Cargo.toml
[build-dependencies]
apus-build = { path = "…/builder" }

# .cargo/config.toml (apus needs it anyway: see Tested on)
[env]
MACOSX_DEPLOYMENT_TARGET = "14.0"
```

```rust
// build.rs: every *.swift in swift/, as module Todos
fn main() {
    apus_build::SwiftModule::new("Todos", "swift").build();
}
```

It is optimized (`-O`) in release builds only (`-Onone` otherwise, to build fast). Its declarations are then
`#[swift_linked = "Todos.…"]` (bound at link time), by name if wanted (`Todos.Todo.title { get }`): apus-build also emits
the module's symbol graph. `examples/swift-data` is the whole
setup. In this repository, `.vscode/settings.json` gives rust-analyzer its own target directory: sharing the one
terminal builds rewrite makes its proc-macro server fail now and then while it reads a macro being rebuilt.

### Typed wrappers

`SwiftObject` and `SwiftValue` hold any Swift type. A program can instead name the types it uses, so that `swift!`
checks every call against its declaration when it compiles:

```rust
apus::swift! {
    class Todo = "Todos.Todo(class)";                                     // pub struct Todo(SwiftObject)
    struct Query = "SwiftData.FetchDescriptor(struct)<Todos.Todo(class)>";  // pub struct Query(SwiftValue); `enum` too

    impl Todo {
        #[swift_linked = "Todos.Todo(class).init(title: String, priority: Int)"]
        fn new(title: &str, priority: i64) -> Todo;
        #[swift_linked = "Todos.Todo(class).title: String { get }"]
        fn title(&self) -> String;                                        // todo.title()?
    }
}
```

- A wrapper stands for one concrete class, struct or enum, and derefs to the `SwiftObject` or `SwiftValue` it holds
  (`into_inner()` gives it up). `Todo::try_from(object)` checks the type when the call is made (a subclass's object
  passes; `CallError::InvalidArgument` if not).
- As a receiver (`&self`, `&mut self` in `impl`), an argument (`&Todo`) or a result (`Todo`, `Option<Todo>` for a
  class's optional, `Vec<Todo>` for an array, read through its `Sequence` conformance), a wrapper must be the
  declaration's type: another struct's or enum's, or another type's result, is a compile error. A class's wrapper
  where another class is declared (a superclass's member) is checked by cast when the call is made.
- No check costs anything at run time but that cast; type-erased receivers are checked on every call instead (see
  Methods and properties).

### Sequences

Any value whose type is a Swift `Sequence` (an `Array`, bridged or not, a `Set`, a `Dictionary`, a framework's
collection) is a Rust `Iterator` of its elements, through the conformance's `makeIterator()` and `next()`:

```rust
use apus::swift::SwiftIter;

for todo in SwiftIter::new(todos.0)? {                         // `todos`: a `-> [Todo]` result (DirectValue)
    let todo = todo.into_object().expect("a class instance");  // each element a SwiftValue of `element_type()`
}
```

An `Array`, `Dictionary`, `Set` or `String` result comes back in registers: `swift!` refuses `-> SwiftValue` for one
and takes `-> DirectValue`.

### Async sequences

Any value whose type is a Swift `AsyncSequence` can be consumed element by element:

```rust
use apus::swift::SwiftStream;

let mut updates = SwiftStream::new(sequence)?;              // e.g. an AsyncStream, or StoreKit's Transaction.updates
while let Some(update) = updates.next().await {
    let update = update?;                                    // Err: the sequence threw, and is finished
    println!("{}", update.description());
}
```

- Each element is awaited in the calling Task (cancellation reaches the sequence). `next_value::<T>()` copies
  the element out as a Rust type.
- Dropping the stream destroys the Swift iterator, which is how sequences are told to stop.
- `.child()` awaits elements in child Tasks so that `within(duration, stream.next())` and `select!` work; a
  pending element is kept for the next `next()`.
- With the `stream` feature, `SwiftStream` implements `futures_core::Stream`.

## Awaiting Swift async closures from Rust

Hand Rust a Swift `async` closure with this helper in your Swift module (9 lines, public Swift only):

```swift
public final class AsyncSwiftClosure {
    let body: () async throws -> Any
    init(_ body: @escaping () async throws -> Any) { self.body = body }
}
/// Hands a Swift async closure to Rust as an owned (+1) pointer.
public func asyncSwiftClosure(_ body: @escaping () async throws -> Any) -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(AsyncSwiftClosure(body)).toOpaque()
}
```

Expose closures through plain C functions, capturing whatever arguments they need:

```swift
@_cdecl("fetch_products")
public func fetchProducts(_ ids: UnsafePointer<CChar>) -> UnsafeMutableRawPointer {
    let ids = String(cString: ids).split(separator: ",").map(String.init)
    return asyncSwiftClosure { try await Product.products(for: ids) }
}
```

Any `() async throws -> T`, `() async -> T`, `() async throws`, `() async` or `() async -> T?` converts to
`() async throws -> Any` implicitly. On the Rust side:

```rust
extern "C" { fn fetch_products(ids: *const std::ffi::c_char) -> *mut std::ffi::c_void; }

let products = unsafe { SwiftAsync::from_raw(fetch_products(c"pro,lite".as_ptr())) }.await?;
```

The result is a `SwiftAny`. `downcast::<T>()` copies it out as `()`, `bool`, `i64`, `i32`, `u64`, `f64`, `String`,
or `Option`, `Vec` and `HashMap` of those (Swift `Optional`, `Array`, `Dictionary`; keys: `String` and integers):

```rust
let names: Vec<String> = products.downcast().expect("a [String]");
```

The closure runs **in the calling Swift Task**: it sees the task's cancellation, priority and task-locals, and a
MainActor task is back on the main actor when the await returns. Awaiting outside a Swift task yields
`CallError::NoCurrentTask`.

### Concurrency rule for Swift calls

A Swift call awaited on its own runs in the current Task, exactly like Swift's `await f()`: the Task is busy
until it returns, so other futures in the same task are not polled meanwhile. Calls that must overlap run as
child Tasks (Swift's `async let`, via a discarding TaskGroup; they inherit cancellation, priority and task-locals):

- two or more calls polled together (`join!`) become child Tasks automatically;
- `.child()` forces a child Task, and `timeout(d, call)` uses one.

Use `.child()` (or `timeout`) inside `select!`/races. Without it the call runs to completion before the other
branch can win: there is no way to know in advance that another branch will wake up later.

Don't block inside tasks: the Swift cooperative pool does not grow to compensate.

A panic in a task's future is caught (`JoinError::Panicked`). A panic that starts inside a Swift callback cannot
unwind through Swift and aborts the process: a `Waker::wake` that panics when a Swift call completes or a task is
cancelled, or the `Drop` of an abandoned call's result.

## Passing Rust closures to Swift

A Swift parameter of function type takes a `SwiftClosure`: a Rust closure as a Swift closure value, which Swift may
call, keep and release like any other.

```rust
use apus::swift::{ByAddress, SwiftClosure, SwiftError, SwiftValue};

apus::swift! {
    #[swift = "Shop.retry(times: Int, _: @escaping @Sendable (Int) async throws -> String) async throws -> String"]
    async fn retry(times: i64, attempt: &SwiftClosure) throws -> String;
    #[swift = "Shop.transform<T>(_: T, _: (T) -> T) -> T"]
    fn transform(x: &SwiftValue, f: &SwiftClosure) -> SwiftValue;
}

let attempt = SwiftClosure::new_async(|n: i64| async move { Ok::<_, SwiftError>(format!("try {n}")) });
let text = retry(3, &attempt).await?;
let plus_one = SwiftClosure::new_sync(|x: ByAddress<i64>| ByAddress(x.0 + 1));   // T is passed by address
```

- Parameters: integers, `bool`, `f64`, `f32`, `String`, `SwiftObject`, and `ByAddress<T>` for a value of generic type
  (`T` a plain value such as `i64`). Results: the same, `()`, and `Result<R, SwiftError>` for a throwing closure (an
  `Err` rethrows a Swift error; Rust cannot create new Swift error types yet).
- `swift!` checks each closure against its declared function type before calling (`CallError::ClosureMismatch`):
  async or sync, `throws`, `@MainActor` (`.on_main_actor()`), and which values go by address. The convention follows
  the declared type: `(T) -> T` passes addresses where `(Int) -> Int` passes registers, so declare the type exactly.
- An async closure runs inside the Swift Task that calls it: its own Swift calls, `.child()` calls and cancellation
  belong to that Task. A `@MainActor` closure moves to the main actor itself, as Swift's callers expect.
- The closure may run many times, at once, on any thread (`Fn + Send + Sync`). A panic in it aborts the process.
- Reflection (`Mirror`) of the closure's context shows no captures.
- An `@autoclosure` parameter is a closure like any other: pass a `SwiftClosure` (`@autoclosure () -> Int` takes
  `SwiftClosure::new_sync(|| 41i64)`).

Swift's other function conventions are one pointer each:

```rust
apus::swift! {
    #[swift = "M.call(_: @convention(c) (Int32, Double) -> Int32) -> Int32"]
    fn call(f: extern "C" fn(i32, f64) -> i32) -> i32;
    #[swift = "M.each(_: @convention(block) (Int) -> Int) -> Int"]
    fn each(f: &block2::Block<dyn Fn(isize) -> isize>) -> i64;            // feature `block2`
    #[swift = "M.adder(_: Int) -> @convention(block) (Int) -> Int"]
    fn adder(k: i64) -> block2::RcBlock<dyn Fn(isize) -> isize>;
}
```

- `@convention(c)`: an `extern "C" fn(…)`, as argument or result; `Option<…>` when optional (nil is null).
- `@convention(block)`: a block2 `&Block<dyn Fn(…) -> R>` argument (an `RcBlock` or a `StackBlock`; apus passes a
  `_Block_copy` of it, held until the call is over or handed to an initializer that consumes it), an `RcBlock<…>`
  result (+1); `Option<…>` when optional.
- Their parameter and result types are checked against the declared ones before the call
  (`CallError::ClosureMismatch`), as Objective-C type encodings: Swift's integers, `Double`, `Float` and `Bool` exactly,
  any other type (a pointer, an object) unchecked. A block result's type is not checked.
- Only as a parameter or result itself, its optional, or a closure's parameter. `@convention(thin)` is refused: no
  public API takes one.

## How it works, and what it relies on

- The Swift runtime ABI apus relies on is written once, in the C header `sys/include/apus/abi.h` of the internal
  crate `apus-sys` (layouts and flags transcribed from `swiftlang/swift` @ `swift-6.4.2-RELEASE`, each item citing
  its source). The C funclets are compiled against it and the Rust declarations are generated from it by bindgen
  (`sys/tests/fresh.rs`; `APUS_BLESS=1` regenerates them).
- Tasks are created with `swift_task_create_common`; small C funclets (`sys/csrc/funclets.c`, clang
  `swiftasynccall`) hold the async entry/step/resume code the runtime calls. Suspension uses
  `swift_continuation_init/await`; the Rust waker resumes the continuation. Child Tasks use a discarding TaskGroup
  and `swift_taskGroup_waitAll`.
- Rust calls the runtime's synchronous Swift-CC functions directly as `extern "C"`, so the glue inlines.
  `sys/tests/lowering.rs` reads the header and checks, for each such function, that clang lowers a `swiftcall` call
  and a C call identically on arm64 and x86_64. A function that does not (`swift_task_isCancelled`, whose Bool
  result is not extended) is reached through a C thunk instead.

**Risks.** `swift_task_create_common`, the continuation and TaskGroup context layouts and the plain-Job metadata
shape are runtime-internal contracts, not public API. They are tied to libdispatch and back-deployment, so they
are stable in practice, but a future OS could change them. A self-check runs before the first spawn (every
runtime symbol resolves, metadata kinds match, a probe Task suspends, resumes and completes) and panics with a
clear message instead of corrupting memory.

## Tested on

macOS 27.3 (arm64, and x86_64 under Rosetta), Swift runtime 6.4.2, rustc 1.99. Other versions are untested.
Apple platforms only. Deployment floor: macOS 14 (macOS 15 with the `macos15` feature). The build stops unless
`MACOSX_DEPLOYMENT_TARGET` is set to the floor or later, because the floor must reach your binary, not only apus's C
code: below macOS 12 the SDK links the Swift concurrency runtime through `@rpath`, which a dependency cannot provide,
and the binary would fail to launch. In your project's `.cargo/config.toml`:

```toml
[env]
MACOSX_DEPLOYMENT_TARGET = "14.0"
```

The test probes need macOS 14 (`MainActor.assumeIsolated`). This repository sets 14.0 the same way. Tests:
`cargo test --workspace --features dispatch,test-probe,stream` (add `macos15` with `MACOSX_DEPLOYMENT_TARGET=15.0`);
`scripts/check-xlto.sh` checks the `xlto` build.

## The process's main

`run_main(future)` drives the main run loop until `future` is done and returns its output. `async_main(future)` is
Swift's async `@main` instead: it hands the future to Swift's own `_runAsyncMain` (exported since Swift 5.5), which
starts it as the main task on the main actor, drains the main queue its own way, and exits the process when it is
done (code 1 on `Err`, 101 on a panic). It never returns.

```rust
fn main() {
    apus::swift::async_main(async {
        // Swift and Rust async code on the main actor
        Ok::<(), String>(())
    })
}
```

A SwiftUI app whose `App` type is written in Swift starts with `App.main()`:
`static (extension in SwiftUI) SwiftUI.App(protocol).main()` (pass the type's metadata and its `App` conformance).

## Isolation and actors

- **`isolated` parameters (`#isolation`)**: only `isolated (any Actor)?` (apus passes nil off the main actor). Declare
  the parameter in Rust as `apus::swift::Isolation` and pass
  `Isolation::current()`. It is decided when the call runs: the main actor when the call runs in a main task
  (`spawn_on_main`, `run_main`), nil in a plain task and in a child Task (`.child()`, `join!`), like Swift's `async let`.
  It never names another actor.
- **Actor and `@MainActor` sync members**: a sync method or getter of an `(actor)` type, or a declaration marked
  `@MainActor`, is declared `async fn` in Rust; apus hops to the executor (`Actor.unownedExecutor`, so custom executors
  work), calls, and hops back. Declared `fn`, it is a compile error. `nonisolated` members are called directly.
  Isolation a member inherits from its type (members of a `@MainActor class`) is not in the symbol: write
  `@MainActor` on each such member's declaration, or apus calls it without hopping.
  ```rust
  apus::swift! {
      #[swift_linked = "MyApp.Store(actor).count: Int { get }"]
      pub async fn count(self: &SwiftObject) -> i64;
      #[swift_linked = "@MainActor MyApp.refreshUI() -> Bool"]
      pub async fn refresh_ui() -> bool;
  }
  ```
- **Typed throws**: only a generic or associated error type (`throws(E)`): pass an `ErrorSlot::of(E's type)` after the
  generic arguments (`swift!` requires it, and refuses it elsewhere) and declare no `throws` in Rust; a thrown error is `CallError::Typed(SwiftValue)`. A concrete error
  type is refused: whether its error comes back in registers or through a slot depends on its layout. `throws(Never)`
  is a function that does not throw, as in Swift.
- **`nonisolated(nonsending)`**: write it in the declaration; apus passes the caller's isolation as the hidden first
  argument. **The symbol is the same as a plain async function's**: a declaration without the attribute calls such a
  function with its arguments shifted, and nothing can detect it.
- `SwiftStream` uses `next(isolation:)` on macOS 15 and later.

## Objective-C objects (`objc2` feature)

On Apple platforms every Swift class instance is an Objective-C object, so an objc2 `Retained<T>` and a `SwiftObject`
are the same owned reference. With the `objc2` feature:

- `SwiftObject::from(retained)`, `object.into_objc()` (`Retained<AnyObject>`) and `object.downcast::<NSView>()` (checks
  the class) convert without retaining or releasing;
- `&Retained<T>` is a class argument, `Retained<T>` / `Option<Retained<T>>` a class result (the declaration's type, not
  checked), and `Retained<T>` a method's `self`.

In a declaration, an Objective-C class is `__C.Name(class)`:

```rust
apus::swift! {
    #[swift_linked = "MyUI.makeHostingView(title: String) -> __C.NSView(class)"]
    pub fn make_hosting_view(title: &str) -> Retained<NSView>;
}
```

Other imported C types are `__C.CGPoint(struct)` and the like. One that Swift names after its typedef is written with
the typedef's name and `typedef struct` or `typedef class`: an anonymous struct (`__C.NSDecimal(typedef struct)`,
Swift's `Decimal`), an `NS_TYPED_ENUM` type (`__C.NSURLResourceKey(typedef struct)`, Swift's `URLResourceKey`), a Core
Foundation class (`__C.CGColorRef(typedef class)`, Swift's `CGColor`). A typedef of another type
(`TimeInterval`, `CLLocationDistance`) is that type: write `Double`.

`SwiftObject` retains and releases with `swift_unknownObjectRetain`/`Release`, so it holds NSObject-derived Swift
classes and Objective-C objects as well as native Swift ones, with or without the feature.

## Boundary symbols and cross-language LTO

apus's C code (the funclets) and its Rust code call each other directly by name. Those names live in their own
namespace, `apus_<version>_<hash>_…` (`APUS_TAG` in `sys/include/apus/abi.h`), so they cannot clash with another
library's symbols, and the C ones are compiled with hidden visibility, so they are never exported. The Rust-defined
ones are exported in an unstripped binary (stable Rust cannot hide them), under that namespace.

Because the calls are direct, LTO can inline across the boundary. With the `xlto` feature apus-sys compiles its C as
LLVM bitcode; link your binary with rustc's linker-plugin LTO and the C and Rust sides are optimized as one unit: the
funclets inline the Rust callbacks, Rust inlines the C helpers, and the boundary symbols become local. It needs an LLVM
whose major version matches rustc's (`rustc -vV`), for example Homebrew's `llvm`. In your project:

```toml
# .cargo/config.toml
[env]
MACOSX_DEPLOYMENT_TARGET = "14.0"
CC_aarch64_apple_darwin = "/opt/homebrew/opt/llvm/bin/clang"

[target.aarch64-apple-darwin]
# scripts/xlto-link from this repository: drops the ELF-style -plugin-opt arguments rustc passes on macOS and links
# with Apple's ld and that LLVM's libLTO (ld64.lld cannot read the current SDK's .tbd files).
linker = "path/to/apus/scripts/xlto-link"
rustflags = ["-Clinker-plugin-lto"]

[profile.release]
lto = "fat"
```

`scripts/xlto-link` links with the LLVM at `APUS_LLVM` (default `/opt/homebrew/opt/llvm`); point it at the same LLVM as
`CC_*`. With a git or registry dependency, copy the script into your project and set `linker` to it.

The C side is compiled for the CPU rustc targets (the last `-C target-cpu`, default `apple-m1`); LLVM does not inline a
Rust function into C compiled for a lesser CPU. `-C target-feature` is not forwarded, so extra features may keep the
callbacks from inlining (it still links and runs). The build stops with a message when the feature is used with Apple
clang, a clang of another LLVM version, a non-macOS target, or without `-Clinker-plugin-lto`. `xlto-link` drops
rustc's `-plugin-opt` arguments, the LTO optimization level among them, so the link uses libLTO's default level.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
The dispatch backend's runnable structure follows [dgrijalva/async-dispatch](https://github.com/dgrijalva/async-dispatch)
(Apache-2.0). Swift runtime layouts are transcribed from [swiftlang/swift](https://github.com/swiftlang/swift)
(Apache-2.0 with Runtime Library Exception); no code is copied.
