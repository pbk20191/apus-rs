# Generics auto, any P calls, Rust closures Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** `swift!` fills generic arguments itself; `any P` values can be opened and their requirements called; Rust closures become Swift closure values.

**Spec:** `docs/superpowers/specs/2026-11-17-generics-existentials-closures-design.md`; spikes M3 (P1/P2), M6.

## Global Constraints

- ABI only from the declaration: closure conventions, generic argument order (mangler's canonical requirement order), dispatch thunks `Tj` only (no witness-index counting).
- Explicit mode keeps working: a Rust signature with more parameters than the Swift declaration's formal ones is passed as written.
- Unsupported cases are compile errors with a message naming the explicit alternative, never silent.
- One new test file per task; full suite, `lowering`, `fresh`, `symbols` stay green; x86_64 run per task.
- Commits on branch `feat/generics-closures`, trailer as usual; no push.

## Review Focus

1. Auto mode for a generic whose `T` appears only in the result: compile error, not a wrong call — Task 1.
2. A non-conforming value in auto mode: `CallError::NotConforming`, call not made — Task 1.
3. `any P` whose value does not fit the inline buffer (boxed) opens to the right address — Task 2.
4. A Rust async closure called concurrently from Swift (two child tasks) and cancelled — Task 3.
5. A closure passed to a generic parameter (`(T) async -> T`) receives values by address — Task 3.

---

### Task 1: automatic generic arguments
- [ ] Mangler: `apus_mangle::generic_plan(decl) -> Result<GenericPlan, Error>` (per generic parameter: index of the formal parameter of exactly that type; per conformance requirement in canonical order: parameter index and protocol descriptor symbol; typed-throws slot; errors for unsupported requirements). Unit tests first.
- [ ] Route: when Rust arg count == formal count and the declaration is generic, emit auto arms that append `GenericArgs` built at run time (`apus::swift::__generic(&[&dyn HasSwiftType], &[(usize, &SwiftProtocol)], slot)`), with `CallError::NotConforming`.
- [ ] Probe generics (`echo<T>`, `describeIt<T: CustomStringConvertible>`, typed throws generic) + `tests/swift_generic_auto.rs` (auto results equal explicit; non-conforming error; result-only `T` compile error checked by hand). Commit.

### Task 2: `any P` values
- [ ] Probe: a small Swift module built with `-enable-library-evolution` (`tests/support/resilient.swift`, module `Res`) with protocol `Shape { func area() -> Double; func scaled(by: Double) -> any Shape }`, a small and a large (boxed) conforming struct, `makeSmall()/makeLarge() -> any Shape`.
- [ ] `SwiftValue::open() -> Option<Opened>`, `SwiftReceiver for Opened` (self = value address, then Self metadata and witness table after the arguments); test opened values (inline and boxed), calls via `swift!` protocol member declarations. Commit.

### Task 3: Rust closures as Swift closures
- [ ] Mangler: function-type parameters (async, throws, `@escaping`, `@Sendable`, `@MainActor`) — corpus cases first.
- [ ] Header + funclets: closure context heap metadata (HeapLocalVariable), async closure entry (all argument registers + context; runs a Rust task in the caller's Task via `start`; `@MainActor` variant hops first), sync closure entry, return through the generic continuation shape.
- [ ] Rust: `SwiftClosure` (async/sync constructors over `ClosureArgs`/`ClosureReturn`), `SwiftArg for &SwiftClosure` (two words), typed wrappers from `swift!` for function-typed parameters.
- [ ] Probes `applyConcrete`, `applyGeneric`, `applySync`, `applyMain`, `applyEscaping` (throws, called twice concurrently via async let) + `tests/swift_closures.rs`. Commit.
