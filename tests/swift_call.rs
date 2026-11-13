#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use async_swift_rs::is_main_thread;
use async_swift_rs::swift::{current_task, spawn, spawn_on_main, CallError, SwiftAsync};
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

fn main() {
    common::run(vec![
        Trial::test("same_task_identity", same_task_identity),
        Trial::test("result_and_error", result_and_error),
        Trial::test("cancel_mid_call", cancel_mid_call),
        Trial::test("no_current_task", no_current_task),
        Trial::test("dropped_swift_async_releases_context", dropped_swift_async_releases_context),
        Trial::test("sequential_calls_keep_task", sequential_calls_keep_task),
        Trial::test("main_actor_kept_after_call", main_actor_kept_after_call),
    ])
}
