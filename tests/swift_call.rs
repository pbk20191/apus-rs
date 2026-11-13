#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use async_swift_rs::is_main_thread;
use async_swift_rs::swift::{current_task, sleep, spawn, spawn_on_main, spawn_with, timeout, CallError, JoinHandle, SwiftAsync};
use async_swift_rs::{Priority, SpawnOptions};
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use common::{block, check, Failed, Trial};
use probe::*;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

fn call(closure: *mut std::ffi::c_void) -> SwiftAsync {
    unsafe { SwiftAsync::from_raw(closure) }
}

fn same_task_identity() -> Result<(), Failed> {
    let r = block(spawn(async {
        let me = current_task().unwrap().as_ptr() as usize;
        let seen = call(unsafe { probe_closure_identity() }).await.unwrap().downcast::<i64>().unwrap() as usize;
        (me, seen)
    }))
    .unwrap();
    check(r.0 == r.1, format!("caller 0x{:x}, closure saw 0x{:x}", r.0, r.1))
}

fn result_and_error() -> Result<(), Failed> {
    let (slept, eio, void) = block(spawn(async {
        let slept = call(unsafe { probe_closure_sleep(50) }).await.map(|v| v.downcast::<String>());
        let eio = match call(unsafe { probe_closure_throw_eio() }).await {
            Err(CallError::Swift(e)) => e.description(),
            other => format!("unexpected {:?}", other.map(|v| v.type_name())),
        };
        let void = call(unsafe { probe_closure_void() }).await.map(|v| v.downcast::<()>());
        (slept, eio, void)
    }))
    .unwrap();
    check(matches!(slept, Ok(Some(ref s)) if s == "slept"), format!("sleep: {slept:?}"))?;
    check(eio.contains("io(code: 5)"), format!("error: {eio}"))?;
    check(matches!(void, Ok(Some(()))), format!("void: {void:?}"))
}

fn cancel_mid_call() -> Result<(), Failed> {
    let h = spawn(async {
        match call(unsafe { probe_closure_sleep(10_000) }).await {
            Err(CallError::Swift(e)) => e.description(),
            other => format!("unexpected {:?}", other.map(|v| v.type_name())),
        }
    });
    std::thread::sleep(Duration::from_millis(10));
    let start = Instant::now();
    h.cancel();
    let r = block(h).unwrap();
    check(r.contains("CancellationError"), format!("got {r}"))?;
    check(start.elapsed() < Duration::from_millis(100), format!("took {:?}", start.elapsed()))
}

fn no_current_task() -> Result<(), Failed> {
    let mut f = pin!(call(unsafe { probe_closure_void() }));
    let r = f.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    check(matches!(r, Poll::Ready(Err(CallError::NoCurrentTask))), format!("got {r:?}"))
}

fn dropped_swift_async_releases_context() -> Result<(), Failed> {
    let mut deinit = false;
    let c = call(unsafe { probe_closure_deinit_flag(&mut deinit) });
    let before = deinit;
    drop(c);
    check(!before && deinit, format!("before {before}, after {deinit}"))
}

fn sequential_calls_keep_task() -> Result<(), Failed> {
    let ok = block(spawn(async {
        let me = current_task().unwrap().as_ptr() as usize;
        let mut all = true;
        for _ in 0..3 {
            let seen = call(unsafe { probe_closure_identity() }).await.unwrap().downcast::<i64>().unwrap() as usize;
            all &= seen == me;
        }
        all
    }))
    .unwrap();
    check(ok, "a call ran in another task")
}

fn main_actor_kept_after_call() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let _ = call(unsafe { probe_closure_sleep(20) }).await;
        is_main_thread() && unsafe { probe_assume_main() }
    }));
    check(matches!(r, Ok(true)), format!("got {r:?}"))
}

async fn join2<A, B>(a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut ra, mut rb) = (None, None);
    poll_fn(|cx| {
        if ra.is_none() { if let Poll::Ready(v) = a.as_mut().poll(cx) { ra = Some(v) } }
        if rb.is_none() { if let Poll::Ready(v) = b.as_mut().poll(cx) { rb = Some(v) } }
        if ra.is_some() && rb.is_some() { Poll::Ready((ra.take().unwrap(), rb.take().unwrap())) } else { Poll::Pending }
    })
    .await
}

async fn race<T>(a: impl Future<Output = T>, b: impl Future<Output = T>) -> T {
    let (mut a, mut b) = (pin!(a), pin!(b));
    poll_fn(|cx| if let Poll::Ready(v) = a.as_mut().poll(cx) { Poll::Ready(v) } else { b.as_mut().poll(cx) }).await
}

fn join_runs_concurrently() -> Result<(), Failed> {
    let (a, b, took) = block(spawn(async {
        let start = Instant::now();
        let (a, b) = join2(call(unsafe { probe_closure_sleep(50) }), call(unsafe { probe_closure_sleep(50) })).await;
        (a.map(|v| v.downcast::<String>()), b.map(|v| v.downcast::<String>()), start.elapsed())
    }))
    .unwrap();
    check(matches!((&a, &b), (Ok(Some(_)), Ok(Some(_)))), format!("{a:?} {b:?}"))?;
    check(took < Duration::from_millis(90), format!("took {took:?}"))
}

fn select_timeout_cancels_child() -> Result<(), Failed> {
    let mut cancelled = false;
    let flag = &mut cancelled as *mut bool as usize;
    let (winner, took) = block(spawn(async move {
        let start = Instant::now();
        let child = call(unsafe { probe_closure_sleep_flag_cancel(10_000, flag as *mut bool) }).child();
        let winner = race(async { child.await.map(|_| "swift") .unwrap_or("swift error") }, async { sleep(Duration::from_millis(30)).await; "timeout" }).await;
        (winner, start.elapsed())
    }))
    .unwrap();
    check(winner == "timeout" && took < Duration::from_millis(100), format!("{winner} after {took:?}"))?;
    let deadline = Instant::now() + Duration::from_millis(500);
    while !unsafe { std::ptr::read_volatile(&cancelled) } && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    check(unsafe { std::ptr::read_volatile(&cancelled) }, "abandoned child never saw CancellationError")
}

fn timeout_helper() -> Result<(), Failed> {
    let (r, took) = block(spawn(async {
        let start = Instant::now();
        let r = timeout(Duration::from_millis(30), call(unsafe { probe_closure_sleep(10_000) })).await;
        (r.is_none(), start.elapsed())
    }))
    .unwrap();
    check(r && took < Duration::from_millis(100), format!("none={r} after {took:?}"))
}

fn select_without_child_runs_to_completion() -> Result<(), Failed> {
    let (winner, took) = block(spawn(async {
        let start = Instant::now();
        let winner = race(async { call(unsafe { probe_closure_sleep(200) }).await.map(|_| "swift").unwrap_or("error") },
                          async { sleep(Duration::from_millis(30)).await; "timeout" }).await;
        (winner, start.elapsed())
    }))
    .unwrap();
    check(winner == "swift" && took >= Duration::from_millis(190), format!("{winner} after {took:?}"))
}

static LOCAL_JOINER: Mutex<Option<JoinHandle<(String, String)>>> = Mutex::new(None);
extern "C" fn spawn_local_joiner() {
    let options = SpawnOptions { priority: Priority::Utility, ..SpawnOptions::default() };
    *LOCAL_JOINER.lock().unwrap() = Some(spawn_with(options, async {
        let (a, b) = join2(call(unsafe { probe_closure_read_local() }), call(unsafe { probe_closure_read_local() })).await;
        (a.unwrap().downcast::<String>().unwrap(), b.unwrap().downcast::<String>().unwrap())
    }));
}

fn child_inherits_locals_and_priority() -> Result<(), Failed> {
    block(spawn(async { unsafe { probe_with_local(c"ctx".as_ptr(), spawn_local_joiner) } })).unwrap();
    let h = LOCAL_JOINER.lock().unwrap().take().unwrap();
    let (a, b) = block(h).unwrap();
    check(a == "ctx/0x11" && b == "ctx/0x11", format!("{a} {b}"))
}

fn parent_waits_for_children() -> Result<(), Failed> {
    let start = Instant::now();
    let finished = std::sync::Arc::new(AtomicBool::new(false));
    let h = spawn(async {
        race(async { call(unsafe { probe_closure_block_ms(80) }).child().await.map(|_| 1).unwrap_or(-1) },
             async { sleep(Duration::from_millis(5)).await; 0 }).await
    });
    let r = block(h);
    finished.store(true, Ordering::SeqCst);
    check(matches!(r, Ok(0)), format!("got {r:?}"))?;
    check(start.elapsed() >= Duration::from_millis(75), format!("JoinHandle resolved after {:?}, before the 80ms child", start.elapsed()))
}

fn main() {
    common::run(vec![
        Trial::test("same_task_identity", same_task_identity),
        Trial::test("result_and_error", result_and_error),
        Trial::test("cancel_mid_call", cancel_mid_call),
        Trial::test("no_current_task", no_current_task),
        Trial::test("dropped_swift_async_releases_context", dropped_swift_async_releases_context),
        Trial::test("sequential_calls_keep_task", sequential_calls_keep_task),
        Trial::test("main_actor_kept_after_call", main_actor_kept_after_call),
        Trial::test("join_runs_concurrently", join_runs_concurrently),
        Trial::test("select_timeout_cancels_child", select_timeout_cancels_child),
        Trial::test("timeout_helper", timeout_helper),
        Trial::test("select_without_child_runs_to_completion", select_without_child_runs_to_completion),
        Trial::test("child_inherits_locals_and_priority", child_inherits_locals_and_priority),
        Trial::test("parent_waits_for_children", parent_waits_for_children),
    ])
}
