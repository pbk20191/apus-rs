#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use async_swift_rs::swift::{current_task, spawn, spawn_with};
use async_swift_rs::{JoinError, Priority, SpawnOptions};
use common::{block, check, Failed, Trial};
use probe::*;
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

fn task_is_swift_task() -> Result<(), Failed> {
    let r = block(spawn(async { (unsafe { probe_has_task() }, current_task().is_some()) }));
    check(matches!(r, Ok((true, true))), format!("got {r:?}"))
}

fn utility_priority_seen_by_swift() -> Result<(), Failed> {
    let options = SpawnOptions { priority: Priority::Utility, ..SpawnOptions::default() };
    let r = block(spawn_with(options, async { (unsafe { probe_priority() }, swift_child_priority()) }));
    check(matches!(r, Ok((0x11, 0x11))), format!("got {r:?}"))
}

fn spawn_returns_value() -> Result<(), Failed> {
    check(matches!(block(spawn(async { 7 })), Ok(7)), "expected Ok(7)")
}

fn panic_becomes_join_error() -> Result<(), Failed> {
    let r = block(spawn(async { panic!("boom") }));
    check(matches!(r, Err(JoinError::Panicked(_))), format!("got {r:?}"))
}

fn dropped_handle_detaches() -> Result<(), Failed> {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    drop(spawn(async move {
        let mut yielded = false;
        poll_fn(|cx| if yielded { Poll::Ready(()) } else { yielded = true; cx.waker().wake_by_ref(); Poll::Pending }).await;
        flag.store(true, Ordering::SeqCst)
    }));
    std::thread::sleep(Duration::from_millis(100));
    check(ran.load(Ordering::SeqCst), "detached task did not run")
}

fn wake_during_poll_repolls() -> Result<(), Failed> {
    let (tx, rx) = std::sync::mpsc::channel();
    let h = spawn(async move {
        let mut polls = 0;
        poll_fn(|cx| {
            polls += 1;
            if polls == 1 { cx.waker().wake_by_ref(); Poll::Pending } else { Poll::Ready(polls) }
        })
        .await
    });
    std::thread::spawn(move || { let _ = tx.send(block(h)); });
    let r = rx.recv_timeout(Duration::from_secs(1)).map_err(|_| "hung")?;
    check(matches!(r, Ok(2)), format!("got {r:?}"))
}

fn main() {
    common::run(vec![
        Trial::test("task_is_swift_task", task_is_swift_task),
        Trial::test("utility_priority_seen_by_swift", utility_priority_seen_by_swift),
        Trial::test("spawn_returns_value", spawn_returns_value),
        Trial::test("panic_becomes_join_error", panic_becomes_join_error),
        Trial::test("dropped_handle_detaches", dropped_handle_detaches),
        Trial::test("wake_during_poll_repolls", wake_during_poll_repolls),
    ])
}
