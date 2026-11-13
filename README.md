# async-swift-rs

Run Rust futures as genuine **Swift Concurrency Tasks** on macOS, or on **libdispatch**.

With the `swift` backend every spawned future *is* a Swift Task, created through the Swift runtime's own ABI
(no Swift source in this crate). Swift code sees it as one: `Task.isCancelled`, `Task.currentPriority` and
priority escalation, `@TaskLocal` copying, MainActor isolation, Instruments' Swift Tasks view. Rust code can
await Swift `async` closures **inside the same Task**.

```toml
[dependencies]
async-swift-rs = { path = "…" }                                          # swift backend (default)
async-swift-rs = { path = "…", default-features = false, features = ["dispatch"] }
```

| Feature | Backend |
|---|---|
| `swift` (default) | `async_swift_rs::swift`: tasks are Swift Tasks on the cooperative pool / MainActor |
| `dispatch` | `async_swift_rs::dispatch`: `async-task` runnables on GCD queues via [`dispatch2`](https://crates.io/crates/dispatch2) (QoS, main queue, serial queues) |
| `stream` | `impl futures_core::Stream for SwiftStream` |
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
use async_swift_rs::swift::SwiftAsyncFn;

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
async_swift_rs::swift! {
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
  at link time. `#[symbol = "$s…"]` / `#[linked = "$s…"]` take a mangled symbol as is.
- **Methods and properties**: a leading `self: &SwiftObject` (a class instance) or `self: &SwiftValue` (a struct
  passed by address) is the receiver.
- **Opaque results**: `-> SwiftValue` (returned by address: large and framework structs) and `-> DirectValue`
  (returned in registers: an array or dictionary whose elements stay opaque) take their Swift type from the
  declaration.

```rust
async_swift_rs::swift! {
    #[swift = "static StoreKit.Product(struct).products<I: Collection>(for: I) async throws -> [Product(struct)] where I.Element == String"]
    async fn products(ids: &SwiftValue, ids_type: SwiftType, collection: WitnessTable) throws -> DirectValue;

    #[swift = "StoreKit.Product(struct).displayName: String { get }"]
    fn display_name(self: &SwiftValue) -> String;
}
let name = display_name(&product)?;
```

- `async_swift_rs::mangle!("…")` is the symbol alone, as a string literal:

```rust
static PRODUCTS: SwiftAsyncFn = SwiftAsyncFn::new(async_swift_rs::mangle!(
    "static StoreKit.Product(struct).products<I: Collection>(for: I) async throws -> [Product(struct)] where I.Element == String"
));
```

**Declaration syntax**: Swift's, fully qualified, without parameter names, and with the kind of every type outside
the standard library spelled out:

```text
[static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name[<U: P>]([label|_]: Type, …)
    [async] [throws] [-> Type] [where …]
[static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name: Type { get }
```

- Types: `Int`, `String`, `Bool`, `Double`, `UInt8`, … ; `[T]`, `[K: V]`, `Set<T>`, `T?`, tuples, `Any`,
  `AnyObject`, `any Proto`, `Task<…>` and the other concurrency types, generic parameters, `T.Associated`.
- Other types name their kind: `Cart(struct)`, `Customer(class)`, `Color(enum)` (`(actor)` too). A bare name is in
  the declaration's module, `Other.Name(kind)` in another; nested types are `Outer(kind).Inner(kind)`; generic
  arguments follow the kind: `Box(struct)<Int>`.
- Members of generic types name the type's parameters, with the type's own constraints:
  `Shop.Box(struct)<T>.get() -> T?`, `Swift.Array<Element>.first: Element? { get }`, `Keyed(struct)<K: Hashable>`.
- Protocol members: `Shop.Shape(protocol).area() -> Double`; the conforming type is `Self`.
- `(extension in Module)` marks a member of an extension that Swift mangles as one: an extension in another
  module than the type's, any constrained extension, every protocol extension. For example
  `Task.sleep(nanoseconds:)`:

```text
static (extension in _Concurrency where Success == Never, Failure == Never)
    _Concurrency.Task<Success, Failure>.sleep(nanoseconds: UInt64) async throws
```

- Not supported yet: function-typed parameters, operators, initializers, subscripts, setters, non-ASCII names,
  isolation and ownership attributes, more than one generic enclosing type. A declaration that cannot be mangled
  is a compile error; for anything unsupported, pass the symbol itself with `#[symbol]`.

The mangler (`mangle/`) ports the Swift compiler's identifier, substitution and generic-signature rules and is
checked against the symbols `swiftc` actually emits for 110 declarations, plus real StoreKit and concurrency
symbols.

### Generic functions, framework structs, synchronous calls

Generic Swift functions take their type arguments as trailing parameters: the metadata of each generic
parameter, then a witness table per conformance requirement. Values of generic type, and large or resilient
structs (every struct of an Apple framework), are passed and returned by address, as a `SwiftValue`.

```rust
use async_swift_rs::swift::{SwiftAsyncFn, SwiftFn, SwiftProtocol, SwiftType, SwiftValue};

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
- `SwiftValue`: `new(rust_value)` (integers, `bool`, `f64`, strings, `Vec`s), `get::<T>()`, `description()`, `clone()`.
- Results: `call::<R>` for the types listed above; `call_returning(ty, args)` for a value returned by address
  (generic, large or resilient); `call_direct(ty, args)` for an opaque value returned in registers (arrays,
  dictionaries, structs of up to four integer words).
- `SwiftFn` calls **synchronous** Swift functions the same way, on the current thread:
  `TOTAL.call::<i64>(()).on_value(&big).invoke()?` (a getter of a struct passed by address).

### Async sequences

Any value whose type is a Swift `AsyncSequence` can be consumed element by element:

```rust
use async_swift_rs::swift::SwiftStream;

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

## How it works, and what it relies on

- Tasks are created with `swift_task_create_common`; a small C file (`csrc/funclets.c`, clang `swiftasynccall`)
  holds the async entry/step/resume funclets the runtime calls. Suspension uses `swift_continuation_init/await`;
  the Rust waker resumes the continuation. Child Tasks use a discarding TaskGroup and `swift_taskGroup_waitAll`.
- Rust calls the runtime's synchronous Swift-CC functions directly as `extern "C"`, so the glue inlines.
  `tests/abi_equivalence.rs` checks, for each of them, that clang lowers a `swiftcall` call and a C call
  identically on arm64 and x86_64. Swift-CC `bool` returns are read as `u8 & 1`.
- Layouts and flags are transcribed from `swiftlang/swift` @ `swift-6.4.2-RELEASE`
  (`include/swift/ABI/{Task,TaskStatus,MetadataValues,MetadataKind,Metadata,Executor}.h`,
  `include/swift/Runtime/Concurrency.h`, `stdlib/public/Concurrency/Task.cpp`, `TaskGroup.cpp`), each cited where used.

**Risks.** `swift_task_create_common`, the continuation and TaskGroup context layouts and the plain-Job metadata
shape are runtime-internal contracts, not public API. They are tied to libdispatch and back-deployment, so they
are stable in practice, but a future OS could change them. A self-check runs before the first spawn (every
runtime symbol resolves, metadata kinds match, a probe Task suspends, resumes and completes) and panics with a
clear message instead of corrupting memory.

## Tested on

macOS 27.3 (arm64, and x86_64 under Rosetta), Swift runtime 6.4.2, rustc 1.99. Other versions are untested.
The test probes need macOS 14 (`MainActor.assumeIsolated`).
