mod common;

use async_swift_rs::dispatch::{sleep, spawn, spawn_with};
use async_swift_rs::{JoinError, Priority, SpawnOptions};
use common::{block, check, current_qos, Failed, Trial};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn with_priority(priority: Priority) -> SpawnOptions {
    SpawnOptions { priority, ..SpawnOptions::default() }
}

fn spawn_returns_value() -> Result<(), Failed> {
    check(matches!(block(spawn(async { 7 })), Ok(7)), "expected Ok(7)")
}

fn ten_thousand_sleeps_overlap() -> Result<(), Failed> {
    let start = Instant::now();
    let handles: Vec<_> = (0..10_000u64)
        .map(|i| spawn(async move { sleep(Duration::from_millis(50)).await; i }))
        .collect();
    let sum: u64 = handles.into_iter().map(|h| block(h).unwrap()).sum();
    check(sum == 49_995_000, format!("sum {sum}"))?;
    check(start.elapsed() < Duration::from_millis(500), format!("took {:?}", start.elapsed()))
}

fn priority_maps_to_qos() -> Result<(), Failed> {
    let utility = block(spawn_with(with_priority(Priority::Utility), async { current_qos() })).unwrap();
    let initiated = block(spawn_with(with_priority(Priority::UserInitiated), async { current_qos() })).unwrap();
    check(utility == 0x11 && initiated == 0x19, format!("utility 0x{utility:x}, user-initiated 0x{initiated:x}"))
}

fn abort_yields_aborted() -> Result<(), Failed> {
    let h = spawn(async { sleep(Duration::from_secs(10)).await; 1 });
    std::thread::sleep(Duration::from_millis(10));
    let start = Instant::now();
    h.abort();
    let r = block(h);
    check(matches!(r, Err(JoinError::Aborted)), format!("got {r:?}"))?;
    check(start.elapsed() < Duration::from_millis(100), format!("took {:?}", start.elapsed()))
}

fn panic_becomes_join_error() -> Result<(), Failed> {
    let r = block(spawn(async { panic!("boom") }));
    check(matches!(r, Err(JoinError::Panicked(_))), format!("got {r:?}"))
}

fn dropped_handle_detaches() -> Result<(), Failed> {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    drop(spawn(async move { sleep(Duration::from_millis(5)).await; flag.store(true, Ordering::SeqCst) }));
    std::thread::sleep(Duration::from_millis(100));
    check(ran.load(Ordering::SeqCst), "detached task did not run")
}

fn main() {
    common::run(vec![
        Trial::test("spawn_returns_value", spawn_returns_value),
        Trial::test("ten_thousand_sleeps_overlap", ten_thousand_sleeps_overlap),
        Trial::test("priority_maps_to_qos", priority_maps_to_qos),
        Trial::test("abort_yields_aborted", abort_yields_aborted),
        Trial::test("panic_becomes_join_error", panic_becomes_join_error),
        Trial::test("dropped_handle_detaches", dropped_handle_detaches),
    ])
}
