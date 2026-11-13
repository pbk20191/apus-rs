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
`is_cancelled()`, `priority()`, `current_task()`, `SwiftAsync`, `timeout()`, `SwiftAny`, `SwiftError`.
Dispatch backend only: `SerialQueue`, `block_on` (not on the main thread).

### Inheritance

| | priority | task-locals | main / MainActor |
|---|---|---|---|
| `spawn` | inherited (`Unspecified`) | copied | stays on main if spawned from a main task |
| `spawn_detached` | `Default` | not copied | no |

`JoinHandle` resolves when the **Swift Task** completes, i.e. after any child Tasks, like Swift's `Task.value`.

## The main run loop

Main tasks run when the main queue is drained by `CFRunLoopRun`/`NSApplication.run` (or `run_main`, which drives
the main run loop through `objc2-core-foundation`).
**Do not use `dispatch_main()`**: it drains the main queue on a worker thread, so MainActor code runs off the
main thread.

## Awaiting Swift async code from Rust

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
