//! `run_main` on the Swift backend runs a MainActor task and drives the main run loop until it completes.
#[path = "common/probe.rs"]
mod probe;

use async_swift_rs::swift::sleep;
use async_swift_rs::{is_main_thread, run_main};
use std::time::Duration;

fn main() {
    let ok = run_main(async {
        sleep(Duration::from_millis(5)).await;
        is_main_thread() && unsafe { probe::probe_has_task() && probe::probe_assume_main() }
    });
    assert!(ok, "run_main future was not a MainActor Swift task after a suspension");
    println!("run_main_runs_swift_main_task ... ok");
}
