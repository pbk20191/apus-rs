//! `block_on` on the main thread would deadlock waiting for main-queue work: it must panic instead.
use apus::dispatch::{block_on, spawn_on_main};

fn main() {
    let err = std::panic::catch_unwind(|| block_on(spawn_on_main(async {}))).expect_err("block_on on main must panic");
    let msg = err.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| err.downcast_ref::<String>().cloned()).unwrap_or_default();
    assert!(msg.contains("use run_main"), "message: {msg}");
    println!("block_on_main_waiting_on_main_panics ... ok");
}
