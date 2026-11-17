# M6 spike: closures passed to Swift (2026-11-17)

`xcrun swiftc -O -parse-as-library -module-name Cl -emit-ir cl.swift -o cl.ll -target arm64-apple-macos14.0`

- A closure argument is two words: the function (for async, its `AsyncFunctionPointer`) and the context, a Swift heap
  object or null.
- Concrete type `(Int) async -> Int`: called as `f(ctx, 20, swiftself context)`; returns through the caller's
  continuation `resume(ctx, i64 result)`; a throwing one `resume(ctx, results…, swiftself error)`.
- Generic type `(T) async -> T`: the same closure shape is called as `f(indirect result, ctx, argument address,
  swiftself context)`: values by address (the maximally abstract convention). Which convention applies follows from the
  declared parameter type, not from the closure.
- `@MainActor (Int) async -> Int`: the caller does not hop; the closure hops to the main actor itself.
- Sync `(Int) -> Int`: `swiftcc f(3, swiftself context)`.
- Mangling of function types: `S3iYaXE` (non-escaping async `(Int) -> Int`), `c` for escaping, `Yb` Sendable,
  `K` throws, `ScMYc` @MainActor.
