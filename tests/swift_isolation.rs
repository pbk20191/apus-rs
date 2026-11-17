//! Isolation-aware Swift APIs: `isolated` parameters (`#isolation`), actor and @MainActor sync members, typed throws,
//! `nonisolated(nonsending)`.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, spawn_on_main, Isolation};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int"]
    async fn with_isolation(x: i64, isolation: Isolation) -> i64;

    #[swift_linked = "Probe.isolationKind(isolation: isolated (any Actor)? = #isolation) -> Int"]
    fn isolation_kind(isolation: Isolation) -> i64;
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
    common::run(vec![Trial::test("isolated_parameter", isolated_parameter)]);
}
