#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use async_swift_rs::is_main_thread;
use async_swift_rs::swift::{cancelled, current_task, priority, sleep, spawn, spawn_detached, spawn_on_main, spawn_with, JoinHandle};
use std::sync::Mutex;
use std::task::Waker;
use std::time::Instant;
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

/// MainActor isolation, checked without trapping: assumeIsolated only runs once we know we are on main.
fn on_main_actor() -> bool {
    is_main_thread() && unsafe { probe_assume_main() }
}

fn main_task_is_main_actor() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let before = unsafe { probe_has_task() } && on_main_actor();
        sleep(Duration::from_millis(10)).await;
        (before, unsafe { probe_has_task() } && on_main_actor())
    }));
    check(matches!(r, Ok((true, true))), format!("got {r:?}"))
}

fn spawn_in_main_inherits_main() -> Result<(), Failed> {
    let r = block(spawn_on_main(async { spawn(async { on_main_actor() }).await }));
    check(matches!(r, Ok(Ok(true))), format!("got {r:?}"))
}

fn detached_inherits_nothing() -> Result<(), Failed> {
    let options = SpawnOptions { priority: Priority::Utility, ..SpawnOptions::default() };
    let r = block(spawn_with(options, async {
        spawn_detached(async { (unsafe { probe_priority() }, is_main_thread()) }).await
    }));
    check(matches!(r, Ok(Ok((0x15, false)))), format!("got {r:?}"))
}

fn sleeps_overlap() -> Result<(), Failed> {
    let start = Instant::now();
    let handles: Vec<_> = (0..10_000u64)
        .map(|i| spawn(async move { sleep(Duration::from_millis(50)).await; i }))
        .collect();
    let sum: u64 = handles.into_iter().map(|h| block(h).unwrap()).sum();
    check(sum == 49_995_000, format!("sum {sum}"))?;
    check(start.elapsed() < Duration::from_millis(500), format!("took {:?}", start.elapsed()))
}

async fn race<T>(a: impl std::future::Future<Output = T>, b: impl std::future::Future<Output = T>) -> T {
    let (mut a, mut b) = (std::pin::pin!(a), std::pin::pin!(b));
    poll_fn(|cx| if let Poll::Ready(v) = a.as_mut().poll(cx) { Poll::Ready(v) } else { b.as_mut().poll(cx) }).await
}

fn cancel_wakes_cancelled_future() -> Result<(), Failed> {
    let h = spawn(async {
        let start = Instant::now();
        let winner = race(async { cancelled().await; "cancelled" }, async { sleep(Duration::from_secs(10)).await; "slept" }).await;
        (winner, unsafe { probe_is_cancelled() }, start.elapsed())
    });
    std::thread::sleep(Duration::from_millis(30));
    h.cancel();
    let r = block(h);
    match r {
        Ok(("cancelled", true, took)) => check(took < Duration::from_millis(130), format!("took {took:?}")),
        other => Err(format!("got {other:?}").into()),
    }
}

fn abort_drops_future() -> Result<(), Failed> {
    struct SetOnDrop(Arc<AtomicBool>);
    impl Drop for SetOnDrop {
        fn drop(&mut self) { self.0.store(true, Ordering::SeqCst) }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = SetOnDrop(dropped.clone());
    let h = spawn(async move { let _g = guard; sleep(Duration::from_secs(10)).await; 1 });
    std::thread::sleep(Duration::from_millis(10));
    let start = Instant::now();
    h.abort();
    let r = block(h);
    check(matches!(r, Err(JoinError::Aborted)), format!("got {r:?}"))?;
    check(start.elapsed() < Duration::from_millis(100), format!("took {:?}", start.elapsed()))?;
    check(dropped.load(Ordering::SeqCst), "future was not dropped")
}

fn escalation_visible_after_resume() -> Result<(), Failed> {
    let options = SpawnOptions { priority: Priority::Utility, ..SpawnOptions::default() };
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let h = spawn_with(options, async move {
        let before = unsafe { probe_priority() };
        started_tx.send(()).unwrap();
        sleep(Duration::from_millis(40)).await;
        (before, unsafe { probe_priority() }, priority())
    });
    // Escalate only once the task has run and read its starting priority (under load it may start late).
    started_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "task never started")?;
    h.escalate(Priority::UserInitiated);
    let r = block(h);
    check(matches!(r, Ok((0x11, 0x19, Priority::UserInitiated))), format!("got {r:?}"))
}

static LOCAL_CHILDREN: Mutex<Vec<JoinHandle<String>>> = Mutex::new(Vec::new());
extern "C" fn spawn_local_readers() {
    let mut children = LOCAL_CHILDREN.lock().unwrap();
    children.push(spawn(async { read_local() }));
    children.push(spawn_with(SpawnOptions { copy_task_locals: false, ..SpawnOptions::default() }, async { read_local() }));
}

fn task_locals_copied() -> Result<(), Failed> {
    block(spawn(async { unsafe { probe_with_local(c"hello".as_ptr(), spawn_local_readers) } })).unwrap();
    let children: Vec<_> = LOCAL_CHILDREN.lock().unwrap().drain(..).collect();
    let values: Vec<String> = children.into_iter().map(|h| block(h).unwrap()).collect();
    check(values == ["hello", "<none>"], format!("got {values:?}"))
}

fn late_wake_after_done_is_noop() -> Result<(), Failed> {
    for _ in 0..1000 {
        let stash: Arc<Mutex<Option<Waker>>> = Arc::default();
        let s = stash.clone();
        block(spawn(poll_fn(move |cx| { *s.lock().unwrap() = Some(cx.waker().clone()); Poll::Ready(()) }))).unwrap();
        let waker = stash.lock().unwrap().take().unwrap();
        waker.wake_by_ref();
        waker.wake();
    }
    Ok(())
}

fn stress_spawn_wake_cancel() -> Result<(), Failed> {
    let handles: Vec<_> = (0..20_000u32)
        .map(|i| {
            let h = spawn(async move {
                match i % 4 {
                    0 => sleep(Duration::from_micros((i % 2000) as u64)).await,
                    1 => {
                        let mut n = 0;
                        poll_fn(|cx| { n += 1; if n > 3 { Poll::Ready(()) } else { cx.waker().wake_by_ref(); Poll::Pending } }).await
                    }
                    2 => { race(cancelled(), sleep(Duration::from_secs(5))).await }
                    _ => {}
                }
                i
            });
            if i % 4 == 2 { h.cancel() }
            h
        })
        .collect();
    let start = Instant::now();
    for h in handles {
        block(h).map_err(|e| format!("join error {e:?}"))?;
    }
    check(start.elapsed() < Duration::from_secs(5), format!("took {:?}", start.elapsed()))
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
        Trial::test("task_is_swift_task", task_is_swift_task),
        Trial::test("utility_priority_seen_by_swift", utility_priority_seen_by_swift),
        Trial::test("spawn_returns_value", spawn_returns_value),
        Trial::test("panic_becomes_join_error", panic_becomes_join_error),
        Trial::test("dropped_handle_detaches", dropped_handle_detaches),
        Trial::test("wake_during_poll_repolls", wake_during_poll_repolls),
        Trial::test("main_task_is_main_actor", main_task_is_main_actor),
        Trial::test("spawn_in_main_inherits_main", spawn_in_main_inherits_main),
        Trial::test("detached_inherits_nothing", detached_inherits_nothing),
        Trial::test("sleeps_overlap", sleeps_overlap),
        Trial::test("cancel_wakes_cancelled_future", cancel_wakes_cancelled_future),
        Trial::test("abort_drops_future", abort_drops_future),
        Trial::test("escalation_visible_after_resume", escalation_visible_after_resume),
        Trial::test("task_locals_copied", task_locals_copied),
        Trial::test("late_wake_after_done_is_noop", late_wake_after_done_is_noop),
        Trial::test("stress_spawn_wake_cancel", stress_spawn_wake_cancel),
    ])
}
