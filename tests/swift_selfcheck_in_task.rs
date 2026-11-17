//! The first spawn may come from inside a Swift Task. The self-check must not block that cooperative-pool
//! thread waiting for a probe Task that needs a pool thread itself. Run with a one-thread pool to make it bite.
#[path = "common/probe.rs"]
mod probe;

use apus::swift::spawn;
use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

const STRICT: &str = "LIBDISPATCH_COOPERATIVE_POOL_STRICT";

extern "C" fn first_spawn_inside_swift_task(ctx: *const c_void, _priority: u8) {
    let tx = unsafe { Box::from_raw(ctx as *mut mpsc::Sender<u32>) };
    drop(spawn(async move { tx.send(7).unwrap() }));
}

fn main() {
    if std::env::var_os(STRICT).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env(STRICT, "1")
            .status()
            .unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }
    let (tx, rx) = mpsc::channel::<u32>();
    unsafe { probe::probe_spawn_swift_task(Box::into_raw(Box::new(tx)).cast(), first_spawn_inside_swift_task) };
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(7) => println!("first_spawn_inside_swift_task_with_one_pool_thread ... ok"),
        other => {
            eprintln!("first_spawn_inside_swift_task_with_one_pool_thread ... FAILED: {other:?}");
            std::process::exit(1);
        }
    }
}
