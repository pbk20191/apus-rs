# async-swift-rs

Run Rust futures as genuine Swift Concurrency Tasks on Apple platforms, or on libdispatch.

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
