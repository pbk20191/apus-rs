# Ergonomics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut what a Rust program needs to use its own Swift module and a Swift-only framework.
The work has five parts:
- `apus-build`
- `swift_type!` / `swift_protocol!`
- implicit class `self`
- concrete generic binding
- rust-analyzer settings

**Architecture:**
- Compile-time work goes in `apus-mangle`, which parses, mangles and plans. `apus-macros` is the bridge: proc macros and
  `__swift_route` markers.
- Run-time work goes in `src/swift`: lazily resolved names, and receiver and argument wrappers.
- `apus-build` is a std-only build-script helper.
- At the end, the SwiftData example is rewritten on top of all of it.

**Tech Stack:** Rust 2021, proc_macro (no syn), the Swift runtime through apus-sys, and `xcrun swiftc`.

**Spec:** `docs/superpowers/specs/2026-11-20-ergonomics-design.md`

## Global Constraints

- **ABI paths:** use only these, all already in use:
  - names mangled per `docs/ABI/Mangling.rst`, resolved with `swift_getTypeByMangledNameInContext` (`SwiftType::from_mangled_name`)
  - protocol descriptors `…Mp` with `swift_conformsToProtocol` (`SwiftType::conformance`)
  - class metadata as swiftself
  - generic arguments after the formal arguments: metadata first, then witness tables
- **Errors:** refuse at compile time where possible. At run time a missing name is `CallError::Unavailable(name)`, and a
  protocol-layout mismatch is the existing `InvalidArgument`.
- **Tests:** few, each seen failing first. Full suite: `cargo test --workspace --features dispatch,test-probe,stream,block2`;
  dispatch-only build: `cargo test --no-default-features --features dispatch`. Zero warnings.
- **Language:** code, comments and commits in English. Follow the existing comment density and style (doc comments that
  name the ABI rule relied on).
- **Commits:** end every commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Commit locally;
  do not push.
- **apus-build:**
  - macOS only, with no dependencies.
  - Flags: `-enable-library-evolution` by default, `-O` in release, `-Onone` otherwise.
  - Target: `-target {arch}-apple-macos{MACOSX_DEPLOYMENT_TARGET}`.

## Review Focus

1. **A concrete-bound parameter of bare type `T` (not class-bound) is passed by address by Swift.**
   - A Rust `i64` there would go in a register. Expect a compile error unless it is `&SwiftValue`.
   - The same holds for a bare `-> T` result: it must be `SwiftValue`.
   - Test: compile_fail doctest in Task 4.
2. **`T` nested in a lowering Swift abstracts**, such as `T?`, `(T, Int)` or `(T) -> Int`, under concrete binding. Expect
   a compile error. Inside a nominal type's arguments (`FetchDescriptor<T>`) it is fine.
   - Test: `bind` unit test in Task 3.
3. **A bound type that does not exist at run time, or does not conform to the protocol:**
   - a missing type gives `CallError::Unavailable`
   - a type that does not conform gives `CallError::Unavailable` with the protocol named, not a crash
   - Test: runtime test in Task 4 binding `T = "Res.Nope(struct)"`.
4. **A generic class's initializer with `self` omitted** (`Res.Box(class)<T>.init`) has no single name. Expect the existing
   "declare `self: &SwiftType`" compile error.
   - Test: compile_fail doctest in Task 2.
5. **A missing `MACOSX_DEPLOYMENT_TARGET` in apus-build.** Expect a panic naming `.cargo/config.toml` `[env]`, not a
   swiftc error.
   - Test: checked by hand in Task 5, with the output pasted in the commit message.

---

### Task 1: `swift_type!` / `swift_protocol!`

**Files:**
- Modify: `mangle/src/lib.rs` (new pub fns next to `result_type`), `mangle/src/mangler.rs` (`protocol_descriptor` is already there)
- Modify: `macros/src/lib.rs` (two proc macros), `src/lib.rs` (re-export next to `mangle!`)
- Test: `mangle/tests/corpus.rs`, `mangle/tests/corpus.swift`

**Interfaces:**
- Produces:
  - `apus_mangle::type_name(ty: &str) -> Result<String, Error>`: no `$s`; the runtime-lookup form.
  - `apus_mangle::protocol_descriptor_of(protocol: &str) -> Result<String, Error>`: with `$s`, ending `Mp`.
  - Macros `apus::swift_type!("…")` and `apus::swift_protocol!("…")`, each expanding to a `&'static str` literal.

- [ ] **Step 1: Failing tests.** Add `mangle/tests/corpus.rs::type_names_match_swiftc` asserting:
  - `type_name("Todos.Todo(class)") == "5Todos4TodoC"`
  - `type_name("SwiftData.FetchResultsCollection(struct)<Todos.Todo(class)>") == "9SwiftData22FetchResultsCollectionVy5Todos4TodoCG"`
  - `type_name("[String]") == "SaySSG"`
  - `protocol_descriptor_of("SwiftData.PersistentModel") == "$s9SwiftData15PersistentModelMp"`
  - `protocol_descriptor_of("Swift.Hashable") == "$sSHMp"`
  - `type_name("Todos.Todo")` is an error, because the kind is missing.

  Then cross-check one probe type the same way the corpus does: add a `corpus.swift` case whose emitted type metadata
  symbol contains the name.
- [ ] **Step 2:** Run `cargo test -p apus-mangle --test corpus type_names`. It should FAIL because the functions do not
  exist yet.
- [ ] **Step 3: Implement `type_name`.** Parse the synthetic declaration `"{first segment}.__apus_type() -> {ty}"`, then go
  through the existing `result_type` path. If the type is not concrete, return the error "the type must be concrete".
- [ ] **Step 4: Implement `protocol_descriptor_of`.** Parse the synthetic `"{module}.__apus_p<T: {protocol}>(_: T)"` and take
  the protocol of its one requirement, or call `Parser::protocol_name` directly if it can be reached. Then call
  `mangler::protocol_descriptor`.
- [ ] **Step 5: Add the proc macros** `swift_type` and `swift_protocol` in `macros/src/lib.rs`, modeled on `mangle`, with one
  string literal. On error, emit `compile_error!("cannot mangle `…`: {error}")`. Re-export them from `apus` with doc
  comments that show `SwiftType::from_mangled_name(apus::swift_type!(…))`. Add one doctest asserting
  `apus::swift_type!("Todos.Todo(class)") == "5Todos4TodoC"`.
- [ ] **Step 6:** Run `cargo test -p apus-mangle` and `cargo test --doc --features test-probe swift_type`. Both should PASS.
- [ ] **Step 7:** Commit: `swift_type! / swift_protocol!: mangled names from Swift syntax`.

### Task 2: Class initializers and static members without `self`

**Files:**
- Modify: `mangle/src/lib.rs` (`Declaration::context_type_name`), `macros/src/lib.rs` (`__swift_route` Thick branch, around line 212)
- Modify: `src/swift/macros.rs` (`__swift_item` arms, plus docs that replace the first compile_fail example), `src/swift/func.rs` (receiver)
- Test: `tests/swift_init.rs`

**Interfaces:**
- Produces:
  - `Declaration::context_type_name(&self) -> Option<String>`: the mangled name (no `$s`) of the enclosing class or actor.
    It is `None` when the type is generic or there is none.
  - `#[doc(hidden)] pub struct DeclaredClass(pub &'static str)` in `src/swift/func.rs`, implementing `SwiftReceiver`. Its
    `bind`/`bind_sync` resolve the name once per name (a `static` `OnceLock` per macro expansion is fine, through
    `__declared_class(&'static OnceLock<Option<SwiftType>>, &'static str)`) and `on_type` it. On failure,
    `lowering.fail(CallError::Unavailable(name))` (async: through `configure()`).
  - Route marker: when TypeSelf is Thick, there is no receiver, and `context_type_name()` is `Some(n)`, `__swift_route`
    replaces the empty receiver group `[]` with `[this : $crate::swift::DeclaredClass]`. It also adds a `classself "n"`
    marker, so `__swift_item` declares no such parameter and binds `&DeclaredClass("n")` instead. Pick whichever of these
    two shapes keeps the `__swift_item` arms smallest; both are fine.

- [ ] **Step 1: Failing test.** In `tests/swift_init.rs`, declare
  `#[swift_linked = "Probe.Minted(class).init(n: Int)"] fn minted_implicit(n: i64) -> SwiftObject;` and
  `#[swift_linked = "static Probe.Minted(class).twice(_: Int) -> Minted(class)"] fn twice_implicit(n: i64) -> SwiftObject;`.
  Assert that `value(&minted_implicit(7).unwrap()) == 7` and `value(&twice_implicit(4).unwrap()) == 8`.
  Add a `compile_fail` doctest in `src/swift/macros.rs`: `#[swift = "Res.Box(class)<T>.init(_: T)"] fn new_box(x: &SwiftValue) -> SwiftObject;`
  is refused because the class is generic. Use a generic class the parser accepts; check `resilient.swift` for an existing
  one, or write the declaration without a probe, since a compile_fail doctest needs no symbol.
- [ ] **Step 2:** Run `cargo test --features test-probe --test swift_init`. It should FAIL to compile with "takes its class
  as `self`".
- [ ] **Step 3: Implement `context_type_name`** by mangling `Ty::Nominal` of `decl.context`, reusing the `type_name`
  machinery from Task 1. Return None if `decl.context_params > 0`.
- [ ] **Step 4: Implement the route change, `DeclaredClass`, and the `__swift_item` binding.** Keep the explicit
  `self: &SwiftType` path unchanged. The error message for the generic case becomes: "`…` belongs to a generic class:
  declare `self: &SwiftType` first, the specialized class's metadata".
- [ ] **Step 5:** Run `cargo test --features test-probe --test swift_init`, then the doctests (`cargo test --doc --features test-probe`).
  Both should PASS.
- [ ] **Step 6:** Update the `swift!` doc bullet "An initializer … takes the type's metatype as `self`…". The new text: `self`
  may be omitted for a non-generic class; the README `swift!` section says the same. Then commit:
  `swift!: a class's initializers and static members need no self`.

### Task 3: Concrete generic binding in apus-mangle

**Files:**
- Modify: `mangle/src/lib.rs` (`Declaration::bind`, `ConcretePlan`)
- Test: unit tests in `mangle/src/lib.rs` next to `generic_plan_gives_metadata_sources_and_witness_order`

**Interfaces:**
- Consumes: `type_name` (Task 1), `mangler::sorted_requirements`, `mangler::protocol_descriptor`.
- Produces:

```rust
pub struct ConcretePlan {
    /// Per generic parameter, in order: the mangled name (no `$s`) of the type it is bound to.
    pub types: Vec<String>,
    /// Witness tables in canonical order: (generic parameter index, protocol descriptor, layout).
    pub witnesses: Vec<(usize, String, ProtocolLayout)>,
    /// The result type with the bindings substituted, when it is concrete then.
    pub result: Option<String>,
    /// Per formal parameter: true when its type is exactly a non-class-bound generic parameter (passed by address:
    /// the Rust argument must be `&SwiftValue`).
    pub by_address: Vec<bool>,
    /// The result is exactly a non-class-bound generic parameter (returned by address: `SwiftValue`).
    pub result_by_address: bool,
}
impl Declaration { pub fn bind(&self, bindings: &[(&str, &str)]) -> Result<ConcretePlan, Error>; }
```

  The parser must keep generic parameter names. They are in `Parser::generics`; carry `(name, GenericParam)` into
  `ast::Decl` as `generic_names: Vec<String>` if the Decl lacks them.

- [ ] **Step 1: Failing test** `bind_fixes_generic_parameters`.
  - Bind `fetch` from the spec with `[("T", "Todos.Todo(class)")]`. Assert:
    - `types == ["5Todos4TodoC"]`
    - `witnesses == [(0, "$s9SwiftData15PersistentModelMp", Class)]`
    - `result == Some("9SwiftData22FetchResultsCollectionVy5Todos4TodoCG")`
    - `by_address == [false, false]`
  - Bind `"Res.zeroOf<T: AdditiveArithmetic>() -> T"` with `T = "Swift.Int"`. Assert `types == ["Si"]`,
    `result == Some("Si")` and `result_by_address`.
  - Each of these is an error, with the message naming the cause:
    - a missing binding (`[]`)
    - an unknown name (`("U", "Swift.Int")`)
    - a generic type that is not concrete (`("T", "Swift.Array<T>")`)
    - `"M.f<T>(_: T?) -> Int"`, where `T` is nested where Swift abstracts it (Review Focus 2)
    - more than one protocol on a parameter
- [ ] **Step 2:** Run `cargo test -p apus-mangle bind_fixes`. It should FAIL because `bind` does not exist.
- [ ] **Step 3: Implement `bind`.**
  - Reuse `generic_plan_of`'s requirement walk and its refusals, factored into a shared helper so both stay in step. The
    witness index is the generic parameter's, not a formal argument's.
  - Get the substituted result by replacing `Ty::Param` with the parsed bound type before `canonical` and the mangling.
  - Allow `T` only at a parameter's or result's top level, or among a nominal type's arguments. Refuse it elsewhere.
- [ ] **Step 4:** Run `cargo test -p apus-mangle`. It should PASS.
- [ ] **Step 5:** Commit: `apus-mangle: bind a declaration's generic parameters to concrete types`.

### Task 4: `#[swift = "…", T = "…"]` in `swift!`

**Files:**
- Modify: `src/swift/macros.rs`:
  - `swift!` arms accept `#[swift = $spec:literal $(, $param:ident = $bound:literal)*]`, and the same for `swift_linked`.
  - `__swift_fn` passes the bindings through.
  - `__swift_call` gets a new `concrete` arm.
- Modify: `macros/src/lib.rs` (`__swift_route` reads the bindings, calls `bind`, emits the marker and the Rust-type checks),
  `src/swift/generic.rs` (`__concrete_args`)
- Modify: `tests/support/resilient.swift` (probe), `tests/swift_generic_auto.rs` (test)

**Interfaces:**
- Consumes: `Declaration::bind` and `ConcretePlan` (Task 3).
- Produces:
  - Marker `concrete [$("type")*] [$(($index "protocol" Layout))*] [$("result")?]`. In `__swift_call` it builds the
    generic argument with `__concrete_args` and passes the result name as the declared result name: a `DeclaredResult`
    with `name: Some(result)`.
  - `#[doc(hidden)] pub fn __concrete_args(types: &'static [(&'static str, &'static OnceLock<Option<SwiftType>>)], witnesses: &[(usize, &'static SwiftProtocol, ProtocolLayout)]) -> ConcreteArgs`.
    `ConcreteArgs: SwiftArg` lowers as `GenericArgs`. When a type is missing it calls
    `into.fail(CallError::Unavailable(name))`; when a conformance is missing it fails with `Unavailable` of the protocol
    symbol. Any resolution shape is fine as long as each name is resolved once.
- Route checks are compile errors:
  - a `by_address` parameter whose Rust type is not `&SwiftValue`
  - `result_by_address` with a Rust return other than `SwiftValue`
  - bindings on a declaration that is not generic
  - bindings with `#[symbol]`/`#[linked]`
  - mixing bindings with explicit generic arguments: the Rust signature must list exactly the formal parameters

- [ ] **Step 1: Probes.** Add to `resilient.swift`:
  - `public func zeroText<T: AdditiveArithmetic>(_ label: String) -> String { "\(label)\(T.zero)" }`
  - `public func zeroOf<T: AdditiveArithmetic>() -> T { .zero }`
  - `public struct Nope {}`, plus `public func nopeType() -> Any.Type { Nope.self }` so it is not stripped. Nope does not
    conform to AdditiveArithmetic.
- [ ] **Step 2: Failing tests** in `tests/swift_generic_auto.rs`, `concrete_bindings`:
  - `#[swift = "Res.zeroText<T: AdditiveArithmetic>(_: String) -> String", T = "Swift.Double"] fn zero_double(label: &str) -> String;`
    should give `"d0.0"` for `zero_double("d")`.
  - `#[swift = "Res.zeroOf<T: AdditiveArithmetic>() -> T", T = "Swift.Int"] fn zero_int() -> SwiftValue;` should give
    `zero_int().unwrap().ty() == SwiftType::of::<i64>()`.
  - The same declaration with `T = "Res.Nope(struct)"` should give `Err(CallError::Unavailable(_))`.

  Add `compile_fail` doctests in `src/swift/macros.rs`:
  - `fn zero_int() -> i64` with `T = "Swift.Int"`: the generic result is returned by address.
  - A missing binding, written as `#[swift = "Res.zeroOf<T: AdditiveArithmetic>() -> T"]` with no `T`, and an empty Rust
    signature. Expect the existing explicit-generic-arguments error.
  - An unknown name (`U = "Swift.Int"`).
- [ ] **Step 3:** Run `cargo test --features test-probe --test swift_generic_auto`. It should FAIL because the
  `#[swift = …, T = …]` arm does not exist.
- [ ] **Step 4: Implement** the macro arms, the route, `__concrete_args`, and the `concrete` arm of `__swift_call`.
  - Class-bound parameters keep their `ClassBound` wrapper.
  - The closure, consumed and wrapped markers still compose. The `concrete` marker sits where `auto` does.
- [ ] **Step 5:** Run `cargo test --features test-probe --test swift_generic_auto`, then the doctests. Both should PASS.
- [ ] **Step 6: Docs.** Add a `swift!` doc bullet and a README section on binding generic parameters. Then commit:
  `swift!: generic parameters bound to concrete types`.

### Task 5: `apus-build`, rust-analyzer settings, and the example rewritten

**Files:**
- Create: `builder/Cargo.toml` (package `apus-build`, `publish = false`, no deps), `builder/src/lib.rs`
- Create: `.vscode/settings.json` (exact content from the spec)
- Modify: root `Cargo.toml` (members gain `"builder"`), `examples/swift-data/{Cargo.toml,build.rs,src/main.rs}`, `README.md`, `CLAUDE.md` (layout row for `builder/`)

**Interfaces:**
- Produces:

```rust
pub struct SwiftModule { /* name, sources, library_evolution: bool, flags: Vec<String> */ }
impl SwiftModule {
    pub fn new(name: &str, sources: impl AsRef<Path>) -> SwiftModule;  // a directory (its *.swift, sorted) or one file
    pub fn library_evolution(self, on: bool) -> SwiftModule;           // default true
    pub fn flag(self, flag: &str) -> SwiftModule;
    pub fn build(self);                                                // panics with a message on any failure
}
```

  The `build` order:
  1. Emit `rerun-if-changed` for the sources and `rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET`.
  2. Panic unless `CARGO_CFG_TARGET_OS == "macos"`.
  3. Panic if `MACOSX_DEPLOYMENT_TARGET` is unset. Use the apus-sys message, adapted:
     `apus-build: set MACOSX_DEPLOYMENT_TARGET (for example in .cargo/config.toml: [env] MACOSX_DEPLOYMENT_TARGET = "14.0")`.
  4. Run swiftc. Write the output to `OUT_DIR/lib{name}.a`, with library name `name` lowercased.
  5. Emit the link lines and the compat path, as the current example build.rs does.

  Example `src/main.rs` after the rewrite:
  - no `SwiftFn`, no `"Tj"`, no hand-written mangled names
  - `new_todo` and `new_context` with no `self`
  - `fetch` as a `swift!` declaration with `T = "Todos.Todo(class)"`
  - `PersistentModel` and the result type no longer needed in `Store`

- [ ] **Step 1:** Write `builder/` and switch `examples/swift-data/build.rs` to `apus_build::SwiftModule::new("Todos", "swift").build();`,
  with `[build-dependencies] apus-build = { path = "../../builder" }`.
- [ ] **Step 2:** Run `cargo run -p apus-example-swift-data`. Expected output, unchanged from before:
  - all todos: `report 3`, `flights 2`, `bank 2`, `plants 1`
  - open todos: `bank`, `report`
  - after reopening: `report`, `[x] flights`, `bank`
- [ ] **Step 3: Review Focus 5.** Run `env -u MACOSX_DEPLOYMENT_TARGET cargo build -p apus-example-swift-data --config 'env.MACOSX_DEPLOYMENT_TARGET="x"'`,
  or build from a temp copy without `.cargo/config.toml`. Check that the panic message names `.cargo/config.toml`.
  Expect apus-sys to panic first; that is acceptable, and is recorded in the commit message.
- [ ] **Step 4:** Rewrite `examples/swift-data/src/main.rs` using Tasks 1, 2 and 4. Rerun step 2; the output must be the same.
- [ ] **Step 5:** Add `.vscode/settings.json`. Update the README: a section "Using a Swift module of your own", covering
  apus-build, the one `.cargo/config.toml` line, and the example. Update the CLAUDE.md layout.
- [ ] **Step 6:** Run the full suite, `cargo test --workspace --features dispatch,test-probe,stream,block2`, and
  `cargo test --no-default-features --features dispatch`. Both should PASS with 0 warnings.
- [ ] **Step 7:** Commit: `apus-build: one-line Swift modules; the SwiftData example without hand-written names`.
