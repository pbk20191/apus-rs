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

    // The task finishes while a run loop callback (like an AppKit event handler running a modal loop) is inside
    // a nested run: run_main must not stop that nested run, and must still return once it ends.
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext};
    unsafe extern "C-unwind" fn run_nested(_info: *mut std::ffi::c_void) {
        CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, 0.5, false);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(5));
        eprintln!("run_main_returns_when_finished_inside_a_nested_run_loop ... FAILED: hung");
        std::process::exit(2);
    });
    let mut context = CFRunLoopSourceContext {
        version: 0, info: std::ptr::null_mut(), retain: None, release: None, copyDescription: None,
        equal: None, hash: None, schedule: None, cancel: None, perform: Some(run_nested),
    };
    let nested = unsafe { CFRunLoopSource::new(None, 0, &mut context) }.unwrap();
    CFRunLoop::main().unwrap().add_source(Some(&nested), unsafe { kCFRunLoopDefaultMode });
    nested.signal(); // fires once run_main starts the run loop
    let started = std::time::Instant::now();
    let value = run_main(async {
        sleep(Duration::from_millis(50)).await;
        7
    });
    nested.invalidate();
    assert_eq!(value, 7);
    // The nested run is someone else's: it must run its full 0.5 s, not be cut short by our stop.
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(450) && took < Duration::from_secs(2), "took {took:?}");
    println!("run_main_returns_when_finished_inside_a_nested_run_loop ... ok");
}
