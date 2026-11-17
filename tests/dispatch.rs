#[path = "common/mod.rs"]
mod common;

use apus::dispatch::{block_on, sleep, spawn, spawn_detached, spawn_on_main, spawn_with, SerialQueue};
use apus::{is_main_thread, run_main};
use apus::{JoinError, Priority, SpawnOptions};
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

fn main_task_runs_on_main_thread() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let before = is_main_thread();
        sleep(Duration::from_millis(5)).await;
        (before, is_main_thread())
    }));
    check(matches!(r, Ok((true, true))), format!("got {r:?}"))
}

fn spawn_inside_main_stays_main() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let inherited = spawn(async { is_main_thread() }).await;
        let detached = spawn_detached(async { is_main_thread() }).await;
        (inherited, detached)
    }));
    check(matches!(r, Ok((Ok(true), Ok(false)))), format!("got {r:?}"))
}

fn priority_inherited() -> Result<(), Failed> {
    let r = block(spawn_with(with_priority(Priority::Utility), async { spawn(async { current_qos() }).await }));
    check(matches!(r, Ok(Ok(0x11))), format!("got {r:?}"))
}

fn serial_queue_never_overlaps() -> Result<(), Failed> {
    use std::sync::atomic::AtomicUsize;
    let queue = SerialQueue::new("apus.test.serial", Priority::Utility);
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let (in_flight, max_seen) = (in_flight.clone(), max_seen.clone());
            queue.spawn(async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_micros(200));
                in_flight.fetch_sub(1, Ordering::SeqCst);
            })
        })
        .collect();
    for h in handles {
        block(h).unwrap();
    }
    check(max_seen.load(Ordering::SeqCst) == 1, format!("max in flight {}", max_seen.load(Ordering::SeqCst)))
}

fn block_on_off_main() -> Result<(), Failed> {
    let v = block_on(async { sleep(Duration::from_millis(5)).await; 3 });
    check(v == 3, format!("got {v}"))
}

fn run_main_off_main_panics() -> Result<(), Failed> {
    let err = std::panic::catch_unwind(|| run_main(async {})).expect_err("run_main off main must panic");
    let msg = err.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| err.downcast_ref::<String>().cloned()).unwrap_or_default();
    check(msg.contains("run_main must be called on the main thread"), format!("message: {msg}"))
}

fn abort_with_panicking_drop() -> Result<(), Failed> {
    let h = spawn(async { let _guard = common::PanicOnDrop; sleep(Duration::from_secs(10)).await; 1 });
    std::thread::sleep(Duration::from_millis(20));
    h.abort();
    let r = common::block_with_timeout(h, Duration::from_secs(2));
    check(matches!(r, Some(Err(JoinError::Panicked(_)))), format!("got {r:?}"))
}

fn sleep_wakes_latest_waker() -> Result<(), Failed> {
    check(common::latest_waker_woken(sleep(Duration::from_millis(30))), "the waker from the latest poll was never woken")
}

fn main() {
    common::run(vec![
        Trial::test("sleep_wakes_latest_waker", sleep_wakes_latest_waker),
        Trial::test("abort_with_panicking_drop", abort_with_panicking_drop),
        Trial::test("spawn_returns_value", spawn_returns_value),
        Trial::test("ten_thousand_sleeps_overlap", ten_thousand_sleeps_overlap),
        Trial::test("priority_maps_to_qos", priority_maps_to_qos),
        Trial::test("abort_yields_aborted", abort_yields_aborted),
        Trial::test("panic_becomes_join_error", panic_becomes_join_error),
        Trial::test("dropped_handle_detaches", dropped_handle_detaches),
        Trial::test("main_task_runs_on_main_thread", main_task_runs_on_main_thread),
        Trial::test("spawn_inside_main_stays_main", spawn_inside_main_stays_main),
        Trial::test("priority_inherited", priority_inherited),
        Trial::test("serial_queue_never_overlaps", serial_queue_never_overlaps),
        Trial::test("block_on_off_main", block_on_off_main),
        Trial::test("run_main_off_main_panics", run_main_off_main_panics),
    ])
}
