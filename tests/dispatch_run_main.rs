//! `run_main` drives the main run loop until its future completes and returns the output.
use apus::dispatch::sleep;
use apus::{is_main_thread, run_main};
use std::time::Duration;

fn main() {
    let (before, after) = run_main(async {
        let before = is_main_thread();
        sleep(Duration::from_millis(5)).await;
        (before, is_main_thread())
    });
    assert!(before && after, "run_main future left the main thread: {before} {after}");
    assert_eq!(run_main(async { 41 + 1 }), 42, "run_main is reusable");
    println!("run_main_returns_on_main ... ok");
}
