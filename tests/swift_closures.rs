//! Rust closures passed to Swift as Swift closures: async and sync, concrete and generic parameter types, @MainActor,
//! throwing, called concurrently, cancelled, kept and released by Swift.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::is_main_thread;
use apus::swift::{cancelled, spawn, ByAddress, CallError, SwiftClosure, SwiftError, SwiftValue};
use common::{block, check, Failed, Trial};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Arc;

apus::swift! {
    #[swift_linked = "Probe.applyConcrete(_: (Int) async -> Int) async -> Int"]
    async fn apply_concrete(f: &SwiftClosure) -> i64;
    #[swift_linked = "Probe.applyGeneric<T>(_: T, _: (T) async -> T) async -> T"]
    async fn apply_generic(x: &SwiftValue, f: &SwiftClosure) -> SwiftValue;
    #[swift_linked = "Probe.applySync(_: (Int) -> Int) -> Int"]
    fn apply_sync(f: &SwiftClosure) -> i64;
    #[swift_linked = "Probe.applySyncDouble(_: (Double, Int) -> Double) -> Double"]
    fn apply_sync_double(f: &SwiftClosure) -> f64;
    #[swift_linked = "Probe.applySyncGeneric<T>(_: T, _: (T) -> T) -> T"]
    fn apply_sync_generic(x: &SwiftValue, f: &SwiftClosure) -> SwiftValue;
    #[swift_linked = "Probe.applyMain(_: @MainActor (Int) async -> Int) async -> Int"]
    async fn apply_main(f: &SwiftClosure) -> i64;
    #[swift_linked = "Probe.applyEscaping(_: @escaping @Sendable (Int, String) async throws -> String) async throws -> String"]
    async fn apply_escaping(f: &SwiftClosure) throws -> String;
    #[swift_linked = "Probe.applyCancelled(_: @escaping @Sendable () async -> Bool) async -> Bool"]
    async fn apply_cancelled(f: &SwiftClosure) -> bool;
    #[swift_linked = "Probe.keep(_: @escaping @Sendable () -> Int)"]
    fn keep(f: &SwiftClosure);
    #[swift_linked = "Probe.callKept() -> Int"]
    fn call_kept() -> i64;
    #[swift_linked = "Probe.dropKept()"]
    fn drop_kept();
    #[swift_linked = "Probe.addAsync(_: Int, _: Int) async -> Int"]
    async fn add_async(a: i64, b: i64) -> i64;
    #[swift_linked = "Probe.throwStruct(code: Int) throws -> Int"]
    fn throw_struct(code: i64) throws -> i64;
}

fn async_closures() -> Result<(), Failed> {
    let (concrete, generic, main) = block(spawn(async {
        // The closure awaits Swift itself, in the caller's Task and in a child Task.
        let f = SwiftClosure::new_async(|x: i64| async move { add_async(x, 1).await.unwrap() + add_async(x, 0).child().await.unwrap() });
        let concrete = apply_concrete(&f).await;
        let g = SwiftClosure::new_async(|x: ByAddress<i64>| async move { ByAddress(x.0 * 3) });
        let generic = apply_generic(&SwiftValue::new(14i64), &g).await.map(|v| v.get::<i64>());
        let m = SwiftClosure::new_async(|x: i64| async move { x + if is_main_thread() { 100 } else { 0 } }).on_main_actor();
        let main = apply_main(&m).await;
        (concrete, generic, main)
    }))
    .unwrap();
    check(matches!(concrete, Ok(42)), format!("concrete {concrete:?}"))?;
    check(matches!(generic, Ok(Some(42))), format!("generic {generic:?}"))?;
    check(matches!(main, Ok(101)), format!("@MainActor {main:?}"))
}

fn sync_closures() -> Result<(), Failed> {
    let f = SwiftClosure::new_sync(|x: i64| x + 4);
    let n = apply_sync(&f);
    check(matches!(n, Ok(70)), format!("sync Int {n:?}"))?;
    let d = SwiftClosure::new_sync(|x: f64, n: i64| x * n as f64);
    let result = apply_sync_double(&d);
    check(matches!(result, Ok(r) if r == 3.0), format!("sync Double {result:?}"))?;
    let g = SwiftClosure::new_sync(|x: ByAddress<i64>| ByAddress(x.0 + 1));
    let result = apply_sync_generic(&SwiftValue::new(41i64), &g).map(|v| v.get::<i64>());
    check(matches!(result, Ok(Some(42))), format!("sync generic {result:?}"))
}

/// Called twice at once (`async let`); the second call rethrows a Swift error.
fn throwing_and_concurrent() -> Result<(), Failed> {
    let (fine, thrown) = block(spawn(async {
        let ok = SwiftClosure::new_async(|x: i64, s: String| async move { Ok::<_, SwiftError>(format!("{s}{x}")) });
        let fine = apply_escaping(&ok).await;
        let failing = SwiftClosure::new_async(|x: i64, s: String| async move {
            match x {
                2 => match throw_struct(7) {
                    Err(CallError::Swift(error)) => Err(error),
                    other => panic!("throwStruct: {other:?}"),
                },
                _ => Ok(s),
            }
        });
        (fine, apply_escaping(&failing).await)
    }))
    .unwrap();
    check(matches!(&fine, Ok(s) if s == "a1b2"), format!("both calls {fine:?}"))?;
    check(matches!(&thrown, Err(CallError::Swift(e)) if e.description().contains("code: 7")), format!("rethrown {thrown:?}"))
}

/// The closure runs in Swift's Task, so cancelling that Task reaches the Rust future.
fn cancellation() -> Result<(), Failed> {
    let f = SwiftClosure::new_async(|| async {
        cancelled().await;
        true
    });
    let seen = block(spawn(async move { apply_cancelled(&f).await }));
    check(matches!(seen, Ok(Ok(true))), format!("{seen:?}"))
}

/// Swift keeps its own reference: the Rust closure lives until Swift and Rust have both let go.
fn kept_by_swift() -> Result<(), Failed> {
    struct Flag(Arc<AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, SeqCst);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = Flag(dropped.clone());
    let f = SwiftClosure::new_sync(move || if flag.0.load(SeqCst) { -2 } else { 5 });
    keep(&f).unwrap();
    drop(f);
    check(matches!(call_kept(), Ok(5)), "called after Rust let go")?;
    check(!dropped.load(SeqCst), "dropped while Swift held it")?;
    drop_kept().unwrap();
    check(dropped.load(SeqCst), "not dropped after Swift let go")
}

/// Marking a closure `@MainActor` after Swift already holds it would change it under Swift: refused.
fn main_actor_only_before_sharing() -> Result<(), Failed> {
    let f = SwiftClosure::new_sync(|| 1i64);
    keep(&f).unwrap();
    let marked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(f.on_main_actor())));
    drop_kept().unwrap();
    check(marked.is_err(), "marked while Swift held it")
}

/// A closure whose shape does not match the declared function type is refused before the call.
fn mismatched_shape() -> Result<(), Failed> {
    let sync = SwiftClosure::new_sync(|x: i64| x);
    let result = block(spawn(async move { apply_concrete(&sync).await })).unwrap();
    check(matches!(result, Err(CallError::ClosureMismatch(_))), format!("sync for async: {result:?}"))?;
    let direct = SwiftClosure::new_async(|x: i64| async move { x });
    let result = block(spawn(async move { apply_generic(&SwiftValue::new(1i64), &direct).await })).unwrap();
    check(matches!(result, Err(CallError::ClosureMismatch(_))), format!("direct for generic: {result:?}"))
}

fn main() {
    common::run(vec![
        Trial::test("async_closures", async_closures),
        Trial::test("sync_closures", sync_closures),
        Trial::test("throwing_and_concurrent", throwing_and_concurrent),
        Trial::test("cancellation", cancellation),
        Trial::test("kept_by_swift", kept_by_swift),
        Trial::test("mismatched_shape", mismatched_shape),
        Trial::test("main_actor_only_before_sharing", main_actor_only_before_sharing),
    ]);
}
