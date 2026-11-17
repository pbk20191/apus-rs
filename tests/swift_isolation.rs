//! Isolation-aware Swift APIs: `isolated` parameters (`#isolation`), actor and @MainActor sync members, typed throws,
//! `nonisolated(nonsending)`.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::is_main_thread;
use apus::swift::{spawn, spawn_on_main, CallError, ErrorSlot, Isolation, SwiftObject, SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int"]
    async fn with_isolation(x: i64, isolation: Isolation) -> i64;

    #[swift_linked = "Probe.isolationKind(isolation: isolated (any Actor)? = #isolation) -> Int"]
    fn isolation_kind(isolation: Isolation) -> i64;

    #[swift_linked = "Probe.makeTally() -> Tally(actor)"]
    fn make_tally() -> SwiftObject;
    #[swift_linked = "Probe.Tally(actor).bump(by: Int) -> Int"]
    async fn bump(self: &SwiftObject, by: i64) -> i64;
    #[swift_linked = "Probe.Tally(actor).count: Int { get }"]
    async fn count(self: &SwiftObject) -> i64;
    #[swift_linked = "nonisolated Probe.Tally(actor).label() -> String"]
    fn label(self: &SwiftObject) -> String;
    #[swift_linked = "@MainActor Probe.onMainThread() -> Bool"]
    async fn on_main_thread() -> bool;
    #[swift_linked = "Probe.makeCustom() -> Custom(actor)"]
    fn make_custom() -> SwiftObject;
    #[swift_linked = "Probe.Custom(actor).onOwnQueue() -> Bool"]
    async fn on_own_queue(self: &SwiftObject) -> bool;

    #[swift_linked = "nonisolated(nonsending) Probe.inherits(_: Int) async -> Int"]
    async fn inherits(x: i64) -> i64;
    #[swift_linked = "nonisolated(nonsending) Probe.inheritsMain() async -> Bool"]
    async fn inherits_main() -> bool;

    #[swift_linked = "Probe.makeBoom(code: Int) -> Boom(struct)"]
    fn make_boom(code: i64) -> SwiftValue;
    #[swift_linked = "Probe.errorCode<E: Error>(_: E) -> Int"]
    fn error_code(error: &SwiftValue, ty: SwiftType, error_witness: WitnessTable) -> i64;
    #[swift_linked = "Probe.genericTyped<E: Error>(_: Bool, _: E) async throws(E) -> Int"]
    async fn generic_typed(fail: bool, error: &SwiftValue, ty: SwiftType, error_witness: WitnessTable, slot: ErrorSlot) -> i64;
    #[swift_linked = "Probe.genericTypedSync<E: Error>(_: Bool, _: E) throws(E) -> Int"]
    fn generic_typed_sync(fail: bool, error: &SwiftValue, ty: SwiftType, error_witness: WitnessTable, slot: ErrorSlot) -> i64;
}

/// A `nonisolated(nonsending)` function gets the caller's isolation as a hidden first argument; its own arguments are
/// not shifted.
fn nonsending_functions() -> Result<(), Failed> {
    let plain = block(spawn(async { (inherits(41).await.unwrap(), inherits_main().await.unwrap()) })).unwrap();
    check(plain == (42, false), format!("plain task: {plain:?}"))?;
    let main = block(spawn_on_main(async { (inherits(41).await.unwrap(), inherits_main().await.unwrap()) })).unwrap();
    check(main == (42, true), format!("main task: {main:?}"))
}

static ERROR: SwiftProtocol = SwiftProtocol::new("$ss5ErrorMp");

/// A generic typed error comes back through the slot the caller passes; a normal return leaves the slot alone.
fn typed_throws_generic() -> Result<(), Failed> {
    let boom = make_boom(7).unwrap();
    let ty = boom.ty();
    let witness = ty.conformance(&ERROR).expect("Boom: Error");
    let code_of = |error: &CallError| match error {
        CallError::Typed(value) => error_code(value, ty, witness).unwrap(),
        other => panic!("expected a typed error, got {other:?}"),
    };
    let b = boom.clone();
    let (ok, thrown) = block(spawn(async move {
        let ok = generic_typed(false, &b, ty, witness, ErrorSlot::of(ty)).await;
        let thrown = generic_typed(true, &b, ty, witness, ErrorSlot::of(ty)).await;
        (ok, thrown)
    }))
    .unwrap();
    check(matches!(ok, Ok(42)), format!("async return: {ok:?}"))?;
    check(thrown.as_ref().err().map(code_of) == Some(7), format!("async throw: {thrown:?}"))?;
    let ok = generic_typed_sync(false, &boom, ty, witness, ErrorSlot::of(ty));
    check(matches!(ok, Ok(42)), format!("sync return: {ok:?}"))?;
    let thrown = generic_typed_sync(true, &boom, ty, witness, ErrorSlot::of(ty));
    check(thrown.as_ref().err().map(code_of) == Some(7), format!("sync throw: {thrown:?}"))
}

/// Sync members of an actor or the main actor run there (the callee traps otherwise), and the task comes back to its
/// own executor afterwards.
fn isolated_sync_members() -> Result<(), Failed> {
    let tally = make_tally().unwrap();
    let t = tally.clone();
    let plain = block(spawn(async move { (bump(&t, 1).await.unwrap(), bump(&t, 2).await.unwrap(), count(&t).await.unwrap()) })).unwrap();
    check(plain == (1, 3, 3), format!("plain task: {plain:?}"))?;
    let t = tally.clone();
    let main = block(spawn_on_main(async move { let n = bump(&t, 1).await.unwrap(); (n, is_main_thread()) })).unwrap();
    check(main == (4, true), format!("main task, back on main: {main:?}"))?;
    let t = tally.clone();
    let child = block(spawn_on_main(async move { let n = bump(&t, 1).child().await.unwrap(); (n, is_main_thread()) })).unwrap();
    check(child == (5, true), format!("child Task: {child:?}"))?;
    check(label(&tally).unwrap() == "tally", "a nonisolated member is called directly")?;
    check(block(spawn(async { on_main_thread().await.unwrap() })).unwrap(), "@MainActor from a plain task runs on main")?;
    let custom = make_custom().unwrap();
    check(block(spawn(async move { on_own_queue(&custom).await.unwrap() })).unwrap(), "a custom executor actor runs on its executor")
}

/// The isolation a call passes is where it actually runs: MainActor in a main task, nil in a plain task and in a child
/// Task (like Swift's `async let`).
fn isolated_parameter() -> Result<(), Failed> {
    let main = block(spawn_on_main(async { with_isolation(5, Isolation::current()).await.unwrap() })).unwrap();
    check(main == 1005, format!("main task: MainActor, got {main}"))?;
    let plain = block(spawn(async { with_isolation(5, Isolation::current()).await.unwrap() })).unwrap();
    check(plain == 5, format!("plain task: nil, got {plain}"))?;
    let child = block(spawn_on_main(async { with_isolation(5, Isolation::current()).child().await.unwrap() })).unwrap();
    check(child == 5, format!("child Task of a main task: nil, got {child}"))?;
    // A sync call runs where it is made.
    let sync_main = block(spawn_on_main(async { isolation_kind(Isolation::current()).unwrap() })).unwrap();
    check(sync_main == 1, format!("sync, main task: MainActor, got {sync_main}"))?;
    let sync_plain = block(spawn(async { isolation_kind(Isolation::current()).unwrap() })).unwrap();
    check(sync_plain == 0, format!("sync, plain task: nil, got {sync_plain}"))?;
    check(isolation_kind(Isolation::current()).unwrap() == 0, "sync, outside a task: nil")
}

fn main() {
    common::run(vec![
        Trial::test("isolated_parameter", isolated_parameter),
        Trial::test("isolated_sync_members", isolated_sync_members),
        Trial::test("typed_throws_generic", typed_throws_generic),
        Trial::test("nonsending_functions", nonsending_functions),
    ]);
}
