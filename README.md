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
the main run loop through `objc2-core-foundation` and is woken by a `CFRunLoopSource`).
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

```rust
async_swift_rs::swift! {
    /// `func greet(_ name: String, times: Int, sleepMs: Int) async throws -> String`
    #[symbol = "$s5Probe5greet_5times7sleepMsS2S_S2itYaKF"]
    pub async fn greet(name: &str, times: i64, sleep_ms: i64) throws -> String;

    /// `func byteCount(_ s: String) -> Int`, bound at link time
    #[linked = "$s5Probe9byteCountySiSSF"]
    fn byte_count(text: &str) -> i64;
}

let text = greet("Kim", 2, 10).await?;      // a SwiftCall<String>: `.child()` and `timeout` apply
let n = byte_count("héllo")?;                // synchronous: Result<i64, CallError>
```

`async fn` is a Swift `async` function, a plain `fn` a synchronous one; `throws` after the parameters marks a
throwing function. `#[symbol]` resolves at run time (missing → `CallError::Unavailable`), `#[linked]` at link
time. Symbols are still written by hand here; computing them from a Swift declaration is planned.

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
