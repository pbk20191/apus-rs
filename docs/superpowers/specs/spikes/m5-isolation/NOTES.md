# M5 spike: isolation, actor sync calls, typed throws (2026-11-17)

swiftc (Xcode, Swift 6.4.2), `-O -target arm64-apple-macos15.0 -emit-ir` on `iso.swift` and `callers.swift`
(regenerate the `.ll` files with the commands below; they are not committed).

```
xcrun swiftc -O -parse-as-library -module-name Iso -emit-ir iso.swift -o iso.ll -target arm64-apple-macos15.0
xcrun swiftc -O -parse-as-library -module-name Cal -emit-ir callers.swift -o callers.ll -target arm64-apple-macos15.0
```

## Findings

1. `isolation: isolated (any Actor)? = #isolation` is two integer arguments: the actor object and its `Actor` witness
   table; nil is `(0, 0)`. The callee hops to that actor itself at entry
   (`swift_task_switch(ctx, resume, executor of the isolation)`), so a caller only has to pass the value.
   MainActor: `MainActor.shared` and the conformance `$sScMScAsWP`.
2. An actor's synchronous method is a plain `swiftcc` function (`(args…, swiftself actor)`), callable through the
   existing sync path. A caller outside the actor hops first: `callBump` does
   `swift_task_switch(ctx, resume, actor, 0)`, the default-actor executor `(actor, 0)`, calls, and hops back.
   Custom-executor actors give their executor through `Actor.unownedExecutor`.
3. Typed throws differs from untyped throws:
   - concrete error type (`throws(Boom)`): the error **value** comes back in the result registers and the
     `swift_context` register is a flag (`1` = threw, `null` = returned). Untyped throws puts the error box there.
   - generic error type (`throws(E)`): the caller passes one more pointer, after the generic arguments: the slot the
     callee writes the error into; the flag is the same.
4. `AsyncIteratorProtocol.next(isolation:)` (macOS 15, dispatch thunk
   `$sScI4next9isolation7ElementQzSgScA_pSgYi_tYa7FailureQzYKFTj`, async function pointer `…FTjTu`) is called as
   `(indirect Element? result, ctx, isolation.0, isolation.1, error slot, swiftself iterator address,
   Self metadata, AsyncIteratorProtocol witness table)`: six integer arguments, which fit the generic call shape on
   arm64 (8) and x86_64 (6).

## Consequences for apus

- `#isolation`: an isolation value the Rust side computes from the current Task (nil, or MainActor for main tasks).
- Actor sync calls: a call path in the funclets that hops to the actor's executor, runs the sync call, hops back.
- Typed throws: `RetRegs.error` is a flag; the error is the result registers (concrete) or a slot the caller provides
  (generic).
- `SwiftStream` can use `next(isolation:)` where present (an optional macOS 15 symbol) and keep `next()` otherwise.

5. `nonisolated(nonsending)` (Swift 6.2) async functions take a hidden isolation argument (two words) **before** their
   own arguments: `inherits(_ x: Int)` is `(ctx, isolation.0, isolation.1, x)`, while a plain async function is
   `(ctx, x)`. The mangled name does not show the difference (`$s2NS8inheritsyS2iYaF` vs `$s2NS5plainyS2iYaF`), so
   only the declaration can say which one a symbol is (`nonsending.swift`).
