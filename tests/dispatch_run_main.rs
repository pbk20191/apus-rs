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

    // Inside a main task the main queue is busy with that task, so a nested run_main could never run its own.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(5));
        eprintln!("nested_run_main_panics ... FAILED: hung");
        std::process::exit(2);
    });
    let refused = run_main(async { std::panic::catch_unwind(|| run_main(async { 1 })).is_err() });
    assert!(refused, "nested run_main returned");
    // The same from a dispatch main-queue task, whichever runtime run_main itself uses.
    let refused = run_main(async {
        apus::dispatch::spawn_on_main(async { std::panic::catch_unwind(|| run_main(async { 1 })).is_err() }).await.unwrap()
    });
    assert!(refused, "nested run_main in a dispatch main task returned");
    println!("nested_run_main_panics ... ok");
}
