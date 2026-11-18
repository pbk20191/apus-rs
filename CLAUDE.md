# CLAUDE.md

## Goal

apus exists to integrate Swift-only Apple system frameworks (StoreKit 2, SwiftData, MusicKit, WeatherKit, SwiftUI
hosting, the Swift overlays of Foundation, …) with a Rust program as completely as the Swift ABI allows. Rust owns
the program: its logic and its tasks, which the Swift runtime and libdispatch run (apus keeps no threads of its own;
only the one-time runtime self-check uses two short-lived ones).
Swift is a thin boundary, written only where the language demands it, and every piece of that boundary apus can
remove is progress.

Judge every change by two questions, in this order:
1. **Is it sound by the Swift ABI?** Only what the ABI guarantees on Apple platforms may be relied on.
2. **Does it let a Rust program use more of a Swift-only framework, or with less Swift code?**

## ABI rules (non-negotiable)

- Rely only on: the Swift runtime's exported entry points (`include/swift/Runtime/*.h`), value witnesses, metadata
  and descriptor layouts in `include/swift/ABI/*.h`, documented mangling (`docs/ABI/Mangling.rst`), the Swift calling
  conventions (swiftcall / swiftasynccall, as clang implements them), and symbols frameworks export (including dispatch
  thunks `…Tj`, async function pointers `…Tu`, enum case tags `…WC`). Name the header/declaration mirrored in a comment.
- Accepted by the owner (2026-11-18), though observed rather than documented: a class-bound generic parameter's value
  and a thick metatype (`T.Type`, a class's, `Any.Type`) are passed and returned as one pointer; a struct's or enum's
  metatype is thin (passed as nothing); an `@autoclosure` parameter is passed as the closure it is (2026-11-18). Apple's
  frameworks are compiled this way, so ABI stability fixes it.
- Anything else (runtime internals, compiler-computed layouts of non-resilient types, private SPI, behavior observed
  but not documented) is **not** built on silently: stop and tell the owner what is unguaranteed and the alternatives,
  and let them decide.
- Check mangling and behavior against `swiftc` with a spike before writing code; say what was observed versus what is
  documented.
- Prefer refusing at compile time (`swift!` / `apus-mangle` errors, `compile_fail` doctests) or with a `CallError` over
  any path that could be undefined behavior. A leak is better than an over-release; neither is acceptable when avoidable.
- Ownership follows Swift's conventions: arguments borrowed by default; initializer arguments and a setter's new value
  consumed (SE-0377); `consuming`/`borrowing` override; consumed things are handed over only when the call starts
  (`Owned::Pending`). `inout` and a struct's `inout self` borrow a Rust value, so synchronous calls only.

## Layout

| Path | What |
|---|---|
| `src/swift/` | the `swift` backend: tasks (`task.rs`), Swift calls (`call.rs` async, `sync.rs` sync, `func.rs` argument lowering), values (`value.rs`, `any.rs`, `enums.rs`), closures, streams, isolation, `swift!` (`macros.rs`) |
| `src/dispatch/` | the libdispatch backend |
| `sys/` | `apus-sys`: the whole runtime ABI in `sys/include/apus/abi.h`, C funclets/thunks in `sys/csrc/`, bindings generated from the header and committed |
| `mangle/` | `apus-mangle`: Swift declaration → symbol, call kind, ownership, generic plan |
| `macros/` | `swift!`'s routing (`__swift_route`), `mangle!`, `result_type!` |
| `tests/support/*.swift` | Swift probes: `probe.swift`/`fns.swift` (module `Probe`, no library evolution), `resilient.swift` (module `Res`, library evolution, like Apple's frameworks) |
| `docs/superpowers/` | dated specs (Korean), plans and spikes; `docs/prior-art.md` |

README.md documents every user-facing rule; keep it in step with behavior changes.

## Working

- New runtime declarations go in `sys/include/apus/abi.h` with their marker (`APUS_C`, `APUS_DIRECT`, `APUS_THUNK`, …);
  then `APUS_BLESS=1 cargo test -p apus-sys --test fresh`. `lowering` must stay green.
- New declaration syntax: add cases to `mangle/tests/corpus.rs` + `corpus.swift` (enum cases: `corpus_enums.swift`),
  which compare against what `swiftc` emits.
- Tests: few, but each written first and seen failing. Add one where it guards something fragile (an ABI or
  ownership rule, a fixed bug); no mutation runs. Full suite:
  `cargo test --workspace --features dispatch,test-probe,stream,block2` (plain `cargo test --workspace` skips most of it),
  and `cargo test --no-default-features --features dispatch` for the dispatch-only build. Zero warnings.
- Probes: a type nothing references is dead-stripped from the static `Probe` archive (return its metadata from a
  function); tests in one binary run in parallel (no shared counters); never release objects under a lock their
  `deinit` takes.
- Builds need `MACOSX_DEPLOYMENT_TARGET` (set in `.cargo/config.toml`).
- Commit locally with clear messages; push only when the owner asks — the repository is public, so check outgoing
  diffs for local paths and secrets first.

## Where the boundary still needs Swift (the roadmap)

- Implementing Swift protocols in Rust (conformances, witness tables): deferred; not ABI-guaranteed as a whole, no
  public runtime registration. Blocks SwiftUI `View`, delegates, `Codable`, `@Observable`/`@Model` types.
- Parameter packs, a declaration inside several generic enclosing types, non-ASCII names: not mangled yet. `some P` results with primary associated types (`some Collection<Int>`), and more than one per result,
  are refused.
- Compile-time features (macros, property wrappers, result builders) and `@inlinable`/`@_alwaysEmitIntoClient` APIs
  without a symbol cannot be called; they need a Swift shim.
- Small non-resilient values in registers (layout computed by the compiler) and making `indirect` enum cases are
  refused on purpose.
