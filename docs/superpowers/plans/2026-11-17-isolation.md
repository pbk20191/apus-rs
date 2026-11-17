# Isolation, actor sync calls, typed throws, next(isolation:) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Call Swift 6 isolation-aware APIs from Rust: `isolated` parameters (`#isolation`), actor / `@MainActor` sync members, generic typed throws, `next(isolation:)`, `nonisolated(nonsending)`.

**Architecture:** The mangler learns `isolated`, `(actor)`, `@MainActor`, `nonisolated`, `throws(E)`, `nonisolated(nonsending)` and reports a call kind to the macro. Isolation registers are filled when a call actually runs (funclet for async calls, Rust for sync), so child Tasks get nil. Actor hops go through `swift_task_switch` with the executor from `Actor.unownedExecutor`. Typed throws only for generic/associated error types (error slot after the generic arguments).

**Tech Stack:** Rust, C funclets (clang swiftcall/swiftasynccall), apus-mangle, apus-macros, swiftc probes.

**Spec:** `docs/superpowers/specs/2026-11-17-isolation-design.md` (Korean); spike `docs/superpowers/specs/spikes/m5-isolation/NOTES.md`.

## Global Constraints

- Only public ABI entry points for hops: `swift_task_switch`, `Actor.unownedExecutor` (`$sScA15unownedExecutorScevgTj`), `swift_conformsToProtocol` with `$sScAMp`, `swift_getObjectType`.
- `Isolation` is created only by `Isolation::current()`; it carries no actor. MainActor only when the call runs in the same Task and that Task is a main task; nil otherwise (child Tasks included).
- Concrete-error `throws(E)` declarations are a compile error with the spec's message; generic/associated error types only.
- `nonisolated(nonsending)` changes no mangling; the hidden isolation goes after an indirect result pointer and before the formal arguments.
- New runtime declarations go in `sys/include/apus/abi.h` with markers; regenerate with `APUS_BLESS=1`; `lowering`/`fresh`/`symbols` stay green.
- Tests: mangler corpus additions + `tests/swift_isolation.rs` (one file); existing suite stays green (98, 99 with objc2).
- Commits on main locally with the trailer; push only when the owner asks.

## Review Focus

1. A call with `Isolation` run via `.child()`/`join!` from a main task: the callee must see nil — Task 1 test.
2. Actor sync call from a main task returns on the main actor (hop back to the Task's executor) — Task 2 test.
3. A typed-throws generic call that does not throw must not read the error slot — Task 3 test.
4. `SwiftStream` over an `AsyncThrowingStream` that throws: same `SwiftError` description as before on macOS 15 — Task 4 test.
5. A `nonisolated(nonsending)` function receives its own argument unshifted — Task 5 test.

---

### Task 1: `isolated` parameters and `Isolation`

**Files:** `mangle/src/{ast,parse,mangler}.rs`, `mangle/tests/corpus.{rs,swift}`, `sys/include/apus/abi.h` (+ regenerated), `sys/csrc/funclets.c`, `src/swift/{func,call,sync}.rs`, new `src/swift/isolation.rs`, `src/swift/mod.rs`, `tests/support/fns.swift`, new `tests/swift_isolation.rs`, `Cargo.toml` ([[test]]).

**Interfaces:** `Param.isolated: bool`; header `CallRegs` gains `int32_t isolation` (first of the two gpr slots, -1 none); C export `apus_main_actor_isolation(const void **out /*[2]*/)` (boundary, C-defined); Rust `pub struct Isolation` (`Isolation::current()`), `impl SwiftArg for Isolation`.

- [ ] Corpus: add `withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int` (corpus.swift + DECLARATIONS). Run `cargo test -p apus-mangle` → FAIL (parse error on `isolated`). Implement parse (`isolated` before the type, `= #isolation` default skipped) and mangling (`Yi` after the parameter's type). → PASS.
- [ ] Probe `withIsolation` (returns `x + 1000` when isolation is non-nil) and test cases: main task same-Task → 1005; plain task → 5; main task `.child()` → 5. Run → FAIL (no `Isolation`). Implement: `SwiftArg for Isolation` reserves two gpr and records `regs.isolation`; funclet `step` fills them (main task → `apus_main_actor_isolation`, else 0,0) before `CALL_GENERIC`; `child_entry` fills 0,0; sync `invoke` fills from the current Rust task's main flag. → PASS.
- [ ] Commit "isolated parameters: Isolation filled when the call runs".

### Task 2: actor and `@MainActor` sync members

**Files:** mangle (`Kind::Actor`, attributes `@MainActor`, `nonisolated`; `call_kind` in `apus_mangle` and a `swift_call_kind!` proc macro), `src/swift/macros.rs`, `sys/include/apus/abi.h` (`swift_getObjectType` C, `$sScAMp` data, `$sScA15unownedExecutorScevgTj` thunk-or-direct per lowering), funclets (hop path), `src/swift/{call,sync}.rs`, probes (`Tally` actor with `bump(by:)`, a `count: Int { get }`, a `nonisolated func label() -> String`, `@MainActor func mainOnly() -> Bool` returning `Thread.isMainThread`, a custom-executor actor), tests.

- [ ] Corpus cases for actor members and `@MainActor`/`nonisolated` declarations (mangling unchanged from class/free function). → FAIL then PASS.
- [ ] Tests: `bump` from plain and main tasks (main task: `is_main_thread()` true after the await), `count`, `label` callable sync, `mainOnly` true from a plain task, custom-executor actor runs on its executor (Swift checks `assumeIsolated`-style flag). → FAIL; implement hop record + funclet `hop_to`/`after_hop` (same-Task and child), executor lookup, macro routing (Rust sync decl of an isolated member → `compile_error!`). → PASS.
- [ ] Commit "actor and @MainActor sync members: hop to the executor and back".

### Task 3: typed throws (generic errors)

**Files:** mangle (`throws(E)` → `YK`; `throws(any Error)` → `K`; concrete E → error from `mangle`/macro), `src/swift/{call,func,sync}.rs` (error slot + `CallError::Typed(SwiftValue)`), probes (`genericTyped<E: Error>(_: E?) async throws(E) -> Int`, sync variant), tests.

- [ ] Corpus `throws(E)` generic + `throws(any Error)`. Concrete `throws(Boom(struct))` → macro error text from the spec (compile-fail checked via a `mangle` unit test returning Err). → FAIL then PASS.
- [ ] Tests: throws → `CallError::Typed` with the value; returns → `Ok(42)` and the slot untouched. → FAIL; implement → PASS.
- [ ] Commit "typed throws for generic error types".

### Task 4: `SwiftStream` on `next(isolation:)`

**Files:** `sys/include/apus/abi.h` (dispatch thunk async pointer `…FTjTu` as APUS_AVAILABLE(15.0) data, `swift_allocError`, `swift_getAssociatedConformanceWitness`, `Failure` descriptors), `src/swift/stream.rs`, tests.

- [ ] Test: stream over `AsyncThrowingStream` that yields 1,2 then throws → same values and `SwiftError` description as `next()`; over `AsyncStream` → values. Record which path ran (test-only counter in apus behind `cfg(test-probe)` or by symbol presence). → FAIL (path not taken); implement → PASS.
- [ ] Commit "SwiftStream uses next(isolation:) on macOS 15".

### Task 5: `nonisolated(nonsending)`

**Files:** mangle (attribute parsed, `CallKind` reports it), macro (prepends `Isolation::current()`), probes (`inherits(_ x: Int) async -> Int` nonsending, returns x+1 and records isolation non-nil), README warning.

- [ ] Corpus case (same symbol as plain) + test (`inherits(41)` → 42 from main task and plain task). → FAIL; implement → PASS.
- [ ] README: isolation section (Isolation, actor calls, typed throws limits, nonsending warning). Commit "nonisolated(nonsending) declarations".
